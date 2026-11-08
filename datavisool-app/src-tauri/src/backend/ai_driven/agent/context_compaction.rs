/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 当前 Agent 运行内的模型可见历史压缩。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use rig_core::agent::{AgentHook, Flow, HookContext, RequestPatch, StepEvent, StepEventKind};
use rig_core::completion::{AssistantContent, CompletionModel, Message, Usage};
use rig_core::message::UserContent;
use rig_core::OneOrMany;
use serde_json::Value;

use super::context_history::{is_stateful_read, project_tool_history_with_budget, state_digest_with_limits};
use super::hooks::AgentRuntimeHook;
use super::limits::{provider_context_input_tokens, AgentContextLimits, ExecutionUsageSnapshot};
use super::steering::AgentSteeringHook;
use super::tool_route::AgentRoutePhase;
use super::{AgentContextPlan, AgentToolRoute};
use crate::backend::ai_driven::tools::canvas::{CREATE_CANVAS_COMPONENTS_TOOL, DELETE_CANVAS_COMPONENTS_TOOL, PATCH_CANVAS_COMPONENTS_TOOL};
use crate::backend::ai_driven::tools::interaction::INSPECT_CURRENT_PAGE_TOOL;

#[path = "compaction_fit.rs"]
mod compaction_fit;
use compaction_fit::summary_request_tokens;
use compaction_fit::{fit_compacted_history, fit_summary_history};

const DEFAULT_OUTPUT_RESERVE_TOKENS: u64 = 4_096;
const NEXT_TURN_RESERVE_TOKENS: u64 = 8_192;
const RECENT_MESSAGE_COUNT: usize = 8;
const FALLBACK_FRAGMENT_CHARS: usize = 512;
const FALLBACK_SUMMARY_CHARS: usize = 4_096;
const MIN_COMPACTION_SAVINGS_TOKENS: u64 = 8_192;
const UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS: u64 = 32_000;

const COMPACTION_PROMPT: &str = "请压缩以上 Agent 运行历史。只输出一份可供后续模型继续执行的中文摘要，必须保留：用户目标与追加要求、任务计划、已调用工具及关键结果、Workspace artifactId/revision/status、校验与预览状态、尚未完成的下一步、组件实际 ID 映射、关键布局与继续创建/修改所需规格、错误及未应用动作。不要复制完整 JSON、工具参数、密钥、Base64 或大段原始数据。";

#[derive(Clone, Copy)]
enum PostActionCompaction {
    AppliedWrite,
    Verification,
}

#[derive(Clone)]
pub(crate) struct AgentContextHook<M>
where
    M: CompletionModel,
{
    model: M,
    threshold_tokens: Option<u64>,
    context_capacity_tokens: u64,
    context_capacity_source: &'static str,
    tool_route: AgentToolRoute,
    tool_schema_bytes: HashMap<String, usize>,
    steering: Option<AgentSteeringHook>,
    runtime_hook: AgentRuntimeHook,
    limits: AgentContextLimits,
    token_tracker: ContextTokenTracker,
    state: Arc<Mutex<CompactionState>>,
}

#[derive(Clone, Default)]
pub(crate) struct ContextTokenTracker {
    estimates: Arc<Mutex<HashMap<usize, u64>>>,
}

impl ContextTokenTracker {
    pub(crate) fn record(&self, turn: usize, estimated_input_tokens: u64) {
        self.estimates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(turn, estimated_input_tokens);
    }

    pub(crate) fn get(&self, turn: usize) -> Option<u64> {
        self.estimates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&turn)
            .copied()
    }

    pub(crate) fn take(&self, turn: usize) -> Option<u64> {
        self.estimates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&turn)
    }
}

#[derive(Default)]
struct CompactionState {
    last_input_tokens: u64,
    /// 记录上一次实际派发主模型请求的本地估算，用于校正供应商 usage
    /// 可能覆盖新追加工具消息的问题。
    last_dispatched_estimate_tokens: u64,
    compaction_count: u64,
    compacted: Option<CompactedWindow>,
}

struct CompactedWindow {
    source_len: usize,
    source_boundary: Option<Message>,
    history: Vec<Message>,
}

impl<M> AgentContextHook<M>
where
    M: CompletionModel,
{
    pub(crate) fn new(
        model: M,
        context_window_tokens: Option<u64>,
        max_output_tokens: Option<u64>,
        tool_route: AgentToolRoute,
        tool_schema_bytes: HashMap<String, usize>,
        runtime_hook: AgentRuntimeHook,
        steering: Option<AgentSteeringHook>,
        limits: AgentContextLimits,
        token_tracker: ContextTokenTracker,
    ) -> Self {
        let (context_capacity_tokens, context_capacity_source) = match context_window_tokens {
            Some(tokens) => (tokens, "model"),
            None => (UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS, "fallback"),
        };
        let threshold_tokens = Some(context_input_limit(context_capacity_tokens, max_output_tokens, &limits));
        Self {
            model,
            threshold_tokens,
            context_capacity_tokens,
            context_capacity_source,
            tool_route,
            tool_schema_bytes,
            steering,
            runtime_hook,
            limits,
            token_tracker,
            state: Arc::new(Mutex::new(CompactionState::default())),
        }
    }

    async fn patch_request(&self, ctx: &HookContext, prompt: &Message, history: &[Message]) -> Flow {
        // 必须在构建请求前冻结路由；同一响应中的 Tool Call 后续只使用该快照准入。
        let route_snapshot = self.tool_route.snapshot_for_step(ctx.turn() as u64);
        let mut route_plan = route_snapshot.context_plan().clone();
        let registered_tools = self.tool_schema_bytes.keys().cloned().collect();
        route_plan.retain_registered_tools(&registered_tools);
        let (mut visible_history, has_compacted_window, mut last_input_tokens, last_dispatched_estimate_tokens, next_compaction_count) = {
            let state = self.lock_state();
            let projected = state.compacted.as_ref().and_then(|window| window.project(history));
            let has_compacted_window = projected.is_some();
            (
                projected.unwrap_or_else(|| history.to_vec()),
                has_compacted_window,
                state.last_input_tokens,
                state.last_dispatched_estimate_tokens,
                state.compaction_count.saturating_add(1),
            )
        };
        // Applied 后仅清理过时的纯文本分析，继续保留规格、错误及完整工具消息。
        let mut history_changed = has_compacted_window;
        let compaction = post_action_compaction(&visible_history, prompt, route_snapshot.phase).filter(|mode| match mode {
            PostActionCompaction::AppliedWrite => current_prompt_result_json(prompt)
                .and_then(|value| value.get("operationId").and_then(Value::as_str).map(str::to_string))
                .is_some_and(|id| self.runtime_hook.claim_applied_compaction(&id)),
            PostActionCompaction::Verification => false,
        });
        if let Some(mode) = compaction {
            let before_tokens = estimate_request_tokens(prompt, &visible_history, &route_plan, &self.tool_schema_bytes);
            let compacted_history = build_post_action_history(&visible_history, prompt, mode, &self.runtime_hook.usage_snapshot());
            {
                let mut state = self.lock_state();
                state.last_input_tokens = 0;
                state.compaction_count = next_compaction_count;
                state.compacted = Some(CompactedWindow {
                    source_len: history.len(),
                    source_boundary: history.last().cloned(),
                    history: compacted_history.clone(),
                });
            }
            let after_tokens = estimate_request_tokens(prompt, &compacted_history, &route_plan, &self.tool_schema_bytes);
            let flow = self
                .runtime_hook
                .record_lightweight_compaction(ctx, before_tokens, after_tokens)
                .await;
            if !matches!(flow, Flow::Continue) {
                return flow;
            }
            visible_history = compacted_history;
            last_input_tokens = 0;
            history_changed = true;
        }
        if let Some(steering) = &self.steering {
            match steering.history_with_pending(&visible_history).await {
                Ok(Some(steered_history)) => {
                    visible_history = steered_history;
                    history_changed = true;
                }
                Ok(None) => {}
                Err(reason) => return Flow::terminate(reason),
            }
        }

        if let Some(warning) = self.runtime_hook.take_budget_warning() {
            visible_history.push(Message::user(format!("[Agent budget warning]\n{warning}")));
            history_changed = true;
            let flow = self.runtime_hook.record_budget_warning(ctx, &warning).await;
            if !matches!(flow, Flow::Continue) {
                return flow;
            }
        }

        let estimated_tokens = estimate_request_tokens(prompt, &visible_history, &route_plan, &self.tool_schema_bytes);
        let corrected_estimated_tokens = corrected_context_tokens(last_input_tokens, last_dispatched_estimate_tokens, estimated_tokens);
        self.runtime_hook
            .record_context(corrected_estimated_tokens, self.context_capacity_tokens, self.context_capacity_source);
        self.token_tracker.record(ctx.turn(), corrected_estimated_tokens);
        let Some(threshold_tokens) = self.threshold_tokens else {
            self.remember_dispatched_estimate(estimated_tokens);
            return context_patch(&route_plan, history_changed.then_some(visible_history));
        };
        if !reached_compaction_threshold_with_correction(last_input_tokens, estimated_tokens, threshold_tokens, last_dispatched_estimate_tokens) {
            self.remember_dispatched_estimate(estimated_tokens);
            return context_patch(&route_plan, history_changed.then_some(visible_history));
        }

        let summary_max_bytes = token_bytes(self.limits.compact_summary_tokens);
        let preview_history = build_compacted_history_with_limits(
            &visible_history,
            "",
            &self.runtime_hook.usage_snapshot(),
            summary_max_bytes,
            self.limits.max_tool_result_tokens,
            self.limits.max_tool_history_tokens,
            self.limits.max_state_digest_tokens,
            Some(prompt),
        );
        if contains_unrecoverable_state_digest(&preview_history) {
            return Flow::terminate("agent.context_compaction.recovery_refs_too_large");
        }
        let preview_tokens = estimate_request_tokens(prompt, &preview_history, &route_plan, &self.tool_schema_bytes);
        // Once the request reaches the threshold, a low projected saving is only
        // an optimization signal. Sending the unbounded history would knowingly
        // exceed the active context capacity, so it must still be compacted.
        if corrected_estimated_tokens < threshold_tokens
            && !should_compact_history(corrected_estimated_tokens, preview_tokens, self.limits.compact_summary_tokens)
        {
            self.remember_dispatched_estimate(estimated_tokens);
            return context_patch(&route_plan, history_changed.then_some(visible_history));
        }

        let Some(summary_history) = fit_summary_history(
            &visible_history,
            prompt,
            self.context_capacity_tokens,
            self.limits.compact_summary_tokens,
            self.limits.max_tool_result_tokens,
            self.limits.max_tool_history_tokens,
        ) else {
            return Flow::terminate("agent.context_compaction.input_too_large");
        };

        let observed_tokens = corrected_estimated_tokens;
        let started = self
            .runtime_hook
            .begin_context_compaction(ctx, next_compaction_count, observed_tokens, visible_history.len())
            .await;
        if !matches!(started, Flow::Continue) {
            return started;
        }

        let (summary, usage, fallback) = match summarize_history(&self.model, &summary_history, self.limits.compact_summary_tokens).await {
            Ok((summary, usage)) => (summary, usage, false),
            Err(_) => {
                let mut fallback_history = visible_history.clone();
                fallback_history.push(prompt.clone());
                (fallback_summary(&fallback_history), Usage::default(), true)
            }
        };
        let summary_input_tokens = summary_request_tokens(&summary_history, 0);
        let compacted_candidate = build_compacted_history_with_limits(
            &visible_history,
            &summary,
            &self.runtime_hook.usage_snapshot(),
            summary_max_bytes,
            self.limits.max_tool_result_tokens,
            self.limits.max_tool_history_tokens,
            self.limits.max_state_digest_tokens,
            Some(prompt),
        );
        let Some(compacted_history) = fit_compacted_history(
            prompt,
            compacted_candidate.clone(),
            &visible_history,
            &route_plan,
            &self.tool_schema_bytes,
            threshold_tokens,
            self.limits.max_tool_result_tokens,
            self.limits.max_tool_history_tokens,
        ) else {
            let candidate_tokens = estimate_request_tokens(prompt, &compacted_candidate, &route_plan, &self.tool_schema_bytes);
            return self
                .runtime_hook
                .fail_context_compaction(
                    ctx,
                    usage,
                    summary_input_tokens,
                    estimate_text_tokens(&summary),
                    &summary,
                    next_compaction_count,
                    observed_tokens,
                    candidate_tokens,
                    visible_history.len(),
                    compacted_candidate.len(),
                    fallback,
                    !usage.has_values(),
                    "agent.context_compaction.unrecoverable",
                )
                .await;
        };
        let compacted_tokens = estimate_request_tokens(prompt, &compacted_history, &route_plan, &self.tool_schema_bytes);
        self.runtime_hook.record_compaction(observed_tokens, compacted_tokens);
        self.runtime_hook
            .record_context(compacted_tokens, self.context_capacity_tokens, self.context_capacity_source);
        self.token_tracker.record(ctx.turn(), compacted_tokens);
        {
            let mut state = self.lock_state();
            state.last_input_tokens = 0;
            state.last_dispatched_estimate_tokens = compacted_tokens;
            state.compaction_count = next_compaction_count;
            state.compacted = Some(CompactedWindow {
                source_len: history.len(),
                source_boundary: history.last().cloned(),
                history: compacted_history.clone(),
            });
        }

        let estimated = !usage.has_values();
        let completed = self
            .runtime_hook
            .complete_context_compaction(
                ctx,
                usage,
                summary_input_tokens,
                estimate_text_tokens(&summary),
                &summary,
                next_compaction_count,
                observed_tokens,
                compacted_tokens,
                visible_history.len(),
                compacted_history.len(),
                fallback,
                estimated,
            )
            .await;
        if !matches!(completed, Flow::Continue) {
            return completed;
        }
        context_patch(&route_plan, Some(compacted_history))
    }

    fn lock_state(&self) -> MutexGuard<'_, CompactionState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn remember_dispatched_estimate(&self, estimated_tokens: u64) {
        self.lock_state().last_dispatched_estimate_tokens = estimated_tokens;
    }
}

impl<S, M> AgentHook<M> for AgentContextHook<S>
where
    S: CompletionModel + 'static,
    M: CompletionModel + 'static,
{
    async fn on_event(&self, ctx: &HookContext, event: StepEvent<'_, M>) -> Flow {
        match event {
            StepEvent::CompletionCall { prompt, history, .. } => self.patch_request(ctx, prompt, history).await,
            StepEvent::ModelTurnFinished { usage, .. } => {
                self.lock_state().last_input_tokens = provider_context_input_tokens(&usage, usage.input_tokens);
                Flow::cont()
            }
            _ => Flow::cont(),
        }
    }

    fn observes(&self, kind: StepEventKind) -> bool {
        matches!(kind, StepEventKind::CompletionCall | StepEventKind::ModelTurnFinished)
    }
}

impl CompactedWindow {
    fn project(&self, source: &[Message]) -> Option<Vec<Message>> {
        if source.len() < self.source_len || source.get(self.source_len.saturating_sub(1)) != self.source_boundary.as_ref() {
            return None;
        }
        let mut history = self.history.clone();
        history.extend_from_slice(&source[self.source_len..]);
        Some(history)
    }
}

async fn summarize_history<M>(model: &M, history: &[Message], max_tokens: u64) -> Result<(String, Usage), rig_core::completion::CompletionError>
where
    M: CompletionModel,
{
    let response = model
        .completion_request(Message::user(COMPACTION_PROMPT))
        .messages(history.iter().cloned())
        .max_tokens(max_tokens)
        .send()
        .await?;
    let summary = response
        .choice
        .iter()
        .filter_map(|content| match content {
            AssistantContent::Text(text) => Some(text.text.trim()),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if summary.is_empty() {
        return Err(rig_core::completion::CompletionError::ResponseError("上下文压缩未返回文本摘要".to_string()));
    }
    Ok((truncate_utf8_bytes(&summary, token_bytes(max_tokens)), response.usage))
}

fn context_input_limit(context_window_tokens: u64, max_output_tokens: Option<u64>, limits: &AgentContextLimits) -> u64 {
    let output_reserve = max_output_tokens
        .unwrap_or(DEFAULT_OUTPUT_RESERVE_TOKENS)
        .min(context_window_tokens / 4);
    let next_turn_reserve = NEXT_TURN_RESERVE_TOKENS.min(context_window_tokens / 8);
    let effective_model_limit = context_window_tokens
        .saturating_sub(output_reserve)
        .saturating_sub(next_turn_reserve);
    effective_model_limit
        .saturating_mul(u64::from(limits.compact_at_percent.min(100)))
        .saturating_div(100)
        .min(limits.max_active_context_tokens)
}

fn should_compact_history(estimated_tokens: u64, preview_tokens: u64, summary_tokens: u64) -> bool {
    let minimum_source = summary_tokens.saturating_mul(4).max(MIN_COMPACTION_SAVINGS_TOKENS);
    let minimum_savings = summary_tokens.saturating_mul(2).max(MIN_COMPACTION_SAVINGS_TOKENS);
    estimated_tokens >= minimum_source && estimated_tokens.saturating_sub(preview_tokens) >= minimum_savings
}

fn corrected_context_tokens(last_input_tokens: u64, previous_estimate_tokens: u64, estimated_tokens: u64) -> u64 {
    estimated_tokens.saturating_add(last_input_tokens.saturating_sub(previous_estimate_tokens))
}

fn reached_compaction_threshold(last_input_tokens: u64, estimated_tokens: u64, threshold_tokens: u64) -> bool {
    last_input_tokens >= threshold_tokens || estimated_tokens >= threshold_tokens
}

fn reached_compaction_threshold_with_correction(
    last_input_tokens: u64,
    estimated_tokens: u64,
    threshold_tokens: u64,
    previous_estimate_tokens: u64,
) -> bool {
    corrected_context_tokens(last_input_tokens, previous_estimate_tokens, estimated_tokens) >= threshold_tokens
}

fn estimate_request_tokens(prompt: &Message, history: &[Message], plan: &AgentContextPlan, tool_schema_bytes: &HashMap<String, usize>) -> u64 {
    let schema_bytes = plan
        .active_tools()
        .iter()
        .filter_map(|tool| tool_schema_bytes.get(tool))
        .sum::<usize>();
    let request_bytes = serde_json::to_vec(&(history, prompt)).map_or(0, |bytes| bytes.len());
    ((request_bytes + plan.system_prompt().len() + schema_bytes) as u64).saturating_add(3) / 4
}

fn context_patch(plan: &AgentContextPlan, history: Option<Vec<Message>>) -> Flow {
    let mut patch = RequestPatch::new()
        .active_tools(plan.active_tools())
        .preamble(plan.system_prompt());
    if let Some(history) = history {
        patch = patch.history(history);
    }
    Flow::patch_request(patch)
}

fn build_compacted_history_with_limits(
    history: &[Message],
    summary: &str,
    usage: &ExecutionUsageSnapshot,
    summary_max_bytes: usize,
    max_tool_result_tokens: u64,
    max_tool_history_tokens: u64,
    max_state_digest_tokens: u64,
    digest_prompt: Option<&Message>,
) -> Vec<Message> {
    let mut digest_history = history.to_vec();
    if let Some(prompt) = digest_prompt {
        digest_history.push(prompt.clone());
    }
    let start = recent_complete_suffix_start(history);
    let mut compacted = Vec::with_capacity(history.len().saturating_sub(start) + 1);
    compacted.extend(preserved_user_requirements(&history[..start]));
    compacted.extend(preserved_runtime_notices(history));
    compacted.push(Message::user(format!(
        "[Agent context summary]\n{}\n\n[Agent state digest]\n{}\n\n[Execution budget]\n{}",
        truncate_utf8_bytes(summary, summary_max_bytes),
        state_digest_with_limits(&digest_history, max_tool_result_tokens, max_tool_history_tokens, max_state_digest_tokens),
        serde_json::to_string(usage).unwrap_or_else(|_| "{}".to_string())
    )));
    let essential = essential_tool_history(&history[..start]);
    compacted.extend(essential);
    // A previous compaction summary is already represented by the new summary
    // and digest below. Keeping it in the recent suffix makes summaries stack
    // across turns and can make an otherwise recoverable window unrecoverable.
    compacted.extend(
        history[start..]
            .iter()
            .filter(|message| !is_context_summary_message(message))
            .cloned(),
    );
    compacted
}

fn is_context_summary_message(message: &Message) -> bool {
    matches!(
        message,
        Message::User { content }
            if content.iter().any(|item| matches!(item, UserContent::Text(text) if text.text.starts_with("[Agent context summary]")))
    )
}

fn contains_unrecoverable_state_digest(history: &[Message]) -> bool {
    history.iter().any(|message| match message {
        Message::User { content } => content.iter().any(|item| match item {
            UserContent::Text(text) if text.text.starts_with("[Agent context summary]") => text.text.contains("\"unrecoverable\":true"),
            _ => false,
        }),
        _ => false,
    })
}

/// 保留继续操作所需的真实回执、规格、快照及错误；完整保留多调用消息和推理签名。
fn essential_tool_history(history: &[Message]) -> Vec<Message> {
    let call_names = history
        .iter()
        .filter_map(|message| match message {
            Message::Assistant { content, .. } => Some(content),
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|item| match item {
            AssistantContent::ToolCall(call) => Some((call.id.clone(), call.function.name.clone())),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let applied_result_ids = history
        .iter()
        .filter_map(|message| match message {
            Message::User { content } => Some(content),
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|item| match item {
            UserContent::ToolResult(result) => {
                let applied = result.content.iter().any(|item| match item {
                    rig_core::message::ToolResultContent::Text(text) => serde_json::from_str::<Value>(&text.text)
                        .ok()
                        .is_some_and(|value| value.get("operationId").and_then(Value::as_str).is_some() && value.get("status").is_some()),
                    _ => false,
                });
                applied.then(|| result.id.clone())
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    let mut keep = HashSet::<String>::new();
    for message in history {
        match message {
            Message::Assistant { content, .. } => {
                for item in content.iter() {
                    if let AssistantContent::ToolCall(call) = item {
                        if is_recovery_tool(call.function.name.as_str()) || applied_result_ids.contains(&call.id) {
                            keep.insert(call.id.clone());
                        }
                    }
                }
            }
            Message::User { content } => {
                for item in content.iter() {
                    if let UserContent::ToolResult(result) = item {
                        if result.content.iter().any(|item| match item {
                            rig_core::message::ToolResultContent::Text(text) => serde_json::from_str::<Value>(&text.text).ok().is_some_and(|value| {
                                (value.get("operationId").and_then(Value::as_str).is_some() && value.get("status").is_some())
                                    || value.get("code").is_some()
                                    || value
                                        .get("status")
                                        .and_then(Value::as_str)
                                        .is_some_and(|s| matches!(s, "failed" | "rejected" | "error"))
                            }),
                            _ => false,
                        }) {
                            keep.insert(result.id.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    // 多 ToolCall / ToolResult 消息不可拆开：闭包包含同一消息上的所有关联 ID。
    loop {
        let before = keep.len();
        for message in history {
            let ids: Vec<&str> = match message {
                Message::Assistant { content, .. } => content
                    .iter()
                    .filter_map(|item| match item {
                        AssistantContent::ToolCall(call) => Some(call.id.as_str()),
                        _ => None,
                    })
                    .collect(),
                Message::User { content } => content
                    .iter()
                    .filter_map(|item| match item {
                        UserContent::ToolResult(result) => Some(result.id.as_str()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            if ids.iter().any(|id| keep.contains(*id)) {
                keep.extend(ids.into_iter().map(str::to_string));
            }
        }
        if keep.len() == before {
            break;
        }
    }
    history
        .iter()
        .filter_map(|message| match message {
            Message::Assistant { content, .. }
                if content
                    .iter()
                    .any(|item| matches!(item, AssistantContent::ToolCall(call) if keep.contains(&call.id))) =>
            {
                Some(message.clone())
            }
            Message::User { content } => {
                let items = content
                    .iter()
                    .filter_map(|item| match item {
                        UserContent::ToolResult(result) if keep.contains(&result.id) => {
                            let tool_name = call_names.get(&result.id).map(String::as_str).unwrap_or("unknown");
                            if is_stateful_read(tool_name) {
                                Some(UserContent::ToolResult(compact_recovery_read_result(tool_name, result)))
                            } else {
                                Some(item.clone())
                            }
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                (!items.is_empty()).then(|| Message::User { content: OneOrMany::many(items).expect("保留内容至少包含一项") })
            }
            _ => None,
        })
        .collect()
}

fn is_recovery_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "resolve_component_specs"
            | "get_component_catalog"
            | "get_component_data_contract"
            | "inspect_current_page"
            | "create_canvas_components"
            | "patch_canvas_components"
            | "delete_canvas_components"
            | "get_canvas_operation_status"
            | "update_plan"
            | "read_workspace_file"
            | "list_workspace_files"
            | "search_workspace_files"
    )
}

fn compact_recovery_read_result(tool_name: &str, result: &rig_core::message::ToolResult) -> rig_core::message::ToolResult {
    let digest =
        serde_json::from_str::<Value>(&super::context_history::tool_result_digest(tool_name, result, None, false)).unwrap_or_else(|_| Value::Null);
    let reference = serde_json::json!({
        "tool": tool_name,
        "toolCallId": result.call_id.as_deref().unwrap_or(&result.id),
        "source": "original_tool_result",
    });
    rig_core::message::ToolResult {
        id: result.id.clone(),
        call_id: result.call_id.clone(),
        content: rig_core::OneOrMany::one(rig_core::message::ToolResultContent::text(
            serde_json::json!({ "recoveryReceipt": true, "reference": reference, "digest": digest }).to_string(),
        )),
    }
}

/// 不调用摘要模型；保留所有工具结果、规格、真实操作和带签名的 Assistant 消息。
fn build_post_action_history(history: &[Message], _prompt: &Message, _mode: PostActionCompaction, _usage: &ExecutionUsageSnapshot) -> Vec<Message> {
    history
        .iter()
        .filter(|message| match message {
            Message::Assistant { content, .. } => content.iter().any(|item| !matches!(item, AssistantContent::Text(_))),
            _ => true,
        })
        .cloned()
        .collect()
}

fn post_action_compaction(history: &[Message], prompt: &Message, phase: AgentRoutePhase) -> Option<PostActionCompaction> {
    let (tool_name, _) = current_prompt_tool_call(history, prompt)?;
    match tool_name {
        CREATE_CANVAS_COMPONENTS_TOOL | PATCH_CANVAS_COMPONENTS_TOOL | DELETE_CANVAS_COMPONENTS_TOOL
            if current_prompt_status(prompt).as_deref() == Some("applied") =>
        {
            Some(PostActionCompaction::AppliedWrite)
        }
        INSPECT_CURRENT_PAGE_TOOL if phase == AgentRoutePhase::Verifying && !current_prompt_is_failed(prompt) => {
            Some(PostActionCompaction::Verification)
        }
        _ => None,
    }
}

fn current_prompt_tool_call<'a>(history: &'a [Message], prompt: &'a Message) -> Option<(&'a str, &'a Message)> {
    let result_id = current_prompt_tool_result_id(prompt)?;
    history.iter().rev().find_map(|message| {
        let Message::Assistant { content, .. } = message else {
            return None;
        };
        content.iter().find_map(|item| match item {
            AssistantContent::ToolCall(call) if call.id == result_id => Some((call.function.name.as_str(), message)),
            _ => None,
        })
    })
}

fn current_prompt_tool_result_id(prompt: &Message) -> Option<&str> {
    let Message::User { content } = prompt else {
        return None;
    };
    content.iter().find_map(|item| match item {
        UserContent::ToolResult(result) => Some(result.id.as_str()),
        _ => None,
    })
}

fn current_prompt_status(prompt: &Message) -> Option<String> {
    current_prompt_result_json(prompt).and_then(|result| result.get("status").and_then(Value::as_str).map(str::to_string))
}

fn current_prompt_is_failed(prompt: &Message) -> bool {
    let Some(result) = current_prompt_result_json(prompt) else {
        return true;
    };
    result
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| matches!(status, "failed" | "rejected" | "error"))
        || result.get("code").is_some()
}

fn current_prompt_result_json(prompt: &Message) -> Option<Value> {
    let Message::User { content } = prompt else {
        return None;
    };
    let result = content.iter().find_map(|item| match item {
        UserContent::ToolResult(result) => Some(result),
        _ => None,
    })?;
    let text = result
        .content
        .iter()
        .filter_map(|content| match content {
            rig_core::message::ToolResultContent::Text(text) => Some(text.text.as_str()),
            rig_core::message::ToolResultContent::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::from_str(&text).ok()
}

fn preserved_user_requirements(history: &[Message]) -> Vec<Message> {
    history
        .iter()
        .filter(|message| match message {
            Message::User { content } => {
                !content.iter().any(|item| matches!(item, UserContent::ToolResult(_)))
                    && content
                        .iter()
                        .any(|item| matches!(item, UserContent::Text(text) if !is_internal_runtime_prompt(&text.text)))
            }
            _ => false,
        })
        .cloned()
        .collect()
}

fn preserved_runtime_notices(history: &[Message]) -> Vec<Message> {
    history
        .iter()
        .filter(|message| matches!(message, Message::User { content } if content.iter().any(|item| matches!(item, UserContent::Text(text) if text.text.starts_with("[Agent budget warning]")))))
        .rev()
        .take(1)
        .cloned()
        .collect()
}

fn is_internal_runtime_prompt(text: &str) -> bool {
    text.starts_with("[Agent context summary]")
        || text.starts_with("[Agent state digest]")
        || text.starts_with("继续当前任务")
        || text.starts_with("任务完成门禁")
        || text.starts_with("仍有画布动作")
        || text.starts_with("[Agent budget warning]")
}

fn recent_complete_suffix_start(history: &[Message]) -> usize {
    let mut start = history.len().saturating_sub(RECENT_MESSAGE_COUNT);
    while start > 0 {
        let calls = tool_call_ids(&history[start..]);
        let results = tool_result_ids(&history[start..]);
        if results.is_subset(&calls) && calls.is_subset(&results) {
            break;
        }
        start -= 1;
    }
    start
}

fn tool_call_ids(history: &[Message]) -> HashSet<&str> {
    history
        .iter()
        .filter_map(|message| match message {
            Message::Assistant { content, .. } => Some(content),
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|content| match content {
            AssistantContent::ToolCall(call) => Some(call.id.as_str()),
            _ => None,
        })
        .collect()
}

fn tool_result_ids(history: &[Message]) -> HashSet<&str> {
    history
        .iter()
        .filter_map(|message| match message {
            Message::User { content } => Some(content),
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|content| match content {
            UserContent::ToolResult(result) => Some(result.id.as_str()),
            _ => None,
        })
        .collect()
}

fn fallback_summary(history: &[Message]) -> String {
    let mut fragments = history
        .iter()
        .filter_map(|message| match message {
            Message::User { content } => {
                let text = content
                    .iter()
                    .filter_map(|item| match item {
                        UserContent::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                (!text.trim().is_empty()).then(|| format!("用户：{}", truncate_chars(&text, FALLBACK_FRAGMENT_CHARS)))
            }
            Message::Assistant { content, .. } => {
                let text = content
                    .iter()
                    .filter_map(|item| match item {
                        AssistantContent::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                (!text.trim().is_empty()).then(|| format!("Agent：{}", truncate_chars(&text, FALLBACK_FRAGMENT_CHARS)))
            }
            Message::System { .. } => None,
        })
        .collect::<Vec<_>>();
    while fragments.join("\n").chars().count() > FALLBACK_SUMMARY_CHARS && fragments.len() > 1 {
        fragments.remove(0);
    }
    let summary = fragments.join("\n");
    if summary.is_empty() {
        "早期历史已压缩；后续以 Workspace 当前产物和最近工具结果为准。".to_string()
    } else {
        summary
    }
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        value.to_string()
    } else {
        value.chars().take(limit).collect::<String>() + "…"
    }
}

fn token_bytes(tokens: u64) -> usize {
    usize::try_from(tokens.saturating_mul(4)).unwrap_or(usize::MAX)
}

fn estimate_text_tokens(value: &str) -> u64 {
    (value.len() as u64).saturating_add(3) / 4
}

fn truncate_utf8_bytes(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut end = limit.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string() + "…"
}

#[cfg(test)]
#[path = "context_compaction_tests.rs"]
mod tests;
