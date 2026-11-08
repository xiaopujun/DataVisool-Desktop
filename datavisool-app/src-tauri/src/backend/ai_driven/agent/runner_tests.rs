/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::fmt::{Display, Formatter};
use std::future::pending;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rig_core::completion::Usage;
use rig_core::memory::ConversationMemory;
use rig_core::message::{ToolResultContent, UserContent};
use rig_core::test_utils::{MockAddTool, MockCompletionModel, MockStreamEvent};
use rig_core::tool::Tool;
use serde::Deserialize;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

use super::*;
use crate::backend::ai_driven::agent::limits::AgentExecutionErrorCode;
use crate::backend::ai_driven::agent::{AgentExecutionLimits, AgentRuntimeHook, ExecutionBudget};
use crate::backend::ai_driven::memory::{
    ConversationMemoryAdapter, ConversationMemoryBackend, ConversationWindowPolicy, SqliteConversationMemoryBackend,
};

use crate::backend::ai_driven::task::{
    AgentExecutionLimitsSnapshot, AgentTaskRepository, AgentTaskRun, AgentTaskStatus, AgentWorkingCheckpointState, ListTaskEvents, NewAgentTaskRun,
    SaveAgentTaskCheckpoint, SaveAgentWorkingCheckpoint, SqliteAgentTaskRepository,
};
use crate::backend::ai_driven::thread::{ClaimTurnRunningInput, CreateThreadInput, ThreadService};
use crate::backend::ai_driven::tools::interaction::REQUEST_USER_INPUT_TOOL;
use crate::backend::ai_driven::tools::task_intent::UPDATE_PLAN_TOOL;

const AGENT_MIGRATION: &str = include_str!("../../../../migrations_desktop/1014_desktop_ai_thread_turn_item.sql");
const WORKING_CHECKPOINT_MIGRATION: &str = include_str!("../../../../migrations_desktop/1017_desktop_ai_working_checkpoint.sql");

struct RetryOnceCompletionGate {
    calls: AtomicUsize,
}

impl AgentCompletionGate for RetryOnceCompletionGate {
    fn evaluate<'a>(&'a self, _final_text: &'a str) -> super::super::CompletionGateFuture<'a> {
        Box::pin(async move {
            if self.calls.fetch_add(1, Ordering::AcqRel) == 0 {
                Ok(AgentCompletionDecision::Continue { prompt: "执行缺失动作后再结束".to_string() })
            } else {
                Ok(AgentCompletionDecision::Complete)
            }
        })
    }
}

pub(super) async fn persisted_runtime(suffix: &str) -> (sqlx::SqlitePool, Arc<dyn AgentTaskRepository>, AgentTaskRun) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE project (id INTEGER PRIMARY KEY, name TEXT NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(AGENT_MIGRATION).execute(&pool).await.unwrap();
    sqlx::raw_sql(WORKING_CHECKPOINT_MIGRATION).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO project (id, name, deleted) VALUES (1, 'project', 0)")
        .execute(&pool)
        .await
        .unwrap();
    let threads = ThreadService::new(pool.clone());
    let thread = threads
        .create_thread(CreateThreadInput { project_id: 1, create_request_id: format!("create-{suffix}"), title: None, created_at: 1 })
        .await
        .unwrap();
    let repository: Arc<dyn AgentTaskRepository> = Arc::new(SqliteAgentTaskRepository::new(pool.clone()));
    let task = repository
        .create(NewAgentTaskRun {
            project_id: 1,
            thread_id: thread.thread_id.clone(),
            request_id: format!("request-{suffix}"),
            request: json!({ "prompt": suffix }),
            input_summary: suffix.to_string(),
            model_config_id: "config".to_string(),
            runtime_model_id: "model".to_string(),
            provider: None,
            entry_digest: format!("sha256:{suffix}"),
            client_session_id: format!("session-{suffix}"),
            task_type: "test".to_string(),
            limits: AgentExecutionLimitsSnapshot::default(),
            project_snapshot_ref: None,
            rig_version: Some("rig-core/0.40.0".to_string()),
            rig_snapshot_ref: None,
            user_item_payload: json!({ "text": suffix }),
            attachments: Vec::new(),
            created_at: 2,
        })
        .await
        .unwrap();
    threads
        .claim_turn_running(ClaimTurnRunningInput {
            project_id: 1,
            thread_id: task.thread_id.clone(),
            turn_id: task.turn_id.clone(),
            run_id: task.run_id.clone(),
            started_at: 3,
        })
        .await
        .unwrap();
    let task = repository.find_by_run_id(task.run_id.clone()).await.unwrap().unwrap();
    repository
        .save_working_checkpoint(SaveAgentWorkingCheckpoint {
            project_id: task.project_id,
            thread_id: task.thread_id.clone(),
            task_id: task.task_id.clone(),
            run_id: task.run_id.clone(),
            turn_id: task.turn_id.clone(),
            expected_version: 0,
            state: AgentWorkingCheckpointState::initial(suffix, task.created_at),
        })
        .await
        .unwrap();
    (pool, repository, task)
}

#[derive(Debug, Deserialize)]
struct PendingArgs {}

#[derive(Debug)]
struct PendingError;

impl Display for PendingError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("pending tool failed")
    }
}

impl std::error::Error for PendingError {}

struct PendingTool;

impl Tool for PendingTool {
    const NAME: &'static str = "pending_tool";
    type Error = PendingError;
    type Args = PendingArgs;
    type Output = String;

    fn description(&self) -> String {
        "never returns".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object", "additionalProperties": false })
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        pending().await
    }
}

struct RoutePlanTool;

impl Tool for RoutePlanTool {
    const NAME: &'static str = UPDATE_PLAN_TOOL;
    type Error = PendingError;
    type Args = serde_json::Value;
    type Output = serde_json::Value;

    fn description(&self) -> String {
        "return the structured plan".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object" })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(args)
    }
}

struct ImmediateUserInputTool;

impl Tool for ImmediateUserInputTool {
    const NAME: &'static str = REQUEST_USER_INPUT_TOOL;
    type Error = PendingError;
    type Args = serde_json::Value;
    type Output = serde_json::Value;

    fn description(&self) -> String {
        "return an immediate test answer".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object" })
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(json!({ "metric": "revenue" }))
    }
}

struct CountingRoutePlanTool(Arc<AtomicUsize>);

impl Tool for CountingRoutePlanTool {
    const NAME: &'static str = UPDATE_PLAN_TOOL;
    type Error = PendingError;
    type Args = serde_json::Value;
    type Output = serde_json::Value;

    fn description(&self) -> String {
        "count executions and return the structured plan".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object" })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0.fetch_add(1, Ordering::AcqRel);
        Ok(args)
    }
}

struct PreviewTool;

impl Tool for PreviewTool {
    const NAME: &'static str = "create_canvas_components";
    type Error = PendingError;
    type Args = PendingArgs;
    type Output = serde_json::Value;

    fn description(&self) -> String {
        "returns a confirmed preview".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object", "additionalProperties": false })
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(json!({ "confirmed": true }))
    }
}

#[derive(Debug, Deserialize)]
struct StrictComponentsArgs {
    #[allow(dead_code)]
    components: Vec<serde_json::Value>,
}

struct StrictCreateTool;

impl Tool for StrictCreateTool {
    const NAME: &'static str = "create_canvas_components";
    type Error = PendingError;
    type Args = StrictComponentsArgs;
    type Output = serde_json::Value;

    fn description(&self) -> String {
        "requires components to be a native array".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "components": { "type": "array", "items": { "type": "object" } } },
            "required": ["components"],
            "additionalProperties": false
        })
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(json!({ "status": "applied" }))
    }
}

#[test]
fn bounds_model_text_saved_in_runtime_snapshot() {
    let text = "a".repeat(2_100);
    let bounded = bounded_final_text(text);
    assert_eq!(bounded.chars().count(), 2_001);
    assert!(bounded.ends_with('…'));
}

#[test]
fn keeps_stable_execution_code_from_hook_termination() {
    assert_eq!(execution_code("terminated: agent_execution.token_limit"), Some("agent_execution.token_limit"));
    assert_eq!(execution_code("other reason"), None);
    assert_eq!(cancellation_code("agent.workspace.storage_failed"), "agent.workspace.storage_failed");
    assert_eq!(cancellation_code("terminated: agent.workspace.storage_failed"), "agent.workspace.storage_failed");
    assert_eq!(cancellation_code("other reason"), "agent.run.terminated");
}

#[test]
fn maps_provider_error_classes_without_exposing_provider_specific_codes() {
    let rate_limited = PromptError::CompletionError(rig_core::completion::CompletionError::from_http_response(
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "rate limit",
    ));
    assert_eq!(provider_error_code(&rate_limited), "agent.model.rate_limited");
}

#[tokio::test]
async fn drives_model_tool_model_loop_with_memory_hooks_and_usage() {
    let (pool, repository, task) = persisted_runtime("loop").await;

    let memory_backend: Arc<dyn ConversationMemoryBackend> = Arc::new(SqliteConversationMemoryBackend::new(pool));
    let memory = ConversationMemoryAdapter::new(1, memory_backend, ConversationWindowPolicy::default());
    let memory_probe = memory.clone();
    let budget = ExecutionBudget::new(AgentExecutionLimits::default());
    let hook = AgentRuntimeHook::new(budget.clone(), repository.clone(), task.clone());
    let usage = Usage { input_tokens: 3, output_tokens: 2, total_tokens: 5, ..Usage::default() };
    let model = MockCompletionModel::from_stream_turns([
        vec![MockStreamEvent::tool_call("call-1", "add", json!({ "x": 2, "y": 3 })), MockStreamEvent::final_response(usage)],
        vec![MockStreamEvent::text("done"), MockStreamEvent::final_response(usage)],
    ]);
    let response = run_with_model_runtime(
        model.clone(),
        RunParts {
            memory,
            tools: vec![Box::new(MockAddTool)],
            preamble: "Use the add tool once, then finish.".to_string(),
            max_model_calls: 4,
            temperature: None,
            max_tokens: None,
            context_window_tokens: None,
            context_limits: AgentContextLimits::default(),
            tool_route: AgentToolRoute::fixed(["add"]),
        },
        Message::user("add two and three"),
        task.thread_id.clone(),
        hook,
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(response.output, "done");
    assert_eq!(model.request_count(), 2);
    assert!(model
        .requests()
        .iter()
        .all(|request| request.tools.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>() == vec!["add"]));
    let usage = budget.usage_snapshot();
    assert_eq!(usage.model_calls, 2);
    assert_eq!(usage.tool_calls, 1);
    assert_eq!(usage.input_tokens, 6);
    assert_eq!(usage.output_tokens, 4);
    let events = repository
        .list_events(ListTaskEvents {
            project_id: task.project_id,
            thread_id: task.thread_id.clone(),
            turn_id: task.turn_id.clone(),
            run_id: task.run_id.clone(),
            after_sequence: 0,
            limit: 50,
        })
        .await
        .unwrap();
    assert!(events.iter().any(|event| event.kind == "agent.tool_call.started"));
    assert!(events.iter().any(|event| event.kind == "agent.tool_call.completed"));
    assert!(events
        .iter()
        .any(|event| event.kind == "item.started" && event.payload["item"]["itemType"] == "toolCall"));
    assert!(events
        .iter()
        .any(|event| event.kind == "item.delta" && event.payload["itemType"] == "agentMessage" && event.payload["delta"] == "done"));
    assert!(events.iter().any(|event| {
        event.kind == "item.completed" && event.payload["item"]["itemType"] == "agentMessage" && event.payload["item"]["payload"]["text"] == "done"
    }));
    assert!(!events.iter().any(|event| event.kind.starts_with("agent.canvas_operation.")));
    assert!(!memory_probe.load(&task.thread_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn same_step_plan_update_does_not_reject_user_input_from_the_planning_snapshot() {
    let (pool, repository, task) = persisted_runtime("plan-then-user-input").await;
    let memory_backend: Arc<dyn ConversationMemoryBackend> = Arc::new(SqliteConversationMemoryBackend::new(pool));
    let memory = ConversationMemoryAdapter::new(1, memory_backend, ConversationWindowPolicy::default());
    let budget = ExecutionBudget::new(AgentExecutionLimits::default());
    let route = AgentToolRoute::new(crate::backend::ai_driven::agent_context::input::AgentRunInput::new("先计划再确认指标"), false);
    let hook = AgentRuntimeHook::new(budget, repository.clone(), task.clone()).with_tool_route(route.clone());
    let model = MockCompletionModel::from_stream_turns([
        vec![
            MockStreamEvent::tool_call(
                "call-plan",
                UPDATE_PLAN_TOOL,
                json!({
                    "plan": [{ "step": "确认指标", "status": "in_progress" }]
                }),
            ),
            MockStreamEvent::tool_call("call-user", REQUEST_USER_INPUT_TOOL, json!({})),
            MockStreamEvent::final_response_with_default_usage(),
        ],
        vec![MockStreamEvent::text("已收到回答"), MockStreamEvent::final_response_with_default_usage()],
    ]);

    let response = run_with_model_runtime(
        model.clone(),
        RunParts {
            memory,
            tools: vec![Box::new(RoutePlanTool), Box::new(ImmediateUserInputTool)],
            preamble: "plan then ask".to_string(),
            max_model_calls: 3,
            temperature: None,
            max_tokens: None,
            context_window_tokens: None,
            context_limits: AgentContextLimits::default(),
            tool_route: route.clone(),
        },
        Message::user("start"),
        task.thread_id.clone(),
        hook,
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(response.output, "已收到回答");
    assert_eq!(model.request_count(), 2);
    assert_eq!(route.phase(), crate::backend::ai_driven::agent::context_plan::AgentRoutePhase::Ready);
    assert!(model.requests()[1]
        .tools
        .iter()
        .any(|tool| tool.name == REQUEST_USER_INPUT_TOOL));
    assert!(model.requests()[1].tools.iter().any(|tool| tool.name == UPDATE_PLAN_TOOL));
    let saved = repository.find_by_run_id(task.run_id.clone()).await.unwrap().unwrap();
    assert_eq!(saved.status, AgentTaskStatus::Running);
    assert!(saved.pending.is_none());
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
    assert!(!events.iter().any(|event| {
        event.payload["code"] == "agent_execution.no_progress_limit" || event.payload["code"] == "agent.tool.not_advertised_in_step"
    }));
    assert!(events.iter().any(|event| {
        event.kind == "agent.tool_call.started"
            && event.payload["toolName"] == REQUEST_USER_INPUT_TOOL
            && event.payload["admittedPhase"] == "ready"
            && event.payload["admissionSource"] == "stepSnapshot"
            && event.payload["concurrencyPolicy"] == "barrier"
    }));
}

#[tokio::test]
async fn calls_after_a_barrier_are_deferred_to_the_next_model_step() {
    let (pool, repository, task) = persisted_runtime("barrier-defers-sibling").await;
    let memory_backend: Arc<dyn ConversationMemoryBackend> = Arc::new(SqliteConversationMemoryBackend::new(pool));
    let memory = ConversationMemoryAdapter::new(1, memory_backend, ConversationWindowPolicy::default());
    let budget = ExecutionBudget::new(AgentExecutionLimits::default());
    let route = AgentToolRoute::new(crate::backend::ai_driven::agent_context::input::AgentRunInput::new("先确认指标再制定计划"), false);
    let hook = AgentRuntimeHook::new(budget, repository.clone(), task.clone()).with_tool_route(route.clone());
    let plan_executions = Arc::new(AtomicUsize::new(0));
    let model = MockCompletionModel::from_stream_turns([
        vec![
            MockStreamEvent::tool_call("call-user", REQUEST_USER_INPUT_TOOL, json!({})),
            MockStreamEvent::tool_call(
                "call-plan",
                UPDATE_PLAN_TOOL,
                json!({
                    "plan": [{ "step": "创建组件", "status": "in_progress" }]
                }),
            ),
            MockStreamEvent::final_response_with_default_usage(),
        ],
        vec![MockStreamEvent::text("将在下一步重新制定计划"), MockStreamEvent::final_response_with_default_usage()],
    ]);

    let response = run_with_model_runtime(
        model.clone(),
        RunParts {
            memory,
            tools: vec![Box::new(ImmediateUserInputTool), Box::new(CountingRoutePlanTool(plan_executions.clone()))],
            preamble: "ask before planning".to_string(),
            max_model_calls: 3,
            temperature: None,
            max_tokens: None,
            context_window_tokens: None,
            context_limits: AgentContextLimits::default(),
            tool_route: route.clone(),
        },
        Message::user("start"),
        task.thread_id.clone(),
        hook,
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(response.output, "将在下一步重新制定计划");
    assert_eq!(plan_executions.load(Ordering::Acquire), 0);
    assert_eq!(route.phase(), crate::backend::ai_driven::agent::context_plan::AgentRoutePhase::Ready);
    assert_eq!(model.request_count(), 2);
    let second_request = &model.requests()[1];
    assert!(second_request.tools.iter().any(|tool| tool.name == REQUEST_USER_INPUT_TOOL));
    assert!(second_request.tools.iter().any(|tool| tool.name == UPDATE_PLAN_TOOL));
    let history = serde_json::to_string(&second_request.chat_history).unwrap();
    assert!(history.contains("agent.tool.deferred_after_barrier"));
    assert!(second_request.chat_history.iter().any(|message| {
        matches!(message, Message::User { content } if content.iter().any(|item| {
            matches!(item, UserContent::ToolResult(result) if result.id == "call-plan" && result.content.iter().any(|content| {
                matches!(content, ToolResultContent::Text(text) if serde_json::from_str::<serde_json::Value>(&text.text)
                    .ok()
                    .and_then(|value| value.get("retryable").and_then(serde_json::Value::as_bool)) == Some(true))
            }))
        }))
    }));

    let saved = repository.find_by_run_id(task.run_id.clone()).await.unwrap().unwrap();
    assert_eq!(saved.status, AgentTaskStatus::Running);
    assert!(saved.pending.is_none());
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
        event.kind == "agent.tool_call.completed"
            && event.payload["toolName"] == UPDATE_PLAN_TOOL
            && event.payload["outcome"] == "deferredAfterBarrier"
            && event.payload["schedulerState"] == "deferred"
    }));
    assert!(!events.iter().any(|event| {
        event.kind == "agent.failed"
            || event.payload["code"] == "agent_execution.no_progress_limit"
            || event.payload["code"] == "agent.tool.not_advertised_in_step"
    }));
}

#[tokio::test]
async fn final_response_candidate_continues_when_completion_gate_rejects_it() {
    let (pool, repository, task) = persisted_runtime("completion-gate").await;
    let memory_backend: Arc<dyn ConversationMemoryBackend> = Arc::new(SqliteConversationMemoryBackend::new(pool));
    let memory = ConversationMemoryAdapter::new(1, memory_backend, ConversationWindowPolicy::default());
    let budget = ExecutionBudget::new(AgentExecutionLimits::default());
    let hook = AgentRuntimeHook::new(budget, repository, task.clone());
    let model = MockCompletionModel::from_stream_turns([
        [
            MockStreamEvent::tool_call("call-1", "add", json!({ "x": 2, "y": 3 })),
            MockStreamEvent::final_response_with_default_usage(),
        ],
        [MockStreamEvent::text("现在创建组件："), MockStreamEvent::final_response_with_default_usage()],
        [MockStreamEvent::text("已完成"), MockStreamEvent::final_response_with_default_usage()],
    ]);

    let response = run_with_model_runtime(
        model.clone(),
        RunParts {
            memory,
            tools: vec![Box::new(MockAddTool)],
            preamble: "finish only after the gate passes".to_string(),
            max_model_calls: 4,
            temperature: None,
            max_tokens: None,
            context_window_tokens: None,
            context_limits: AgentContextLimits::default(),
            tool_route: AgentToolRoute::fixed(["add"]),
        },
        Message::user("create"),
        task.thread_id,
        hook,
        None,
        Some(Arc::new(RetryOnceCompletionGate { calls: AtomicUsize::new(0) })),
    )
    .await
    .unwrap();

    assert_eq!(response.output, "已完成");
    assert_eq!(model.request_count(), 3);
    assert!(model.requests()[2].chat_history.iter().any(|message| {
            matches!(message, Message::User { content } if content.iter().any(|item| matches!(item, UserContent::ToolResult(result) if result.id == "call-1")))
        }));
}

#[tokio::test]
async fn resumes_task_after_waiting_tool_returns() {
    let (pool, repository, task) = persisted_runtime("preview-resume").await;

    let memory_backend: Arc<dyn ConversationMemoryBackend> = Arc::new(SqliteConversationMemoryBackend::new(pool));
    let memory = ConversationMemoryAdapter::new(1, memory_backend, ConversationWindowPolicy::default());
    let budget = ExecutionBudget::new(AgentExecutionLimits::default());
    let hook = AgentRuntimeHook::new(budget, repository.clone(), task.clone());
    let model = MockCompletionModel::from_stream_turns([
        [
            MockStreamEvent::tool_call("call-preview", "create_canvas_components", json!({})),
            MockStreamEvent::final_response_with_default_usage(),
        ],
        [MockStreamEvent::text("done"), MockStreamEvent::final_response_with_default_usage()],
    ]);

    run_with_model_runtime(
        model,
        RunParts {
            memory,
            tools: vec![Box::new(PreviewTool)],
            preamble: "Show and confirm the preview once.".to_string(),
            max_model_calls: 4,
            temperature: None,
            max_tokens: None,
            context_window_tokens: None,
            context_limits: AgentContextLimits::default(),
            tool_route: AgentToolRoute::fixed(["create_canvas_components"]),
        },
        Message::user("show preview"),
        task.thread_id.clone(),
        hook,
        None,
        None,
    )
    .await
    .unwrap();

    let saved = repository.find_by_run_id(task.run_id.clone()).await.unwrap().unwrap();
    assert_eq!(saved.status, AgentTaskStatus::Running);
    assert!(saved.pending.is_none());
    let completed = repository
        .save_checkpoint(SaveAgentTaskCheckpoint {
            project_id: saved.project_id,
            thread_id: saved.thread_id.clone(),
            turn_id: saved.turn_id.clone(),
            run_id: saved.run_id.clone(),
            status: AgentTaskStatus::Completed,
            pending: None,
            usage: saved.usage.clone(),
            last_artifact: saved.last_artifact.clone(),
            error: None,
            thread_preview: Some("done".to_string()),
            project_snapshot_ref: saved.project_snapshot_ref.clone(),
            rig_version: saved.rig_version.clone(),
            rig_snapshot_ref: saved.rig_snapshot_ref.clone(),
            updated_at: saved.updated_at + 1,
            ended_at: Some(saved.updated_at + 1),
        })
        .await
        .unwrap();
    assert_eq!(completed.task.status, AgentTaskStatus::Completed);
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
    assert!(events.iter().any(|event| event.kind == "turn.waiting"));
    assert!(events.iter().any(|event| event.kind == "turn.resumed"));
    assert!(events.iter().any(|event| event.kind == "turn.completed"));
}

#[tokio::test]
async fn stringified_components_return_compact_shape_feedback_to_the_next_model_turn() {
    let (pool, repository, task) = persisted_runtime("compact-component-shape-error").await;
    let memory_backend: Arc<dyn ConversationMemoryBackend> = Arc::new(SqliteConversationMemoryBackend::new(pool));
    let memory = ConversationMemoryAdapter::new(1, memory_backend, ConversationWindowPolicy::default());
    let hook = AgentRuntimeHook::new(ExecutionBudget::new(AgentExecutionLimits::default()), repository, task.clone());
    let invalid_payload = "invalid-payload-marker".repeat(500);
    let model = MockCompletionModel::from_stream_turns([
        vec![
            MockStreamEvent::tool_call("call-invalid-components", "create_canvas_components", json!({ "components": invalid_payload })),
            MockStreamEvent::final_response_with_default_usage(),
        ],
        vec![MockStreamEvent::text("done"), MockStreamEvent::final_response_with_default_usage()],
    ]);

    let response = run_with_model_runtime(
        model.clone(),
        RunParts {
            memory,
            tools: vec![Box::new(StrictCreateTool)],
            preamble: "Create components.".to_string(),
            max_model_calls: 3,
            temperature: None,
            max_tokens: None,
            context_window_tokens: None,
            context_limits: AgentContextLimits::default(),
            tool_route: AgentToolRoute::fixed(["create_canvas_components"]),
        },
        Message::user("create"),
        task.thread_id,
        hook,
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(response.output, "done");
    let requests = model.requests();
    let feedback = requests[1]
        .chat_history
        .iter()
        .find_map(|message| match message {
            Message::User { content } => content.iter().find_map(|item| match item {
                UserContent::ToolResult(result) if result.id == "call-invalid-components" => serde_json::to_string(result).ok(),
                _ => None,
            }),
            _ => None,
        })
        .expect("the next request must contain the compact tool feedback");
    assert!(feedback.contains("agent.tool.invalid_argument_type"));
    assert!(feedback.contains(r#"\"fieldPath\":\"components\""#));
    assert!(feedback.contains("regenerate_arguments_with_native_array"));
    assert!(!feedback.contains(&"invalid-payload-marker".repeat(10)));
}

#[tokio::test]
async fn tool_timeout_terminates_the_runner_with_a_stable_execution_code() {
    let (pool, repository, task) = persisted_runtime("timeout").await;

    let memory_backend: Arc<dyn ConversationMemoryBackend> = Arc::new(SqliteConversationMemoryBackend::new(pool));
    let memory = ConversationMemoryAdapter::new(1, memory_backend, ConversationWindowPolicy::default());
    let mut limits = AgentExecutionLimits::default();
    limits.max_tool_duration = Duration::from_millis(5);
    let budget = ExecutionBudget::new(limits);
    let hook = AgentRuntimeHook::new(budget.clone(), repository.clone(), task.clone());
    let model = MockCompletionModel::from_stream_turns([[
        MockStreamEvent::tool_call("call-timeout", "pending_tool", json!({})),
        MockStreamEvent::final_response_with_default_usage(),
    ]]);
    let tools = with_tool_timeout(vec![Box::new(PendingTool)], budget.limits().max_tool_duration);

    let error = run_with_model_runtime(
        model.clone(),
        RunParts {
            memory,
            tools,
            preamble: "Call pending_tool once.".to_string(),
            max_model_calls: 4,
            temperature: None,
            max_tokens: None,
            context_window_tokens: None,
            context_limits: AgentContextLimits::default(),
            tool_route: AgentToolRoute::fixed(["pending_tool"]),
        },
        Message::user("wait forever"),
        task.thread_id.clone(),
        hook,
        None,
        None,
    )
    .await
    .expect_err("the timeout hook must terminate the runner");
    assert_eq!(error.code, AgentExecutionErrorCode::ToolDurationLimit.as_str());
    assert_eq!(model.request_count(), 1, "tool timeout must not be sent back for a model retry");
    let usage = budget.usage_snapshot();
    assert_eq!(usage.tool_calls, 1);
    assert_eq!(usage.consecutive_failures, 1);
    let events = repository
        .list_events(ListTaskEvents {
            project_id: task.project_id,
            thread_id: task.thread_id.clone(),
            turn_id: task.turn_id.clone(),
            run_id: task.run_id.clone(),
            after_sequence: 0,
            limit: 20,
        })
        .await
        .unwrap();
    assert!(events
        .iter()
        .any(|event| { event.kind == "agent.failed" && event.payload["code"] == AgentExecutionErrorCode::ToolDurationLimit.as_str() }));
}

#[path = "runner_long_task_tests.rs"]
mod long_task_tests;
