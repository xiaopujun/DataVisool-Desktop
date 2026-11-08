/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 真实 Runner 的长任务回归测试。
//!
//! 这些测试故意让模型跨越多次上下文压缩继续工作，避免只验证压缩辅助函数而
//! 漏掉 Rig 的模型请求、工具结果配对和 Hook 的实际调用链。

use std::collections::HashSet;
use std::fmt::{Display, Formatter};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rig_core::completion::{AssistantContent, CompletionError, CompletionModel, CompletionRequest, CompletionResponse, Message, Usage};
use rig_core::message::UserContent;
use rig_core::streaming::StreamingCompletionResponse;
use rig_core::test_utils::{MockCompletionModel, MockResponse, MockStreamEvent, MockTurn};
use rig_core::tool::Tool;
use serde_json::{json, Value};

use super::super::{run_with_model_runtime, RunParts};
use super::persisted_runtime;
use crate::backend::ai_driven::agent::{AgentContextLimits, AgentExecutionLimits, AgentRuntimeHook, AgentToolRoute, ExecutionBudget};
use crate::backend::ai_driven::memory::{
    ConversationMemoryAdapter, ConversationMemoryBackend, ConversationWindowPolicy, SqliteConversationMemoryBackend,
};
use crate::backend::ai_driven::tools::file::READ_WORKSPACE_FILE_TOOL;

#[derive(Clone)]
struct HybridCompletionModel {
    stream: MockCompletionModel,
    completion: MockCompletionModel,
}

impl HybridCompletionModel {
    fn new(stream: MockCompletionModel, completion: MockCompletionModel) -> Self {
        Self { stream, completion }
    }

    fn stream_requests(&self) -> Vec<CompletionRequest> {
        self.stream.requests()
    }

    fn completion_request_count(&self) -> usize {
        self.completion.request_count()
    }
}

impl CompletionModel for HybridCompletionModel {
    type Response = MockResponse;
    type StreamingResponse = MockResponse;
    type Client = ();

    fn make(_: &Self::Client, _: impl Into<String>) -> Self {
        Self::new(MockCompletionModel::default(), MockCompletionModel::default())
    }

    async fn completion(&self, request: CompletionRequest) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        self.completion.completion(request).await
    }

    async fn stream(&self, request: CompletionRequest) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        self.stream.stream(request).await
    }
}

#[derive(Debug)]
struct TestToolError;

impl Display for TestToolError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("long task test tool failed")
    }
}

impl std::error::Error for TestToolError {}

#[derive(Clone)]
struct ReadFactTool {
    calls: Arc<AtomicUsize>,
    result_size: usize,
}

impl Tool for ReadFactTool {
    const NAME: &'static str = READ_WORKSPACE_FILE_TOOL;
    type Error = TestToolError;
    type Args = Value;
    type Output = String;

    fn description(&self) -> String {
        "Read a workspace path and return a versioned fact.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        })
    }

    async fn call(&self, args: Value) -> Result<String, Self::Error> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let path = args.get("path").and_then(Value::as_str).unwrap_or("unknown");
        Ok(json!({
            "path": path,
            "sourceVersion": path,
            "fact": "x".repeat(self.result_size)
        })
        .to_string())
    }
}

#[derive(Clone)]
struct BatchCreateTool {
    created_ids: Arc<Mutex<Vec<String>>>,
}

impl Tool for BatchCreateTool {
    const NAME: &'static str = "create_batch";
    type Error = TestToolError;
    type Args = Value;
    type Output = String;

    fn description(&self) -> String {
        "Create one batch of components and return all created IDs.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "batch": { "type": "integer" },
                "componentIds": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["batch", "componentIds"]
        })
    }

    async fn call(&self, args: Value) -> Result<String, Self::Error> {
        let ids = args
            .get("componentIds")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        self.created_ids
            .lock()
            .expect("created IDs mutex should not be poisoned")
            .extend(ids.iter().cloned());
        Ok(json!({
            "status": "applied",
            "operationId": format!("operation-{}", args.get("batch").and_then(Value::as_u64).unwrap_or_default()),
            "toolCallId": "test-tool-call",
            "kind": "createComponents",
            "projectId": "1",
            "batch": args.get("batch"),
            "componentIds": ids
        })
        .to_string())
    }
}

fn tool_turn(id: impl Into<String>, name: impl Into<String>, args: Value) -> Vec<MockStreamEvent> {
    vec![MockStreamEvent::tool_call(id, name, args), MockStreamEvent::final_response_with_default_usage()]
}

fn text_turn(text: impl Into<String>) -> Vec<MockStreamEvent> {
    vec![MockStreamEvent::text(text), MockStreamEvent::final_response_with_default_usage()]
}

fn compact_limits() -> AgentContextLimits {
    AgentContextLimits {
        max_active_context_tokens: 6_000,
        compact_at_percent: 80,
        max_tool_result_tokens: 10_000,
        max_tool_history_tokens: 16_000,
        max_state_digest_tokens: 4_096,
        compact_summary_tokens: 128,
    }
}

fn request_history_tokens(request: &CompletionRequest) -> usize {
    serde_json::to_vec(&request.chat_history)
        .expect("mock request history should serialize")
        .len()
        .div_ceil(4)
}

fn full_request_tokens_with_output_reserve(request: &CompletionRequest) -> usize {
    serde_json::to_vec(request)
        .expect("mock request should serialize")
        .len()
        .div_ceil(4)
        + request.max_tokens.unwrap_or_default() as usize
}

fn assert_tool_result_pairs(request: &CompletionRequest) {
    let mut calls = HashSet::new();
    let mut results = HashSet::new();
    for message in request.chat_history.iter() {
        match message {
            Message::Assistant { content, .. } => {
                for item in content.iter() {
                    if let AssistantContent::ToolCall(call) = item {
                        calls.insert(call.id.clone());
                    }
                }
            }
            Message::User { content } => {
                for item in content.iter() {
                    if let UserContent::ToolResult(result) = item {
                        results.insert(result.id.clone());
                    }
                }
            }
            Message::System { .. } => {}
        }
    }
    assert_eq!(calls, results, "every model request must preserve complete tool call/result pairs");
}

fn usage_for_test() -> Usage {
    Usage { input_tokens: 12, output_tokens: 8, total_tokens: 20, ..Usage::default() }
}

#[tokio::test]
async fn long_runner_compacts_and_continues_three_create_batches() {
    let (pool, repository, task) = persisted_runtime("long-cross-compaction").await;
    let memory_backend: Arc<dyn ConversationMemoryBackend> = Arc::new(SqliteConversationMemoryBackend::new(pool));
    let memory = ConversationMemoryAdapter::new(1, memory_backend, ConversationWindowPolicy::default());
    let budget = ExecutionBudget::new(AgentExecutionLimits::default());
    let hook = AgentRuntimeHook::new(budget.clone(), repository.clone(), task.clone());
    let read_calls = Arc::new(AtomicUsize::new(0));
    let created_ids = Arc::new(Mutex::new(Vec::new()));

    let mut stream_turns = Vec::new();
    for batch in 0..3 {
        for offset in 0..7 {
            let index = batch * 7 + offset;
            stream_turns.push(tool_turn(
                format!("read-call-{index}"),
                READ_WORKSPACE_FILE_TOOL,
                json!({ "path": format!("workspace/fact-{index}.json") }),
            ));
        }
        let ids = (0..20).map(|offset| format!("component-{batch}-{offset}")).collect::<Vec<_>>();
        stream_turns.push(tool_turn(format!("create-call-{batch}"), "create_batch", json!({ "batch": batch, "componentIds": ids })));
    }
    stream_turns.push(text_turn("全部批次已完成，保留用户要求。"));

    let stream_model = MockCompletionModel::from_stream_turns(stream_turns);
    let summary_model = MockCompletionModel::new(
        (0..20).map(|_| MockTurn::text("压缩摘要：保留用户目标、追加要求、全部已创建 ID 和可恢复操作事实。").with_usage(usage_for_test())),
    );
    let model = HybridCompletionModel::new(stream_model, summary_model);
    let model_probe = model.clone();
    let response = run_with_model_runtime(
        model,
        RunParts {
            memory,
            tools: vec![
                Box::new(ReadFactTool { calls: read_calls.clone(), result_size: 2_500 }),
                Box::new(BatchCreateTool { created_ids: created_ids.clone() }),
            ],
            preamble: "Read the required facts, then create all batches and finish.".to_string(),
            max_model_calls: 40,
            temperature: None,
            max_tokens: Some(128),
            context_window_tokens: Some(8_192),
            context_limits: compact_limits(),
            tool_route: AgentToolRoute::fixed([READ_WORKSPACE_FILE_TOOL, "create_batch"]),
        },
        Message::user("用户目标：完成跨压缩批量创建。追加要求：保留全部组件 ID 并继续后续批次。"),
        task.thread_id.clone(),
        hook,
        None,
        None,
    )
    .await;
    let response = response.expect("the runner should continue after each safe compaction");

    assert!(response.output.contains("全部批次已完成"));
    assert_eq!(read_calls.load(Ordering::Acquire), 21, "all three read segments must execute");
    let created_ids = created_ids.lock().expect("created IDs mutex should not be poisoned");
    assert_eq!(created_ids.len(), 60);
    assert_eq!(created_ids.iter().collect::<HashSet<_>>().len(), 60);
    assert!(budget.usage_snapshot().compaction_count > 0, "the real context hook must compact during this run");
    assert!(model_probe.completion_request_count() > 0, "compaction must call the summary model");

    let requests = model_probe.stream_requests();
    assert!(requests.len() >= 25, "all 21 reads, 3 creates and final response must reach the model");
    for request in &requests {
        assert!(full_request_tokens_with_output_reserve(request) <= 8_192, "no known-over-capacity request may be dispatched");
        assert_tool_result_pairs(request);
    }
    let serialized_history = serde_json::to_string(&requests.last().expect("final request should exist").chat_history).unwrap();
    assert!(serialized_history.contains("用户目标：完成跨压缩批量创建"));
    assert!(serialized_history.contains("追加要求：保留全部组件 ID"));
    for batch in 0..3 {
        assert!(serialized_history.contains(&format!("operation-{batch}")));
        for offset in 0..20 {
            assert!(serialized_history.contains(&format!("component-{batch}-{offset}")));
        }
    }
}

#[tokio::test]
async fn failed_summary_falls_back_without_dispatching_over_capacity_history() {
    let (pool, repository, task) = persisted_runtime("long-summary-failure").await;
    let memory_backend: Arc<dyn ConversationMemoryBackend> = Arc::new(SqliteConversationMemoryBackend::new(pool));
    let memory = ConversationMemoryAdapter::new(1, memory_backend, ConversationWindowPolicy::default());
    let budget = ExecutionBudget::new(AgentExecutionLimits::default());
    let hook = AgentRuntimeHook::new(budget, repository, task.clone());
    let read_calls = Arc::new(AtomicUsize::new(0));
    let mut stream_turns = Vec::new();
    for index in 0..14 {
        stream_turns.push(tool_turn(
            format!("failure-read-call-{index}"),
            READ_WORKSPACE_FILE_TOOL,
            json!({ "path": format!("workspace/failure-fact-{index}.json") }),
        ));
    }
    stream_turns.push(text_turn("摘要服务失败后仍安全完成。"));
    let stream_model = MockCompletionModel::from_stream_turns(stream_turns);
    let summary_model = MockCompletionModel::new([MockTurn::error("summary provider unavailable")]);
    let model = HybridCompletionModel::new(stream_model, summary_model);
    let model_probe = model.clone();

    let response = run_with_model_runtime(
        model,
        RunParts {
            memory,
            tools: vec![Box::new(ReadFactTool { calls: read_calls.clone(), result_size: 2_000 })],
            preamble: "Read facts, then finish.".to_string(),
            max_model_calls: 16,
            temperature: None,
            max_tokens: Some(128),
            context_window_tokens: Some(8_192),
            context_limits: compact_limits(),
            tool_route: AgentToolRoute::fixed([READ_WORKSPACE_FILE_TOOL]),
        },
        Message::user("保留用户目标并在摘要失败后继续。"),
        task.thread_id,
        hook,
        None,
        None,
    )
    .await
    .expect("bounded fallback summary should let the runner continue");

    assert!(response.output.contains("摘要服务失败后仍安全完成"));
    assert_eq!(read_calls.load(Ordering::Acquire), 14);
    assert!(model_probe.completion_request_count() > 0);
    for request in model_probe.stream_requests() {
        assert!(full_request_tokens_with_output_reserve(&request) <= 8_192);
        assert_tool_result_pairs(&request);
    }
}

#[tokio::test]
async fn oversized_current_tool_result_is_rejected_before_next_model_request() {
    let (pool, repository, task) = persisted_runtime("long-oversized-result").await;
    let memory_backend: Arc<dyn ConversationMemoryBackend> = Arc::new(SqliteConversationMemoryBackend::new(pool));
    let memory = ConversationMemoryAdapter::new(1, memory_backend, ConversationWindowPolicy::default());
    let budget = ExecutionBudget::new(AgentExecutionLimits::default());
    let hook = AgentRuntimeHook::new(budget, repository, task.clone());
    let stream_model = MockCompletionModel::from_stream_turns([
        tool_turn("oversized-read", READ_WORKSPACE_FILE_TOOL, json!({ "path": "workspace/oversized" })),
        text_turn("this response must never be sent"),
    ]);
    let summary_model = MockCompletionModel::new([MockTurn::text("unreachable summary")]);
    let model = HybridCompletionModel::new(stream_model, summary_model);
    let model_probe = model.clone();

    let result = run_with_model_runtime(
        model,
        RunParts {
            memory,
            tools: vec![Box::new(ReadFactTool { calls: Arc::new(AtomicUsize::new(0)), result_size: 30_000 })],
            preamble: "Read the path and finish.".to_string(),
            max_model_calls: 4,
            temperature: None,
            max_tokens: Some(64),
            context_window_tokens: Some(1_024),
            context_limits: AgentContextLimits {
                max_active_context_tokens: 900,
                compact_at_percent: 80,
                max_tool_result_tokens: 40_000,
                max_tool_history_tokens: 40_000,
                max_state_digest_tokens: 4_096,
                compact_summary_tokens: 64,
            },
            tool_route: AgentToolRoute::fixed([READ_WORKSPACE_FILE_TOOL]),
        },
        Message::user("超大读取结果必须安全停止。"),
        task.thread_id,
        hook,
        None,
        None,
    )
    .await;

    assert!(result.is_err(), "an unrepresentable current tool pair must terminate safely");
    assert_eq!(model_probe.stream_requests().len(), 1, "the known-over-capacity follow-up must not be dispatched");
    assert_eq!(
        model_probe.completion_request_count(),
        1,
        "the bounded summary request may run before the unrecoverable main request"
    );
    assert!(request_history_tokens(&model_probe.stream_requests()[0]) <= 1_024, "the initial request must still fit");
}
