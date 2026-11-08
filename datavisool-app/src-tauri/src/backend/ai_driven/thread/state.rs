/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Thread、Turn、Item 与 Timeline 的稳定协议类型。

use serde::{Deserialize, Serialize};
use serde_json::Value;

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
        pub(crate) enum $name {
            $(#[serde(rename = $wire)] $variant),+
        }

        impl $name {
            pub(crate) fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }

            pub(crate) fn from_str(value: &str) -> Option<Self> {
                match value {
                    $($wire => Some(Self::$variant)),+,
                    _ => None,
                }
            }
        }
    };
}

string_enum!(ThreadStatus {
    Active => "active",
    Archived => "archived",
});

string_enum!(ThreadTitleSource {
    Auto => "auto",
    User => "user",
});

string_enum!(TurnStatus {
    Created => "created",
    Running => "running",
    Verifying => "verifying",
    WaitingFrontendTool => "waitingFrontendTool",
    WaitingUserInput => "waitingUserInput",
    WaitingUserConfirmation => "waitingUserConfirmation",
    Completed => "completed",
    Cancelled => "cancelled",
    Failed => "failed",
    Interrupted => "interrupted",
    BudgetExhausted => "budgetExhausted",
});

impl TurnStatus {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed | Self::Interrupted | Self::BudgetExhausted)
    }

    pub(crate) fn is_waiting(self) -> bool {
        matches!(self, Self::WaitingFrontendTool | Self::WaitingUserInput | Self::WaitingUserConfirmation)
    }
}

string_enum!(ItemStatus {
    Created => "created",
    InProgress => "inProgress",
    Waiting => "waiting",
    Completed => "completed",
    Failed => "failed",
    Cancelled => "cancelled",
    Declined => "declined",
});

impl ItemStatus {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled | Self::Declined)
    }
}

string_enum!(TurnInputStatus {
    Accepted => "accepted",
    Consumed => "consumed",
    Abandoned => "abandoned",
});

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThread {
    pub thread_id: String,
    pub project_id: i64,
    pub title: Option<String>,
    pub title_source: ThreadTitleSource,
    pub preview: Option<String>,
    pub status: ThreadStatus,
    pub create_request_id: String,
    pub row_version: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_activity_at: i64,
    pub archived_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadSummary {
    #[serde(flatten)]
    pub thread: AgentThread,
    pub runtime_status: String,
    pub active_turn_id: Option<String>,
    pub current_run_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentTurn {
    pub turn_id: String,
    pub project_id: i64,
    pub thread_id: String,
    pub turn_sequence: u64,
    pub request_id: String,
    pub status: TurnStatus,
    /// 最近一次执行 Run 冻结的预算授权；旧数据库记录可能没有该字段。
    /// TODO(legacy-compat): 确认当前记录均包含授权信息后，移除旧记录缺失字段的兼容。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<Value>,
    pub request: Value,
    pub input_summary: String,
    pub model_config_id: String,
    pub runtime_model_id: String,
    pub provider: Option<String>,
    pub entry_digest: String,
    pub usage: Option<Value>,
    pub last_artifact_id: Option<String>,
    pub last_artifact_revision: Option<u64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub row_version: u64,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub updated_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentItem {
    pub item_id: String,
    pub turn_id: String,
    pub item_sequence: u64,
    pub source_run_id: Option<String>,
    pub logical_key: Option<String>,
    pub item_type: String,
    pub status: ItemStatus,
    pub schema_version: u32,
    pub payload: Value,
    pub last_event_sequence: u64,
    pub row_version: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentTimelineEvent {
    pub event_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: Option<String>,
    pub run_id: String,
    pub sequence: u64,
    pub thread_sequence: u64,
    pub kind: String,
    pub payload: Value,
    pub dedupe_key: String,
    pub schema_version: u32,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentTurnInput {
    pub input_id: String,
    pub turn_id: String,
    pub user_item_id: String,
    pub content_digest: String,
    pub status: TurnInputStatus,
    pub row_version: u64,
    pub created_at: i64,
    pub consumed_at: Option<i64>,
    pub abandoned_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentTaskRunRef {
    pub task_id: String,
    pub run_id: String,
    pub request_id: String,
    pub entry_digest: String,
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub attempt_no: u32,
    pub client_session_id: String,
    pub status: String,
    pub task_type: String,
    pub limits: Value,
    pub pending: Option<Value>,
    pub usage: Option<Value>,
    pub last_artifact_id: Option<String>,
    pub last_artifact_revision: Option<u64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub project_snapshot_ref: Option<String>,
    pub rig_version: Option<String>,
    pub rig_snapshot_ref: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub updated_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Clone, Debug)]
pub(crate) struct CreateResumeAttemptInput {
    pub budget_extension: Option<crate::backend::ai_driven::task::BudgetExtension>,
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub previous_run_id: String,
    pub expected_checkpoint_version: u64,
    pub client_session_id: String,
    pub created_at: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct CreateResumeAttemptResult {
    pub run: AgentTaskRunRef,
    pub events: Vec<AgentTimelineEvent>,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadCursor {
    pub last_activity_at: i64,
    pub thread_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadPage<T, C> {
    pub data: Vec<T>,
    pub next_cursor: Option<C>,
}

#[derive(Clone, Debug)]
pub(crate) struct CreateThreadInput {
    pub project_id: i64,
    pub create_request_id: String,
    pub title: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct ListThreadsQuery {
    pub project_id: i64,
    pub archived: bool,
    pub cursor: Option<ThreadCursor>,
    pub limit: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct UpdateThreadTitleInput {
    pub project_id: i64,
    pub thread_id: String,
    pub title: String,
    pub expected_version: u64,
    pub updated_at: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct SetThreadArchivedInput {
    pub project_id: i64,
    pub thread_id: String,
    pub archived: bool,
    pub expected_version: u64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachmentClaim {
    pub attachment_id: String,
    pub expected_sha256: Option<String>,
    pub expected_byte_size: Option<u64>,
}

#[derive(Clone, Debug)]
pub(crate) struct CreateTurnInput {
    pub project_id: i64,
    pub thread_id: String,
    pub request_id: String,
    pub request: Value,
    pub input_summary: String,
    pub model_config_id: String,
    pub runtime_model_id: String,
    pub provider: Option<String>,
    pub entry_digest: String,
    pub client_session_id: String,
    pub task_type: String,
    pub limits: Value,
    pub project_snapshot_ref: Option<String>,
    pub rig_version: Option<String>,
    pub rig_snapshot_ref: Option<String>,
    pub user_item_payload: Value,
    pub attachments: Vec<AttachmentClaim>,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CreateTurnResult {
    pub turn: AgentTurn,
    pub user_item: AgentItem,
    pub run: AgentTaskRunRef,
    pub events: Vec<AgentTimelineEvent>,
    pub replayed: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ListTurnsQuery {
    pub project_id: i64,
    pub thread_id: String,
    pub after_turn_sequence: u64,
    pub limit: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct ListItemsQuery {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub after_item_sequence: u64,
    pub limit: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct ListThreadEventsQuery {
    pub project_id: i64,
    pub thread_id: String,
    pub after_thread_sequence: u64,
    pub limit: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct ListRunEventsQuery {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub after_sequence: u64,
    pub limit: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct ClaimTurnRunningInput {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub started_at: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct MarkTurnWaitingInput {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub status: TurnStatus,
    pub waiting_item_id: String,
    pub pending: Value,
    pub updated_at: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct ResumeTurnInput {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct TransitionTurnExecutionStatusInput {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub status: TurnStatus,
    pub updated_at: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct StartItemInput {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub item_id: Option<String>,
    pub logical_key: Option<String>,
    pub item_type: String,
    pub status: ItemStatus,
    pub schema_version: u32,
    pub payload: Value,
    pub event_payload: Value,
    pub dedupe_key: String,
    pub created_at: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct ApplyItemEventInput {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub item_id: String,
    pub expected_row_version: u64,
    pub status: ItemStatus,
    pub schema_version: u32,
    pub payload: Value,
    pub event_kind: String,
    pub event_payload: Value,
    pub dedupe_key: String,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Clone, Debug)]
pub(crate) struct AppendTimelineEventInput {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub item_id: Option<String>,
    pub kind: String,
    pub payload: Value,
    pub dedupe_key: String,
    pub schema_version: u32,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TimelineWriteResult {
    pub event: AgentTimelineEvent,
    pub item: Option<AgentItem>,
    pub deduplicated: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct AcceptTurnInput {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub input_id: String,
    pub content_digest: String,
    pub user_item_payload: Value,
    pub schema_version: u32,
    pub accepted_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AcceptedTurnInput {
    pub input: AgentTurnInput,
    pub user_item: AgentItem,
    pub events: Vec<AgentTimelineEvent>,
    pub replayed: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ConsumeTurnInput {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub input_id: String,
    pub expected_row_version: u64,
    pub consumed_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ConsumedTurnInput {
    pub input: AgentTurnInput,
    pub turn: AgentTurn,
    pub run: AgentTaskRunRef,
    pub completed_waiting_item: Option<AgentItem>,
    pub events: Vec<AgentTimelineEvent>,
    pub replayed: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct CompleteTurnInput {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub status: TurnStatus,
    pub usage: Option<Value>,
    pub last_artifact_id: Option<String>,
    pub last_artifact_revision: Option<u64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub thread_preview: Option<String>,
    pub ended_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CompleteTurnResult {
    pub turn: AgentTurn,
    pub run: AgentTaskRunRef,
    pub closed_items: Vec<AgentItem>,
    pub events: Vec<AgentTimelineEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConsistencyIssue {
    pub kind: String,
    pub entity_id: String,
    pub detail: String,
}
