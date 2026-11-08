/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::collections::BTreeSet;

use serde::Serialize;

use crate::backend::ai_driven::time::now_ms;
use crate::backend::ai_driven::tools::context::AgentToolScope;

use super::SecurityDecisionError;
use super::TaskResourceRegistry;

pub(crate) const MAX_TARGETS_PER_ACTION: usize = 20;
const GRANT_TTL_MS: i64 = 30 * 60 * 1000;

/// 能力授权中的真实动作类别。Tool 名称不等于业务权限。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AgentOperation {
    CanvasCreate,
    CanvasPatch,
    CanvasDelete,
    WorkspaceWrite,
}

/// 任务开始时由 Rust 固化的最小能力授权。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentCapabilityGrant {
    pub(crate) task_id: String,
    pub(crate) run_id: String,
    pub(crate) project_id: String,
    pub(crate) manager_id: Option<String>,
    pub(crate) allowed_component_ids: BTreeSet<String>,
    pub(crate) allowed_operations: BTreeSet<AgentOperation>,
    pub(crate) allowed_data_source_types: BTreeSet<String>,
    pub(crate) allowed_network_hosts: BTreeSet<String>,
    pub(crate) workspace_id: String,
    pub(crate) max_targets_per_action: usize,
    pub(crate) expires_at_ms: i64,
    #[serde(skip)]
    resources: TaskResourceRegistry,
}

impl AgentCapabilityGrant {
    pub(crate) fn for_task(scope: &AgentToolScope, allowed_component_ids: impl IntoIterator<Item = String>, workspace_id: impl Into<String>) -> Self {
        let allowed_component_ids = allowed_component_ids
            .into_iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect();
        Self {
            task_id: scope.task_id.clone(),
            run_id: scope.run_id.clone(),
            project_id: scope.project_id.clone(),
            manager_id: scope.manager_id.clone(),
            allowed_component_ids,
            allowed_operations: BTreeSet::from([
                AgentOperation::CanvasCreate,
                AgentOperation::CanvasPatch,
                AgentOperation::CanvasDelete,
                AgentOperation::WorkspaceWrite,
            ]),
            // 新任务默认仅能写静态数据；外部数据源需由受控的非 Agent 流程配置。
            allowed_data_source_types: BTreeSet::from(["static".to_string()]),
            // 默认断网。后续如引入受控代理，再为具体任务填入已确认域名。
            allowed_network_hosts: BTreeSet::new(),
            workspace_id: workspace_id.into(),
            max_targets_per_action: MAX_TARGETS_PER_ACTION,
            expires_at_ms: now_ms().saturating_add(GRANT_TTL_MS),
            resources: TaskResourceRegistry::default(),
        }
    }

    pub(crate) fn resources(&self) -> &TaskResourceRegistry {
        &self.resources
    }

    /// 由宿主根据本次任务的 Workspace capability 固化文件写权限。
    /// 该方法只收紧或恢复启动时的能力集合，不读取模型声明来扩权。
    pub(crate) fn with_workspace_write_enabled(mut self, enabled: bool) -> Self {
        if enabled {
            self.allowed_operations.insert(AgentOperation::WorkspaceWrite);
        } else {
            self.allowed_operations.remove(&AgentOperation::WorkspaceWrite);
        }
        self
    }

    pub(crate) fn validate_scope(&self, scope: &AgentToolScope) -> Result<(), SecurityDecisionError> {
        if self.expires_at_ms <= now_ms() {
            return Err(SecurityDecisionError::deny("agent.security.grant_expired", "任务能力授权已过期，请重新发起任务"));
        }
        if self.task_id != scope.task_id || self.run_id != scope.run_id || self.project_id != scope.project_id || self.manager_id != scope.manager_id
        {
            return Err(SecurityDecisionError::deny(
                "agent.security.scope_changed",
                "任务、项目或画布作用域已发生变化，旧授权不能继续执行",
            ));
        }
        Ok(())
    }

    pub(crate) fn authorize_operation(&self, operation: AgentOperation) -> Result<(), SecurityDecisionError> {
        if self.allowed_operations.contains(&operation) {
            Ok(())
        } else {
            Err(SecurityDecisionError::deny("agent.security.operation_forbidden", "当前任务未被授予该业务动作权限"))
        }
    }

    pub(crate) fn authorize_workspace(&self, workspace_id: &str) -> Result<(), SecurityDecisionError> {
        if self.workspace_id == workspace_id.trim() {
            Ok(())
        } else {
            Err(SecurityDecisionError::deny("agent.security.workspace_out_of_scope", "当前任务只能写入启动时绑定的 Workspace"))
        }
    }

    pub(crate) fn authorize_existing_targets(&self, targets: &[String]) -> Result<(), SecurityDecisionError> {
        if targets.is_empty() || targets.len() > self.max_targets_per_action {
            return Err(SecurityDecisionError::deny("agent.security.target_count_exceeded", "本次动作目标数量超出任务授权上限"));
        }
        if targets.iter().any(|target| {
            let target = target.trim();
            self.resources.is_revoked(target) || (!self.allowed_component_ids.contains(target) && !self.resources.contains(target))
        }) {
            return Err(SecurityDecisionError::deny(
                "agent.security.target_out_of_scope",
                "动作包含入口选区外的组件；请由用户重新选择目标后发起新任务",
            ));
        }
        Ok(())
    }

    pub(crate) fn authorize_create_count(&self, count: usize) -> Result<(), SecurityDecisionError> {
        if count == 0 || count > self.max_targets_per_action {
            return Err(SecurityDecisionError::deny(
                "agent.security.target_count_exceeded",
                format!(
                    "单次最多创建 {} 个组件；请保留完整组件方案，按每批不超过 {} 个分批创建",
                    self.max_targets_per_action, self.max_targets_per_action
                ),
            ));
        }
        Ok(())
    }

    pub(crate) fn authorize_data_source(&self, source_type: &str) -> Result<(), SecurityDecisionError> {
        if self.allowed_data_source_types.contains(source_type) {
            Ok(())
        } else {
            Err(SecurityDecisionError::deny(
                "agent.security.data_source_forbidden",
                format!("当前任务不允许 Agent 配置 {source_type} 数据源"),
            ))
        }
    }

    pub(crate) fn authorize_network_host(&self, host: &str) -> Result<(), SecurityDecisionError> {
        if self.allowed_network_hosts.contains(host) {
            Ok(())
        } else {
            Err(SecurityDecisionError::deny("agent.security.network_host_forbidden", "当前任务未获准访问该网络目标"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant() -> (AgentToolScope, AgentCapabilityGrant) {
        let scope = AgentToolScope::new("task", "run", "thread", "project")
            .unwrap()
            .with_manager_id(Some("manager"));
        let grant = AgentCapabilityGrant::for_task(&scope, ["selected".to_string()], "workspace");
        (scope, grant)
    }

    #[test]
    fn existing_targets_must_be_in_entry_scope() {
        let (_, grant) = grant();
        assert!(grant.authorize_existing_targets(&["selected".to_string()]).is_ok());
        assert_eq!(
            grant.authorize_existing_targets(&["outside".to_string()]).unwrap_err().code(),
            "agent.security.target_out_of_scope"
        );
    }

    #[test]
    fn default_grant_denies_external_data_sources() {
        let (_, grant) = grant();
        assert!(grant.authorize_data_source("static").is_ok());
        assert_eq!(grant.authorize_data_source("api").unwrap_err().code(), "agent.security.data_source_forbidden");
    }

    #[test]
    fn workspace_must_match_task_grant() {
        let (_, grant) = grant();
        assert!(grant.authorize_workspace("workspace").is_ok());
        assert_eq!(grant.authorize_workspace("other-workspace").unwrap_err().code(), "agent.security.workspace_out_of_scope");
    }

    #[test]
    fn workspace_write_capability_is_host_bound() {
        let (_, grant) = grant();
        let restricted = grant.clone().with_workspace_write_enabled(false);
        assert!(!restricted.allowed_operations.contains(&AgentOperation::WorkspaceWrite));
        assert!(grant
            .with_workspace_write_enabled(true)
            .allowed_operations
            .contains(&AgentOperation::WorkspaceWrite));
    }

    #[test]
    fn revoked_resource_overrides_an_entry_scope_match() {
        let scope = AgentToolScope::new("task", "run", "thread", "project")
            .unwrap()
            .with_manager_id(Some("manager"));
        let resource_id = super::super::task_resources::reserved_component_id("run:call", "source");
        let grant = AgentCapabilityGrant::for_task(&scope, [resource_id.clone()], "workspace");
        let mut receipt = crate::backend::ai_driven::domain::operation::CanvasOperationReceipt::prepared(
            "run:call".to_string(),
            "call".to_string(),
            crate::backend::ai_driven::domain::operation::CanvasOperationKind::CreateComponents,
            "project".to_string(),
            Some("manager".to_string()),
            vec![resource_id.clone()],
        );
        receipt.status = crate::backend::ai_driven::domain::operation::CanvasOperationStatus::Applied;
        receipt.applied_at = Some(1);
        receipt.component_bindings = std::collections::BTreeMap::from([("source".to_string(), resource_id.clone())]);
        grant.resources.register_created(&scope, &receipt).unwrap();
        let mut deleted = receipt.clone();
        deleted.kind = crate::backend::ai_driven::domain::operation::CanvasOperationKind::DeleteComponents;
        grant.resources.revoke_deleted(&scope, &deleted);
        assert_eq!(grant.authorize_existing_targets(&[resource_id]).unwrap_err().code(), "agent.security.target_out_of_scope");
    }

    #[test]
    fn create_count_error_instructs_the_agent_to_batch_without_reducing_scope() {
        let (_, grant) = grant();

        let error = grant.authorize_create_count(MAX_TARGETS_PER_ACTION + 1).unwrap_err();

        assert_eq!(error.code(), "agent.security.target_count_exceeded");
        assert!(error
            .to_string()
            .contains(&format!("单次最多创建 {} 个组件", MAX_TARGETS_PER_ACTION)));
        assert!(error.to_string().contains("保留完整组件方案"));
        assert!(error.to_string().contains("分批创建"));
    }
}
