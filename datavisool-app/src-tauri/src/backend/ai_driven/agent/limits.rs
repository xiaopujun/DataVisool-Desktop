/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Agent 统一执行预算、取消信号和用量快照。
//!
//! 所有计数和结果记录都通过同一把锁更新，克隆后的预算共享状态，适用于 Hook、
//! 模型调用和工具调用从不同线程上报用量。时间限制使用单调时钟，不受系统时间回拨影响。

use std::fmt::{Display, Formatter};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rig_core::completion::Usage;
use serde::{Deserialize, Serialize};

use crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceFileLimits;

/// 单次 Agent 运行的全部硬限制。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentExecutionLimits {
    pub token_budget_enabled: bool,
    pub call_budget_enabled: bool,
    pub duration_budget_enabled: bool,
    pub max_model_calls: u64,
    pub max_tool_calls: u64,
    pub max_write_calls: u64,
    pub max_total_tokens: u64,
    pub max_duration: Duration,
    pub max_tool_duration: Duration,
    pub max_consecutive_failures: u64,
    pub max_frontend_interactions: u64,
    pub max_same_read_repeats: u64,
    pub max_completion_continuations: u64,
    pub workspace_files: WorkspaceFileLimits,
}

/// 活动模型窗口与单项工具结果的限制。持久化窗口不使用这些值。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentContextLimits {
    pub max_active_context_tokens: u64,
    pub compact_at_percent: u8,
    pub max_tool_result_tokens: u64,
    pub max_tool_history_tokens: u64,
    pub max_state_digest_tokens: u64,
    pub compact_summary_tokens: u64,
}

impl Default for AgentContextLimits {
    fn default() -> Self {
        Self {
            max_active_context_tokens: 128_000,
            compact_at_percent: 90,
            max_tool_result_tokens: 10_000,
            max_tool_history_tokens: 16_000,
            max_state_digest_tokens: 4_096,
            compact_summary_tokens: 4_096,
        }
    }
}

impl Default for AgentExecutionLimits {
    fn default() -> Self {
        Self {
            token_budget_enabled: true,
            call_budget_enabled: true,
            duration_budget_enabled: true,
            max_model_calls: 120,
            max_tool_calls: 320,
            max_write_calls: 120,
            max_total_tokens: 1_000_000,
            max_duration: Duration::from_secs(75 * 60),
            max_tool_duration: Duration::from_secs(10 * 60),
            max_consecutive_failures: 25,
            max_frontend_interactions: 40,
            max_same_read_repeats: 3,
            max_completion_continuations: 12,
            workspace_files: WorkspaceFileLimits::default(),
        }
    }
}

/// 工具调用的受限类别；所有类别都计入工具调用总数。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolCallKind {
    Read,
    Write,
    FrontendInteraction,
    WriteFrontendInteraction,
}

/// 工具调用完成状态，用于维护连续失败计数。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolCallOutcome {
    Succeeded,
    Failed,
}

/// 可在任务注册表、Runner 和长时间领域服务之间共享的取消信号。
#[derive(Clone, Debug, Default)]
pub(crate) struct ExecutionCancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl ExecutionCancellationToken {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// 任意时刻可安全读取、可序列化的累计用量快照。
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ExecutionUsageSnapshot {
    pub token_budget_enabled: bool,
    pub call_budget_enabled: bool,
    pub duration_budget_enabled: bool,
    pub active_context_tokens: u64,
    pub context_capacity_tokens: u64,
    pub context_capacity_source: String,
    pub compaction_count: u64,
    pub compaction_before_tokens: u64,
    pub compaction_after_tokens: u64,
    pub budget_warning_issued: bool,
    pub model_calls: u64,
    pub tool_calls: u64,
    pub write_calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub tool_use_prompt_tokens: u64,
    pub reasoning_tokens: u64,
    pub context_input_tokens: u64,
    pub usage_reported: bool,
    pub estimated_token_calls: u64,
    pub elapsed_ms: u64,
    pub last_tool_duration_ms: Option<u64>,
    pub max_observed_tool_duration_ms: u64,
    pub consecutive_failures: u64,
    pub frontend_interactions: u64,
    pub remaining_model_calls: Option<u64>,
    pub remaining_tool_calls: Option<u64>,
    pub remaining_write_calls: Option<u64>,
    pub remaining_total_tokens: Option<u64>,
    pub remaining_frontend_interactions: Option<u64>,
    pub remaining_duration_ms: Option<u64>,
    pub cancelled: bool,
}

#[derive(Debug, Default)]
struct ExecutionUsage {
    active_context_tokens: u64,
    context_capacity_tokens: u64,
    context_capacity_source: String,
    compaction_count: u64,
    compaction_before_tokens: u64,
    compaction_after_tokens: u64,
    budget_warning_issued: bool,
    restored_elapsed_ms: u64,
    model_calls: u64,
    tool_calls: u64,
    write_calls: u64,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    cached_input_tokens: u64,
    cache_creation_input_tokens: u64,
    tool_use_prompt_tokens: u64,
    reasoning_tokens: u64,
    context_input_tokens: u64,
    usage_reported: bool,
    estimated_token_calls: u64,
    last_tool_duration: Option<Duration>,
    max_observed_tool_duration: Duration,
    consecutive_failures: u64,
    frontend_interactions: u64,
}

/// 机器可读且跨调用点保持稳定的执行终止原因。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum AgentExecutionErrorCode {
    #[serde(rename = "agent_execution.cancelled")]
    Cancelled,
    #[serde(rename = "agent_execution.model_call_limit")]
    ModelCallLimit,
    #[serde(rename = "agent_execution.ready_phase_model_call_limit")]
    ReadyPhaseModelCallLimit,
    #[serde(rename = "agent_execution.tool_call_limit")]
    ToolCallLimit,
    #[serde(rename = "agent_execution.write_call_limit")]
    WriteCallLimit,
    #[serde(rename = "agent_execution.token_limit")]
    TokenLimit,
    #[serde(rename = "agent_execution.ready_phase_token_limit")]
    ReadyPhaseTokenLimit,
    #[serde(rename = "agent_execution.duration_limit")]
    DurationLimit,
    #[serde(rename = "agent_execution.tool_duration_limit")]
    ToolDurationLimit,
    #[serde(rename = "agent_execution.consecutive_failure_limit")]
    ConsecutiveFailureLimit,
    #[serde(rename = "agent_execution.frontend_interaction_limit")]
    FrontendInteractionLimit,
    #[serde(rename = "agent_execution.repeated_read_limit")]
    RepeatedReadLimit,
    #[serde(rename = "agent_execution.no_progress_limit")]
    NoProgressLimit,
    #[serde(rename = "agent.tool.not_advertised_in_step")]
    ToolNotAdvertisedInStep,
    #[serde(rename = "agent_execution.completion_continuation_limit")]
    CompletionContinuationLimit,
}

impl AgentExecutionErrorCode {
    pub(crate) fn is_run_budget(self) -> bool {
        matches!(
            self,
            Self::ModelCallLimit
                | Self::ToolCallLimit
                | Self::WriteCallLimit
                | Self::TokenLimit
                | Self::DurationLimit
                | Self::FrontendInteractionLimit
        )
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "agent_execution.cancelled",
            Self::ModelCallLimit => "agent_execution.model_call_limit",
            Self::ReadyPhaseModelCallLimit => "agent_execution.ready_phase_model_call_limit",
            Self::ToolCallLimit => "agent_execution.tool_call_limit",
            Self::WriteCallLimit => "agent_execution.write_call_limit",
            Self::TokenLimit => "agent_execution.token_limit",
            Self::ReadyPhaseTokenLimit => "agent_execution.ready_phase_token_limit",
            Self::DurationLimit => "agent_execution.duration_limit",
            Self::ToolDurationLimit => "agent_execution.tool_duration_limit",
            Self::ConsecutiveFailureLimit => "agent_execution.consecutive_failure_limit",
            Self::FrontendInteractionLimit => "agent_execution.frontend_interaction_limit",
            Self::RepeatedReadLimit => "agent_execution.repeated_read_limit",
            Self::NoProgressLimit => "agent_execution.no_progress_limit",
            Self::ToolNotAdvertisedInStep => "agent.tool.not_advertised_in_step",
            Self::CompletionContinuationLimit => "agent_execution.completion_continuation_limit",
        }
    }
}

impl Display for AgentExecutionErrorCode {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 限制或取消触发时返回的结构化错误，始终携带触发时的用量。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentExecutionError {
    pub code: AgentExecutionErrorCode,
    pub message: String,
    pub usage: ExecutionUsageSnapshot,
}

impl Display for AgentExecutionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AgentExecutionError {}

/// 一次 Agent 运行共享的线程安全预算。
#[derive(Clone, Debug)]
pub(crate) struct ExecutionBudget {
    limits: Arc<AgentExecutionLimits>,
    started_at: Instant,
    cancellation: ExecutionCancellationToken,
    usage: Arc<Mutex<ExecutionUsage>>,
}

impl ExecutionBudget {
    pub(crate) fn new(limits: AgentExecutionLimits) -> Self {
        Self::with_cancellation(limits, ExecutionCancellationToken::default())
    }

    pub(crate) fn with_cancellation(limits: AgentExecutionLimits, cancellation: ExecutionCancellationToken) -> Self {
        Self {
            limits: Arc::new(limits),
            started_at: Instant::now(),
            cancellation,
            usage: Arc::new(Mutex::new(ExecutionUsage::default())),
        }
    }

    pub(crate) fn cancellation_token(&self) -> ExecutionCancellationToken {
        self.cancellation.clone()
    }

    pub(crate) fn limits(&self) -> &AgentExecutionLimits {
        self.limits.as_ref()
    }

    /// 恢复逻辑任务的累计用量；重复恢复取最大值，不能重复累加或清零额度。
    pub(crate) fn restore_usage(&self, snapshot: &ExecutionUsageSnapshot) {
        let mut usage = self.lock_usage();
        macro_rules! restore { ($($field:ident),* $(,)?) => { $(usage.$field = usage.$field.max(snapshot.$field);)* }; }
        restore!(
            model_calls,
            tool_calls,
            write_calls,
            input_tokens,
            output_tokens,
            total_tokens,
            cached_input_tokens,
            cache_creation_input_tokens,
            tool_use_prompt_tokens,
            reasoning_tokens,
            context_input_tokens,
            estimated_token_calls,
            consecutive_failures,
            frontend_interactions
        );
        usage.usage_reported |= snapshot.usage_reported;
        usage.restored_elapsed_ms = usage.restored_elapsed_ms.max(snapshot.elapsed_ms);
        usage.budget_warning_issued |= snapshot.budget_warning_issued;
        usage.compaction_count = usage.compaction_count.max(snapshot.compaction_count);
        usage.max_observed_tool_duration = usage
            .max_observed_tool_duration
            .max(Duration::from_millis(snapshot.max_observed_tool_duration_ms));
    }

    /// 在模型、工具、前端等待及长任务阶段边界调用。
    pub(crate) fn check(&self) -> Result<(), AgentExecutionError> {
        let usage = self.lock_usage();
        self.ensure_runtime_available(&usage)
    }

    /// 模型调用前原子预留一次调用额度。
    pub(crate) fn start_model_call(&self) -> Result<(), AgentExecutionError> {
        let mut usage = self.lock_usage();
        self.ensure_runtime_available(&usage)?;
        if self.limits.call_budget_enabled && usage.model_calls >= self.limits.max_model_calls {
            return Err(self.error(AgentExecutionErrorCode::ModelCallLimit, "达到模型调用次数上限", &usage));
        }
        usage.model_calls += 1;
        Ok(())
    }

    /// 模型调用后记录真实 Token；即使越界也保留实际用量。
    pub(crate) fn finish_model_call(&self, input_tokens: u64, output_tokens: u64) -> Result<(), AgentExecutionError> {
        self.finish_model_call_with_estimate(input_tokens, output_tokens, false)
    }

    pub(crate) fn finish_model_call_with_estimate(&self, input_tokens: u64, output_tokens: u64, estimated: bool) -> Result<(), AgentExecutionError> {
        let usage = Usage {
            input_tokens,
            output_tokens,
            total_tokens: input_tokens.saturating_add(output_tokens),
            ..Usage::default()
        };
        self.finish_model_call_with_usage(&usage, input_tokens, output_tokens, estimated)
    }

    /// 记录 Rig 完整 Usage；Provider 未返回 Usage 时才使用本地估算值。
    pub(crate) fn finish_model_call_with_usage(
        &self,
        provider_usage: &Usage,
        estimated_input_tokens: u64,
        estimated_output_tokens: u64,
        estimated: bool,
    ) -> Result<(), AgentExecutionError> {
        let mut usage = self.lock_usage();
        let reported = provider_usage.has_values() && !estimated;
        let input_tokens = if reported { provider_usage.input_tokens } else { estimated_input_tokens };
        let output_tokens = if reported {
            provider_usage.output_tokens
        } else {
            estimated_output_tokens
        };
        let total_tokens = if reported && provider_usage.total_tokens > 0 {
            provider_usage.total_tokens
        } else {
            input_tokens.saturating_add(output_tokens)
        };
        usage.input_tokens = usage.input_tokens.saturating_add(input_tokens);
        usage.output_tokens = usage.output_tokens.saturating_add(output_tokens);
        usage.total_tokens = usage.total_tokens.saturating_add(total_tokens);
        if reported {
            usage.cached_input_tokens = usage.cached_input_tokens.saturating_add(provider_usage.cached_input_tokens);
            usage.cache_creation_input_tokens = usage
                .cache_creation_input_tokens
                .saturating_add(provider_usage.cache_creation_input_tokens);
            usage.tool_use_prompt_tokens = usage
                .tool_use_prompt_tokens
                .saturating_add(provider_usage.tool_use_prompt_tokens);
            usage.reasoning_tokens = usage.reasoning_tokens.saturating_add(provider_usage.reasoning_tokens);
            usage.context_input_tokens = usage
                .context_input_tokens
                .saturating_add(provider_context_input_tokens(provider_usage, input_tokens));
            usage.usage_reported = true;
        } else {
            usage.context_input_tokens = usage.context_input_tokens.saturating_add(input_tokens);
        }
        usage.estimated_token_calls = usage
            .estimated_token_calls
            .saturating_add(u64::from(estimated || !provider_usage.has_values()));
        self.ensure_runtime_available(&usage)?;
        Ok(())
    }

    /// 工具调用前同时检查总量和工具类别额度，全部通过后才更新计数。
    pub(crate) fn start_tool_call(&self, kind: ToolCallKind) -> Result<(), AgentExecutionError> {
        let mut usage = self.lock_usage();
        self.ensure_runtime_available(&usage)?;
        if self.limits.call_budget_enabled && usage.tool_calls >= self.limits.max_tool_calls {
            return Err(self.error(AgentExecutionErrorCode::ToolCallLimit, "达到工具调用次数上限", &usage));
        }
        let is_write = matches!(kind, ToolCallKind::Write | ToolCallKind::WriteFrontendInteraction);
        let is_frontend = matches!(kind, ToolCallKind::FrontendInteraction | ToolCallKind::WriteFrontendInteraction);
        if self.limits.call_budget_enabled && is_write && usage.write_calls >= self.limits.max_write_calls {
            return Err(self.error(AgentExecutionErrorCode::WriteCallLimit, "达到写入工具调用次数上限", &usage));
        }
        if self.limits.call_budget_enabled && is_frontend && usage.frontend_interactions >= self.limits.max_frontend_interactions {
            return Err(self.error(AgentExecutionErrorCode::FrontendInteractionLimit, "达到前端交互次数上限", &usage));
        }

        usage.tool_calls += 1;
        if is_write {
            usage.write_calls += 1;
        }
        if is_frontend {
            usage.frontend_interactions += 1;
        }
        Ok(())
    }

    /// 工具调用后记录耗时和结果；失败达到阈值时立即终止，成功会清零连续失败。
    pub(crate) fn finish_tool_call(&self, duration: Duration, outcome: ToolCallOutcome) -> Result<(), AgentExecutionError> {
        let mut usage = self.lock_usage();
        record_tool_outcome(&mut usage, duration, outcome);

        self.ensure_runtime_available(&usage)?;
        if duration > self.limits.max_tool_duration {
            return Err(self.error(AgentExecutionErrorCode::ToolDurationLimit, "单次工具执行时间超过上限", &usage));
        }
        if outcome == ToolCallOutcome::Failed && usage.consecutive_failures >= self.limits.max_consecutive_failures {
            return Err(self.error(AgentExecutionErrorCode::ConsecutiveFailureLimit, "连续失败工具调用达到上限", &usage));
        }
        Ok(())
    }

    /// 统一 ToolDyn 超时边界已经终止了运行中的 Future；该事实不依赖 Hook 计时精度。
    pub(crate) fn finish_timed_out_tool_call(&self, duration: Duration) -> AgentExecutionError {
        let mut usage = self.lock_usage();
        record_tool_outcome(&mut usage, duration, ToolCallOutcome::Failed);
        self.error(AgentExecutionErrorCode::ToolDurationLimit, "单次工具执行时间超过上限", &usage)
    }

    pub(crate) fn usage_snapshot(&self) -> ExecutionUsageSnapshot {
        let usage = self.lock_usage();
        self.snapshot(&usage)
    }

    fn ensure_runtime_available(&self, usage: &ExecutionUsage) -> Result<(), AgentExecutionError> {
        if self.cancellation.is_cancelled() {
            return Err(self.error(AgentExecutionErrorCode::Cancelled, "Agent 任务已取消", usage));
        }
        if self.limits.token_budget_enabled && usage.total_tokens >= self.limits.max_total_tokens {
            return Err(self.error(AgentExecutionErrorCode::TokenLimit, "达到任务累计 Token 上限", usage));
        }
        if self.limits.duration_budget_enabled && self.elapsed(usage) >= self.limits.max_duration {
            return Err(self.error(AgentExecutionErrorCode::DurationLimit, "Agent 任务执行时间超过上限", usage));
        }
        Ok(())
    }

    fn error(&self, code: AgentExecutionErrorCode, message: impl Into<String>, usage: &ExecutionUsage) -> AgentExecutionError {
        AgentExecutionError { code, message: message.into(), usage: self.snapshot(usage) }
    }

    fn snapshot(&self, usage: &ExecutionUsage) -> ExecutionUsageSnapshot {
        ExecutionUsageSnapshot {
            token_budget_enabled: self.limits.token_budget_enabled,
            call_budget_enabled: self.limits.call_budget_enabled,
            duration_budget_enabled: self.limits.duration_budget_enabled,
            active_context_tokens: usage.active_context_tokens,
            context_capacity_tokens: usage.context_capacity_tokens,
            context_capacity_source: usage.context_capacity_source.clone(),
            compaction_count: usage.compaction_count,
            compaction_before_tokens: usage.compaction_before_tokens,
            compaction_after_tokens: usage.compaction_after_tokens,
            budget_warning_issued: usage.budget_warning_issued,
            model_calls: usage.model_calls,
            tool_calls: usage.tool_calls,
            write_calls: usage.write_calls,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            cache_creation_input_tokens: usage.cache_creation_input_tokens,
            tool_use_prompt_tokens: usage.tool_use_prompt_tokens,
            reasoning_tokens: usage.reasoning_tokens,
            context_input_tokens: usage.context_input_tokens,
            usage_reported: usage.usage_reported,
            estimated_token_calls: usage.estimated_token_calls,
            elapsed_ms: duration_ms(self.elapsed(usage)),
            last_tool_duration_ms: usage.last_tool_duration.map(duration_ms),
            max_observed_tool_duration_ms: duration_ms(usage.max_observed_tool_duration),
            consecutive_failures: usage.consecutive_failures,
            frontend_interactions: usage.frontend_interactions,
            remaining_model_calls: self
                .limits
                .call_budget_enabled
                .then(|| self.limits.max_model_calls.saturating_sub(usage.model_calls)),
            remaining_tool_calls: self
                .limits
                .call_budget_enabled
                .then(|| self.limits.max_tool_calls.saturating_sub(usage.tool_calls)),
            remaining_write_calls: self
                .limits
                .call_budget_enabled
                .then(|| self.limits.max_write_calls.saturating_sub(usage.write_calls)),
            remaining_total_tokens: self
                .limits
                .token_budget_enabled
                .then(|| self.limits.max_total_tokens.saturating_sub(usage.total_tokens)),
            remaining_frontend_interactions: self.limits.call_budget_enabled.then(|| {
                self.limits
                    .max_frontend_interactions
                    .saturating_sub(usage.frontend_interactions)
            }),
            remaining_duration_ms: self
                .limits
                .duration_budget_enabled
                .then(|| duration_ms(self.limits.max_duration.saturating_sub(self.elapsed(usage)))),
            cancelled: self.cancellation.is_cancelled(),
        }
    }

    fn elapsed(&self, usage: &ExecutionUsage) -> Duration {
        Duration::from_millis(usage.restored_elapsed_ms).saturating_add(self.started_at.elapsed())
    }

    pub(crate) fn record_context(&self, active: u64, capacity: u64, source: &str) {
        let mut usage = self.lock_usage();
        usage.active_context_tokens = active;
        usage.context_capacity_tokens = capacity;
        usage.context_capacity_source = source.to_string();
    }

    pub(crate) fn record_compaction(&self, before: u64, after: u64) {
        let mut usage = self.lock_usage();
        usage.compaction_count = usage.compaction_count.saturating_add(1);
        usage.compaction_before_tokens = before;
        usage.compaction_after_tokens = after;
        usage.active_context_tokens = after;
    }

    /// One warning per budget authorization, persisted with usage across attempts.
    pub(crate) fn take_budget_warning(&self) -> Option<String> {
        let mut usage = self.lock_usage();
        let near = |used: u64, cap: u64| used >= cap.saturating_mul(4) / 5;
        let reached = self.limits.token_budget_enabled && near(usage.total_tokens, self.limits.max_total_tokens)
            || self.limits.call_budget_enabled
                && (near(usage.model_calls, self.limits.max_model_calls)
                    || near(usage.tool_calls, self.limits.max_tool_calls)
                    || near(usage.write_calls, self.limits.max_write_calls)
                    || near(usage.frontend_interactions, self.limits.max_frontend_interactions))
            || self.limits.duration_budget_enabled && near(duration_ms(self.elapsed(&usage)), duration_ms(self.limits.max_duration));
        if usage.budget_warning_issued || !reached {
            return None;
        }
        usage.budget_warning_issued = true;
        Some("用户授权的运行预算已接近上限。优先完成当前批次并保存剩余工作；预算耗尽后须由用户延长预算才能继续。".to_string())
    }

    fn lock_usage(&self) -> MutexGuard<'_, ExecutionUsage> {
        self.usage.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn record_tool_outcome(usage: &mut ExecutionUsage, duration: Duration, outcome: ToolCallOutcome) {
    usage.last_tool_duration = Some(duration);
    usage.max_observed_tool_duration = usage.max_observed_tool_duration.max(duration);
    match outcome {
        ToolCallOutcome::Succeeded => usage.consecutive_failures = 0,
        ToolCallOutcome::Failed => usage.consecutive_failures = usage.consecutive_failures.saturating_add(1),
    }
}

pub(crate) fn provider_context_input_tokens(usage: &Usage, fallback_input_tokens: u64) -> u64 {
    if usage.total_tokens > 0 {
        usage.total_tokens.saturating_sub(usage.output_tokens)
    } else {
        fallback_input_tokens
            .saturating_add(usage.cached_input_tokens)
            .saturating_add(usage.cache_creation_input_tokens)
    }
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
