/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 上下文压缩的预算与 Timeline 事件。

use rig_core::agent::{Flow, HookContext};
use rig_core::completion::Usage;
use serde_json::json;

use super::AgentRuntimeHook;
use crate::backend::ai_driven::agent::limits::provider_context_input_tokens;
use crate::backend::utils::{debug_log_json, sanitize_debug_log_text};

impl AgentRuntimeHook {
    pub(crate) async fn begin_context_compaction(&self, ctx: &HookContext, compaction_count: u64, input_tokens: u64, message_count: usize) -> Flow {
        if let Err(error) = self.budget.start_model_call() {
            return self.terminate_for_budget(ctx, error).await;
        }
        tracing::debug!(
            target: "agent.context",
            event = "context_compaction_started",
            project_id = self.task.project_id,
            thread_id = %self.task.thread_id,
            task_id = %self.task.task_id,
            turn_id = %self.task.turn_id,
            run_id = %self.task.run_id,
            request_id = %self.task.request_id,
            step_index = ctx.turn(),
            compaction_count,
            input_tokens,
            message_count,
            usage = %debug_log_json(&self.budget.usage_snapshot()),
            "Agent context compaction started"
        );
        let payload = self.payload(
            ctx,
            json!({
                "stepIndex": ctx.turn(),
                "compactionCount": compaction_count,
                "inputTokens": input_tokens,
                "messageCount": message_count,
                "usage": self.budget.usage_snapshot(),
            }),
        );
        self.persist_or_terminate("agent.context_compaction.started", None, payload)
            .await
    }

    pub(crate) async fn complete_context_compaction(
        &self,
        ctx: &HookContext,
        provider_usage: Usage,
        estimated_input_tokens: u64,
        estimated_output_tokens: u64,
        summary: &str,
        compaction_count: u64,
        before_tokens: u64,
        after_tokens: u64,
        before_messages: usize,
        after_messages: usize,
        fallback: bool,
        estimated: bool,
    ) -> Flow {
        let usage_reported = provider_usage.has_values() && !estimated;
        let input_tokens = if usage_reported {
            provider_context_input_tokens(&provider_usage, provider_usage.input_tokens)
        } else {
            estimated_input_tokens
        };
        let output_tokens = if usage_reported {
            provider_usage.output_tokens
        } else {
            estimated_output_tokens
        };
        let budget_result = self
            .budget
            .finish_model_call_with_usage(&provider_usage, estimated_input_tokens, estimated_output_tokens, estimated);
        tracing::debug!(
            target: "agent.context",
            event = "context_compaction_completed",
            project_id = self.task.project_id,
            thread_id = %self.task.thread_id,
            task_id = %self.task.task_id,
            turn_id = %self.task.turn_id,
            run_id = %self.task.run_id,
            request_id = %self.task.request_id,
            step_index = ctx.turn(),
            compaction_count,
            input_tokens,
            output_tokens,
            provider_total_tokens = provider_usage.total_tokens,
            cached_input_tokens = provider_usage.cached_input_tokens,
            cache_creation_input_tokens = provider_usage.cache_creation_input_tokens,
            tool_use_prompt_tokens = provider_usage.tool_use_prompt_tokens,
            reasoning_tokens = provider_usage.reasoning_tokens,
            usage_reported,
            before_tokens,
            after_tokens,
            before_messages,
            after_messages,
            fallback,
            estimated,
            summary = %sanitize_debug_log_text(summary),
            usage = %debug_log_json(&self.budget.usage_snapshot()),
            "Agent context compaction completed"
        );
        let payload = self.payload(
            ctx,
            json!({
                "stepIndex": ctx.turn(),
                "compactionCount": compaction_count,
                "inputTokens": input_tokens,
                "outputTokens": output_tokens,
                "providerInputTokens": provider_usage.input_tokens,
                "providerOutputTokens": provider_usage.output_tokens,
                "providerTotalTokens": provider_usage.total_tokens,
                "cachedInputTokens": provider_usage.cached_input_tokens,
                "cacheCreationInputTokens": provider_usage.cache_creation_input_tokens,
                "toolUsePromptTokens": provider_usage.tool_use_prompt_tokens,
                "reasoningTokens": provider_usage.reasoning_tokens,
                "usageReported": usage_reported,
                "beforeTokens": before_tokens,
                "afterTokens": after_tokens,
                "beforeMessages": before_messages,
                "afterMessages": after_messages,
                "fallback": fallback,
                "estimated": estimated,
                "usage": self.budget.usage_snapshot(),
            }),
        );
        let persisted = self.persist_or_terminate("agent.context_compacted", None, payload).await;
        if !matches!(persisted, Flow::Continue) {
            return persisted;
        }
        match budget_result {
            Ok(()) => Flow::cont(),
            Err(error) => self.terminate_for_budget(ctx, error).await,
        }
    }

    pub(crate) async fn fail_context_compaction(
        &self,
        ctx: &HookContext,
        provider_usage: Usage,
        estimated_input_tokens: u64,
        estimated_output_tokens: u64,
        summary: &str,
        compaction_count: u64,
        before_tokens: u64,
        candidate_tokens: u64,
        before_messages: usize,
        candidate_messages: usize,
        fallback: bool,
        estimated: bool,
        reason: &str,
    ) -> Flow {
        let usage_reported = provider_usage.has_values() && !estimated;
        let input_tokens = if usage_reported {
            provider_context_input_tokens(&provider_usage, provider_usage.input_tokens)
        } else {
            estimated_input_tokens
        };
        let output_tokens = if usage_reported {
            provider_usage.output_tokens
        } else {
            estimated_output_tokens
        };
        self.record_compaction(before_tokens, candidate_tokens);
        let budget_result = self
            .budget
            .finish_model_call_with_usage(&provider_usage, estimated_input_tokens, estimated_output_tokens, estimated);
        let payload = self.payload(
            ctx,
            json!({
                "stepIndex": ctx.turn(),
                "compactionCount": compaction_count,
                "inputTokens": input_tokens,
                "outputTokens": output_tokens,
                "providerInputTokens": provider_usage.input_tokens,
                "providerOutputTokens": provider_usage.output_tokens,
                "providerTotalTokens": provider_usage.total_tokens,
                "usageReported": usage_reported,
                "beforeTokens": before_tokens,
                "candidateTokens": candidate_tokens,
                "beforeMessages": before_messages,
                "candidateMessages": candidate_messages,
                "fallback": fallback,
                "estimated": estimated,
                "reason": reason,
                "summary": sanitize_debug_log_text(summary),
                "usage": self.budget.usage_snapshot(),
            }),
        );
        let persisted = self
            .persist_or_terminate("agent.context_compaction.failed", None, payload)
            .await;
        if !matches!(persisted, Flow::Continue) {
            return persisted;
        }
        match budget_result {
            Err(error) => self.terminate_for_budget(ctx, error).await,
            Ok(()) => Flow::terminate(reason),
        }
    }
}
