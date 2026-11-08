/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Agent 的服务端安全边界。
//!
//! 模型、前端回执和画布内容均不可信；只有本模块签发的能力授权与审批票据可以
//! 放行真实业务动作。

mod approval;
mod audit;
mod capability;
mod decision;
mod task_resources;

pub(crate) use approval::{ApprovalTicket, ApprovalTicketStore};
pub(crate) use audit::{AgentSecurityAudit, SecurityAuditEvent};
pub(crate) use capability::{AgentCapabilityGrant, AgentOperation, MAX_TARGETS_PER_ACTION};
pub(crate) use decision::{
    authorize_canvas_create, authorize_canvas_delete, authorize_canvas_patch, AuthorizationDecision, SecurityDecisionError, SecurityRisk,
};
pub(crate) use task_resources::{reserved_component_id, TaskResourceRegistry};

use std::sync::Arc;

/// 单次任务共享的安全服务。
#[derive(Clone)]
pub(crate) struct AgentSecurity {
    grant: Arc<AgentCapabilityGrant>,
    approvals: ApprovalTicketStore,
    audit: AgentSecurityAudit,
    resources: TaskResourceRegistry,
}

impl AgentSecurity {
    pub(crate) fn new(grant: AgentCapabilityGrant) -> Self {
        let resources = grant.resources().clone();
        Self {
            grant: Arc::new(grant),
            approvals: ApprovalTicketStore::default(),
            audit: AgentSecurityAudit,
            resources,
        }
    }

    pub(crate) fn grant(&self) -> &AgentCapabilityGrant {
        &self.grant
    }

    pub(crate) fn approvals(&self) -> &ApprovalTicketStore {
        &self.approvals
    }

    pub(crate) fn audit(&self) -> &AgentSecurityAudit {
        &self.audit
    }

    /// 用已由当前任务操作日志核验的 Applied 回执恢复可操作对象。
    /// 无效或旧格式回执会被拒绝登记，不影响其他可信回执恢复。
    pub(crate) fn restore_created_resources(
        &self,
        scope: &crate::backend::ai_driven::tools::context::AgentToolScope,
        receipts: &[crate::backend::ai_driven::domain::operation::CanvasOperationReceipt],
    ) -> usize {
        if self.grant.validate_scope(scope).is_err() {
            return 0;
        }
        self.resources.restore_created(scope, receipts)
    }

    pub(crate) fn register_created_resource(
        &self,
        scope: &crate::backend::ai_driven::tools::context::AgentToolScope,
        receipt: &crate::backend::ai_driven::domain::operation::CanvasOperationReceipt,
    ) -> Result<(), SecurityDecisionError> {
        self.grant.validate_scope(scope)?;
        self.resources.register_created(scope, receipt)
    }

    pub(crate) fn revoke_deleted_resources(
        &self,
        scope: &crate::backend::ai_driven::tools::context::AgentToolScope,
        receipt: &crate::backend::ai_driven::domain::operation::CanvasOperationReceipt,
    ) -> Result<(), SecurityDecisionError> {
        self.grant.validate_scope(scope)?;
        self.resources.revoke_deleted(scope, receipt);
        Ok(())
    }

    /// 权限事实版本。动态资源每次登记或撤销成功后递增。
    pub(crate) fn authorization_revision(&self) -> u64 {
        self.resources.revision()
    }
}
