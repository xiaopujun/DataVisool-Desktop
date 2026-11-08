/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Rig Tool 可见的稳定任务上下文。

use std::sync::Arc;

use tokio::sync::watch;

use crate::backend::ai_driven::security::AgentSecurity;

use super::policy::{AgentToolEffect, AgentToolPolicy, AgentToolPolicyError};

/// Tool 调用的业务作用域，不包含数据库密码、API Key 或任意文件路径。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentToolScope {
    pub(crate) task_id: String,
    pub(crate) run_id: String,
    pub(crate) conversation_id: String,
    pub(crate) project_id: String,
    pub(crate) manager_id: Option<String>,
    pub(crate) turn_id: Option<String>,
}

impl AgentToolScope {
    /// 构造任务作用域并拒绝空标识。
    pub(crate) fn new(
        task_id: impl Into<String>,
        run_id: impl Into<String>,
        conversation_id: impl Into<String>,
        project_id: impl Into<String>,
    ) -> Result<Self, AgentToolContextError> {
        Ok(Self {
            task_id: required_id("task_id", task_id.into())?,
            run_id: required_id("run_id", run_id.into())?,
            conversation_id: required_id("conversation_id", conversation_id.into())?,
            project_id: required_id("project_id", project_id.into())?,
            manager_id: None,
            turn_id: None,
        })
    }

    pub(crate) fn with_manager_id(mut self, manager_id: Option<&str>) -> Self {
        self.manager_id = manager_id.map(str::trim).filter(|value| !value.is_empty()).map(str::to_string);
        self
    }

    pub(crate) fn with_turn_id(mut self, turn_id: &str) -> Self {
        self.turn_id = Some(turn_id.trim().to_string()).filter(|value| !value.is_empty());
        self
    }
}

/// 所有业务 Tool 共享的最小运行上下文。
#[derive(Clone)]
pub(crate) struct AgentToolContext {
    scope: AgentToolScope,
    policy: Arc<AgentToolPolicy>,
    security: Arc<AgentSecurity>,
    cancel_rx: watch::Receiver<bool>,
}

impl AgentToolContext {
    pub(crate) fn new(scope: AgentToolScope, policy: Arc<AgentToolPolicy>, security: Arc<AgentSecurity>, cancel_rx: watch::Receiver<bool>) -> Self {
        Self { scope, policy, security, cancel_rx }
    }

    pub(crate) fn scope(&self) -> &AgentToolScope {
        &self.scope
    }

    /// 当前任务的服务端能力授权、审批票据和安全审计服务。
    pub(crate) fn security(&self) -> &AgentSecurity {
        &self.security
    }

    /// 在 Tool 执行前同时检查取消和服务端权限。
    pub(crate) fn authorize(&self, tool_name: &str, effect: AgentToolEffect, per_call_approved: bool) -> Result<(), AgentToolContextError> {
        self.ensure_active()?;
        self.policy
            .authorize(tool_name, effect, per_call_approved)
            .map_err(AgentToolContextError::Policy)
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        *self.cancel_rx.borrow()
    }

    pub(crate) fn ensure_active(&self) -> Result<(), AgentToolContextError> {
        if self.is_cancelled() {
            Err(AgentToolContextError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// 为需要在 await 边界监听取消的 Tool 创建独立订阅。
    pub(crate) fn cancel_receiver(&self) -> watch::Receiver<bool> {
        self.cancel_rx.clone()
    }
}

/// Tool Context 的稳定错误。
#[derive(Debug)]
pub(crate) enum AgentToolContextError {
    InvalidScope { field: &'static str },
    Policy(AgentToolPolicyError),
    Cancelled,
}

impl AgentToolContextError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::InvalidScope { .. } => "agent.tool.invalid_scope",
            Self::Policy(error) => error.code(),
            Self::Cancelled => "agent.cancelled",
        }
    }
}

impl std::fmt::Display for AgentToolContextError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidScope { field } => write!(formatter, "Tool 作用域字段不能为空：{field}"),
            Self::Policy(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("Agent 任务已取消"),
        }
    }
}

impl std::error::Error for AgentToolContextError {}

fn required_id(field: &'static str, value: String) -> Result<String, AgentToolContextError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        Err(AgentToolContextError::InvalidScope { field })
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::security::{AgentCapabilityGrant, AgentSecurity};
    use crate::backend::ai_driven::tools::policy::WriteApprovalPolicy;

    fn context() -> (watch::Sender<bool>, AgentToolContext) {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let scope = AgentToolScope::new("task-1", "run-1", "conversation-1", "project-1").unwrap();
        let policy = Arc::new(AgentToolPolicy::new(["read_artifact"], false, false, WriteApprovalPolicy::SecurityDecision));
        let security = Arc::new(AgentSecurity::new(AgentCapabilityGrant::for_task(&scope, [], "workspace")));
        (cancel_tx, AgentToolContext::new(scope, policy, security, cancel_rx))
    }

    #[test]
    fn rejects_empty_scope_identifier() {
        let error = AgentToolScope::new("task-1", " ", "conversation-1", "project-1").expect_err("run id must be required");

        assert_eq!(error.code(), "agent.tool.invalid_scope");
    }

    #[test]
    fn authorizes_only_declared_tool() {
        let (_, context) = context();

        assert!(context.authorize("read_artifact", AgentToolEffect::Read, false).is_ok());
        assert_eq!(
            context
                .authorize("shell", AgentToolEffect::Read, false)
                .expect_err("shell must be rejected")
                .code(),
            "agent.tool.forbidden"
        );
    }

    #[test]
    fn cancellation_is_checked_before_policy() {
        let (cancel_tx, context) = context();
        cancel_tx.send(true).unwrap();

        let error = context
            .authorize("shell", AgentToolEffect::Read, false)
            .expect_err("cancelled task must stop first");
        assert_eq!(error.code(), "agent.cancelled");
    }
}
