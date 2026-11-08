/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::thread;

use super::*;

fn limits() -> AgentExecutionLimits {
    AgentExecutionLimits {
        token_budget_enabled: true,
        call_budget_enabled: true,
        duration_budget_enabled: true,
        max_model_calls: 2,
        max_tool_calls: 3,
        max_write_calls: 1,
        max_total_tokens: 10,
        max_duration: Duration::from_secs(60),
        max_tool_duration: Duration::from_millis(50),
        max_consecutive_failures: 2,
        max_frontend_interactions: 1,
        max_same_read_repeats: 3,
        max_completion_continuations: 12,
        workspace_files: WorkspaceFileLimits::default(),
    }
}

#[test]
fn enforces_model_call_limit_without_overcounting() {
    let budget = ExecutionBudget::new(limits());
    budget.start_model_call().unwrap();
    budget.start_model_call().unwrap();

    let error = budget.start_model_call().unwrap_err();
    assert_eq!(error.code, AgentExecutionErrorCode::ModelCallLimit);
    assert_eq!(error.usage.model_calls, 2);
}

#[test]
fn records_actual_tokens_when_token_limit_is_exceeded() {
    let budget = ExecutionBudget::new(limits());
    budget.finish_model_call(6, 3).unwrap();

    let error = budget.finish_model_call(2, 0).unwrap_err();
    assert_eq!(error.code, AgentExecutionErrorCode::TokenLimit);
    assert_eq!(error.usage.input_tokens, 8);
    assert_eq!(error.usage.output_tokens, 3);
    assert_eq!(error.usage.total_tokens, 11);
}

#[test]
fn marks_only_fallback_token_measurements_as_estimated() {
    let budget = ExecutionBudget::new(limits());
    budget.finish_model_call_with_estimate(2, 1, true).unwrap();
    budget.finish_model_call_with_estimate(2, 1, false).unwrap();

    let usage = budget.usage_snapshot();
    assert_eq!(usage.estimated_token_calls, 1);
    assert_eq!(usage.remaining_total_tokens, Some(4));
}

#[test]
fn accumulates_all_reported_rig_usage_fields() {
    let budget = ExecutionBudget::new(AgentExecutionLimits { max_total_tokens: 100, ..limits() });
    let provider_usage = Usage {
        input_tokens: 10,
        output_tokens: 3,
        total_tokens: 20,
        cached_input_tokens: 2,
        cache_creation_input_tokens: 1,
        tool_use_prompt_tokens: 4,
        reasoning_tokens: 5,
    };

    budget.finish_model_call_with_usage(&provider_usage, 99, 99, false).unwrap();

    let usage = budget.usage_snapshot();
    assert_eq!(usage.input_tokens, 10);
    assert_eq!(usage.output_tokens, 3);
    assert_eq!(usage.total_tokens, 20);
    assert_eq!(usage.cached_input_tokens, 2);
    assert_eq!(usage.cache_creation_input_tokens, 1);
    assert_eq!(usage.tool_use_prompt_tokens, 4);
    assert_eq!(usage.reasoning_tokens, 5);
    assert_eq!(usage.context_input_tokens, 17);
    assert!(usage.usage_reported);
}

#[test]
fn old_usage_snapshots_remain_deserializable() {
    let usage: ExecutionUsageSnapshot =
        serde_json::from_str(r#"{"modelCalls":1,"toolCalls":0,"writeCalls":0,"inputTokens":4,"outputTokens":2,"totalTokens":6}"#).unwrap();

    assert_eq!(usage.total_tokens, 6);
    assert_eq!(usage.cached_input_tokens, 0);
    assert!(!usage.usage_reported);
}

#[test]
fn enforces_tool_call_limit() {
    let mut configured = limits();
    configured.max_write_calls = 3;
    let budget = ExecutionBudget::new(configured);
    budget.start_tool_call(ToolCallKind::Read).unwrap();
    budget.start_tool_call(ToolCallKind::Write).unwrap();
    budget.start_tool_call(ToolCallKind::Read).unwrap();

    let error = budget.start_tool_call(ToolCallKind::Read).unwrap_err();
    assert_eq!(error.code, AgentExecutionErrorCode::ToolCallLimit);
    assert_eq!(error.usage.tool_calls, 3);
}

#[test]
fn enforces_write_call_limit_atomically() {
    let budget = ExecutionBudget::new(limits());
    budget.start_tool_call(ToolCallKind::Write).unwrap();

    let error = budget.start_tool_call(ToolCallKind::Write).unwrap_err();
    assert_eq!(error.code, AgentExecutionErrorCode::WriteCallLimit);
    assert_eq!(error.usage.tool_calls, 1);
    assert_eq!(error.usage.write_calls, 1);
}

#[test]
fn enforces_frontend_interaction_limit_atomically() {
    let budget = ExecutionBudget::new(limits());
    budget.start_tool_call(ToolCallKind::FrontendInteraction).unwrap();

    let error = budget.start_tool_call(ToolCallKind::FrontendInteraction).unwrap_err();
    assert_eq!(error.code, AgentExecutionErrorCode::FrontendInteractionLimit);
    assert_eq!(error.usage.tool_calls, 1);
    assert_eq!(error.usage.frontend_interactions, 1);
}

#[test]
fn write_frontend_interaction_consumes_both_budgets_atomically() {
    let mut configured = limits();
    configured.max_write_calls = 2;
    let budget = ExecutionBudget::new(configured);
    budget.start_tool_call(ToolCallKind::WriteFrontendInteraction).unwrap();

    let usage = budget.usage_snapshot();
    assert_eq!(usage.tool_calls, 1);
    assert_eq!(usage.write_calls, 1);
    assert_eq!(usage.frontend_interactions, 1);

    let error = budget.start_tool_call(ToolCallKind::WriteFrontendInteraction).unwrap_err();
    assert_eq!(error.code, AgentExecutionErrorCode::FrontendInteractionLimit);
    assert_eq!(error.usage.tool_calls, 1);
    assert_eq!(error.usage.write_calls, 1);
    assert_eq!(error.usage.frontend_interactions, 1);
}

#[test]
fn enforces_single_tool_duration() {
    let budget = ExecutionBudget::new(limits());
    budget.start_tool_call(ToolCallKind::Read).unwrap();

    let error = budget
        .finish_tool_call(Duration::from_millis(51), ToolCallOutcome::Succeeded)
        .unwrap_err();
    assert_eq!(error.code, AgentExecutionErrorCode::ToolDurationLimit);
    assert_eq!(error.usage.last_tool_duration_ms, Some(51));
}

#[test]
fn structured_timeout_always_uses_the_tool_duration_limit_code() {
    let budget = ExecutionBudget::new(limits());
    budget.start_tool_call(ToolCallKind::Read).unwrap();

    let error = budget.finish_timed_out_tool_call(Duration::from_millis(50));

    assert_eq!(error.code, AgentExecutionErrorCode::ToolDurationLimit);
    assert_eq!(error.usage.last_tool_duration_ms, Some(50));
    assert_eq!(error.usage.consecutive_failures, 1);
}

#[test]
fn enforces_and_resets_consecutive_failures() {
    let budget = ExecutionBudget::new(limits());
    budget.finish_tool_call(Duration::ZERO, ToolCallOutcome::Failed).unwrap();
    budget.finish_tool_call(Duration::ZERO, ToolCallOutcome::Succeeded).unwrap();
    budget.finish_tool_call(Duration::ZERO, ToolCallOutcome::Failed).unwrap();

    let error = budget.finish_tool_call(Duration::ZERO, ToolCallOutcome::Failed).unwrap_err();
    assert_eq!(error.code, AgentExecutionErrorCode::ConsecutiveFailureLimit);
    assert_eq!(error.usage.consecutive_failures, 2);
}

#[test]
fn enforces_total_duration() {
    let mut configured = limits();
    configured.max_duration = Duration::ZERO;
    let budget = ExecutionBudget::new(configured);

    let error = budget.check().unwrap_err();
    assert_eq!(error.code, AgentExecutionErrorCode::DurationLimit);
}

#[test]
fn cancellation_is_checked_before_and_after_calls() {
    let budget = ExecutionBudget::new(limits());
    budget.start_tool_call(ToolCallKind::Read).unwrap();
    budget.cancellation_token().cancel();

    let finish_error = budget
        .finish_tool_call(Duration::from_millis(4), ToolCallOutcome::Succeeded)
        .unwrap_err();
    assert_eq!(finish_error.code, AgentExecutionErrorCode::Cancelled);
    assert_eq!(finish_error.usage.last_tool_duration_ms, Some(4));
    assert!(finish_error.usage.cancelled);

    let start_error = budget.start_model_call().unwrap_err();
    assert_eq!(start_error.code, AgentExecutionErrorCode::Cancelled);
}

#[test]
fn snapshots_are_consistent_across_threads() {
    let mut configured = limits();
    configured.max_model_calls = 800;
    let budget = ExecutionBudget::new(configured);
    let workers = (0..8)
        .map(|_| {
            let budget = budget.clone();
            thread::spawn(move || {
                for _ in 0..100 {
                    budget.start_model_call().unwrap();
                }
            })
        })
        .collect::<Vec<_>>();

    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(budget.usage_snapshot().model_calls, 800);
}

#[test]
fn error_codes_have_stable_wire_values() {
    assert_eq!(AgentExecutionErrorCode::Cancelled.as_str(), "agent_execution.cancelled");
    assert_eq!(AgentExecutionErrorCode::ModelCallLimit.as_str(), "agent_execution.model_call_limit");
    assert_eq!(AgentExecutionErrorCode::ToolCallLimit.as_str(), "agent_execution.tool_call_limit");
    assert_eq!(AgentExecutionErrorCode::WriteCallLimit.as_str(), "agent_execution.write_call_limit");
    assert_eq!(AgentExecutionErrorCode::TokenLimit.as_str(), "agent_execution.token_limit");
    assert_eq!(AgentExecutionErrorCode::RepeatedReadLimit.as_str(), "agent_execution.repeated_read_limit");
    assert_eq!(AgentExecutionErrorCode::NoProgressLimit.as_str(), "agent_execution.no_progress_limit");
    assert_eq!(AgentExecutionErrorCode::ToolNotAdvertisedInStep.as_str(), "agent.tool.not_advertised_in_step");
    assert_eq!(AgentExecutionErrorCode::CompletionContinuationLimit.as_str(), "agent_execution.completion_continuation_limit");
    assert_eq!(AgentExecutionErrorCode::DurationLimit.as_str(), "agent_execution.duration_limit");
    assert_eq!(AgentExecutionErrorCode::ToolDurationLimit.as_str(), "agent_execution.tool_duration_limit");
    assert_eq!(AgentExecutionErrorCode::ConsecutiveFailureLimit.as_str(), "agent_execution.consecutive_failure_limit");
    assert_eq!(AgentExecutionErrorCode::FrontendInteractionLimit.as_str(), "agent_execution.frontend_interaction_limit");
}
#[test]
fn restoring_usage_preserves_budget_without_double_counting() {
    let budget = ExecutionBudget::new(AgentExecutionLimits::default());
    let snapshot = ExecutionUsageSnapshot {
        model_calls: 5,
        input_tokens: 100,
        output_tokens: 20,
        total_tokens: 120,
        cached_input_tokens: 40,
        ..Default::default()
    };
    budget.restore_usage(&snapshot);
    budget.restore_usage(&snapshot);
    budget.start_model_call().unwrap();
    let usage = budget.usage_snapshot();
    assert_eq!(usage.model_calls, 6);
    assert_eq!(usage.total_tokens, 120);
    assert_eq!(usage.cached_input_tokens, 40);
}

#[test]
fn automatic_context_mode_has_no_implicit_run_budget() {
    let mut limits = limits();
    limits.token_budget_enabled = false;
    limits.call_budget_enabled = false;
    limits.duration_budget_enabled = false;
    limits.max_duration = Duration::ZERO;
    let budget = ExecutionBudget::new(limits);
    for _ in 0..35 {
        budget.start_model_call().unwrap();
        budget.finish_model_call(10_000, 100).unwrap();
        budget.start_tool_call(ToolCallKind::Read).unwrap();
        budget.finish_tool_call(Duration::ZERO, ToolCallOutcome::Succeeded).unwrap();
    }
    assert_eq!(budget.usage_snapshot().input_tokens, 350_000);
    assert!(budget.take_budget_warning().is_none());
}

#[test]
fn exact_token_exhaustion_blocks_next_dispatch() {
    let budget = ExecutionBudget::new(limits());
    assert_eq!(budget.finish_model_call(9, 1).unwrap_err().code, AgentExecutionErrorCode::TokenLimit);
    assert!(budget.start_tool_call(ToolCallKind::Write).is_err());
    assert_eq!(budget.usage_snapshot().tool_calls, 0);
}

#[test]
fn restored_duration_and_warning_cannot_reset() {
    let budget = ExecutionBudget::new(limits());
    let snapshot = ExecutionUsageSnapshot { elapsed_ms: 60_000, budget_warning_issued: true, ..Default::default() };
    budget.restore_usage(&snapshot);
    budget.restore_usage(&snapshot);
    assert_eq!(budget.check().unwrap_err().code, AgentExecutionErrorCode::DurationLimit);
    assert!(budget.usage_snapshot().elapsed_ms >= 60_000);
    assert!(budget.usage_snapshot().elapsed_ms < 61_000);
    assert!(budget.take_budget_warning().is_none());
}

#[test]
fn warning_is_once_and_context_is_separate_from_cumulative_usage() {
    let budget = ExecutionBudget::new(limits());
    budget.finish_model_call(7, 1).unwrap();
    assert!(budget.take_budget_warning().is_some());
    assert!(budget.take_budget_warning().is_none());
    budget.record_context(2000, 10000, "fallback");
    budget.record_compaction(2000, 500);
    let usage = budget.usage_snapshot();
    assert_eq!(usage.input_tokens, 7);
    assert_eq!(usage.active_context_tokens, 500);
    assert_eq!(usage.compaction_count, 1);
}
