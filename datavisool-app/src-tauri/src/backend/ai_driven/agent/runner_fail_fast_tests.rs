/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use rig_core::completion::Message;
use rig_core::message::{ToolResultContent, UserContent};
use rig_core::test_utils::{MockAddTool, MockCompletionModel, MockStreamEvent};
use rig_core::tool::{Tool, ToolFailure};
use serde::Deserialize;
use serde_json::json;

use super::tests::persisted_runtime;
use super::{run_with_model_runtime, RunParts};
use crate::backend::ai_driven::agent::{AgentContextLimits, AgentExecutionLimits, AgentRuntimeHook, AgentToolRoute, ExecutionBudget};
use crate::backend::ai_driven::memory::{
    ConversationMemoryAdapter, ConversationMemoryBackend, ConversationWindowPolicy, SqliteConversationMemoryBackend,
};
use crate::backend::ai_driven::task::ListTaskEvents;

#[derive(Debug, Deserialize)]
struct FatalArgs {}

#[derive(Debug)]
struct FatalStorageError;

impl Display for FatalStorageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("workspace storage failed")
    }
}

impl std::error::Error for FatalStorageError {}

struct FatalStorageTool;

impl Tool for FatalStorageTool {
    const NAME: &'static str = "fatal_storage";
    type Error = FatalStorageError;
    type Args = FatalArgs;
    type Output = String;

    fn description(&self) -> String {
        "always returns a non-retryable storage failure".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object", "additionalProperties": false })
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Err(FatalStorageError)
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        ToolFailure::other(error.to_string())
            .with_code("agent.workspace.storage_failed")
            .with_retryable(false)
    }
}

#[tokio::test]
async fn non_retryable_storage_failure_stops_after_one_model_call() {
    let (pool, repository, task) = persisted_runtime("fatal-storage").await;
    let memory_backend: Arc<dyn ConversationMemoryBackend> = Arc::new(SqliteConversationMemoryBackend::new(pool));
    let memory = ConversationMemoryAdapter::new(1, memory_backend, ConversationWindowPolicy::default());
    let budget = ExecutionBudget::new(AgentExecutionLimits::default());
    let hook = AgentRuntimeHook::new(budget.clone(), repository.clone(), task.clone());
    let model = MockCompletionModel::from_stream_turns([[
        MockStreamEvent::tool_call("call-storage", "fatal_storage", json!({})),
        MockStreamEvent::final_response_with_default_usage(),
    ]]);

    let error = run_with_model_runtime(
        model.clone(),
        RunParts {
            memory,
            tools: vec![Box::new(FatalStorageTool)],
            preamble: "Call fatal_storage once.".to_string(),
            max_model_calls: 4,
            temperature: None,
            max_tokens: None,
            context_window_tokens: None,
            context_limits: AgentContextLimits::default(),
            tool_route: AgentToolRoute::fixed(["fatal_storage"]),
        },
        Message::user("trigger storage failure"),
        task.thread_id.clone(),
        hook,
        None,
        None,
    )
    .await
    .expect_err("non-retryable storage failure must terminate the runner");

    assert_eq!(error.code, "agent.workspace.storage_failed");
    assert_eq!(model.request_count(), 1);
    assert_eq!(budget.usage_snapshot().tool_calls, 1);
    let events = repository
        .list_events(ListTaskEvents {
            project_id: task.project_id,
            thread_id: task.thread_id,
            turn_id: task.turn_id,
            run_id: task.run_id,
            after_sequence: 0,
            limit: 20,
        })
        .await
        .unwrap();
    assert!(events
        .iter()
        .any(|event| event.kind == "agent.failed" && event.payload["code"] == "agent.workspace.storage_failed"));
}

#[tokio::test]
async fn invalid_tool_returns_feedback_and_allows_model_to_correct_the_call() {
    // 同时覆盖模型编造工具名，以及调用已注册但当前步骤未开放的工具。
    for invalid_name in ["inspect_canvas_component", "fatal_storage"] {
        let (pool, repository, task) = persisted_runtime(invalid_name).await;
        let memory = ConversationMemoryAdapter::new(1, Arc::new(SqliteConversationMemoryBackend::new(pool)), ConversationWindowPolicy::default());
        let budget = ExecutionBudget::new(AgentExecutionLimits::default());
        let route = AgentToolRoute::fixed(["add"]);
        let hook = AgentRuntimeHook::new(budget.clone(), repository.clone(), task.clone()).with_tool_route(route.clone());
        let model = MockCompletionModel::from_stream_turns([
            vec![
                MockStreamEvent::tool_call("call-invalid", invalid_name, json!({})),
                MockStreamEvent::final_response_with_default_usage(),
            ],
            vec![
                MockStreamEvent::tool_call("call-corrected", "add", json!({ "x": 2, "y": 3 })),
                MockStreamEvent::final_response_with_default_usage(),
            ],
            vec![MockStreamEvent::text("done"), MockStreamEvent::final_response_with_default_usage()],
        ]);

        let response = run_with_model_runtime(
            model.clone(),
            RunParts {
                memory,
                tools: vec![Box::new(MockAddTool), Box::new(FatalStorageTool)],
                preamble: "Use only the tools provided for this step.".to_string(),
                max_model_calls: 4,
                temperature: None,
                max_tokens: None,
                context_window_tokens: None,
                context_limits: AgentContextLimits::default(),
                tool_route: route,
            },
            Message::user("add two and three"),
            task.thread_id.clone(),
            hook,
            None,
            None,
        )
        .await
        .expect("invalid tool calls should allow one model correction");

        assert_eq!(response.output, "done");
        assert_eq!(model.request_count(), 3);
        assert_eq!(budget.usage_snapshot().tool_calls, 1, "only the corrected tool should execute");
        let requests = model.requests();
        assert_eq!(requests[1].tools.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>(), vec!["add"]);
        let feedback = requests[1]
            .chat_history
            .iter()
            .find_map(|message| match message {
                Message::User { content } => content.iter().find_map(|item| match item {
                    UserContent::ToolResult(result) if result.id == "call-invalid" => result.content.iter().find_map(|content| match content {
                        ToolResultContent::Text(text) => serde_json::from_str::<serde_json::Value>(&text.text).ok(),
                        _ => None,
                    }),
                    _ => None,
                }),
                _ => None,
            })
            .expect("the next model request must include structured error feedback for the rejected call");
        assert_eq!(feedback["code"], "agent.tool.not_advertised_in_step");
        assert_eq!(feedback["toolName"], invalid_name);
        assert_eq!(feedback["availableTools"], json!(["add"]));
        assert_eq!(feedback["retryable"], true);

        let events = repository
            .list_events(ListTaskEvents {
                project_id: task.project_id,
                thread_id: task.thread_id,
                turn_id: task.turn_id,
                run_id: task.run_id,
                after_sequence: 0,
                limit: 100,
            })
            .await
            .unwrap();
        assert!(events.iter().any(|event| {
            event.kind == "agent.tool_call.invalid"
                && event.payload["toolName"] == invalid_name
                && event.payload["willRetry"] == true
                && event.payload["repairAttempt"] == 1
        }));
        assert!(!events.iter().any(|event| event.kind == "agent.failed"));
        assert!(!events
            .iter()
            .any(|event| event.kind == "agent.tool_call.started" && event.payload["toolName"] == invalid_name));
    }
}

#[tokio::test]
async fn invalid_tool_retries_stop_at_the_configured_limit() {
    for retry_limit in [0, 1] {
        let (pool, repository, task) = persisted_runtime(&format!("invalid-limit-{retry_limit}")).await;
        let memory = ConversationMemoryAdapter::new(1, Arc::new(SqliteConversationMemoryBackend::new(pool)), ConversationWindowPolicy::default());
        let budget = ExecutionBudget::new(AgentExecutionLimits::default());
        let route = AgentToolRoute::fixed(["add"]);
        let hook = AgentRuntimeHook::new(budget.clone(), repository.clone(), task.clone())
            .with_tool_route(route.clone())
            .with_invalid_tool_retry_limit(retry_limit);
        let model = MockCompletionModel::from_stream_turns((0..3).map(|index| {
            vec![
                MockStreamEvent::tool_call(format!("call-invalid-{index}"), "inspect_canvas_component", json!({})),
                MockStreamEvent::final_response_with_default_usage(),
            ]
        }));

        let error = run_with_model_runtime(
            model.clone(),
            RunParts {
                memory,
                tools: vec![Box::new(MockAddTool)],
                preamble: "Use only the tools provided for this step.".to_string(),
                max_model_calls: 4,
                temperature: None,
                max_tokens: None,
                context_window_tokens: None,
                context_limits: AgentContextLimits::default(),
                tool_route: route,
            },
            Message::user("start"),
            task.thread_id.clone(),
            hook,
            None,
            None,
        )
        .await
        .expect_err("repeated invalid tool calls must not loop indefinitely");

        assert_eq!(error.code, "agent.tool.not_advertised_in_step");
        assert_eq!(model.request_count(), usize::from(retry_limit) + 1);
        assert_eq!(budget.usage_snapshot().tool_calls, 0);
        let events = repository
            .list_events(ListTaskEvents {
                project_id: task.project_id,
                thread_id: task.thread_id,
                turn_id: task.turn_id,
                run_id: task.run_id,
                after_sequence: 0,
                limit: 100,
            })
            .await
            .unwrap();
        let invalid_events = events
            .iter()
            .filter(|event| event.kind == "agent.tool_call.invalid")
            .collect::<Vec<_>>();
        assert_eq!(invalid_events.len(), usize::from(retry_limit) + 1);
        assert_eq!(invalid_events.last().unwrap().payload["willRetry"], false);
        assert_eq!(invalid_events.last().unwrap().payload["repairAttempt"], serde_json::Value::Null);
    }
}
