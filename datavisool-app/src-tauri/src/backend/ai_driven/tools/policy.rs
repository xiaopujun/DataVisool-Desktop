/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Agent Tool 服务端权限策略。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Tool 的副作用类别。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AgentToolEffect {
    /// 不改变业务状态的读取操作。
    Read,
    /// 只写当前任务 Workspace 的操作。
    WorkspaceWrite,
    /// 等待或调用受控前端能力。
    FrontendInteraction,
    /// 在校验边界内修改真实业务状态。
    Commit,
}

/// 写操作审批方式。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WriteApprovalPolicy {
    /// 真实业务动作必须由参数化的服务端安全决策器授权。
    SecurityDecision,
    /// 每次业务动作都必须携带单独审批结果。
    PerCommit,
}

/// Tool 权限检查错误。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentToolPolicyError {
    code: &'static str,
    message: String,
}

impl AgentToolPolicyError {
    /// 稳定错误码。
    pub(crate) fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for AgentToolPolicyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AgentToolPolicyError {}

/// 单次 Agent 任务的 Tool 服务端权限集合。
#[derive(Clone, Debug)]
pub(crate) struct AgentToolPolicy {
    allowed_tools: HashSet<String>,
    allow_workspace_writes: bool,
    allow_frontend_interactions: bool,
    write_approval: WriteApprovalPolicy,
}

impl AgentToolPolicy {
    /// 创建最小权限策略。工具名在构建任务时一次性确定。
    pub(crate) fn new(
        allowed_tools: impl IntoIterator<Item = impl Into<String>>,
        allow_workspace_writes: bool,
        allow_frontend_interactions: bool,
        write_approval: WriteApprovalPolicy,
    ) -> Self {
        Self {
            allowed_tools: allowed_tools
                .into_iter()
                .map(Into::into)
                .map(|name: String| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect(),
            allow_workspace_writes,
            allow_frontend_interactions,
            write_approval,
        }
    }

    /// 校验一次 Tool 调用。Prompt 中声明工具不能替代此服务端检查。
    pub(crate) fn authorize(&self, tool_name: &str, effect: AgentToolEffect, per_call_approved: bool) -> Result<(), AgentToolPolicyError> {
        let tool_name = tool_name.trim();
        if tool_name.is_empty() {
            return Err(policy_error("agent.tool.empty_name", "Tool 名称不能为空"));
        }
        if !self.allowed_tools.contains(tool_name) {
            return Err(policy_error("agent.tool.forbidden", format!("当前任务不允许调用 Tool：{tool_name}")));
        }

        match effect {
            AgentToolEffect::Read => Ok(()),
            AgentToolEffect::WorkspaceWrite if self.allow_workspace_writes => Ok(()),
            AgentToolEffect::WorkspaceWrite => {
                Err(policy_error("agent.tool.workspace_write_forbidden", format!("当前任务不允许写入 Workspace：{tool_name}")))
            }
            AgentToolEffect::FrontendInteraction if self.allow_frontend_interactions => Ok(()),
            AgentToolEffect::FrontendInteraction => Err(policy_error(
                "agent.tool.frontend_interaction_forbidden",
                format!("当前任务不允许调用前端交互 Tool：{tool_name}"),
            )),
            AgentToolEffect::Commit if self.write_approval == WriteApprovalPolicy::SecurityDecision || per_call_approved => Ok(()),
            AgentToolEffect::Commit => Err(policy_error("agent.tool.approval_required", format!("提交 Tool 需要用户审批：{tool_name}"))),
        }
    }
}

fn policy_error(code: &'static str, message: impl Into<String>) -> AgentToolPolicyError {
    AgentToolPolicyError { code, message: message.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(write_approval: WriteApprovalPolicy) -> AgentToolPolicy {
        AgentToolPolicy::new(["read_artifact", "apply_artifact_patch", "ask_user", "commit_artifact"], true, true, write_approval)
    }

    #[test]
    fn rejects_tool_outside_task_allow_list() {
        let error = policy(WriteApprovalPolicy::SecurityDecision)
            .authorize("shell", AgentToolEffect::Read, false)
            .expect_err("undeclared tool must be rejected");

        assert_eq!(error.code(), "agent.tool.forbidden");
    }

    #[test]
    fn enforces_workspace_and_frontend_capabilities() {
        let read_only = AgentToolPolicy::new(["read", "write", "ask"], false, false, WriteApprovalPolicy::SecurityDecision);

        assert!(read_only.authorize("read", AgentToolEffect::Read, false).is_ok());
        assert_eq!(
            read_only
                .authorize("write", AgentToolEffect::WorkspaceWrite, false)
                .expect_err("workspace write must be rejected")
                .code(),
            "agent.tool.workspace_write_forbidden"
        );
        assert_eq!(
            read_only
                .authorize("ask", AgentToolEffect::FrontendInteraction, false)
                .expect_err("frontend interaction must be rejected")
                .code(),
            "agent.tool.frontend_interaction_forbidden"
        );
    }

    #[test]
    fn per_commit_policy_requires_explicit_approval() {
        let policy = policy(WriteApprovalPolicy::PerCommit);

        let error = policy
            .authorize("commit_artifact", AgentToolEffect::Commit, false)
            .expect_err("commit must require approval");
        assert_eq!(error.code(), "agent.tool.approval_required");
        assert!(policy.authorize("commit_artifact", AgentToolEffect::Commit, true).is_ok());
    }
}
