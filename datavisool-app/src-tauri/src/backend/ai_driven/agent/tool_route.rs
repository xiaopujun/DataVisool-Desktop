/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 模型可见 Tool 的宿主路由状态机。
//!
//! 计划是进度信息；工具是否可见由宿主能力和当前操作事实决定。

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

use crate::backend::ai_driven::agent_context::input::AgentRunInput;
use crate::backend::ai_driven::domain::operation::{CanvasOperationReceipt, CanvasOperationStatus};
use crate::backend::ai_driven::security::{AgentOperation, AgentSecurity};
use crate::backend::ai_driven::tools::canvas::{
    CREATE_CANVAS_COMPONENTS_TOOL, DELETE_CANVAS_COMPONENTS_TOOL, GET_CANVAS_OPERATION_STATUS_TOOL, GET_CURRENT_CANVAS_SELECTION_TOOL,
    PATCH_CANVAS_COMPONENTS_TOOL,
};
use crate::backend::ai_driven::tools::component_catalog::GET_COMPONENT_CATALOG_TOOL;
use crate::backend::ai_driven::tools::component_data_contract::GET_COMPONENT_DATA_CONTRACT_TOOL;
use crate::backend::ai_driven::tools::component_resolution::RESOLVE_COMPONENT_SPECS_TOOL;
use crate::backend::ai_driven::tools::file::{
    APPLY_WORKSPACE_PATCH_TOOL, DELETE_WORKSPACE_PATH_TOOL, LIST_WORKSPACE_FILES_TOOL, MOVE_WORKSPACE_PATH_TOOL, READ_WORKSPACE_FILE_TOOL,
    SEARCH_WORKSPACE_FILES_TOOL,
};
use crate::backend::ai_driven::tools::interaction::{INSPECT_CURRENT_PAGE_TOOL, REQUEST_USER_INPUT_TOOL};
use crate::backend::ai_driven::tools::selection::REQUEST_COMPONENT_SELECTION_TOOL;
use crate::backend::ai_driven::tools::task_intent::UPDATE_PLAN_TOOL;

use super::context_plan::AgentContextPlan;

#[derive(Clone)]
pub(crate) struct AgentToolRoute {
    state: Arc<Mutex<AgentToolRouteState>>,
}

struct AgentToolRouteState {
    input: AgentRunInput,
    workspace_tools_enabled: bool,
    fixed_active_tools: Option<Vec<String>>,
    phase: AgentRoutePhase,
    security: Option<Arc<AgentSecurity>>,
    applying_operation_id: Option<String>,
    pending_operation_ids: BTreeSet<String>,
    route_revision: u64,
    snapshot_sequence: u64,
    active_snapshot: Option<AgentToolRouteSnapshot>,
}

/// 单次模型步骤使用的不可变 Tool 路由事实。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentToolRouteSnapshot {
    pub step_index: u64,
    pub route_revision: u64,
    pub phase: AgentRoutePhase,
    pub allowed_tools: BTreeSet<String>,
    applying_operation_id: Option<String>,
    generation: u64,
    context_plan: AgentContextPlan,
}

/// 模型可见 Tool 的宿主事实阶段。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentRoutePhase {
    Ready,
    Executing,
    Applying,
    Verifying,
    Completed,
    Failed,
}

impl AgentRoutePhase {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Executing => "executing",
            Self::Applying => "applying",
            Self::Verifying => "verifying",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

impl AgentToolRouteSnapshot {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn context_plan(&self) -> &AgentContextPlan {
        &self.context_plan
    }

    /// 快照只校验模型生成调用时是否暴露该 Tool；资源前置条件由 Tool 实时检查。
    pub(crate) fn allows_tool_call(&self, tool_name: &str, args: &str) -> bool {
        if !self.allowed_tools.contains(tool_name) {
            return false;
        }
        self.phase != AgentRoutePhase::Applying
            || (tool_name == GET_CANVAS_OPERATION_STATUS_TOOL
                && self
                    .applying_operation_id
                    .as_deref()
                    .is_some_and(|expected| operation_id(args).is_some_and(|value| value == expected)))
    }
}

impl AgentToolRoute {
    pub(crate) fn new(input: AgentRunInput, workspace_tools_enabled: bool) -> Self {
        Self {
            state: Arc::new(Mutex::new(AgentToolRouteState {
                input,
                workspace_tools_enabled,
                fixed_active_tools: None,
                phase: AgentRoutePhase::Ready,
                security: None,
                applying_operation_id: None,
                pending_operation_ids: BTreeSet::new(),
                route_revision: 0,
                snapshot_sequence: 0,
                active_snapshot: None,
            })),
        }
    }

    #[cfg(test)]
    pub(crate) fn fixed(active_tools: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(AgentToolRouteState {
                input: AgentRunInput::default(),
                workspace_tools_enabled: false,
                fixed_active_tools: Some(active_tools.into_iter().map(Into::into).collect()),
                phase: AgentRoutePhase::Ready,
                security: None,
                applying_operation_id: None,
                pending_operation_ids: BTreeSet::new(),
                route_revision: 0,
                snapshot_sequence: 0,
                active_snapshot: None,
            })),
        }
    }

    pub(crate) fn snapshot_for_step(&self, step_index: u64) -> AgentToolRouteSnapshot {
        let mut state = self.lock();
        let context_plan = context_plan_for_state(&state);
        state.snapshot_sequence = state.snapshot_sequence.saturating_add(1);
        let snapshot = AgentToolRouteSnapshot {
            step_index,
            route_revision: state.route_revision,
            phase: state.phase,
            allowed_tools: context_plan.active_tools().into_iter().collect(),
            applying_operation_id: state.applying_operation_id.clone(),
            generation: state.snapshot_sequence,
            context_plan,
        };
        state.active_snapshot = Some(snapshot.clone());
        snapshot
    }

    pub(crate) fn active_snapshot(&self) -> Option<AgentToolRouteSnapshot> {
        self.lock().active_snapshot.clone()
    }

    pub(crate) fn current_plan(&self) -> AgentContextPlan {
        context_plan_for_state(&self.lock())
    }

    pub(crate) fn phase(&self) -> AgentRoutePhase {
        self.lock().phase
    }

    /// Bind the authoritative host capability state before a model step starts.
    /// The initial route never derives authorization from model output.
    pub(crate) fn bind_security(&self, security: Arc<AgentSecurity>) {
        let mut state = self.lock();
        state.security = Some(security);
        state.route_revision = state.route_revision.saturating_add(1);
    }

    pub(crate) fn authorization_revision(&self) -> u64 {
        self.lock()
            .security
            .as_ref()
            .map_or(0, |security| security.authorization_revision())
    }

    /// 由画布动作或状态查询的真实回执推进路由。
    pub(crate) fn observe_operation_result(&self, operation: &CanvasOperationReceipt) {
        let mut state = self.lock();
        if state.fixed_active_tools.is_some() {
            return;
        }
        let before = (state.phase, state.applying_operation_id.clone(), state.pending_operation_ids.clone());
        match operation.status {
            CanvasOperationStatus::Prepared | CanvasOperationStatus::Applying => {
                state.pending_operation_ids.insert(operation.operation_id.clone());
                state.applying_operation_id = Some(operation.operation_id.clone());
                state.phase = AgentRoutePhase::Applying;
            }
            CanvasOperationStatus::Applied => {
                state.pending_operation_ids.remove(&operation.operation_id);
                if state.applying_operation_id.as_deref() == Some(operation.operation_id.as_str()) {
                    state.applying_operation_id = state.pending_operation_ids.iter().next().cloned();
                }
                state.phase = if state.pending_operation_ids.is_empty() {
                    AgentRoutePhase::Executing
                } else {
                    AgentRoutePhase::Applying
                };
            }
            CanvasOperationStatus::Rejected | CanvasOperationStatus::Failed => {
                state.pending_operation_ids.remove(&operation.operation_id);
                if state.applying_operation_id.as_deref() == Some(operation.operation_id.as_str()) {
                    state.applying_operation_id = state.pending_operation_ids.iter().next().cloned();
                }
                state.phase = if state.pending_operation_ids.is_empty() {
                    AgentRoutePhase::Executing
                } else {
                    AgentRoutePhase::Applying
                };
            }
        }
        if before != (state.phase, state.applying_operation_id.clone(), state.pending_operation_ids.clone()) {
            state.route_revision = state.route_revision.saturating_add(1);
        }
    }

    #[cfg(test)]
    pub(crate) fn allows_tool_call(&self, tool_name: &str, args: &str) -> bool {
        let state = self.lock();
        if let Some(fixed_active_tools) = &state.fixed_active_tools {
            return fixed_active_tools.iter().any(|tool| tool == tool_name);
        }
        match state.phase {
            AgentRoutePhase::Ready | AgentRoutePhase::Executing => active_tools(&state).contains(tool_name),
            AgentRoutePhase::Applying => {
                tool_name == GET_CANVAS_OPERATION_STATUS_TOOL
                    && state
                        .applying_operation_id
                        .as_deref()
                        .is_some_and(|expected| operation_id(args).is_some_and(|value| value == expected))
            }
            AgentRoutePhase::Verifying => active_tools(&state).contains(tool_name),
            AgentRoutePhase::Completed | AgentRoutePhase::Failed => false,
        }
    }

    fn lock(&self) -> MutexGuard<'_, AgentToolRouteState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn context_plan_for_state(state: &AgentToolRouteState) -> AgentContextPlan {
    let active_tools = if let Some(active_tools) = &state.fixed_active_tools {
        active_tools.clone()
    } else {
        active_tools(state).into_iter().map(str::to_string).collect()
    };
    AgentContextPlan::for_route(&state.input, state.workspace_tools_enabled, state.phase, active_tools)
}

fn active_tools(state: &AgentToolRouteState) -> BTreeSet<&'static str> {
    match state.phase {
        AgentRoutePhase::Ready | AgentRoutePhase::Executing | AgentRoutePhase::Verifying => ready_tools(state),
        AgentRoutePhase::Applying => BTreeSet::from([GET_CANVAS_OPERATION_STATUS_TOOL]),
        AgentRoutePhase::Completed | AgentRoutePhase::Failed => BTreeSet::new(),
    }
}

fn ready_tools(state: &AgentToolRouteState) -> BTreeSet<&'static str> {
    let mut tools = BTreeSet::from([
        REQUEST_USER_INPUT_TOOL,
        UPDATE_PLAN_TOOL,
        INSPECT_CURRENT_PAGE_TOOL,
        GET_COMPONENT_CATALOG_TOOL,
        RESOLVE_COMPONENT_SPECS_TOOL,
        GET_COMPONENT_DATA_CONTRACT_TOOL,
        GET_CURRENT_CANVAS_SELECTION_TOOL,
        REQUEST_COMPONENT_SELECTION_TOOL,
        GET_CANVAS_OPERATION_STATUS_TOOL,
    ]);
    if operation_allowed(state, AgentOperation::CanvasCreate) {
        tools.insert(CREATE_CANVAS_COMPONENTS_TOOL);
    }
    if operation_allowed(state, AgentOperation::CanvasPatch) {
        tools.insert(PATCH_CANVAS_COMPONENTS_TOOL);
    }
    if operation_allowed(state, AgentOperation::CanvasDelete) {
        tools.insert(DELETE_CANVAS_COMPONENTS_TOOL);
    }
    if state.workspace_tools_enabled {
        tools.extend([LIST_WORKSPACE_FILES_TOOL, SEARCH_WORKSPACE_FILES_TOOL, READ_WORKSPACE_FILE_TOOL]);
        if operation_allowed(state, AgentOperation::WorkspaceWrite) {
            tools.extend([APPLY_WORKSPACE_PATCH_TOOL, MOVE_WORKSPACE_PATH_TOOL, DELETE_WORKSPACE_PATH_TOOL]);
        }
    }
    tools
}

fn operation_allowed(state: &AgentToolRouteState, operation: AgentOperation) -> bool {
    state
        .security
        .as_ref()
        .map_or(true, |security| security.grant().authorize_operation(operation).is_ok())
}

fn operation_id(args: &str) -> Option<String> {
    serde_json::from_str::<Value>(args)
        .ok()?
        .get("operationId")?
        .as_str()
        .map(str::trim)
        .filter(|operation_id| !operation_id.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::backend::ai_driven::domain::operation::{CanvasOperationKind, CanvasOperationReceipt};
    use crate::backend::ai_driven::tools::canvas::{CREATE_CANVAS_COMPONENTS_TOOL, DELETE_CANVAS_COMPONENTS_TOOL, PATCH_CANVAS_COMPONENTS_TOOL};

    fn operation(id: &str, status: CanvasOperationStatus) -> CanvasOperationReceipt {
        CanvasOperationReceipt {
            operation_id: id.to_string(),
            tool_call_id: format!("call-{id}"),
            kind: CanvasOperationKind::PatchComponents,
            status,
            project_id: "1".to_string(),
            manager_id: None,
            component_ids: vec!["c1".to_string()],
            component_bindings: BTreeMap::new(),
            component_fields: BTreeMap::new(),
            configured_component_count: 0,
            default_config_component_count: 0,
            base_revision: None,
            result_revision: None,
            applied_at: None,
            error: None,
        }
    }

    #[test]
    fn ready_exposes_host_capability_tools_without_a_model_declaration() {
        let route = AgentToolRoute::new(AgentRunInput::new("解释页面"), true);

        assert_eq!(route.phase(), AgentRoutePhase::Ready);
        assert!(route.allows_tool_call(INSPECT_CURRENT_PAGE_TOOL, "{}"));
        assert!(route.allows_tool_call(GET_COMPONENT_CATALOG_TOOL, "{}"));
        assert!(route.allows_tool_call(RESOLVE_COMPONENT_SPECS_TOOL, "{}"));
        assert!(!route.allows_tool_call("get_component_document", "{}"));
        assert!(!route.allows_tool_call("search_component_documents", "{}"));
        assert!(!route.allows_tool_call("search_available_components", "{}"));
        assert!(route.allows_tool_call(READ_WORKSPACE_FILE_TOOL, "{}"));
        assert!(route.allows_tool_call(CREATE_CANVAS_COMPONENTS_TOOL, "{}"));
        assert!(route.allows_tool_call(PATCH_CANVAS_COMPONENTS_TOOL, "{}"));
        assert!(route.allows_tool_call(DELETE_CANVAS_COMPONENTS_TOOL, "{}"));
        assert!(route.allows_tool_call(APPLY_WORKSPACE_PATCH_TOOL, "{}"));
    }

    #[test]
    fn applying_only_allows_its_operation_status_query() {
        let route = AgentToolRoute::new(AgentRunInput::new("修改标题"), false);
        route.observe_operation_result(&operation("op-1", CanvasOperationStatus::Applying));

        assert_eq!(route.phase(), AgentRoutePhase::Applying);
        assert!(route.allows_tool_call(GET_CANVAS_OPERATION_STATUS_TOOL, r#"{"operationId":"op-1"}"#));
        assert!(!route.allows_tool_call(GET_CANVAS_OPERATION_STATUS_TOOL, r#"{"operationId":"other"}"#));
        assert!(!route.allows_tool_call(PATCH_CANVAS_COMPONENTS_TOOL, "{}"));
        assert!(!route.allows_tool_call(UPDATE_PLAN_TOOL, "{}"));

        let snapshot = route.snapshot_for_step(1);
        assert!(snapshot.allows_tool_call(GET_CANVAS_OPERATION_STATUS_TOOL, r#"{"operationId":"op-1"}"#));
        assert!(!snapshot.allows_tool_call(GET_CANVAS_OPERATION_STATUS_TOOL, r#"{"operationId":"other"}"#));
    }

    #[test]
    fn applied_operation_returns_to_executing_and_keeps_writes_available() {
        let route = AgentToolRoute::new(AgentRunInput::new("修改标题"), false);
        route.observe_operation_result(&operation("op-1", CanvasOperationStatus::Applied));

        assert_eq!(route.phase(), AgentRoutePhase::Executing);
        assert!(route.allows_tool_call(PATCH_CANVAS_COMPONENTS_TOOL, "{}"));
        assert!(route.allows_tool_call(DELETE_CANVAS_COMPONENTS_TOOL, "{}"));
    }

    #[test]
    fn applied_does_not_enter_a_declaration_only_verifying_phase() {
        let route = AgentToolRoute::new(AgentRunInput::new("修改标题"), true);
        route.observe_operation_result(&operation("op-1", CanvasOperationStatus::Applied));

        assert_eq!(route.phase(), AgentRoutePhase::Executing);
        assert!(route.allows_tool_call(INSPECT_CURRENT_PAGE_TOOL, "{}"));
        assert!(route.allows_tool_call(REQUEST_USER_INPUT_TOOL, "{}"));
        assert!(route.allows_tool_call(UPDATE_PLAN_TOOL, "{}"));
        assert!(route.allows_tool_call(PATCH_CANVAS_COMPONENTS_TOOL, "{}"));
        assert!(route.allows_tool_call(GET_COMPONENT_DATA_CONTRACT_TOOL, "{}"));
        assert!(route.allows_tool_call(APPLY_WORKSPACE_PATCH_TOOL, "{}"));
        assert!(route.allows_tool_call(GET_COMPONENT_CATALOG_TOOL, "{}"));
        assert!(route.allows_tool_call(RESOLVE_COMPONENT_SPECS_TOOL, "{}"));
    }

    #[test]
    fn applying_blocks_unrelated_tools_until_the_matching_operation_is_terminal() {
        let route = AgentToolRoute::new(AgentRunInput::new("修改并删除组件"), false);
        route.observe_operation_result(&operation("op-1", CanvasOperationStatus::Applying));
        assert!(!route.allows_tool_call(PATCH_CANVAS_COMPONENTS_TOOL, "{}"));
        assert!(!route.allows_tool_call(GET_COMPONENT_CATALOG_TOOL, "{}"));
        route.observe_operation_result(&operation("op-1", CanvasOperationStatus::Applied));
        assert!(route.allows_tool_call(DELETE_CANVAS_COMPONENTS_TOOL, "{}"));
    }

    #[test]
    fn terminal_receipt_does_not_clear_another_pending_operation() {
        let route = AgentToolRoute::new(AgentRunInput::new("分批修改组件"), false);
        route.observe_operation_result(&operation("op-1", CanvasOperationStatus::Applying));
        route.observe_operation_result(&operation("op-2", CanvasOperationStatus::Applying));
        route.observe_operation_result(&operation("op-1", CanvasOperationStatus::Applied));

        assert_eq!(route.phase(), AgentRoutePhase::Applying);
        let snapshot = route.snapshot_for_step(2);
        assert!(snapshot.allows_tool_call(GET_CANVAS_OPERATION_STATUS_TOOL, r#"{"operationId":"op-2"}"#));
        assert!(!snapshot.allows_tool_call(GET_CANVAS_OPERATION_STATUS_TOOL, r#"{"operationId":"op-1"}"#));

        route.observe_operation_result(&operation("op-2", CanvasOperationStatus::Applied));
        assert_eq!(route.phase(), AgentRoutePhase::Executing);
        assert!(route.allows_tool_call(PATCH_CANVAS_COMPONENTS_TOOL, "{}"));
    }

    #[test]
    fn bound_host_capability_filters_write_tools_without_model_state() {
        let scope = crate::backend::ai_driven::tools::context::AgentToolScope::new("task", "run", "thread", "1").unwrap();
        // The grant is immutable through AgentSecurity; build a restricted grant
        // before binding it to verify route visibility follows host capability.
        let mut raw_grant = crate::backend::ai_driven::security::AgentCapabilityGrant::for_task(&scope, [], "workspace");
        raw_grant.allowed_operations.remove(&AgentOperation::CanvasPatch);
        raw_grant.allowed_operations.remove(&AgentOperation::WorkspaceWrite);
        let security = Arc::new(AgentSecurity::new(raw_grant));
        let route = AgentToolRoute::new(AgentRunInput::new("查看页面"), true);
        route.bind_security(security);

        assert!(route.allows_tool_call(CREATE_CANVAS_COMPONENTS_TOOL, "{}"));
        assert!(!route.allows_tool_call(PATCH_CANVAS_COMPONENTS_TOOL, "{}"));
        assert!(!route.allows_tool_call(APPLY_WORKSPACE_PATCH_TOOL, "{}"));
        assert_eq!(route.authorization_revision(), 0);
    }
}
