/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 进程内取消通道与持久化 Inbox 的唤醒邮箱。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use serde_json::Value;
use tokio::sync::watch;

use crate::backend::ai_driven::agent::{AgentRunOutput, ExecutionUsageSnapshot};
use crate::backend::ai_driven::domain::operation::CanvasOperationReceipt;
use crate::backend::ai_driven::thread::ThreadTokenUsage;

use super::state::AGENT_RESTART_INTERRUPTED;
use super::{AgentArtifactRef, AgentTaskError, AgentTaskRun, AgentTaskStatus};

static TASK_REGISTRY: OnceLock<AiDrivenTaskRegistry> = OnceLock::new();
const STEER_MAILBOX_CAPACITY: usize = 32;

pub(crate) fn ai_driven_task_registry() -> &'static AiDrivenTaskRegistry {
    TASK_REGISTRY.get_or_init(AiDrivenTaskRegistry::default)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiDrivenTaskRunSnapshot {
    pub limits: super::AgentExecutionLimitsSnapshot,
    pub task_id: String,
    pub run_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub event_cursor: u64,
    pub request_id: String,
    pub project_id: i64,
    pub client_session_id: String,
    pub status: String,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_version: Option<u64>,
    pub recoverable: bool,
    pub workflow_state: Option<Value>,
    pub runtime: AiDrivenRuntimeSnapshot,
    pub diagnostics: AiDrivenDiagnosticsSnapshot,
    #[serde(skip)]
    has_persisted_execution_usage: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiDrivenRuntimeSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub usage: AiDrivenRuntimeUsageSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<AgentArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<CanvasOperationReceipt>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiDrivenRuntimeUsageSnapshot {
    #[serde(flatten)]
    pub execution: ExecutionUsageSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_usage: Option<ThreadTokenUsage>,
}

impl From<ExecutionUsageSnapshot> for AiDrivenRuntimeUsageSnapshot {
    fn from(execution: ExecutionUsageSnapshot) -> Self {
        Self { execution, thread_usage: None }
    }
}

impl AiDrivenRuntimeUsageSnapshot {
    fn replace_execution_if_fresher(&mut self, usage: Option<Value>, preserve_equal: bool) {
        let Some(Ok(latest)) = usage.map(serde_json::from_value::<ExecutionUsageSnapshot>) else {
            return;
        };
        if has_older_cumulative_usage(&latest, &self.execution) {
            return;
        }
        if preserve_equal && same_cumulative_usage(&latest, &self.execution) {
            return;
        }
        self.execution = latest;
    }
}

fn effective_total_tokens(usage: &ExecutionUsageSnapshot) -> u64 {
    usage.total_tokens.max(usage.input_tokens.saturating_add(usage.output_tokens))
}

fn has_older_cumulative_usage(candidate: &ExecutionUsageSnapshot, current: &ExecutionUsageSnapshot) -> bool {
    effective_total_tokens(candidate) < effective_total_tokens(current)
        || (effective_total_tokens(candidate) == effective_total_tokens(current)
            && (candidate.input_tokens < current.input_tokens
                || candidate.output_tokens < current.output_tokens
                || candidate.cached_input_tokens < current.cached_input_tokens
                || candidate.cache_creation_input_tokens < current.cache_creation_input_tokens
                || candidate.model_calls < current.model_calls
                || candidate.tool_calls < current.tool_calls
                || candidate.write_calls < current.write_calls
                || candidate.frontend_interactions < current.frontend_interactions
                || candidate.estimated_token_calls < current.estimated_token_calls
                || candidate.compaction_count < current.compaction_count
                || candidate.elapsed_ms < current.elapsed_ms))
}

fn same_cumulative_usage(left: &ExecutionUsageSnapshot, right: &ExecutionUsageSnapshot) -> bool {
    effective_total_tokens(left) == effective_total_tokens(right)
        && left.input_tokens == right.input_tokens
        && left.output_tokens == right.output_tokens
        && left.cached_input_tokens == right.cached_input_tokens
        && left.cache_creation_input_tokens == right.cache_creation_input_tokens
        && left.model_calls == right.model_calls
        && left.tool_calls == right.tool_calls
        && left.write_calls == right.write_calls
        && left.frontend_interactions == right.frontend_interactions
        && left.estimated_token_calls == right.estimated_token_calls
        && left.compaction_count == right.compaction_count
        && left.elapsed_ms == right.elapsed_ms
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiDrivenDiagnosticsSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AgentTaskError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persistence_error: Option<AgentTaskError>,
    pub events_recorded: u64,
}

impl AiDrivenTaskRunSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn completed(
        task: &AgentTaskRun,
        provider: String,
        model: String,
        output: AgentRunOutput,
        artifact: Option<AgentArtifactRef>,
        events_recorded: u64,
    ) -> Self {
        Self {
            limits: task.limits.clone(),
            task_id: task.task_id.clone(),
            run_id: task.run_id.clone(),
            thread_id: task.thread_id.clone(),
            turn_id: task.turn_id.clone(),
            event_cursor: 0,
            request_id: task.request_id.clone(),
            project_id: task.project_id,
            client_session_id: task.client_session_id.clone(),
            status: "completed".to_string(),
            created_at: task.created_at,
            checkpoint_version: None,
            recoverable: false,
            workflow_state: None,
            runtime: AiDrivenRuntimeSnapshot {
                provider: Some(provider),
                model: Some(model),
                usage: output.usage.into(),
                final_text: (!output.final_text.is_empty()).then_some(output.final_text),
                artifact,
                operations: output.operations,
            },
            diagnostics: AiDrivenDiagnosticsSnapshot { error: None, persistence_error: None, events_recorded },
            has_persisted_execution_usage: true,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn terminal_error(
        task: &AgentTaskRun,
        status: AgentTaskStatus,
        provider: Option<String>,
        model: Option<String>,
        usage: ExecutionUsageSnapshot,
        error: AgentTaskError,
        events_recorded: u64,
    ) -> Self {
        Self {
            limits: task.limits.clone(),
            task_id: task.task_id.clone(),
            run_id: task.run_id.clone(),
            thread_id: task.thread_id.clone(),
            turn_id: task.turn_id.clone(),
            event_cursor: 0,
            request_id: task.request_id.clone(),
            project_id: task.project_id,
            client_session_id: task.client_session_id.clone(),
            status: if status == AgentTaskStatus::Failed && error.code == AGENT_RESTART_INTERRUPTED {
                "interrupted".to_string()
            } else {
                frontend_status(status).to_string()
            },
            created_at: task.created_at,
            checkpoint_version: None,
            recoverable: false,
            workflow_state: None,
            runtime: AiDrivenRuntimeSnapshot {
                provider,
                model,
                usage: usage.into(),
                final_text: None,
                artifact: task.last_artifact.clone(),
                operations: Vec::new(),
            },
            diagnostics: AiDrivenDiagnosticsSnapshot { error: Some(error), persistence_error: None, events_recorded },
            has_persisted_execution_usage: true,
        }
    }

    pub(crate) fn from_persisted(task: &AgentTaskRun) -> Self {
        let usage = task.usage.clone().and_then(|value| serde_json::from_value(value).ok());
        let has_persisted_execution_usage = usage.is_some();
        let usage: ExecutionUsageSnapshot = usage.unwrap_or_default();
        Self {
            limits: task.limits.clone(),
            task_id: task.task_id.clone(),
            run_id: task.run_id.clone(),
            thread_id: task.thread_id.clone(),
            turn_id: task.turn_id.clone(),
            event_cursor: 0,
            request_id: task.request_id.clone(),
            project_id: task.project_id,
            client_session_id: task.client_session_id.clone(),
            status: frontend_task_status(task).to_string(),
            created_at: task.created_at,
            checkpoint_version: None,
            recoverable: false,
            workflow_state: None,
            runtime: AiDrivenRuntimeSnapshot { usage: usage.into(), artifact: task.last_artifact.clone(), ..AiDrivenRuntimeSnapshot::default() },
            diagnostics: AiDrivenDiagnosticsSnapshot { error: task.error.clone(), persistence_error: None, events_recorded: 0 },
            has_persisted_execution_usage,
        }
    }

    pub(crate) fn with_persistence_error(mut self, error: AgentTaskError) -> Self {
        self.diagnostics.persistence_error = Some(error);
        self
    }

    pub(crate) fn with_operations(mut self, operations: Vec<CanvasOperationReceipt>) -> Self {
        self.runtime.operations = operations;
        self
    }

    pub(crate) fn with_thread_usage(mut self, usage: ThreadTokenUsage) -> Self {
        self.runtime.usage.thread_usage = Some(usage);
        self
    }

    pub(crate) fn with_optional_latest_usage_snapshot(mut self, usage: Option<Value>) -> Self {
        let terminal = matches!(self.status.as_str(), "completed" | "failed" | "cancelled" | "budgetExhausted" | "interrupted" | "declined");
        self.runtime
            .usage
            .replace_execution_if_fresher(usage, terminal && self.has_persisted_execution_usage);
        self
    }

    pub(crate) fn with_checkpoint_version(mut self, checkpoint_version: Option<u64>) -> Self {
        self.checkpoint_version = checkpoint_version;
        self.recoverable = matches!(self.status.as_str(), "interrupted" | "budgetExhausted") && checkpoint_version.is_some();
        self
    }
}

fn frontend_status(status: AgentTaskStatus) -> &'static str {
    match status {
        AgentTaskStatus::Created => "queued",
        AgentTaskStatus::Running => "running",
        AgentTaskStatus::Verifying => "verifying",
        AgentTaskStatus::WaitingFrontendTool => "waitingFrontendTool",
        AgentTaskStatus::WaitingUserInput => "waitingUserInput",
        AgentTaskStatus::WaitingUserConfirmation => "waitingUserConfirmation",
        AgentTaskStatus::Completed => "completed",
        AgentTaskStatus::Cancelled => "cancelled",
        AgentTaskStatus::Failed => "failed",
        AgentTaskStatus::BudgetExhausted => "budgetExhausted",
    }
}

fn frontend_task_status(task: &AgentTaskRun) -> &'static str {
    if task.status == AgentTaskStatus::Failed && task.error.as_ref().is_some_and(|error| error.code == AGENT_RESTART_INTERRUPTED) {
        "interrupted"
    } else {
        frontend_status(task.status)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AiDrivenTaskIdentity {
    pub request_id: String,
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub task_id: String,
    pub run_id: String,
    pub entry_digest: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AiDrivenTaskRegistrationError {
    AlreadyRunning,
    ProjectBusy,
    ThreadBusy,
    IdentityMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AiDrivenTaskCancelError {
    NotFound,
    ScopeMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AiDrivenTaskSteerError {
    NotFound,
    ScopeMismatch,
    TurnMismatch,
    MailboxFull,
    NotAccepting,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentSteerInput {
    pub input_id: String,
    pub text: String,
    pub accepted_at: i64,
}

#[derive(Debug, Default)]
struct SteeringState {
    accepting: bool,
    inputs: VecDeque<AgentSteerInput>,
    reservations: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct AgentSteeringMailbox {
    state: Arc<Mutex<SteeringState>>,
}

impl AgentSteeringMailbox {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(SteeringState { accepting: true, inputs: VecDeque::new(), reservations: 0 })),
        }
    }

    fn reserve(&self) -> Result<(), AiDrivenTaskSteerError> {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !state.accepting {
            return Err(AiDrivenTaskSteerError::NotAccepting);
        }
        if state.inputs.len() + state.reservations >= STEER_MAILBOX_CAPACITY {
            return Err(AiDrivenTaskSteerError::MailboxFull);
        }
        state.reservations += 1;
        Ok(())
    }

    fn commit_reserved(&self, input: AgentSteerInput) {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        debug_assert!(state.reservations > 0, "steer reservation must be held before commit");
        state.reservations = state.reservations.saturating_sub(1);
        state.inputs.push_back(input);
    }

    fn release_reservation(&self) {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.reservations = state.reservations.saturating_sub(1);
    }

    pub(crate) fn drain(&self) -> Vec<AgentSteerInput> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .inputs
            .drain(..)
            .collect()
    }

    pub(crate) fn close_if_empty(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !state.inputs.is_empty() || state.reservations > 0 {
            return false;
        }
        state.accepting = false;
        true
    }

    pub(crate) fn close(&self) {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).accepting = false;
    }
}

/// `task/steer` 在写 SQLite 前持有的 finishing barrier。
pub(crate) struct AgentSteerReservation {
    mailbox: AgentSteeringMailbox,
    thread_id: String,
    turn_id: String,
    active: bool,
}

impl AgentSteerReservation {
    pub(crate) fn commit(mut self, input: AgentSteerInput) -> AiDrivenTaskSteerReceipt {
        self.mailbox.commit_reserved(input.clone());
        self.active = false;
        AiDrivenTaskSteerReceipt { thread_id: self.thread_id.clone(), turn_id: self.turn_id.clone(), accepted_input_id: input.input_id }
    }
}

impl Drop for AgentSteerReservation {
    fn drop(&mut self) {
        if self.active {
            self.mailbox.release_reservation();
        }
    }
}

#[derive(Debug)]
pub(crate) struct AgentTaskRuntimeChannels {
    pub cancel_rx: watch::Receiver<bool>,
    pub steering: AgentSteeringMailbox,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiDrivenTaskSteerReceipt {
    pub thread_id: String,
    pub turn_id: String,
    pub accepted_input_id: String,
}

struct TaskEntry {
    identity: AiDrivenTaskIdentity,
    cancel_tx: watch::Sender<bool>,
    steering: AgentSteeringMailbox,
    completion_tx: watch::Sender<Option<AiDrivenTaskRunSnapshot>>,
}

pub(crate) struct AiDrivenTaskCompletionWaiter {
    completion_rx: watch::Receiver<Option<AiDrivenTaskRunSnapshot>>,
}

impl AiDrivenTaskCompletionWaiter {
    pub(crate) async fn wait(mut self) -> Option<AiDrivenTaskRunSnapshot> {
        if let Some(snapshot) = self.completion_rx.borrow().clone() {
            return Some(snapshot);
        }
        while self.completion_rx.changed().await.is_ok() {
            if let Some(snapshot) = self.completion_rx.borrow().clone() {
                return Some(snapshot);
            }
        }
        None
    }
}

#[derive(Default)]
pub(crate) struct AiDrivenTaskRegistry {
    tasks: Mutex<HashMap<String, Arc<TaskEntry>>>,
}

impl AiDrivenTaskRegistry {
    pub(crate) fn register(&self, identity: AiDrivenTaskIdentity) -> Result<AgentTaskRuntimeChannels, AiDrivenTaskRegistrationError> {
        let mut tasks = self.tasks.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = tasks.get(&identity.request_id) {
            return if existing.identity == identity {
                Err(AiDrivenTaskRegistrationError::AlreadyRunning)
            } else {
                Err(AiDrivenTaskRegistrationError::IdentityMismatch)
            };
        }
        if tasks
            .values()
            .any(|entry| entry.identity.project_id == identity.project_id && entry.identity.thread_id == identity.thread_id)
        {
            return Err(AiDrivenTaskRegistrationError::ThreadBusy);
        }
        if tasks.values().any(|entry| entry.identity.project_id == identity.project_id) {
            return Err(AiDrivenTaskRegistrationError::ProjectBusy);
        }
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let (completion_tx, _) = watch::channel(None);
        let steering = AgentSteeringMailbox::new();
        tasks.insert(
            identity.request_id.clone(),
            Arc::new(TaskEntry { identity, cancel_tx, steering: steering.clone(), completion_tx }),
        );
        Ok(AgentTaskRuntimeChannels { cancel_rx, steering })
    }

    #[cfg(test)]
    pub(crate) fn steer(
        &self,
        thread_id: &str,
        expected_turn_id: &str,
        project_id: i64,
        input: AgentSteerInput,
    ) -> Result<AiDrivenTaskSteerReceipt, AiDrivenTaskSteerError> {
        let run_id = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .find(|entry| entry.identity.thread_id == thread_id)
            .map(|entry| entry.identity.run_id.clone())
            .ok_or(AiDrivenTaskSteerError::NotFound)?;
        let reservation = self.reserve_steer(project_id, thread_id, expected_turn_id, &run_id)?;
        Ok(reservation.commit(input))
    }

    pub(crate) fn reserve_steer(
        &self,
        project_id: i64,
        thread_id: &str,
        expected_turn_id: &str,
        run_id: &str,
    ) -> Result<AgentSteerReservation, AiDrivenTaskSteerError> {
        let entry = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .find(|entry| entry.identity.run_id == run_id)
            .cloned()
            .ok_or(AiDrivenTaskSteerError::NotFound)?;
        if entry.identity.project_id != project_id {
            return Err(AiDrivenTaskSteerError::ScopeMismatch);
        }
        if entry.identity.turn_id != expected_turn_id {
            return Err(AiDrivenTaskSteerError::TurnMismatch);
        }
        if entry.identity.thread_id != thread_id || entry.identity.run_id != run_id {
            return Err(AiDrivenTaskSteerError::ScopeMismatch);
        }
        entry.steering.reserve()?;
        Ok(AgentSteerReservation {
            mailbox: entry.steering.clone(),
            thread_id: entry.identity.thread_id.clone(),
            turn_id: entry.identity.turn_id.clone(),
            active: true,
        })
    }

    pub(crate) fn cancel(
        &self,
        project_id: i64,
        thread_id: &str,
        turn_id: &str,
        run_id: &str,
    ) -> Result<AiDrivenTaskCompletionWaiter, AiDrivenTaskCancelError> {
        let entry = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .find(|entry| entry.identity.run_id == run_id)
            .cloned()
            .ok_or(AiDrivenTaskCancelError::NotFound)?;
        if entry.identity.project_id != project_id
            || entry.identity.thread_id != thread_id
            || entry.identity.turn_id != turn_id
            || entry.identity.run_id != run_id
        {
            return Err(AiDrivenTaskCancelError::ScopeMismatch);
        }
        let _ = entry.cancel_tx.send(true);
        Ok(AiDrivenTaskCompletionWaiter { completion_rx: entry.completion_tx.subscribe() })
    }

    pub(crate) fn complete(&self, snapshot: AiDrivenTaskRunSnapshot) -> bool {
        let entry = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&snapshot.request_id)
            .cloned();
        if let Some(entry) = entry {
            if entry.identity.project_id != snapshot.project_id
                || entry.identity.thread_id != snapshot.thread_id
                || entry.identity.turn_id != snapshot.turn_id
                || entry.identity.task_id != snapshot.task_id
                || entry.identity.run_id != snapshot.run_id
            {
                return false;
            }
            entry.completion_tx.send_replace(Some(snapshot));
            true
        } else {
            false
        }
    }

    pub(crate) fn unregister(&self, identity: &AiDrivenTaskIdentity) {
        let mut tasks = self.tasks.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if tasks.get(&identity.request_id).is_some_and(|entry| entry.identity == *identity) {
            tasks.remove(&identity.request_id);
        }
    }

    pub(crate) fn is_active(&self, project_id: i64, thread_id: &str, turn_id: &str, run_id: &str) -> bool {
        self.tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .any(|entry| {
                entry.identity.project_id == project_id
                    && entry.identity.thread_id == thread_id
                    && entry.identity.turn_id == turn_id
                    && entry.identity.run_id == run_id
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_usage_keeps_budget_fields_flat_and_adds_nested_thread_usage() {
        let usage = AiDrivenRuntimeUsageSnapshot {
            execution: ExecutionUsageSnapshot { input_tokens: 7, ..Default::default() },
            thread_usage: Some(ThreadTokenUsage { input_tokens: 17, ..Default::default() }),
        };
        let value = serde_json::to_value(usage).expect("serialize runtime usage");
        assert_eq!(value["inputTokens"], 7);
        assert_eq!(value["threadUsage"]["inputTokens"], 17);
        assert!(value.get("execution").is_none());
    }

    #[test]
    fn missing_run_usage_event_preserves_initialized_budget_and_later_snapshot_refreshes_it() {
        let mut usage = AiDrivenRuntimeUsageSnapshot {
            execution: ExecutionUsageSnapshot { remaining_total_tokens: Some(4_000), ..Default::default() },
            thread_usage: None,
        };

        usage.replace_execution_if_fresher(None, false);
        assert_eq!(usage.execution.remaining_total_tokens, Some(4_000));

        usage.replace_execution_if_fresher(
            Some(serde_json::json!({
                "inputTokens": 150,
                "outputTokens": 30,
                "totalTokens": 180,
                "remainingTotalTokens": 3_250,
            })),
            false,
        );
        assert_eq!(usage.execution.input_tokens, 150);
        assert_eq!(usage.execution.remaining_total_tokens, Some(3_250));
    }

    #[test]
    fn stale_event_cannot_replace_a_newer_terminal_task_usage_snapshot() {
        let task = AgentTaskRun {
            task_id: "task-1".to_string(),
            run_id: "run-1".to_string(),
            request_id: "request-1".to_string(),
            entry_digest: "sha256:test".to_string(),
            project_id: 1,
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            client_session_id: "session-1".to_string(),
            task_type: "general".to_string(),
            status: AgentTaskStatus::Failed,
            limits: Default::default(),
            pending: None,
            usage: Some(serde_json::json!({
                "inputTokens": 200,
                "outputTokens": 50,
                "totalTokens": 250,
                "remainingTotalTokens": 0,
                "modelCalls": 2,
            })),
            last_artifact: None,
            error: None,
            project_snapshot_ref: None,
            rig_version: None,
            rig_snapshot_ref: None,
            created_at: 1,
            started_at: Some(1),
            updated_at: 2,
            ended_at: Some(2),
        };

        let snapshot = AiDrivenTaskRunSnapshot::from_persisted(&task).with_optional_latest_usage_snapshot(Some(serde_json::json!({
            "inputTokens": 150,
            "outputTokens": 20,
            "totalTokens": 170,
            "remainingTotalTokens": 80,
            "modelCalls": 1,
        })));

        assert_eq!(snapshot.runtime.usage.execution.total_tokens, 250);
        assert_eq!(snapshot.runtime.usage.execution.remaining_total_tokens, Some(0));
    }

    fn identity(request: &str, project: i64, thread: &str, turn: &str) -> AiDrivenTaskIdentity {
        AiDrivenTaskIdentity {
            request_id: request.to_string(),
            project_id: project,
            thread_id: thread.to_string(),
            turn_id: turn.to_string(),
            task_id: format!("task-{request}"),
            run_id: format!("run-{request}"),
            entry_digest: format!("sha256:{request}"),
        }
    }

    #[test]
    fn project_and_thread_accept_only_one_active_turn() {
        let registry = AiDrivenTaskRegistry::default();
        registry.register(identity("one", 1, "thread-1", "turn-1")).unwrap();
        assert_eq!(
            registry.register(identity("same-thread", 1, "thread-1", "turn-2")).unwrap_err(),
            AiDrivenTaskRegistrationError::ThreadBusy
        );
        assert_eq!(
            registry.register(identity("two", 1, "thread-2", "turn-2")).unwrap_err(),
            AiDrivenTaskRegistrationError::ProjectBusy
        );
        assert!(registry.register(identity("three", 2, "thread-3", "turn-3")).is_ok());
    }

    #[test]
    fn steering_validates_real_project_thread_and_turn_scope() {
        let registry = AiDrivenTaskRegistry::default();
        let runtime = registry.register(identity("one", 1, "thread-1", "turn-1")).unwrap();
        let input = AgentSteerInput { input_id: "input-1".to_string(), text: "continue".to_string(), accepted_at: 1 };
        assert_eq!(registry.steer("thread-1", "turn-1", 2, input.clone()).unwrap_err(), AiDrivenTaskSteerError::ScopeMismatch);
        assert_eq!(registry.steer("thread-1", "other-turn", 1, input.clone()).unwrap_err(), AiDrivenTaskSteerError::TurnMismatch);
        registry.steer("thread-1", "turn-1", 1, input.clone()).unwrap();
        assert_eq!(runtime.steering.drain(), vec![input]);
    }

    #[test]
    fn steer_reservation_blocks_finishing_until_sqlite_acceptance_commits() {
        let registry = AiDrivenTaskRegistry::default();
        let runtime = registry.register(identity("one", 1, "thread-1", "turn-1")).unwrap();
        let reservation = registry
            .reserve_steer(1, "thread-1", "turn-1", "run-one")
            .expect("reserve steer");
        assert!(!runtime.steering.close_if_empty());

        let input = AgentSteerInput { input_id: "input-1".to_string(), text: "continue".to_string(), accepted_at: 1 };
        reservation.commit(input.clone());
        assert!(!runtime.steering.close_if_empty());
        assert_eq!(runtime.steering.drain(), vec![input]);
        assert!(runtime.steering.close_if_empty());
    }

    #[test]
    fn dropped_steer_reservation_releases_finishing_barrier() {
        let registry = AiDrivenTaskRegistry::default();
        let runtime = registry.register(identity("one", 1, "thread-1", "turn-1")).unwrap();
        let reservation = registry.reserve_steer(1, "thread-1", "turn-1", "run-one").unwrap();
        assert!(!runtime.steering.close_if_empty());
        drop(reservation);
        assert!(runtime.steering.close_if_empty());
    }
}
