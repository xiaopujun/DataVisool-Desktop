/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Rig Agent 的统一预算、取消和持久化 Timeline Hook。
mod compaction_events;
mod persistence;
mod recovery;
#[cfg(test)]
mod recovery_tests;
mod tool_events;
use super::context_compaction::ContextTokenTracker;
use super::item_state::{ActiveAgentMessage, AGENT_MESSAGE_DELTA_BATCH_CHARS, AGENT_MESSAGE_DELTA_BATCH_INTERVAL};
use super::limits::{provider_context_input_tokens, AgentExecutionError, AgentExecutionErrorCode, ExecutionBudget, ToolCallKind};
use super::progress_guard::ProgressCallMetadata;
use super::progress_guard::ProgressGuard;
use super::tool_concurrency::{canonical_arguments, ToolBatchCompletionGuard, ToolBatchTracker, ToolCallScheduler, ToolConcurrencyPolicy};
use super::tool_failure_policy::ToolFailureTracker;
use super::tool_route::{AgentRoutePhase, AgentToolRoute};
use crate::backend::ai_driven::provider::ContextTraceIdentity;
use crate::backend::ai_driven::task::{
    AgentPendingKind, AgentPendingSnapshot, AgentTaskEventPublisher, AgentTaskRepository, AgentTaskRun, AgentTaskStatus, AgentWorkingCheckpointState,
    CompletionRequirementsState, TaskRepositoryError,
};
use crate::backend::ai_driven::thread::{new_item_id, AgentItem, ItemStatus};
use crate::backend::ai_driven::tools::canvas::{
    CREATE_CANVAS_COMPONENTS_TOOL, DELETE_CANVAS_COMPONENTS_TOOL, GET_CANVAS_OPERATION_STATUS_TOOL, GET_CURRENT_CANVAS_SELECTION_TOOL,
    PATCH_CANVAS_COMPONENTS_TOOL,
};
use crate::backend::ai_driven::tools::component_catalog::GET_COMPONENT_CATALOG_TOOL;
use crate::backend::ai_driven::tools::component_data_contract::GET_COMPONENT_DATA_CONTRACT_TOOL;
use crate::backend::ai_driven::tools::component_resolution::RESOLVE_COMPONENT_SPECS_TOOL;
use crate::backend::ai_driven::tools::file::{
    APPLY_WORKSPACE_PATCH_TOOL, LIST_WORKSPACE_FILES_TOOL, READ_WORKSPACE_FILE_TOOL, SEARCH_WORKSPACE_FILES_TOOL,
};
use crate::backend::ai_driven::tools::interaction::{INSPECT_CURRENT_PAGE_TOOL, REQUEST_USER_INPUT_TOOL};
use crate::backend::ai_driven::tools::runtime::AgentToolInvocationTracker;
use crate::backend::ai_driven::tools::selection::REQUEST_COMPONENT_SELECTION_TOOL;
use crate::backend::ai_driven::tools::task_intent::UPDATE_PLAN_TOOL;
use crate::backend::utils::{debug_log_json, sanitize_debug_log_text, sha256_hex};
use persistence::PersistenceCoordinator;
use rig_core::agent::{AgentHook, Flow, HookContext, StepEvent, StepEventKind};
use rig_core::completion::{CompletionModel, Usage};
use rig_core::message::AssistantContent;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const DEFAULT_INVALID_TOOL_RETRY_LIMIT: u8 = 1;
const MAX_INVALID_TOOL_RETRY_LIMIT: u8 = 1;
struct ActiveToolCall {
    started_at: Instant,
    kind: ToolCallKind,
    item_id: String,
    row_version: u64,
    step_index: u64,
    route_revision: u64,
    admitted_phase: AgentRoutePhase,
    admission_source: &'static str,
    policy: ToolConcurrencyPolicy,
    batch_position: u64,
    deferred_progress: Option<ProgressCallMetadata>,
    batch_completion: ToolBatchCompletionGuard,
    arguments: String,
    authorization_revision: u64,
}

/// 每次持久化 TaskRun 创建一个实例，并直接交给 Rig `AgentBuilder::add_hook`。
#[derive(Clone)]
pub(crate) struct AgentRuntimeHook {
    budget: ExecutionBudget,
    repository: Arc<dyn AgentTaskRepository>,
    task: AgentTaskRun,
    model_calls: Arc<Mutex<HashMap<usize, Instant>>>,
    tool_calls: Arc<Mutex<HashMap<String, ActiveToolCall>>>,
    agent_messages: Arc<Mutex<HashMap<usize, ActiveAgentMessage>>>,
    invalid_tool_retries: Arc<AtomicU8>,
    invalid_tool_retry_limit: u8,
    event_publisher: Option<AgentTaskEventPublisher>,
    invocation_tracker: AgentToolInvocationTracker,
    tool_failure_tracker: Arc<Mutex<ToolFailureTracker>>,
    progress_guard: ProgressGuard,
    tool_route: Option<AgentToolRoute>,
    persistence: PersistenceCoordinator,
    completion_requirements: CompletionRequirementsState,
    context_token_tracker: ContextTokenTracker,
    tool_scheduler: Arc<ToolCallScheduler>,
    tool_batch: ToolBatchTracker,
    compacted_operations: Arc<Mutex<std::collections::BTreeSet<String>>>,
}

impl AgentRuntimeHook {
    pub(crate) fn new(budget: ExecutionBudget, repository: Arc<dyn AgentTaskRepository>, task: AgentTaskRun) -> Self {
        let progress_guard = ProgressGuard::new(budget.limits());
        let persistence = PersistenceCoordinator::new(task.run_id.clone(), task.turn_id.clone());
        Self {
            budget,
            repository,
            task,
            model_calls: Arc::new(Mutex::new(HashMap::new())),
            tool_calls: Arc::new(Mutex::new(HashMap::new())),
            agent_messages: Arc::new(Mutex::new(HashMap::new())),
            invalid_tool_retries: Arc::new(AtomicU8::new(0)),
            invalid_tool_retry_limit: DEFAULT_INVALID_TOOL_RETRY_LIMIT,
            event_publisher: None,
            invocation_tracker: AgentToolInvocationTracker::default(),
            tool_failure_tracker: Arc::new(Mutex::new(ToolFailureTracker::default())),
            progress_guard,
            tool_route: None,
            persistence,
            completion_requirements: CompletionRequirementsState::default(),
            context_token_tracker: ContextTokenTracker::default(),
            tool_scheduler: Arc::new(ToolCallScheduler::new()),
            tool_batch: ToolBatchTracker::default(),
            compacted_operations: Arc::default(),
        }
    }

    /// Rig 无效 Tool 调用最多只允许一次模型自修复；传入更大值也会收紧到一次。
    pub(crate) fn with_invalid_tool_retry_limit(mut self, limit: u8) -> Self {
        self.invalid_tool_retry_limit = limit.min(MAX_INVALID_TOOL_RETRY_LIMIT);
        self
    }

    pub(crate) fn with_event_publisher(mut self, publisher: AgentTaskEventPublisher) -> Self {
        self.event_publisher = Some(publisher);
        self
    }

    pub(crate) fn with_invocation_tracker(mut self, invocation_tracker: AgentToolInvocationTracker) -> Self {
        self.invocation_tracker = invocation_tracker;
        self
    }

    pub(crate) fn with_completion_requirements(mut self, completion_requirements: CompletionRequirementsState) -> Self {
        self.completion_requirements = completion_requirements;
        self
    }

    pub(crate) fn with_tool_route(mut self, tool_route: AgentToolRoute) -> Self {
        self.tool_route = Some(tool_route);
        self
    }

    /// 为恢复中的 Task 注入已持久化的组件目录回执。
    pub(crate) fn with_cached_component_catalog(self, result: Option<String>) -> Self {
        if let Some(result) = result {
            self.progress_guard.restore_component_catalog(result);
        }
        self
    }

    pub(crate) fn with_context_token_tracker(mut self, context_token_tracker: ContextTokenTracker) -> Self {
        self.context_token_tracker = context_token_tracker;
        self
    }

    pub(crate) fn with_tool_scheduler(mut self, tool_scheduler: Arc<ToolCallScheduler>) -> Self {
        self.tool_scheduler = tool_scheduler;
        self
    }

    pub(crate) fn usage_snapshot(&self) -> super::ExecutionUsageSnapshot {
        self.budget.usage_snapshot()
    }

    pub(crate) fn record_context(&self, active: u64, capacity: u64, source: &str) {
        self.budget.record_context(active, capacity, source);
    }

    pub(crate) fn record_compaction(&self, before: u64, after: u64) {
        self.budget.record_compaction(before, after);
    }

    pub(crate) fn take_budget_warning(&self) -> Option<String> {
        self.budget.take_budget_warning()
    }

    pub(crate) async fn record_budget_warning(&self, ctx: &HookContext, message: &str) -> Flow {
        let payload = self.payload(
            ctx,
            json!({
                "message": message,
                "usage": self.budget.usage_snapshot(),
            }),
        );
        let flow = self.persist_or_terminate("agent.execution.budget_warning", None, payload).await;
        if matches!(flow, Flow::Continue) && self.persist_run_recovery().await.is_err() {
            return Flow::terminate("agent.task_checkpoint_failed");
        }
        flow
    }

    pub(crate) fn context_trace_identity(&self) -> ContextTraceIdentity {
        ContextTraceIdentity {
            project_id: self.task.project_id,
            thread_id: self.task.thread_id.clone(),
            task_id: self.task.task_id.clone(),
            turn_id: self.task.turn_id.clone(),
            run_id: self.task.run_id.clone(),
            request_id: self.task.request_id.clone(),
        }
    }

    async fn on_completion_call(&self, ctx: &HookContext, step_index: usize) -> Flow {
        if let Err(error) = self.budget.start_model_call() {
            return self.terminate_for_budget(ctx, error).await;
        }
        self.lock_model_calls().insert(step_index, Instant::now());
        if let Some(snapshot) = self.tool_route.as_ref().and_then(AgentToolRoute::active_snapshot) {
            self.tool_batch.reset(snapshot.generation());
        }
        tracing::debug!(
            target: "agent.model",
            event = "model_call_started",
            call_sequence = self.budget.usage_snapshot().model_calls,
            project_id = self.task.project_id,
            thread_id = %self.task.thread_id,
            task_id = %self.task.task_id,
            turn_id = %self.task.turn_id,
            run_id = %self.task.run_id,
            request_id = %self.task.request_id,
            step_index,
            estimated_input_tokens = ?self.context_token_tracker.get(step_index),
            usage = %debug_log_json(&self.budget.usage_snapshot()),
            "Agent model call started"
        );
        let payload = self.payload(ctx, json!({ "stepIndex": step_index, "usage": self.budget.usage_snapshot() }));
        self.persist_or_terminate("agent.model_call.started", None, payload).await
    }

    async fn on_text_delta(&self, ctx: &HookContext, step_index: usize, delta: &str) -> Flow {
        let new_item = {
            let mut messages = self.lock_agent_messages();
            if messages.contains_key(&step_index) {
                None
            } else {
                let item_id = new_item_id();
                messages.insert(step_index, ActiveAgentMessage::new(item_id.clone()));
                Some(item_id)
            }
        };
        if let Some(item_id) = new_item {
            let snapshot = item_snapshot(
                &item_id,
                "agentMessage",
                ItemStatus::InProgress,
                1,
                json!({ "stepIndex": step_index, "text": "", "truncated": false }),
            );
            if self
                .start_item(ctx, item_id.clone(), None, "agentMessage", ItemStatus::InProgress, snapshot)
                .await
                .is_err()
            {
                self.lock_agent_messages().remove(&step_index);
                return Flow::terminate("agent.item_persistence_failed");
            }
        }

        let pending = {
            let mut messages = self.lock_agent_messages();
            let Some(state) = messages.get_mut(&step_index) else {
                return Flow::terminate("agent.item_state_missing");
            };
            if !state.push(delta) {
                return Flow::cont();
            }
            let should_flush = state.pending_chars >= AGENT_MESSAGE_DELTA_BATCH_CHARS
                || state.last_flush.elapsed() >= AGENT_MESSAGE_DELTA_BATCH_INTERVAL
                || state.text.chars().count() == delta.chars().count();
            should_flush.then(|| {
                let pending_delta = state.take_pending();
                (state.item_id.clone(), state.row_version, state.text.clone(), state.truncated, pending_delta)
            })
        };
        let Some((item_id, row_version, text, truncated, pending_delta)) = pending else {
            return Flow::cont();
        };
        match self
            .apply_message_delta(ctx, step_index, &item_id, row_version, &text, truncated, pending_delta)
            .await
        {
            Ok(item) => {
                if let Some(state) = self.lock_agent_messages().get_mut(&step_index) {
                    state.row_version = item.row_version;
                }
                Flow::cont()
            }
            Err(_) => Flow::terminate("agent.item_persistence_failed"),
        }
    }

    async fn complete_agent_message(&self, ctx: &HookContext, step_index: usize, status: ItemStatus) -> Flow {
        let Some(mut state) = self.lock_agent_messages().remove(&step_index) else {
            return Flow::cont();
        };
        let pending_delta = state.take_pending();
        if !pending_delta.is_empty() {
            match self
                .apply_message_delta(ctx, step_index, &state.item_id, state.row_version, &state.text, state.truncated, pending_delta)
                .await
            {
                Ok(item) => state.row_version = item.row_version,
                Err(_) => return Flow::terminate("agent.item_persistence_failed"),
            }
        }
        let snapshot = item_snapshot(
            &state.item_id,
            "agentMessage",
            status,
            state.row_version + 1,
            json!({ "stepIndex": step_index, "text": state.text, "truncated": state.truncated }),
        );
        let event_kind = if status == ItemStatus::Completed {
            "item.completed"
        } else {
            "item.failed"
        };
        match self
            .apply_item(ctx, &state.item_id, state.row_version, status, snapshot, event_kind, now_ms())
            .await
        {
            Ok(_) => Flow::cont(),
            Err(_) => Flow::terminate("agent.item_persistence_failed"),
        }
    }

    async fn apply_message_delta(
        &self,
        ctx: &HookContext,
        step_index: usize,
        item_id: &str,
        row_version: u64,
        text: &str,
        truncated: bool,
        delta: String,
    ) -> Result<AgentItem, TaskRepositoryError> {
        let snapshot = item_snapshot(
            item_id,
            "agentMessage",
            ItemStatus::InProgress,
            row_version + 1,
            json!({ "stepIndex": step_index, "text": text, "truncated": truncated }),
        );
        let event_payload = self.payload(
            ctx,
            json!({
                "itemId": item_id,
                "itemType": "agentMessage",
                "delta": delta,
                "item": snapshot,
            }),
        );
        self.apply_item_with_payload(item_id, row_version, ItemStatus::InProgress, snapshot, "item.delta", event_payload, None)
            .await
    }

    async fn on_model_turn_finished(
        &self,
        ctx: &HookContext,
        step_index: usize,
        provider_usage: Usage,
        estimated_output_tokens: u64,
        content: &str,
    ) -> Flow {
        let duration = self.take_model_duration(step_index);
        let item_persisted = self.complete_agent_message(ctx, step_index, ItemStatus::Completed).await;
        if !matches!(item_persisted, Flow::Continue) {
            return item_persisted;
        }
        let estimated_input_tokens = self.context_token_tracker.take(step_index).unwrap_or_default();
        let usage_reported = provider_usage.has_values();
        let effective_input_tokens = if usage_reported {
            provider_context_input_tokens(&provider_usage, provider_usage.input_tokens)
        } else {
            estimated_input_tokens
        };
        let effective_output_tokens = if usage_reported {
            provider_usage.output_tokens
        } else {
            estimated_output_tokens
        };
        let estimated = !usage_reported;
        let budget_result = self
            .budget
            .finish_model_call_with_usage(&provider_usage, estimated_input_tokens, estimated_output_tokens, estimated);
        tracing::debug!(
            target: "agent.model",
            event = "model_call_completed",
            call_sequence = self.budget.usage_snapshot().model_calls,
            project_id = self.task.project_id,
            thread_id = %self.task.thread_id,
            task_id = %self.task.task_id,
            turn_id = %self.task.turn_id,
            run_id = %self.task.run_id,
            request_id = %self.task.request_id,
            step_index,
            duration_ms = duration_ms(duration),
            input_tokens = effective_input_tokens,
            output_tokens = effective_output_tokens,
            active_context_tokens = effective_input_tokens,
            provider_input_tokens = provider_usage.input_tokens,
            provider_output_tokens = provider_usage.output_tokens,
            provider_total_tokens = provider_usage.total_tokens,
            cached_input_tokens = provider_usage.cached_input_tokens,
            cache_creation_input_tokens = provider_usage.cache_creation_input_tokens,
            tool_use_prompt_tokens = provider_usage.tool_use_prompt_tokens,
            reasoning_tokens = provider_usage.reasoning_tokens,
            usage_reported,
            estimated,
            content = %content,
            usage = %debug_log_json(&self.budget.usage_snapshot()),
            "Agent model response received"
        );
        let payload = self.payload(
            ctx,
            json!({
                "stepIndex": step_index,
                "durationMs": duration_ms(duration),
                "inputTokens": effective_input_tokens,
                "outputTokens": effective_output_tokens,
                "activeContextTokens": effective_input_tokens,
                "providerInputTokens": provider_usage.input_tokens,
                "providerOutputTokens": provider_usage.output_tokens,
                "providerTotalTokens": provider_usage.total_tokens,
                "cachedInputTokens": provider_usage.cached_input_tokens,
                "cacheCreationInputTokens": provider_usage.cache_creation_input_tokens,
                "toolUsePromptTokens": provider_usage.tool_use_prompt_tokens,
                "reasoningTokens": provider_usage.reasoning_tokens,
                "usageReported": usage_reported,
                "estimated": estimated,
                "usage": self.budget.usage_snapshot(),
            }),
        );
        let persisted = self.persist_or_terminate("agent.model_call.completed", None, payload).await;
        if !matches!(persisted, Flow::Continue) {
            return persisted;
        }
        match budget_result {
            Ok(()) => Flow::cont(),
            Err(error) => self.terminate_for_budget(ctx, error).await,
        }
    }

    async fn on_invalid_tool_call(
        &self,
        ctx: &HookContext,
        tool_name: &str,
        tool_call_id: Option<&str>,
        internal_call_id: Option<&str>,
        available_tools: &[String],
    ) -> Flow {
        let step_index = ctx.turn();
        let available_tool_count = available_tools.len();
        tracing::debug!(
            target: "agent.tool",
            event = "invalid_tool_call",
            project_id = self.task.project_id,
            thread_id = %self.task.thread_id,
            task_id = %self.task.task_id,
            turn_id = %self.task.turn_id,
            run_id = %self.task.run_id,
            request_id = %self.task.request_id,
            step_index,
            tool_name,
            tool_call_id,
            internal_call_id,
            available_tool_count,
            "Agent invalid tool call"
        );
        let duration = self.take_model_duration(step_index);
        let item_persisted = self.complete_agent_message(ctx, step_index, ItemStatus::Failed).await;
        if !matches!(item_persisted, Flow::Continue) {
            return item_persisted;
        }
        let estimated_input_tokens = self.context_token_tracker.take(step_index).unwrap_or_default();
        if let Err(error) = self.budget.finish_model_call_with_estimate(estimated_input_tokens, 0, true) {
            return self.terminate_for_budget(ctx, error).await;
        }
        tracing::debug!(
            target: "agent.model",
            event = "model_call_completed",
            project_id = self.task.project_id,
            thread_id = %self.task.thread_id,
            task_id = %self.task.task_id,
            turn_id = %self.task.turn_id,
            run_id = %self.task.run_id,
            request_id = %self.task.request_id,
            step_index,
            duration_ms = duration_ms(duration),
            input_tokens = estimated_input_tokens,
            output_tokens = 0,
            estimated = true,
            outcome = "invalid_tool_call",
            usage = %debug_log_json(&self.budget.usage_snapshot()),
            "Agent model call ended with invalid tool call"
        );
        let model_payload = self.payload(
            ctx,
            json!({
                "stepIndex": step_index,
                "durationMs": duration_ms(duration),
                "outcome": "invalidToolCall",
                "tokenUsageReported": false,
                "estimated": true,
                "inputTokens": estimated_input_tokens,
                "usage": self.budget.usage_snapshot(),
            }),
        );
        let persisted = self
            .persist_or_terminate("agent.model_call.completed", None, model_payload)
            .await;
        if !matches!(persisted, Flow::Continue) {
            return persisted;
        }

        let not_advertised_in_step = self
            .tool_route
            .as_ref()
            .and_then(AgentToolRoute::active_snapshot)
            .is_some_and(|snapshot| !snapshot.allowed_tools.contains(tool_name));
        let retry_number = self.invalid_tool_retries.fetch_add(1, Ordering::AcqRel);
        let will_retry = retry_number < self.invalid_tool_retry_limit;
        let payload = self.payload(
            ctx,
            json!({
                "stepIndex": step_index,
                "toolName": tool_name,
                "toolCallId": tool_call_id,
                "internalCallId": internal_call_id,
                "availableToolCount": available_tool_count,
                "availableTools": available_tools,
                "code": not_advertised_in_step.then_some("agent.tool.not_advertised_in_step"),
                "willRetry": will_retry,
                "repairAttempt": if will_retry { Some(retry_number + 1) } else { None },
            }),
        );
        let persisted = self.persist_or_terminate("agent.tool_call.invalid", None, payload).await;
        if !matches!(persisted, Flow::Continue) {
            return persisted;
        }
        if will_retry {
            // 拒绝本次调用并将错误作为 Tool 回执交回模型，不执行或自动映射未知 Tool。
            Flow::retry(
                json!({
                    "code": if not_advertised_in_step { "agent.tool.not_advertised_in_step" } else { "agent.tool.unknown" },
                    "toolName": tool_name,
                    "availableTools": available_tools,
                    "retryable": true,
                    "message": "该 Tool 名称无效或不在本轮允许列表中，本次调用未执行。请仅使用当前提供的 Tool 并按其规格重新生成参数；若已有回执足以确认完成，可直接给出结果。",
                })
                .to_string(),
            )
        } else if not_advertised_in_step {
            Flow::terminate("agent.tool.not_advertised_in_step")
        } else {
            Flow::fail()
        }
    }

    async fn persist_plan_item(&self, ctx: &HookContext, internal_call_id: &str, plan: &Value) -> Result<(), TaskRepositoryError> {
        let item_id = new_item_id();
        let extra = json!({
            "explanation": plan.get("explanation").cloned().unwrap_or(Value::Null),
            "steps": plan.get("plan").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
        });
        let started = item_snapshot(&item_id, "plan", ItemStatus::InProgress, 1, extra.clone());
        let item = self
            .start_item(ctx, item_id.clone(), Some(format!("plan:{internal_call_id}")), "plan", ItemStatus::InProgress, started)
            .await?;
        let completed = item_snapshot(&item_id, "plan", ItemStatus::Completed, item.row_version + 1, extra);
        self.apply_item(ctx, &item_id, item.row_version, ItemStatus::Completed, completed, "item.completed", now_ms())
            .await?;
        Ok(())
    }

    pub(crate) async fn terminate_for_budget(&self, ctx: &HookContext, error: AgentExecutionError) -> Flow {
        if error.code.is_run_budget() && self.persist_run_recovery().await.is_err() {
            return Flow::terminate("agent.task_checkpoint_failed");
        }
        let event_kind = if error.code == AgentExecutionErrorCode::Cancelled {
            "agent.cancelled"
        } else if error.code.is_run_budget() {
            "agent.budget_exhausted"
        } else {
            "agent.failed"
        };
        tracing::debug!(
            target: "agent.run",
            event = "run_termination_requested",
            project_id = self.task.project_id,
            thread_id = %self.task.thread_id,
            task_id = %self.task.task_id,
            turn_id = %self.task.turn_id,
            run_id = %self.task.run_id,
            request_id = %self.task.request_id,
            code = error.code.as_str(),
            message = %sanitize_debug_log_text(&error.message),
            usage = %debug_log_json(&error.usage),
            "Agent execution budget terminated the run"
        );
        let payload = self.payload(ctx, json!({ "code": error.code.as_str(), "message": error.message, "usage": error.usage }));
        let persisted = self.persist_or_terminate(event_kind, None, payload).await;
        if matches!(persisted, Flow::Continue) {
            Flow::terminate(error.code.as_str())
        } else {
            persisted
        }
    }

    async fn persist_or_terminate(&self, kind: &str, item_id: Option<String>, payload: Value) -> Flow {
        match self.append_event(kind, item_id, payload).await {
            Ok(()) => {
                if self.tool_route.is_some()
                    && matches!(
                        kind,
                        "agent.model_call.started"
                            | "agent.model_call.completed"
                            | "agent.tool_call.completed"
                            | "agent.context_compaction.started"
                            | "agent.context_compaction.completed"
                    )
                {
                    if self.update_working_checkpoint(now_ms(), |_| {}).await.is_err() {
                        return Flow::terminate("agent.task_checkpoint_failed");
                    }
                }
                Flow::cont()
            }
            Err(_) => Flow::terminate("agent.event_persistence_failed"),
        }
    }

    async fn append_event(&self, kind: &str, item_id: Option<String>, payload: Value) -> Result<(), TaskRepositoryError> {
        persistence::append_event(self, kind, item_id, payload).await
    }

    async fn start_item(
        &self,
        ctx: &HookContext,
        item_id: String,
        logical_key: Option<String>,
        item_type: &str,
        status: ItemStatus,
        snapshot: Value,
    ) -> Result<AgentItem, TaskRepositoryError> {
        persistence::start_item(self, ctx, item_id, logical_key, item_type, status, snapshot).await
    }

    async fn apply_item(
        &self,
        ctx: &HookContext,
        item_id: &str,
        row_version: u64,
        status: ItemStatus,
        snapshot: Value,
        event_kind: &str,
        updated_at: i64,
    ) -> Result<AgentItem, TaskRepositoryError> {
        persistence::apply_item(self, ctx, item_id, row_version, status, snapshot, event_kind, updated_at).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_item_with_payload(
        &self,
        item_id: &str,
        row_version: u64,
        status: ItemStatus,
        snapshot: Value,
        event_kind: &str,
        event_payload: Value,
        completed_at: Option<i64>,
    ) -> Result<AgentItem, TaskRepositoryError> {
        persistence::apply_item_with_payload(self, item_id, row_version, status, snapshot, event_kind, event_payload, completed_at).await
    }

    async fn save_task_status(
        &self,
        status: AgentTaskStatus,
        pending: Option<AgentPendingSnapshot>,
        updated_at: i64,
    ) -> Result<(), TaskRepositoryError> {
        persistence::save_task_status(self, status, pending, updated_at).await
    }

    pub(super) async fn update_working_checkpoint(
        &self,
        updated_at: i64,
        update: impl FnOnce(&mut AgentWorkingCheckpointState),
    ) -> Result<(), TaskRepositoryError> {
        persistence::update_working_checkpoint(self, updated_at, update).await
    }

    fn publish(&self, event: crate::backend::ai_driven::thread::AgentTimelineEvent) {
        if let Some(publisher) = &self.event_publisher {
            publisher.publish(event);
        }
    }

    fn payload(&self, ctx: &HookContext, payload: Value) -> Value {
        let mut payload = match payload {
            Value::Object(payload) => payload,
            _ => Map::new(),
        };
        payload.insert("runId".to_string(), Value::String(self.task.run_id.clone()));
        payload.insert("requestId".to_string(), Value::String(self.task.request_id.clone()));
        payload.insert("rigRunId".to_string(), Value::String(ctx.run_id().as_str().to_string()));
        payload.insert("callSequence".to_string(), Value::from(self.budget.usage_snapshot().model_calls));
        Value::Object(payload)
    }

    fn take_model_duration(&self, step_index: usize) -> Duration {
        self.lock_model_calls()
            .remove(&step_index)
            .map(|started_at| started_at.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    fn lock_model_calls(&self) -> MutexGuard<'_, HashMap<usize, Instant>> {
        self.model_calls.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_tool_calls(&self) -> MutexGuard<'_, HashMap<String, ActiveToolCall>> {
        self.tool_calls.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_agent_messages(&self) -> MutexGuard<'_, HashMap<usize, ActiveAgentMessage>> {
        self.agent_messages.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl<M> AgentHook<M> for AgentRuntimeHook
where
    M: CompletionModel,
{
    async fn on_event(&self, ctx: &HookContext, event: StepEvent<'_, M>) -> Flow {
        match event {
            StepEvent::CompletionCall { turn: step_index, .. } => self.on_completion_call(ctx, step_index).await,
            StepEvent::ModelTurnFinished { turn: step_index, content, usage, .. } => {
                if let Some(snapshot) = self.tool_route.as_ref().and_then(AgentToolRoute::active_snapshot) {
                    let calls = content
                        .iter()
                        .filter_map(|item| match item {
                            AssistantContent::ToolCall(call) => Some((
                                call.id.clone(),
                                call.call_id.clone(),
                                call.function.name.clone(),
                                canonical_arguments(&serde_json::to_string(&call.function.arguments).unwrap_or_default()),
                            )),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    self.tool_batch.prepare(snapshot.generation(), &calls);
                }
                let estimated_output_tokens = serde_json::to_vec(content).map_or(0, |bytes| (bytes.len() as u64).saturating_add(3) / 4);
                let content = debug_log_json(content);
                self.on_model_turn_finished(ctx, step_index, usage.clone(), estimated_output_tokens, &content)
                    .await
            }
            StepEvent::InvalidToolCall(invalid) => {
                self.on_invalid_tool_call(
                    ctx,
                    &invalid.tool_name,
                    invalid.tool_call_id.as_deref(),
                    invalid.internal_call_id.as_deref(),
                    &invalid.available_tools,
                )
                .await
            }
            StepEvent::ToolCallDelta { tool_call_id, internal_call_id, tool_name, .. } => {
                if tool_name.is_some() {
                    let generation = self
                        .tool_route
                        .as_ref()
                        .and_then(AgentToolRoute::active_snapshot)
                        .map_or(0, |snapshot| snapshot.generation());
                    self.tool_batch.record_stream_call(generation, tool_call_id, internal_call_id);
                }
                Flow::cont()
            }
            StepEvent::ToolCall { tool_name, tool_call_id, internal_call_id, args } => {
                self.on_tool_call(ctx, tool_name, tool_call_id, internal_call_id, args).await
            }
            StepEvent::ToolResult { tool_name, tool_call_id, internal_call_id, result, outcome, .. } => {
                self.on_tool_result(ctx, tool_name, tool_call_id, internal_call_id, result, outcome)
                    .await
            }
            StepEvent::TextDelta { delta, .. } => self.on_text_delta(ctx, ctx.turn(), delta).await,
            _ => Flow::cont(),
        }
    }

    fn observes(&self, kind: StepEventKind) -> bool {
        matches!(
            kind,
            StepEventKind::CompletionCall
                | StepEventKind::ModelTurnFinished
                | StepEventKind::InvalidToolCall
                | StepEventKind::ToolCallDelta
                | StepEventKind::ToolCall
                | StepEventKind::ToolResult
                | StepEventKind::TextDelta
        )
    }
}

pub(crate) fn classify_tool_call(tool_name: &str) -> ToolCallKind {
    match tool_name {
        LIST_WORKSPACE_FILES_TOOL
        | SEARCH_WORKSPACE_FILES_TOOL
        | READ_WORKSPACE_FILE_TOOL
        | RESOLVE_COMPONENT_SPECS_TOOL
        | UPDATE_PLAN_TOOL => ToolCallKind::Read,
        INSPECT_CURRENT_PAGE_TOOL
        | GET_COMPONENT_CATALOG_TOOL
        | GET_COMPONENT_DATA_CONTRACT_TOOL
        | REQUEST_USER_INPUT_TOOL
        | REQUEST_COMPONENT_SELECTION_TOOL
        | GET_CANVAS_OPERATION_STATUS_TOOL
        | GET_CURRENT_CANVAS_SELECTION_TOOL => ToolCallKind::FrontendInteraction,
        APPLY_WORKSPACE_PATCH_TOOL => ToolCallKind::Write,
        CREATE_CANVAS_COMPONENTS_TOOL | PATCH_CANVAS_COMPONENTS_TOOL | DELETE_CANVAS_COMPONENTS_TOOL => ToolCallKind::WriteFrontendInteraction,
        _ => ToolCallKind::Write,
    }
}

fn waiting_interaction(tool_name: &str) -> Option<(AgentTaskStatus, AgentPendingKind, &'static str)> {
    match tool_name {
        REQUEST_USER_INPUT_TOOL => Some((AgentTaskStatus::WaitingUserInput, AgentPendingKind::UserInput, "userInput")),
        CREATE_CANVAS_COMPONENTS_TOOL | PATCH_CANVAS_COMPONENTS_TOOL | DELETE_CANVAS_COMPONENTS_TOOL | REQUEST_COMPONENT_SELECTION_TOOL => {
            Some((AgentTaskStatus::WaitingUserConfirmation, AgentPendingKind::UserConfirmation, "approval"))
        }
        _ => None,
    }
}

fn item_snapshot(item_id: &str, item_type: &str, status: ItemStatus, row_version: u64, extra: Value) -> Value {
    let payload = match extra {
        Value::Object(object) => Value::Object(object),
        value => json!({"value": value}),
    };
    json!({"itemId": item_id, "itemType": item_type, "status": status.as_str(), "rowVersion": row_version, "payload": payload})
}

fn tool_kind_name(kind: ToolCallKind) -> &'static str {
    match kind {
        ToolCallKind::Read => "read",
        ToolCallKind::Write => "write",
        ToolCallKind::FrontendInteraction => "frontendInteraction",
        ToolCallKind::WriteFrontendInteraction => "writeFrontendInteraction",
    }
}

fn content_summary(content: &str) -> Value {
    let json_type = serde_json::from_str::<Value>(content).ok().map(|value| value_type(&value));
    json!({ "byteLength": content.len(), "jsonType": json_type })
}

fn event_dedupe_key(run_id: &str, kind: &str, item_id: Option<&str>, payload: &Value) -> Result<String, serde_json::Error> {
    let canonical = serde_json::to_vec(&canonical_json(payload))?;
    Ok(format!("run:{run_id}:event:{kind}:item:{}:sha256:{}", item_id.unwrap_or("-"), sha256_hex(&canonical)))
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
            let mut canonical = Map::new();
            for (key, value) in entries {
                canonical.insert(key.clone(), canonical_json(value));
            }
            Value::Object(canonical)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical_json).collect()),
        value => value.clone(),
    }
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_tools_and_fails_closed_for_unknown_tools() {
        assert_eq!(classify_tool_call(READ_WORKSPACE_FILE_TOOL), ToolCallKind::Read);
        assert_eq!(classify_tool_call("search_component_documents"), ToolCallKind::Write);
        assert_eq!(classify_tool_call(GET_COMPONENT_CATALOG_TOOL), ToolCallKind::FrontendInteraction);
        assert_eq!(classify_tool_call("search_available_components"), ToolCallKind::Write);
        assert_eq!(classify_tool_call(RESOLVE_COMPONENT_SPECS_TOOL), ToolCallKind::Read);
        assert_eq!(classify_tool_call(REQUEST_USER_INPUT_TOOL), ToolCallKind::FrontendInteraction);
        assert_eq!(classify_tool_call(PATCH_CANVAS_COMPONENTS_TOOL), ToolCallKind::WriteFrontendInteraction);
        assert_eq!(classify_tool_call("future_tool"), ToolCallKind::Write);
    }

    #[test]
    fn persisted_item_snapshot_uses_stable_keys_and_nested_payload() {
        let snapshot = item_snapshot("item_opaque", "agentMessage", ItemStatus::InProgress, 3, json!({ "text": "ok" }));
        assert_eq!(snapshot["itemId"], "item_opaque");
        assert_eq!(snapshot["itemType"], "agentMessage");
        assert!(snapshot.get("id").is_none());
        assert!(snapshot.get("type").is_none());
        assert_eq!(snapshot["payload"]["text"], "ok");
        assert_eq!(snapshot["rowVersion"], 3);
    }

    #[test]
    fn content_summary_never_copies_values() {
        let secret = "secret-api-key-value";
        let summary = content_summary(&format!(r#"{{"apiKey":"{secret}"}}"#));
        let encoded = serde_json::to_string(&summary).unwrap();
        assert_eq!(summary["jsonType"], "object");
        assert!(!encoded.contains(secret));
        assert!(!encoded.contains("apiKey"));
    }

    #[test]
    fn event_dedupe_key_is_stable_for_canonical_payload() {
        let first = event_dedupe_key("run_1", "agent.started", None, &json!({ "b": 2, "a": { "d": 4, "c": 3 } })).unwrap();
        let second = event_dedupe_key("run_1", "agent.started", None, &json!({ "a": { "c": 3, "d": 4 }, "b": 2 })).unwrap();
        assert_eq!(first, second);
        assert_ne!(
            first,
            event_dedupe_key("run_1", "agent.started", Some("item_1"), &json!({ "a": { "c": 3, "d": 4 }, "b": 2 })).unwrap()
        );
    }
}
