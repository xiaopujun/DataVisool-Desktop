/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! CompletionModel 请求级上下文观测；DEBUG 日志会记录已脱敏的完整请求内容。

use rig_core::completion::{AssistantContent, CompletionError, CompletionModel, CompletionRequest, CompletionResponse, Message, Usage};
use rig_core::message::{ToolResultContent, UserContent};
use rig_core::streaming::StreamingCompletionResponse;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::utils::{debug_log_json, sanitize_debug_log_text};

#[derive(Clone, Debug, Default)]
pub(crate) struct ContextTraceIdentity {
    pub project_id: i64,
    pub thread_id: String,
    pub task_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub request_id: String,
}

#[derive(Clone)]
pub(crate) struct ObservedCompletionModel<M> {
    inner: M,
    identity: ContextTraceIdentity,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextSegmentMetric {
    pub bytes: usize,
    pub estimated_tokens: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextRequestMetrics {
    pub total_bytes: usize,
    pub estimated_input_tokens: u64,
    pub system_rules: ContextSegmentMetric,
    pub tool_schemas: ContextSegmentMetric,
    pub current_task: ContextSegmentMetric,
    pub history: ContextSegmentMetric,
    pub tool_results: ContextSegmentMetric,
    pub compaction_summary: ContextSegmentMetric,
    pub message_count: usize,
    pub tool_count: usize,
    pub image_count: usize,
    pub image_bytes: usize,
    pub requested_component_count: usize,
    pub requested_document_count: usize,
    pub requested_field_count: usize,
    pub truncation_count: usize,
}

impl<M> ObservedCompletionModel<M> {
    pub(crate) fn new(inner: M, identity: ContextTraceIdentity) -> Self {
        Self { inner, identity }
    }
}

impl<M> CompletionModel for ObservedCompletionModel<M>
where
    M: CompletionModel,
{
    type Response = M::Response;
    type StreamingResponse = M::StreamingResponse;
    type Client = M::Client;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        Self { inner: M::make(client, model), identity: ContextTraceIdentity::default() }
    }

    async fn completion(&self, request: CompletionRequest) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        self.observe("completion", &request);
        let result = self.inner.completion(request).await;
        if let Ok(response) = &result {
            self.observe_usage("completion", &response.usage);
        }
        self.observe_failure("completion", &result);
        result
    }

    async fn stream(&self, request: CompletionRequest) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        self.observe("stream", &request);
        let result = self.inner.stream(request).await;
        if let Ok(response) = &result {
            let usage = response.usage();
            if usage.has_values() {
                self.observe_usage("stream", &usage);
            }
        }
        self.observe_failure("stream", &result);
        result
    }

    fn composes_native_output_with_tools(&self) -> bool {
        self.inner.composes_native_output_with_tools()
    }
}

impl<M> ObservedCompletionModel<M> {
    fn observe(&self, call_kind: &str, request: &CompletionRequest) {
        let metrics = measure_completion_request(request);
        tracing::debug!(
            target: "agent.context",
            event = "model_request",
            project_id = self.identity.project_id,
            thread_id = %self.identity.thread_id,
            task_id = %self.identity.task_id,
            turn_id = %self.identity.turn_id,
            run_id = %self.identity.run_id,
            request_id = %self.identity.request_id,
            call_kind,
            total_bytes = metrics.total_bytes,
            estimated_input_tokens = metrics.estimated_input_tokens,
            system_bytes = metrics.system_rules.bytes,
            system_estimated_tokens = metrics.system_rules.estimated_tokens,
            tool_schema_bytes = metrics.tool_schemas.bytes,
            tool_schema_estimated_tokens = metrics.tool_schemas.estimated_tokens,
            current_task_bytes = metrics.current_task.bytes,
            current_task_estimated_tokens = metrics.current_task.estimated_tokens,
            history_bytes = metrics.history.bytes,
            history_estimated_tokens = metrics.history.estimated_tokens,
            tool_result_bytes = metrics.tool_results.bytes,
            tool_result_estimated_tokens = metrics.tool_results.estimated_tokens,
            compaction_bytes = metrics.compaction_summary.bytes,
            compaction_estimated_tokens = metrics.compaction_summary.estimated_tokens,
            message_count = metrics.message_count,
            tool_count = metrics.tool_count,
            image_count = metrics.image_count,
            image_bytes = metrics.image_bytes,
            requested_components = metrics.requested_component_count,
            requested_documents = metrics.requested_document_count,
            requested_fields = metrics.requested_field_count,
            truncation_count = metrics.truncation_count,
            request = %debug_log_json(request),
            "Agent model request sent"
        );
    }

    fn observe_failure<T>(&self, call_kind: &str, result: &Result<T, CompletionError>) {
        let Err(error) = result else {
            return;
        };
        tracing::debug!(
            target: "agent.model",
            event = "model_request_failed",
            project_id = self.identity.project_id,
            thread_id = %self.identity.thread_id,
            task_id = %self.identity.task_id,
            turn_id = %self.identity.turn_id,
            run_id = %self.identity.run_id,
            request_id = %self.identity.request_id,
            call_kind,
            error = %sanitize_debug_log_text(&error.to_string()),
            "Agent model request failed"
        );
    }

    fn observe_usage(&self, call_kind: &str, usage: &Usage) {
        tracing::debug!(
            target: "agent.model",
            event = "model_usage_reported",
            project_id = self.identity.project_id,
            thread_id = %self.identity.thread_id,
            task_id = %self.identity.task_id,
            turn_id = %self.identity.turn_id,
            run_id = %self.identity.run_id,
            request_id = %self.identity.request_id,
            call_kind,
            usage_reported = usage.has_values(),
            provider_input_tokens = usage.input_tokens,
            provider_output_tokens = usage.output_tokens,
            provider_total_tokens = usage.total_tokens,
            cached_input_tokens = usage.cached_input_tokens,
            cache_creation_input_tokens = usage.cache_creation_input_tokens,
            tool_use_prompt_tokens = usage.tool_use_prompt_tokens,
            reasoning_tokens = usage.reasoning_tokens,
            "Provider model usage observed"
        );
    }
}

pub(crate) fn measure_completion_request(request: &CompletionRequest) -> ContextRequestMetrics {
    let mut metrics = ContextRequestMetrics {
        total_bytes: serialized_len(request),
        message_count: request.chat_history.len(),
        tool_count: request.tools.len(),
        ..ContextRequestMetrics::default()
    };
    add_value_bytes(&mut metrics.tool_schemas, &request.tools, 0);
    if let Some(preamble) = request.preamble.as_ref() {
        add_text_bytes(&mut metrics.system_rules, preamble);
    }
    for message in request.chat_history.iter() {
        measure_message(message, &mut metrics);
    }
    for document in &request.documents {
        add_value_bytes(&mut metrics.history, document, 0);
        metrics.requested_document_count += 1;
    }
    metrics.estimated_input_tokens = metrics.system_rules.estimated_tokens
        + metrics.tool_schemas.estimated_tokens
        + metrics.current_task.estimated_tokens
        + metrics.history.estimated_tokens
        + metrics.tool_results.estimated_tokens
        + metrics.compaction_summary.estimated_tokens;
    metrics
}

fn measure_message(message: &Message, metrics: &mut ContextRequestMetrics) {
    match message {
        Message::System { content } => add_text_bytes(&mut metrics.system_rules, content),
        Message::User { content } => {
            let serialized = serialized_len(message);
            let image_bytes = content
                .iter()
                .filter_map(|item| match item {
                    UserContent::Image(image) => Some(serialized_len(image)),
                    _ => None,
                })
                .sum::<usize>();
            metrics.image_count += content.iter().filter(|item| matches!(item, UserContent::Image(_))).count();
            metrics.image_bytes += image_bytes;
            let has_tool_results = content.iter().any(|item| matches!(item, UserContent::ToolResult(_)));
            let is_current_task = user_text_starts_with(content, "[Agent task context]");
            if is_current_task {
                for item in content.iter() {
                    if let UserContent::Text(text) = item {
                        if let Some(json_start) = text.text.find('{') {
                            if let Ok(value) = serde_json::from_str::<Value>(&text.text[json_start..]) {
                                count_requested_items(&value, metrics);
                            }
                        }
                    }
                }
            }
            let segment = if has_tool_results {
                for item in content.iter() {
                    if let UserContent::ToolResult(result) = item {
                        for result_content in result.content.iter() {
                            if let ToolResultContent::Text(text) = result_content {
                                if let Ok(value) = serde_json::from_str::<Value>(&text.text) {
                                    count_truncations(&value, metrics);
                                } else {
                                    metrics.truncation_count += text.text.matches("\"truncated\":true").count();
                                }
                            }
                        }
                    }
                }
                &mut metrics.tool_results
            } else if is_current_task {
                &mut metrics.current_task
            } else if user_text_starts_with(content, "[Agent context summary]") || user_text_starts_with(content, "[Agent state digest]") {
                &mut metrics.compaction_summary
            } else {
                &mut metrics.history
            };
            add_raw_bytes(segment, serialized, image_bytes);
        }
        Message::Assistant { content, .. } => {
            for item in content.iter() {
                if let AssistantContent::ToolCall(call) = item {
                    count_requested_items(&call.function.arguments, metrics);
                }
            }
            add_value_bytes(&mut metrics.history, message, 0);
        }
    }
}

fn user_text_starts_with(content: &rig_core::OneOrMany<UserContent>, prefix: &str) -> bool {
    content
        .iter()
        .any(|item| matches!(item, UserContent::Text(text) if text.text.starts_with(prefix)))
}

fn count_requested_items(value: &Value, metrics: &mut ContextRequestMetrics) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let count = child.as_array().map_or(0, Vec::len);
                match key.as_str() {
                    "componentIds" | "components" | "targets" => metrics.requested_component_count += count,
                    "componentTypes" | "docs" => metrics.requested_document_count += count,
                    "fields" | "sections" => metrics.requested_field_count += count,
                    "truncationCount" => metrics.truncation_count += child.as_u64().unwrap_or(0) as usize,
                    _ => {}
                }
                count_requested_items(child, metrics);
            }
        }
        Value::Array(items) => {
            for child in items {
                count_requested_items(child, metrics);
            }
        }
        _ => {}
    }
}

fn count_truncations(value: &Value, metrics: &mut ContextRequestMetrics) {
    match value {
        Value::Object(object) => {
            if let Some(count) = object.get("truncationCount").and_then(Value::as_u64) {
                metrics.truncation_count += count as usize;
            } else if object.get("truncated").and_then(Value::as_bool) == Some(true) {
                metrics.truncation_count += 1;
            }
            for child in object.values() {
                count_truncations(child, metrics);
            }
        }
        Value::Array(items) => {
            for child in items {
                count_truncations(child, metrics);
            }
        }
        _ => {}
    }
}

fn add_text_bytes(segment: &mut ContextSegmentMetric, value: &str) {
    add_raw_bytes(segment, value.len(), 0);
}

fn add_value_bytes(segment: &mut ContextSegmentMetric, value: &impl Serialize, image_bytes: usize) {
    add_raw_bytes(segment, serialized_len(value), image_bytes);
}

fn add_raw_bytes(segment: &mut ContextSegmentMetric, bytes: usize, non_text_bytes: usize) {
    segment.bytes = segment.bytes.saturating_add(bytes);
    segment.estimated_tokens = segment
        .estimated_tokens
        .saturating_add((bytes.saturating_sub(non_text_bytes) as u64).saturating_add(3) / 4);
}

fn serialized_len(value: &impl Serialize) -> usize {
    serde_json::to_vec(value).map_or(0, |bytes| bytes.len())
}

#[cfg(test)]
mod tests {
    use rig_core::completion::{CompletionRequest, Message, ToolDefinition};
    use rig_core::message::{ToolResultContent, UserContent};
    use rig_core::OneOrMany;
    use serde_json::json;

    use super::*;

    #[test]
    fn separates_context_segments_and_counts_requested_scope() {
        let request = CompletionRequest {
            model: None,
            preamble: Some("规则".to_string()),
            chat_history: OneOrMany::many([
                Message::user("[Agent task context]\n{\"goal\":\"改图表\"}"),
                Message::Assistant {
                    id: None,
                    content: OneOrMany::one(AssistantContent::tool_call(
                        "call-1",
                        "inspect_current_page",
                        json!({ "componentIds": ["c1", "c2"], "fields": ["style"] }),
                    )),
                },
                Message::User {
                    content: OneOrMany::one(UserContent::tool_result(
                        "call-1",
                        OneOrMany::one(ToolResultContent::text("{\"truncated\":true,\"truncationCount\":2}")),
                    )),
                },
                Message::user("[Agent context summary]\n旧历史"),
            ])
            .unwrap(),
            tools: vec![ToolDefinition {
                name: "inspect_current_page".to_string(),
                description: "读取".to_string(),
                parameters: json!({"type": "object"}),
            }],
            documents: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        };

        let metrics = measure_completion_request(&request);

        assert!(metrics.system_rules.bytes > 0);
        assert!(metrics.tool_schemas.bytes > 0);
        assert!(metrics.current_task.bytes > 0);
        assert!(metrics.tool_results.bytes > 0);
        assert!(metrics.compaction_summary.bytes > 0);
        assert_eq!(metrics.requested_component_count, 2);
        assert_eq!(metrics.requested_field_count, 1);
        assert!(metrics.truncation_count >= 2);
    }
}
