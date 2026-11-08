/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 与执行器内部实现解耦的 TaskRun 协议。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::ai_driven::settings::{AgentContextSettings, AgentModelRequestSettings};
use crate::backend::ai_driven::thread::{AgentItem, AgentTimelineEvent, AttachmentClaim, TurnStatus};

pub(crate) const AGENT_RESTART_INTERRUPTED: &str = "agent.restart_interrupted";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AgentTaskStatus {
    Created,
    Running,
    Verifying,
    WaitingFrontendTool,
    WaitingUserInput,
    WaitingUserConfirmation,
    Completed,
    Cancelled,
    Failed,
    BudgetExhausted,
}

impl AgentTaskStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Running => "running",
            Self::Verifying => "verifying",
            Self::WaitingFrontendTool => "waitingFrontendTool",
            Self::WaitingUserInput => "waitingUserInput",
            Self::WaitingUserConfirmation => "waitingUserConfirmation",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::BudgetExhausted => "budgetExhausted",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value {
            "created" => Some(Self::Created),
            "running" => Some(Self::Running),
            "verifying" => Some(Self::Verifying),
            "waitingFrontendTool" => Some(Self::WaitingFrontendTool),
            "waitingUserInput" => Some(Self::WaitingUserInput),
            "waitingUserConfirmation" => Some(Self::WaitingUserConfirmation),
            "completed" => Some(Self::Completed),
            "cancelled" => Some(Self::Cancelled),
            "failed" => Some(Self::Failed),
            "budgetExhausted" => Some(Self::BudgetExhausted),
            _ => None,
        }
    }

    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed | Self::BudgetExhausted)
    }

    pub(crate) fn is_waiting(self) -> bool {
        matches!(self, Self::WaitingFrontendTool | Self::WaitingUserInput | Self::WaitingUserConfirmation)
    }

    pub(crate) fn turn_status(self) -> TurnStatus {
        match self {
            Self::Created => TurnStatus::Created,
            Self::Running => TurnStatus::Running,
            Self::Verifying => TurnStatus::Verifying,
            Self::WaitingFrontendTool => TurnStatus::WaitingFrontendTool,
            Self::WaitingUserInput => TurnStatus::WaitingUserInput,
            Self::WaitingUserConfirmation => TurnStatus::WaitingUserConfirmation,
            Self::Completed => TurnStatus::Completed,
            Self::Cancelled => TurnStatus::Cancelled,
            Self::Failed => TurnStatus::Failed,
            Self::BudgetExhausted => TurnStatus::BudgetExhausted,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentExecutionLimitsSnapshot {
    pub token_budget_enabled: bool,
    pub call_budget_enabled: bool,
    pub duration_budget_enabled: bool,
    #[serde(default)]
    pub max_model_calls: u32,
    #[serde(default)]
    pub max_tool_calls: u32,
    #[serde(default)]
    pub max_write_calls: u32,
    #[serde(default)]
    pub max_total_tokens: u64,
    #[serde(default)]
    pub max_duration_ms: u64,
    #[serde(default)]
    pub max_tool_duration_ms: u64,
    #[serde(default)]
    pub max_consecutive_failures: u32,
    #[serde(default)]
    pub max_frontend_interactions: u32,
    #[serde(default)]
    pub max_same_read_repeats: u32,
    #[serde(default)]
    pub max_completion_continuations: u32,
    #[serde(default)]
    pub workspace_files: crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceFileLimits,
    #[serde(default)]
    pub context: AgentContextSettings,
    #[serde(default)]
    pub model_request: AgentModelRequestSettings,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AgentPendingKind {
    FrontendTool,
    UserInput,
    UserConfirmation,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentPendingSnapshot {
    pub kind: AgentPendingKind,
    /// 产生等待调用的 Rig 模型步骤。
    pub step_index: u64,
    /// 产生等待调用时冻结的 Tool 路由版本。
    pub route_revision: u64,
    /// 与等待 checkpoint 原子关联的持久化 Item。
    pub waiting_item_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub request_summary: Value,
    pub suspended_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentArtifactRef {
    pub artifact_id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentTaskError {
    pub code: String,
    pub message: String,
}

/// TaskRun 的真实持久化身份。Thread/Turn 不再由项目或请求计算。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentTaskRun {
    pub task_id: String,
    pub run_id: String,
    pub request_id: String,
    pub entry_digest: String,
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub client_session_id: String,
    pub task_type: String,
    pub status: AgentTaskStatus,
    pub limits: AgentExecutionLimitsSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<AgentPendingSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_artifact: Option<AgentArtifactRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AgentTaskError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_snapshot_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rig_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rig_snapshot_ref: Option<String>,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
}

/// 创建 TaskRun 即在指定 Thread 下幂等创建一个 Turn。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NewAgentTaskRun {
    pub project_id: i64,
    pub thread_id: String,
    pub request_id: String,
    #[serde(default)]
    pub request: Value,
    pub input_summary: String,
    pub model_config_id: String,
    pub runtime_model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub entry_digest: String,
    pub client_session_id: String,
    pub task_type: String,
    pub limits: AgentExecutionLimitsSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_snapshot_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rig_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rig_snapshot_ref: Option<String>,
    #[serde(default)]
    pub user_item_payload: Value,
    #[serde(default)]
    pub attachments: Vec<AttachmentClaim>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveAgentTaskCheckpoint {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub status: AgentTaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<AgentPendingSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_artifact: Option<AgentArtifactRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AgentTaskError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_snapshot_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rig_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rig_snapshot_ref: Option<String>,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentTaskCheckpointResult {
    pub task: AgentTaskRun,
    pub events: Vec<AgentTimelineEvent>,
    pub closed_items: Vec<AgentItem>,
}

#[derive(Clone, Debug)]
pub(crate) struct AgentTaskResumeResult {
    pub task: AgentTaskRun,
    pub events: Vec<AgentTimelineEvent>,
    pub replayed: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NewAgentTaskEvent {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
    pub dedupe_key: String,
    pub schema_version: u32,
    pub created_at: i64,
}

pub(crate) type AgentTaskEvent = AgentTimelineEvent;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListTaskEvents {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub after_sequence: u64,
    pub limit: u32,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{AgentPendingKind, AgentPendingSnapshot};

    #[test]
    fn pending_snapshot_restores_route_identity() {
        let current: AgentPendingSnapshot = serde_json::from_value(json!({
            "kind": "userInput",
            "stepIndex": 4,
            "routeRevision": 7,
            "waitingItemId": "item-1",
            "toolName": "request_user_input",
            "requestSummary": { "question": "metric" },
            "suspendedAt": 10
        }))
        .unwrap();
        assert_eq!(current.kind, AgentPendingKind::UserInput);
        assert_eq!((current.step_index, current.route_revision), (4, 7));
    }
}
