/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Agent 单次运行的完整模型对话与工具执行历史。

use std::sync::{Arc, Mutex, MutexGuard};

use rig_core::agent::{AgentHook, Flow, HookContext, StepEvent, StepEventKind};
use rig_core::completion::{AssistantContent, CompletionModel, Message};
use serde::Serialize;
use serde_json::{json, Value};

use crate::backend::ai_driven::provider::ContextTraceIdentity;
use crate::backend::utils::debug_log_json;

pub(crate) const COMPLETE_HISTORY_PREFIX: &str = "==================完整历史=================";

#[derive(Clone)]
pub(crate) struct AgentHistoryHook {
    state: Arc<HistoryState>,
}

struct HistoryState {
    identity: ContextTraceIdentity,
    preamble: String,
    transcript: Mutex<HistoryTranscript>,
}

#[derive(Default)]
struct HistoryTranscript {
    messages: Vec<Message>,
    tool_executions: Vec<ToolExecutionHistory>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolExecutionHistory {
    sequence: usize,
    step_index: usize,
    tool_name: String,
    tool_call_id: Option<String>,
    internal_call_id: String,
    arguments: Value,
    result: Option<Value>,
    outcome: Option<String>,
    failure: Option<String>,
}

impl AgentHistoryHook {
    pub(crate) fn new(identity: ContextTraceIdentity, preamble: String) -> Self {
        Self {
            state: Arc::new(HistoryState { identity, preamble, transcript: Mutex::new(HistoryTranscript::default()) }),
        }
    }

    fn lock_transcript(&self) -> MutexGuard<'_, HistoryTranscript> {
        self.state.transcript.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn sync_model_request(&self, history: &[Message], prompt: &Message) {
        let mut messages = history.to_vec();
        messages.push(prompt.clone());
        self.lock_transcript().messages = messages;
    }

    fn record_model_turn(&self, content: &rig_core::OneOrMany<AssistantContent>) {
        let message = Message::Assistant { id: None, content: content.clone() };
        let mut transcript = self.lock_transcript();
        if transcript.messages.last() != Some(&message) {
            transcript.messages.push(message);
        }
    }

    fn record_tool_call(&self, step_index: usize, tool_name: &str, tool_call_id: Option<&str>, internal_call_id: &str, args: &str) {
        let mut transcript = self.lock_transcript();
        if transcript
            .tool_executions
            .iter()
            .any(|execution| execution.internal_call_id == internal_call_id)
        {
            return;
        }
        let sequence = transcript.tool_executions.len().saturating_add(1);
        transcript.tool_executions.push(ToolExecutionHistory {
            sequence,
            step_index,
            tool_name: tool_name.to_string(),
            tool_call_id: tool_call_id.map(str::to_string),
            internal_call_id: internal_call_id.to_string(),
            arguments: json_value(args),
            result: None,
            outcome: None,
            failure: None,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn record_tool_result(
        &self,
        step_index: usize,
        tool_name: &str,
        tool_call_id: Option<&str>,
        internal_call_id: &str,
        args: &str,
        result: &str,
        outcome: &rig_core::tool::ToolOutcome,
    ) {
        let mut transcript = self.lock_transcript();
        let position = transcript
            .tool_executions
            .iter()
            .position(|execution| execution.internal_call_id == internal_call_id);
        let execution = match position {
            Some(position) => &mut transcript.tool_executions[position],
            None => {
                let sequence = transcript.tool_executions.len().saturating_add(1);
                transcript.tool_executions.push(ToolExecutionHistory {
                    sequence,
                    step_index,
                    tool_name: tool_name.to_string(),
                    tool_call_id: tool_call_id.map(str::to_string),
                    internal_call_id: internal_call_id.to_string(),
                    arguments: json_value(args),
                    result: None,
                    outcome: None,
                    failure: None,
                });
                transcript.tool_executions.last_mut().expect("just inserted tool execution")
            }
        };
        execution.result = Some(json_value(result));
        execution.outcome = Some(outcome.as_str().to_string());
        execution.failure = outcome.failure().map(|failure| failure.to_string());

        // Rig 只会在整轮成功结束后写入 ConversationMemory。这里提前保留工具回执，
        // 确保取消、超时或预算终止时的日志仍包含最后一次方法返回。
        let result_id = tool_call_id.unwrap_or(internal_call_id);
        let message = Message::tool_result(result_id, result.to_string());
        if transcript.messages.last() != Some(&message) {
            transcript.messages.push(message);
        }
    }
}

impl<M> AgentHook<M> for AgentHistoryHook
where
    M: CompletionModel,
{
    async fn on_event(&self, ctx: &HookContext, event: StepEvent<'_, M>) -> Flow {
        match event {
            StepEvent::CompletionCall { prompt, history, .. } => self.sync_model_request(history, prompt),
            StepEvent::ModelTurnFinished { content, .. } => self.record_model_turn(content),
            StepEvent::ToolCall { tool_name, tool_call_id, internal_call_id, args } => {
                self.record_tool_call(ctx.turn(), tool_name, tool_call_id, internal_call_id, args)
            }
            StepEvent::ToolResult { tool_name, tool_call_id, internal_call_id, args, result, outcome, .. } => {
                self.record_tool_result(ctx.turn(), tool_name, tool_call_id, internal_call_id, args, result, outcome)
            }
            _ => {}
        }
        Flow::cont()
    }

    fn observes(&self, kind: StepEventKind) -> bool {
        matches!(
            kind,
            StepEventKind::CompletionCall | StepEventKind::ModelTurnFinished | StepEventKind::ToolCall | StepEventKind::ToolResult
        )
    }
}

impl Drop for HistoryState {
    fn drop(&mut self) {
        let transcript = self.transcript.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let payload = history_payload(&self.identity, &self.preamble, &transcript);
        tracing::debug!(
            target: "agent.history",
            "{}{}",
            COMPLETE_HISTORY_PREFIX,
            debug_log_json(&payload)
        );
    }
}

fn history_payload(identity: &ContextTraceIdentity, preamble: &str, transcript: &HistoryTranscript) -> Value {
    json!({
        "schemaVersion": "datavisool-agent-history/v1",
        "projectId": identity.project_id,
        "threadId": identity.thread_id,
        "taskId": identity.task_id,
        "turnId": identity.turn_id,
        "runId": identity.run_id,
        "requestId": identity.request_id,
        "endedAt": chrono::Local::now().to_rfc3339(),
        "systemPreamble": preamble,
        "messageCount": transcript.messages.len(),
        "toolExecutionCount": transcript.tool_executions.len(),
        "messages": transcript.messages,
        "toolExecutions": transcript.tool_executions,
    })
}

fn json_value(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
}

#[cfg(test)]
mod tests {
    use rig_core::completion::AssistantContent;
    use rig_core::tool::ToolOutcome;
    use rig_core::OneOrMany;
    use serde_json::json;

    use super::*;

    fn identity() -> ContextTraceIdentity {
        ContextTraceIdentity {
            project_id: 7,
            thread_id: "thread-history".to_string(),
            task_id: "task-history".to_string(),
            turn_id: "turn-history".to_string(),
            run_id: "run-history".to_string(),
            request_id: "request-history".to_string(),
        }
    }

    #[test]
    fn builds_complete_history_with_reasoning_and_tool_receipt() {
        let hook = AgentHistoryHook::new(identity(), "system rules".to_string());
        hook.sync_model_request(&[], &Message::user("创建指标卡"));
        hook.record_model_turn(
            &OneOrMany::many([
                AssistantContent::reasoning("先读取组件规格"),
                AssistantContent::tool_call("call-1", "resolve_component_specs", json!({ "componentTypes": ["BaseText"] })),
            ])
            .unwrap(),
        );
        hook.record_tool_call(1, "resolve_component_specs", Some("call-1"), "internal-1", r#"{"componentTypes":["BaseText"]}"#);
        hook.record_tool_result(
            1,
            "resolve_component_specs",
            Some("call-1"),
            "internal-1",
            r#"{"componentTypes":["BaseText"]}"#,
            r#"{"status":"ok"}"#,
            &ToolOutcome::Success,
        );

        let transcript = hook.lock_transcript();
        let payload = history_payload(&hook.state.identity, &hook.state.preamble, &transcript);

        assert_eq!(payload["schemaVersion"], "datavisool-agent-history/v1");
        assert_eq!(payload["messageCount"], 3);
        assert_eq!(payload["toolExecutionCount"], 1);
        assert_eq!(payload["toolExecutions"][0]["outcome"], "success");
        let encoded = debug_log_json(&payload);
        assert!(encoded.contains("先读取组件规格"));
        assert!(encoded.contains("resolve_component_specs"));
        assert!(encoded.contains(r#"\"status\":\"ok\""#));
    }

    #[test]
    fn later_request_replaces_provisional_tool_result_without_duplication() {
        let hook = AgentHistoryHook::new(identity(), "system rules".to_string());
        let assistant = Message::Assistant { id: None, content: OneOrMany::one(AssistantContent::tool_call("call-1", "read", json!({}))) };
        hook.sync_model_request(&[], &Message::user("读取"));
        hook.record_model_turn(match &assistant {
            Message::Assistant { content, .. } => content,
            _ => unreachable!(),
        });
        hook.record_tool_result(1, "read", Some("call-1"), "internal-1", "{}", "done", &ToolOutcome::Success);
        let tool_result = Message::tool_result("call-1", "done");
        hook.sync_model_request(&[Message::user("读取"), assistant], &tool_result);

        let transcript = hook.lock_transcript();
        assert_eq!(transcript.messages.len(), 3);
        assert_eq!(transcript.messages.last(), Some(&tool_result));
    }
}
