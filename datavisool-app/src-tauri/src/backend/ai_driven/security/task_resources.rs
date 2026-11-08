/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, RwLock,
};

use crate::backend::ai_driven::domain::operation::{CanvasOperationKind, CanvasOperationReceipt, CanvasOperationStatus};
use crate::backend::ai_driven::tools::context::AgentToolScope;
use crate::backend::utils::sha256_hex;

use super::SecurityDecisionError;

/// 当前任务中由宿主确认创建的真实画布对象。
///
/// 该表只存在于一次 Agent 运行的安全边界内；重启时必须从同一任务的
/// 持久化 Applied 创建回执重新构建，不能由模型或页面查询结果填充。
#[derive(Clone, Debug, Default)]
pub(crate) struct TaskResourceRegistry {
    resources: Arc<RwLock<BTreeMap<String, TaskResource>>>,
    revoked: Arc<RwLock<BTreeSet<String>>>,
    revision: Arc<AtomicU64>,
}

#[derive(Clone, Debug)]
struct TaskResource {
    task_id: String,
    run_id: String,
    conversation_id: String,
    project_id: String,
    manager_id: String,
    operation_id: String,
}

impl TaskResourceRegistry {
    pub(crate) fn contains(&self, component_id: &str) -> bool {
        let component_id = component_id.trim();
        if self
            .revoked
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(component_id)
        {
            return false;
        }
        self.resources
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(component_id.trim())
    }

    pub(crate) fn is_revoked(&self, component_id: &str) -> bool {
        self.revoked
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(component_id.trim())
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub(crate) fn register_created(&self, scope: &AgentToolScope, receipt: &CanvasOperationReceipt) -> Result<(), SecurityDecisionError> {
        validate_created_receipt(scope, receipt)?;
        let mut resources = self.resources.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        let revoked = self.revoked.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut changed = false;
        for component_id in receipt
            .component_bindings
            .values()
            .map(|value| value.trim())
            .filter(|component_id| !revoked.contains(*component_id))
        {
            if let Some(existing) = resources.get(component_id) {
                if existing.operation_id != receipt.operation_id {
                    return Err(SecurityDecisionError::deny("agent.security.created_resource_conflict", "创建回执包含已归属于其他动作的组件"));
                }
            }
            if !resources.contains_key(component_id) {
                changed = true;
            }
        }
        for component_id in receipt
            .component_bindings
            .values()
            .map(|value| value.trim())
            .filter(|component_id| !revoked.contains(*component_id))
        {
            resources.insert(component_id.to_string(), TaskResource::from_receipt(scope, receipt));
        }
        drop(revoked);
        drop(resources);
        if changed {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
        Ok(())
    }

    pub(crate) fn revoke_deleted(&self, scope: &AgentToolScope, receipt: &CanvasOperationReceipt) {
        if receipt.kind != CanvasOperationKind::DeleteComponents
            || receipt.status != CanvasOperationStatus::Applied
            || receipt.project_id != scope.project_id
            || receipt.manager_id.as_deref() != scope.manager_id.as_deref()
            || receipt.manager_id.as_deref().is_none()
        {
            return;
        }
        let mut resources = self.resources.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut revoked = self.revoked.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut changed = false;
        for component_id in &receipt.component_ids {
            let component_id = component_id.trim();
            if component_id.is_empty() {
                continue;
            }
            let owned_by_scope = resources.get(component_id).is_some_and(|resource| resource.belongs_to(scope));
            if owned_by_scope {
                resources.remove(component_id);
            }
            // Entry targets are not in the dynamic registry, but a successful
            // delete must still prevent an old create receipt from re-granting
            // the now-deleted object.
            changed |= revoked.insert(component_id.to_string());
        }
        drop(revoked);
        drop(resources);
        if changed {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
    }

    pub(crate) fn restore_created(&self, scope: &AgentToolScope, receipts: &[CanvasOperationReceipt]) -> usize {
        let mut restored = 0;
        for receipt in receipts {
            if receipt.kind != CanvasOperationKind::CreateComponents || receipt.status != CanvasOperationStatus::Applied {
                continue;
            }
            if self.register_created(scope, receipt).is_ok() {
                restored += receipt.component_bindings.len();
            }
        }
        for receipt in receipts {
            self.revoke_deleted(scope, receipt);
        }
        restored
    }
}

/// 为一次创建动作按来源 ID 派生稳定的新画布 ID。
///
/// 该值由宿主在 accept 请求中下发，前端只能按这个映射写入；恢复时可从
/// 持久化 operation_id 与绑定键重新计算，旧格式或被篡改的回执不会扩权。
pub(crate) fn reserved_component_id(operation_id: &str, source_component_id: &str) -> String {
    let material = serde_json::to_vec(&(operation_id.trim(), source_component_id.trim())).expect("tuple of strings is serializable");
    format!("ai-{}", sha256_hex(&material))
}

impl TaskResource {
    fn from_receipt(scope: &AgentToolScope, receipt: &CanvasOperationReceipt) -> Self {
        Self {
            task_id: scope.task_id.clone(),
            run_id: scope.run_id.clone(),
            conversation_id: scope.conversation_id.clone(),
            project_id: receipt.project_id.clone(),
            manager_id: receipt.manager_id.clone().unwrap_or_default(),
            operation_id: receipt.operation_id.clone(),
        }
    }

    fn belongs_to(&self, scope: &AgentToolScope) -> bool {
        self.task_id == scope.task_id
            && self.run_id == scope.run_id
            && self.conversation_id == scope.conversation_id
            && self.project_id == scope.project_id
            && self.manager_id == scope.manager_id.as_deref().unwrap_or_default()
    }
}

fn validate_created_receipt(scope: &AgentToolScope, receipt: &CanvasOperationReceipt) -> Result<(), SecurityDecisionError> {
    if receipt.kind != CanvasOperationKind::CreateComponents
        || receipt.status != CanvasOperationStatus::Applied
        || receipt.operation_id.trim().is_empty()
        || receipt.tool_call_id.trim().is_empty()
        || !receipt.applied_at.is_some_and(|applied_at| applied_at >= 0)
    {
        return Err(SecurityDecisionError::deny(
            "agent.security.created_resource_unverified",
            "只有已核验的 Applied 创建回执才能登记任务对象",
        ));
    }
    if receipt.project_id != scope.project_id
        || receipt.manager_id.as_deref() != scope.manager_id.as_deref()
        || receipt.manager_id.as_deref().is_none()
    {
        return Err(SecurityDecisionError::deny(
            "agent.security.created_resource_scope_mismatch",
            "创建回执与当前项目或画布 Manager 不匹配",
        ));
    }
    let keys = receipt.component_bindings.keys().map(|id| id.trim()).collect::<BTreeSet<_>>();
    let values = receipt.component_bindings.values().map(|id| id.trim()).collect::<Vec<_>>();
    let actual = receipt.component_ids.iter().map(|id| id.trim()).collect::<BTreeSet<_>>();
    let expected_ids = receipt
        .component_bindings
        .iter()
        .map(|(source_id, actual_id)| reserved_component_id(&receipt.operation_id, source_id) == actual_id.trim())
        .all(|matches| matches);
    if keys.is_empty()
        || keys.len() != receipt.component_bindings.len()
        || keys.iter().any(|id| id.is_empty())
        || values.iter().any(|id| id.is_empty())
        || values.len() != actual.len()
        || actual.len() != receipt.component_ids.len()
        || actual != values.iter().copied().collect::<BTreeSet<_>>()
        || !expected_ids
    {
        return Err(SecurityDecisionError::deny(
            "agent.security.created_resource_unverified",
            "创建回执的来源 ID、真实 ID 或绑定数量未通过宿主校验",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn scope() -> AgentToolScope {
        AgentToolScope::new("task", "run", "thread", "project")
            .unwrap()
            .with_manager_id(Some("manager"))
    }

    fn receipt(status: CanvasOperationStatus) -> CanvasOperationReceipt {
        let operation_id = "run:call".to_string();
        let actual_id = reserved_component_id(&operation_id, "source-1");
        CanvasOperationReceipt {
            operation_id,
            tool_call_id: "call".to_string(),
            kind: CanvasOperationKind::CreateComponents,
            status,
            project_id: "project".to_string(),
            manager_id: Some("manager".to_string()),
            component_ids: vec![actual_id.clone()],
            component_bindings: BTreeMap::from([("source-1".to_string(), actual_id)]),
            component_fields: BTreeMap::new(),
            configured_component_count: 1,
            default_config_component_count: 0,
            base_revision: None,
            result_revision: None,
            applied_at: Some(1),
            error: None,
        }
    }

    #[test]
    fn registers_only_strictly_bound_applied_creation() {
        let registry = TaskResourceRegistry::default();
        let scope = scope();
        registry
            .register_created(&scope, &receipt(CanvasOperationStatus::Applied))
            .unwrap();
        let actual_id = reserved_component_id("run:call", "source-1");
        assert!(registry.contains(&actual_id));
        let revision = registry.revision();
        assert!(registry
            .register_created(&scope, &receipt(CanvasOperationStatus::Applied))
            .is_ok());
        assert_eq!(registry.revision(), revision);
    }

    #[test]
    fn rejects_unverified_binding_and_wrong_scope() {
        let registry = TaskResourceRegistry::default();
        let scope = scope();
        let mut invalid = receipt(CanvasOperationStatus::Applied);
        invalid.component_bindings = BTreeMap::from([("other".to_string(), invalid.component_ids[0].clone())]);
        assert_eq!(registry.register_created(&scope, &invalid).unwrap_err().code(), "agent.security.created_resource_unverified");
        let other_scope = AgentToolScope::new("task", "run", "thread", "project").unwrap();
        assert_eq!(
            registry
                .register_created(&other_scope, &receipt(CanvasOperationStatus::Applied))
                .unwrap_err()
                .code(),
            "agent.security.created_resource_scope_mismatch"
        );
    }

    #[test]
    fn deletion_revokes_registered_object() {
        let registry = TaskResourceRegistry::default();
        let scope = scope();
        registry
            .register_created(&scope, &receipt(CanvasOperationStatus::Applied))
            .unwrap();
        let mut deleted = receipt(CanvasOperationStatus::Applied);
        deleted.kind = CanvasOperationKind::DeleteComponents;
        deleted.component_ids = vec![deleted.component_ids[0].clone()];
        registry.revoke_deleted(&scope, &deleted);
        assert!(!registry.contains(&deleted.component_ids[0]));
    }

    #[test]
    fn deletion_tombstones_entry_target_without_dynamic_registration() {
        let registry = TaskResourceRegistry::default();
        let scope = scope();
        let mut deleted = receipt(CanvasOperationStatus::Applied);
        deleted.kind = CanvasOperationKind::DeleteComponents;
        deleted.component_ids = vec!["entry-component".to_string()];
        deleted.component_bindings.clear();
        registry.revoke_deleted(&scope, &deleted);

        assert!(!registry.contains("entry-component"));
        assert!(registry.is_revoked("entry-component"));
    }

    #[test]
    fn deleted_object_is_not_resurrected_by_old_create_receipt() {
        let registry = TaskResourceRegistry::default();
        let scope = scope();
        let created = receipt(CanvasOperationStatus::Applied);
        let actual_id = created.component_ids[0].clone();
        registry.register_created(&scope, &created).unwrap();
        let mut deleted = created.clone();
        deleted.kind = CanvasOperationKind::DeleteComponents;
        registry.revoke_deleted(&scope, &deleted);
        registry.register_created(&scope, &created).unwrap();
        assert!(!registry.contains(&actual_id));
        assert!(registry.is_revoked(&actual_id));
    }
}
