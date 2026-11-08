/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Rig AgentRunner 的唯一生产调用入口。
use super::completion::{AgentCompletionDecision, AgentCompletionGate};
use super::context_compaction::AgentContextHook;
use super::context_compaction::ContextTokenTracker;
use super::factory::BuiltAgent;
use super::history::AgentHistoryHook;
use super::hooks::AgentRuntimeHook;
use super::output::{AgentRunFailure, AgentRunOutput};
use super::steering::AgentSteeringHook;
use super::tool_concurrency::{with_tool_concurrency, ToolCallScheduler};
use super::tool_timeout::with_tool_timeout;
use super::AgentContextLimits;
use super::AgentToolRoute;
use super::ExecutionBudget;
use crate::backend::ai_driven::memory::RunScopedConversationMemory;
use crate::backend::ai_driven::provider::{ObservedCompletionModel, RigCompletionModel};
use crate::backend::utils::sanitize_debug_log_text;
use futures::StreamExt;
use rig_core::agent::{AgentBuilder, MultiTurnStreamItem, PromptResponse, StreamingError};
use rig_core::completion::{CompletionError, CompletionModel, Message, PromptError};
use std::collections::HashMap;
use std::sync::Arc;
const MAX_TOOL_CONCURRENCY: usize = 16;

struct RunParts {
    memory: crate::backend::ai_driven::memory::ConversationMemoryAdapter,
    tools: Vec<Box<dyn rig_core::tool::ToolDyn>>,
    preamble: String,
    max_model_calls: usize,
    temperature: Option<f64>,
    max_tokens: Option<u64>,
    context_window_tokens: Option<u64>,
    context_limits: AgentContextLimits,
    tool_route: AgentToolRoute,
}

/// 驱动一次受控 Agent 运行。所有 Provider 都必须经过此入口。
pub(crate) async fn run_built_agent(
    built: BuiltAgent,
    prompt: Message,
    conversation_id: String,
    hook: AgentRuntimeHook,
    steering: AgentSteeringHook,
    budget: ExecutionBudget,
    completion_gate: Arc<dyn AgentCompletionGate>,
) -> Result<AgentRunOutput, AgentRunFailure> {
    let BuiltAgent {
        model,
        memory,
        tools,
        preamble,
        max_model_calls,
        temperature,
        max_tokens,
        context_window_tokens,
        context_limits,
        tool_route,
    } = built;
    let tools = with_tool_timeout(tools, budget.limits().max_tool_duration);
    let parts = RunParts {
        memory,
        tools,
        preamble,
        max_model_calls,
        temperature,
        max_tokens,
        context_window_tokens,
        context_limits,
        tool_route,
    };
    let response = match model.into_model() {
        RigCompletionModel::OpenAi(model) => {
            run_with_model_runtime(model, parts, prompt, conversation_id, hook, Some(steering), Some(completion_gate)).await
        }
        RigCompletionModel::Anthropic(model) => {
            run_with_model_runtime(model, parts, prompt, conversation_id, hook, Some(steering), Some(completion_gate)).await
        }
        RigCompletionModel::DeepSeek(model) => {
            run_with_model_runtime(model, parts, prompt, conversation_id, hook, Some(steering), Some(completion_gate)).await
        }
        RigCompletionModel::Mimo(model) => {
            run_with_model_runtime(model, parts, prompt, conversation_id, hook, Some(steering), Some(completion_gate)).await
        }
        RigCompletionModel::Gemini(model) => {
            run_with_model_runtime(model, parts, prompt, conversation_id, hook, Some(steering), Some(completion_gate)).await
        }
        RigCompletionModel::Grok(model) => {
            run_with_model_runtime(model, parts, prompt, conversation_id, hook, Some(steering), Some(completion_gate)).await
        }
    }?;

    // Hook 已逐轮记录真实 usage；这里读取同一预算快照，避免依赖 Rig 序列化格式。
    Ok(AgentRunOutput {
        final_text: bounded_final_text(response.output),
        operations: Vec::new(),
        usage: budget.usage_snapshot(),
    })
}

async fn run_with_model_runtime<M>(
    model: M,
    mut parts: RunParts,
    prompt: Message,
    conversation_id: String,
    hook: AgentRuntimeHook,
    steering: Option<AgentSteeringHook>,
    completion_gate: Option<Arc<dyn AgentCompletionGate>>,
) -> Result<PromptResponse, AgentRunFailure>
where
    M: CompletionModel + 'static,
{
    let tool_scheduler = Arc::new(ToolCallScheduler::new());
    // 生产入口已在内部 Tool 上设置超时；调度包装位于外层，因此排队时间不计入 Tool 超时。
    parts.tools = with_tool_concurrency(parts.tools, tool_scheduler.clone());
    let hook = hook.with_tool_scheduler(tool_scheduler);
    let tool_schema_bytes = parts
        .tools
        .iter()
        .map(|tool| {
            let name = tool.name();
            let bytes = serde_json::to_vec(&(name.as_str(), tool.description(), tool.parameters())).map_or(0, |value| value.len());
            (name, bytes)
        })
        .collect::<HashMap<_, _>>();
    let identity = hook.context_trace_identity();
    let observed_model = ObservedCompletionModel::new(model, identity.clone());
    let history = AgentHistoryHook::new(identity, parts.preamble.clone());
    let token_tracker = ContextTokenTracker::default();
    let hook = hook.with_context_token_tracker(token_tracker.clone());
    let context = AgentContextHook::new(
        observed_model.clone(),
        parts.context_window_tokens,
        parts.max_tokens,
        parts.tool_route,
        tool_schema_bytes,
        hook.clone(),
        steering.clone(),
        parts.context_limits,
        token_tracker,
    );
    let mut builder = AgentBuilder::new(observed_model)
        .name("datavisool-desktop-agent")
        .preamble(&parts.preamble)
        .memory(RunScopedConversationMemory::new(parts.memory))
        .conversation(conversation_id.clone())
        .default_max_turns(parts.max_model_calls);
    if let Some(temperature) = parts.temperature {
        builder = builder.temperature(temperature);
    }
    if let Some(max_tokens) = parts.max_tokens {
        builder = builder.max_tokens(max_tokens);
    }
    let agent = builder.tools(parts.tools).build();
    let mut next_prompt = prompt;
    loop {
        let runner = agent
            .runner(next_prompt)
            .conversation(conversation_id.clone())
            .max_turns(parts.max_model_calls)
            .max_invalid_tool_call_retries(1)
            // Rig 负责启动同批 Future；任务级读写闸门决定哪些 Tool 真正重叠。
            .tool_concurrency(MAX_TOOL_CONCURRENCY)
            .add_hook(context.clone())
            .add_hook(history.clone())
            .add_hook(hook.clone());
        let mut stream = runner.stream().await;
        let mut response = None;
        while let Some(item) = stream.next().await {
            match item {
                Ok(MultiTurnStreamItem::FinalResponse(final_response)) => response = Some(final_response),
                Ok(_) => {}
                Err(error) => {
                    let error = streaming_prompt_error(error);
                    let identity = hook.context_trace_identity();
                    tracing::debug!(
                        target: "agent.model",
                        event = "model_stream_failed",
                        project_id = identity.project_id,
                        thread_id = %identity.thread_id,
                        task_id = %identity.task_id,
                        turn_id = %identity.turn_id,
                        run_id = %identity.run_id,
                        request_id = %identity.request_id,
                        error = %sanitize_debug_log_text(&error.to_string()),
                        "Agent model stream failed"
                    );
                    return Err(map_prompt_error(error));
                }
            }
        }
        let response = response
            .ok_or_else(|| map_prompt_error(PromptError::CompletionError(CompletionError::ResponseError("Agent 流未返回终态".to_string()))))?;
        if let Some(steering) = &steering {
            if steering.has_pending().await {
                steering.reset_injected();
                next_prompt = Message::user("继续当前任务，并优先处理运行中刚刚追加的用户要求。");
                continue;
            }
        }
        if let Some(completion_gate) = &completion_gate {
            match completion_gate.evaluate(&response.output).await? {
                AgentCompletionDecision::Complete => {
                    if let Some(steering) = &steering {
                        if !steering.close_if_idle().await {
                            steering.reset_injected();
                            next_prompt = Message::user("继续当前任务，并优先处理完成核验期间追加的用户要求。");
                            continue;
                        }
                    }
                    return Ok(response);
                }
                AgentCompletionDecision::Continue { prompt } => {
                    next_prompt = Message::user(prompt);
                    continue;
                }
                AgentCompletionDecision::Fail { code, message } => return Err(AgentRunFailure { code, message }),
            }
        }
        if let Some(steering) = &steering {
            if !steering.close_if_idle().await {
                steering.reset_injected();
                next_prompt = Message::user("继续当前任务，并优先处理运行中刚刚追加的用户要求。");
                continue;
            }
        }
        return Ok(response);
    }
}

fn streaming_prompt_error(error: StreamingError) -> PromptError {
    match error {
        StreamingError::Completion(error) => PromptError::CompletionError(error),
        StreamingError::Prompt(error) => *error,
        StreamingError::Tool(error) => PromptError::ToolError(error),
    }
}

fn map_prompt_error(error: PromptError) -> AgentRunFailure {
    let code = match &error {
        PromptError::MaxTurnsError { .. } => "agent_execution.model_call_limit",
        PromptError::PromptCancelled { reason, .. } => cancellation_code(reason),
        PromptError::UnknownToolCall { .. } => "agent.tool.unknown",
        PromptError::ToolError(_) | PromptError::ToolServerError(_) => "agent.tool.execution_failed",
        PromptError::CompletionError(_) => provider_error_code(&error),
    };
    AgentRunFailure { code: code.to_string(), message: error.to_string() }
}

fn provider_error_code(error: &PromptError) -> &'static str {
    match error.provider_response_status().map(|status| status.as_u16()) {
        Some(401 | 403) => "agent.model.authentication_failed",
        Some(408) => "agent.model.timeout",
        Some(429) => "agent.model.rate_limited",
        Some(500..=599) => "agent.model.upstream_unavailable",
        Some(400..=499) => "agent.model.request_rejected",
        Some(_) => "agent.model.response_invalid",
        None if error.to_string().to_ascii_lowercase().contains("timed out") => "agent.model.timeout",
        None => "agent.model.invocation_failed",
    }
}

fn execution_code(reason: &str) -> Option<&'static str> {
    [
        "agent_execution.cancelled",
        "agent_execution.model_call_limit",
        "agent_execution.ready_phase_model_call_limit",
        "agent_execution.tool_call_limit",
        "agent_execution.write_call_limit",
        "agent_execution.token_limit",
        "agent_execution.ready_phase_token_limit",
        "agent_execution.duration_limit",
        "agent_execution.tool_duration_limit",
        "agent_execution.consecutive_failure_limit",
        "agent_execution.frontend_interaction_limit",
        "agent_execution.repeated_read_limit",
        "agent_execution.no_progress_limit",
        "agent_execution.completion_continuation_limit",
        "agent.tool.not_advertised_in_step",
    ]
    .into_iter()
    .find(|code| reason.contains(code))
}

fn cancellation_code(reason: &str) -> &str {
    execution_code(reason).unwrap_or_else(|| {
        let candidate = reason.split_ascii_whitespace().next_back().unwrap_or(reason);
        if candidate.starts_with("agent.") {
            candidate
        } else {
            "agent.run.terminated"
        }
    })
}

fn bounded_final_text(text: String) -> String {
    const LIMIT: usize = 2_000;
    let text = text.trim();
    if text.chars().count() <= LIMIT {
        return text.to_string();
    }
    text.chars().take(LIMIT).collect::<String>() + "…"
}

#[cfg(test)]
#[path = "runner_fail_fast_tests.rs"]
mod fail_fast_tests;

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
