/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 所有 Rig Tool 共享的最小运行依赖。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use crate::backend::ai_driven::ai_task_execution::workspace::{
    WorkspaceFileLimits, WorkspaceFileService, WorkspaceFileTrace, WorkspaceHandle, WorkspaceSearchService,
};
use crate::backend::ai_driven::domain::validation::AgentArtifactValidationContext;
use crate::backend::ai_driven::domain::validation_evidence::AgentValidationEvidence;
use crate::backend::ai_driven::ripgrep::RipgrepSearchService;
use crate::backend::ai_driven::task::{CanvasOperationJournal, CompletionRequirementsState};
use crate::backend::{FrontendEffectApi, FrontendEffectOptions, FrontendEffectRegistry};

use super::context::AgentToolContext;
use super::error::AgentToolError;

mod invocation;

pub(crate) use invocation::{AgentToolInvocation, AgentToolInvocationTracker};

/// 可克隆的前端 Effect 调用句柄。
#[derive(Clone)]
pub(crate) struct AgentFrontendRuntime {
    app_handle: tauri::AppHandle,
    registry: FrontendEffectRegistry,
    canvas_baselines: CanvasSnapshotBaselines,
}

impl AgentFrontendRuntime {
    pub(crate) fn new(app_handle: tauri::AppHandle, registry: FrontendEffectRegistry) -> Self {
        Self { app_handle, registry, canvas_baselines: CanvasSnapshotBaselines::default() }
    }

    pub(crate) async fn call<TReq, TResp>(
        &self,
        tool_name: &str,
        request: TReq,
        timeout: Option<std::time::Duration>,
        context: &AgentToolContext,
    ) -> Result<TResp, AgentToolError>
    where
        TReq: Serialize + Send,
        TResp: DeserializeOwned + Send,
    {
        let request = scoped_frontend_request(request, context)?;
        FrontendEffectApi::new(&self.registry)
            .call_tool_with_cancel(&self.app_handle, tool_name, request, FrontendEffectOptions { timeout }, Some(context.cancel_receiver()))
            .await
            .map_err(Into::into)
    }

    /// 真实写入发送后不响应 Turn 取消，避免前端已落地而后端丢失回执。
    pub(crate) async fn call_irreversible<TReq, TResp>(
        &self,
        tool_name: &str,
        request: TReq,
        timeout: Option<std::time::Duration>,
        context: &AgentToolContext,
    ) -> Result<TResp, AgentToolError>
    where
        TReq: Serialize + Send,
        TResp: DeserializeOwned + Send,
    {
        let request = scoped_frontend_request(request, context)?;
        FrontendEffectApi::new(&self.registry)
            .call_tool_with_options(&self.app_handle, tool_name, request, FrontendEffectOptions { timeout })
            .await
            .map_err(Into::into)
    }

    /// 查询工具返回的快照只作为下一次动作的并发基线。
    pub(crate) fn record_canvas_snapshot_response(&self, response: &Value) -> Result<(), AgentToolError> {
        self.canvas_baselines.record_response(response)
    }

    pub(crate) fn expected_component_patch_snapshot(
        &self,
        component_id: &str,
        component_type: &str,
        include_style: bool,
        include_data: bool,
        include_layer: bool,
        include_filter: bool,
    ) -> Result<ExpectedComponentSnapshot, AgentToolError> {
        let baseline = self.canvas_baselines.require(component_id, component_type, "config")?;
        let style = include_style
            .then(|| baseline.style.ok_or_else(|| missing_baseline(component_id, "style 或 config")))
            .transpose()?;
        let data = include_data
            .then(|| baseline.data.ok_or_else(|| missing_baseline(component_id, "data 或 config")))
            .transpose()?;
        let layer = include_layer
            .then(|| baseline.layer.ok_or_else(|| missing_baseline(component_id, "layer")))
            .transpose()?;
        let filter = include_filter
            .then(|| {
                baseline
                    .filter
                    .ok_or_else(|| missing_baseline(component_id, "filter 或 config"))
            })
            .transpose()?;
        Ok(ExpectedComponentSnapshot {
            component_id: component_id.to_string(),
            component_type: component_type.to_string(),
            layer,
            style,
            data,
            filter,
        })
    }

    pub(crate) fn expected_component_layer_snapshot(
        &self,
        component_id: &str,
        component_type: &str,
    ) -> Result<ExpectedComponentSnapshot, AgentToolError> {
        let baseline = self.canvas_baselines.require(component_id, component_type, "layer")?;
        let layer = baseline.layer.ok_or_else(|| missing_baseline(component_id, "layer"))?;
        Ok(ExpectedComponentSnapshot {
            component_id: component_id.to_string(),
            component_type: component_type.to_string(),
            layer: Some(layer),
            style: None,
            data: None,
            filter: None,
        })
    }
}

fn scoped_frontend_request<T: Serialize>(request: T, context: &AgentToolContext) -> Result<Value, AgentToolError> {
    let mut request =
        serde_json::to_value(request).map_err(|error| AgentToolError::internal("agent.frontend.request_serialize_failed", error.to_string()))?;
    let object = request
        .as_object_mut()
        .ok_or_else(|| AgentToolError::internal("agent.frontend.request_invalid", "前端 Tool 请求必须序列化为对象"))?;
    let scope = context.scope();
    object.insert(
        "agentScope".to_string(),
        json!({
            "taskId": scope.task_id,
            "runId": scope.run_id,
            "conversationId": scope.conversation_id,
            "projectId": scope.project_id,
            "managerId": scope.manager_id,
            "turnId": scope.turn_id,
        }),
    );
    Ok(request)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExpectedComponentSnapshot {
    component_id: String,
    component_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    layer: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    style: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<Value>,
}

#[derive(Clone, Debug)]
struct CanvasSnapshotBaseline {
    component_type: String,
    layer: Option<Value>,
    style: Option<Value>,
    data: Option<Value>,
    filter: Option<Value>,
}

#[derive(Clone, Default)]
struct CanvasSnapshotBaselines {
    components: Arc<Mutex<HashMap<String, CanvasSnapshotBaseline>>>,
}

impl CanvasSnapshotBaselines {
    fn record_response(&self, response: &Value) -> Result<(), AgentToolError> {
        let components = response
            .get("components")
            .and_then(Value::as_array)
            .ok_or_else(|| AgentToolError::validation("agent.snapshot.invalid_response", "组件快照返回值缺少 components 数组"))?;
        let mut baselines = self.components.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for component in components {
            let object = component
                .as_object()
                .ok_or_else(|| AgentToolError::validation("agent.snapshot.invalid_response", "组件快照返回了无效记录"))?;
            let component_id = object
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| AgentToolError::validation("agent.snapshot.invalid_response", "组件快照缺少 id"))?;
            let Some(component_type) = snapshot_component_type(object) else {
                continue;
            };
            let style = snapshot_config_field(object, "style");
            let data = snapshot_config_field(object, "data");
            let layer = object.get("layer").cloned();
            let filter = snapshot_config_field(object, "filter");
            let entry = baselines
                .entry(component_id.to_string())
                .or_insert_with(|| CanvasSnapshotBaseline {
                    component_type: component_type.to_string(),
                    layer: None,
                    style: None,
                    data: None,
                    filter: None,
                });
            if entry.component_type != component_type {
                *entry = CanvasSnapshotBaseline { component_type: component_type.to_string(), layer: None, style: None, data: None, filter: None };
            }
            if let Some(layer) = layer {
                entry.layer = Some(layer);
            }
            if let Some(style) = style {
                entry.style = Some(style);
            }
            if let Some(data) = data {
                entry.data = Some(data);
            }
            if let Some(filter) = filter {
                entry.filter = Some(filter);
            }
        }
        Ok(())
    }

    fn require(&self, component_id: &str, component_type: &str, field: &str) -> Result<CanvasSnapshotBaseline, AgentToolError> {
        let baseline = self
            .components
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(component_id)
            .cloned()
            .ok_or_else(|| missing_baseline(component_id, field))?;
        if baseline.component_type != component_type {
            return Err(AgentToolError::validation(
                "agent.operation.snapshot_type_mismatch",
                format!("组件 {component_id} 的当前类型为 {}，动作参数要求 {component_type}", baseline.component_type),
            ));
        }
        Ok(baseline)
    }
}

fn snapshot_component_type(object: &serde_json::Map<String, Value>) -> Option<&str> {
    object
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| object.get("config")?.get("base")?.get("type")?.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn snapshot_config_field(object: &serde_json::Map<String, Value>, field: &str) -> Option<Value> {
    object
        .get(field)
        .or_else(|| object.get("config")?.as_object()?.get(field))
        .cloned()
}

fn missing_baseline(component_id: &str, field: &str) -> AgentToolError {
    AgentToolError::validation("agent.operation.snapshot_baseline_required", format!("修改组件 {component_id} 前必须重新读取 {field} 快照"))
}

/// 一次任务所有 Tool 共享的受控依赖。
#[derive(Clone)]
pub(crate) struct AgentToolRuntime {
    context: AgentToolContext,
    files: WorkspaceFileService,
    workspace_search: WorkspaceSearchService,
    operation_validation: AgentArtifactValidationContext,
    validation_evidence: AgentValidationEvidence,
    frontend: Option<AgentFrontendRuntime>,
    workspace_mutating: Arc<AtomicBool>,
    operation_journal: CanvasOperationJournal,
    invocation_tracker: AgentToolInvocationTracker,
    completion_requirements: CompletionRequirementsState,
}

impl AgentToolRuntime {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        app_handle: tauri::AppHandle,
        context: AgentToolContext,
        workspace: WorkspaceHandle,
        operation_validation: AgentArtifactValidationContext,
        validation_evidence: AgentValidationEvidence,
        frontend: Option<AgentFrontendRuntime>,
        file_limits: WorkspaceFileLimits,
        file_trace: WorkspaceFileTrace,
        operation_journal: CanvasOperationJournal,
        invocation_tracker: AgentToolInvocationTracker,
        completion_requirements: CompletionRequirementsState,
    ) -> Self {
        let files = WorkspaceFileService::new_with_trace(workspace.layout.clone(), file_limits.clone(), file_trace);
        let ripgrep = RipgrepSearchService::new(app_handle.clone());
        let workspace_search = WorkspaceSearchService::new(workspace.layout, files.clone(), ripgrep.clone(), file_limits.max_tool_output_bytes);
        Self {
            context,
            files,
            workspace_search,
            operation_validation,
            validation_evidence,
            frontend,
            workspace_mutating: Arc::new(AtomicBool::new(false)),
            operation_journal,
            invocation_tracker,
            completion_requirements,
        }
    }

    pub(crate) fn context(&self) -> &AgentToolContext {
        &self.context
    }

    pub(crate) fn files(&self) -> &WorkspaceFileService {
        &self.files
    }

    pub(crate) fn workspace_search(&self) -> &WorkspaceSearchService {
        &self.workspace_search
    }

    pub(crate) fn operation_journal(&self) -> Result<&CanvasOperationJournal, AgentToolError> {
        Ok(&self.operation_journal)
    }

    pub(crate) fn operation_validation(&self) -> &AgentArtifactValidationContext {
        &self.operation_validation
    }

    pub(crate) fn validation_evidence(&self) -> &AgentValidationEvidence {
        &self.validation_evidence
    }

    pub(crate) fn frontend(&self) -> Result<&AgentFrontendRuntime, AgentToolError> {
        self.frontend
            .as_ref()
            .ok_or_else(|| AgentToolError::internal("agent.frontend.unavailable", "当前 Agent 运行缺少前端 Effect 环境"))
    }

    pub(crate) fn invocation(&self, tool_name: &str) -> Result<AgentToolInvocation, AgentToolError> {
        self.invocation_tracker.require(tool_name)
    }

    pub(crate) fn completion_requirements(&self) -> &CompletionRequirementsState {
        &self.completion_requirements
    }

    pub(crate) fn begin_workspace_mutation(&self) -> Result<WorkspaceMutationGuard, AgentToolError> {
        self.workspace_mutating
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| AgentToolError::validation("agent.workspace.concurrent_mutation", "Workspace 正在执行另一项修改"))?;
        Ok(WorkspaceMutationGuard { state: self.workspace_mutating.clone() })
    }
}

pub(crate) struct WorkspaceMutationGuard {
    state: Arc<AtomicBool>,
}

impl Drop for WorkspaceMutationGuard {
    fn drop(&mut self) {
        self.state.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_baseline_requires_matching_component_type() {
        let baselines = CanvasSnapshotBaselines::default();
        baselines
            .record_response(&json!({
                "components": [{"id": "c1", "type": "BaseText", "style": {"color": "red"}}]
            }))
            .unwrap();
        assert!(baselines.require("c1", "BaseText", "style").is_ok());
        let error = baselines.require("c1", "BaseImage", "style").unwrap_err();
        assert_eq!(error.code, "agent.operation.snapshot_type_mismatch");
    }
}
