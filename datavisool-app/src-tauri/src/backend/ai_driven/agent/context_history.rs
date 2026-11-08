/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 对 Tool 结果做单项与滚动总量限长，并保持调用与结果配对。

use std::collections::{BTreeMap, HashMap, HashSet};

use rig_core::completion::{AssistantContent, Message};
use rig_core::message::{ToolResult, ToolResultContent, UserContent};
use rig_core::OneOrMany;
use serde_json::{json, Map, Value};

const MIN_TOOL_RESULT_BYTES: usize = 64;

#[derive(Default, Debug, Eq, PartialEq)]
pub(super) struct HistoryProjectionStats {
    pub projected_results: usize,
    pub budget_projected_results: usize,
    pub orphan_results_removed: usize,
    pub synthetic_results: usize,
    pub tool_result_bytes_before: usize,
    pub tool_result_bytes_after: usize,
}

pub(super) fn project_tool_history(history: &[Message], max_tool_result_tokens: u64) -> (Vec<Message>, HistoryProjectionStats) {
    let call_names = tool_call_names(history);
    let max_tool_result_bytes = usize::try_from(max_tool_result_tokens.saturating_mul(4)).unwrap_or(usize::MAX);
    let mut stats = HistoryProjectionStats::default();
    let projected = history
        .iter()
        .map(|message| bound_message(message, &call_names, max_tool_result_bytes, &mut stats))
        .collect();
    (projected, stats)
}

/// 限制工具结果总量，并在投影前规范化 Tool call/result 配对。
pub(super) fn project_tool_history_with_budget(
    history: &[Message],
    current_prompt: Option<&Message>,
    max_tool_result_tokens: u64,
    max_tool_history_tokens: u64,
) -> (Vec<Message>, HistoryProjectionStats) {
    let (projected, mut stats) = project_tool_history(history, max_tool_result_tokens);
    let mut projected = normalize_tool_pairs(&projected, current_prompt, &mut stats);
    let call_names = tool_call_names(&projected);
    let mut locations = Vec::new();
    for (message_index, message) in projected.iter().enumerate() {
        if let Message::User { content } = message {
            locations.extend(
                content
                    .iter()
                    .enumerate()
                    .filter_map(|(item_index, item)| matches!(item, UserContent::ToolResult(_)).then_some((message_index, item_index))),
            );
        }
    }
    stats.tool_result_bytes_before = locations
        .iter()
        .filter_map(|(message_index, item_index)| match &projected[*message_index] {
            Message::User { content } => content.iter().nth(*item_index).and_then(|item| match item {
                UserContent::ToolResult(result) => Some(tool_result_bytes(result)),
                _ => None,
            }),
            _ => None,
        })
        .sum();

    let mut remaining_bytes = token_bytes(max_tool_history_tokens);
    let mut remaining_results = locations.len();
    for (message_index, item_index) in locations.into_iter().rev() {
        remaining_results = remaining_results.saturating_sub(1);
        let Some(result) = (match &projected[message_index] {
            Message::User { content } => content.iter().nth(item_index).and_then(|item| match item {
                UserContent::ToolResult(result) => Some(result.clone()),
                _ => None,
            }),
            _ => None,
        }) else {
            continue;
        };
        let result_bytes = tool_result_bytes(&result);
        let reserved_bytes = MIN_TOOL_RESULT_BYTES.saturating_mul(remaining_results);
        let available_bytes = remaining_bytes.saturating_sub(reserved_bytes);
        if result_bytes <= available_bytes {
            remaining_bytes = remaining_bytes.saturating_sub(result_bytes);
            continue;
        }
        let tool_name = call_names.get(&result.id).map(String::as_str).unwrap_or("unknown");
        let replacement = budgeted_tool_result(tool_name, &result, available_bytes, max_tool_result_tokens);
        let replacement_bytes = tool_result_bytes(&replacement);
        if let Message::User { content } = &mut projected[message_index] {
            let mut items = content.iter().cloned().collect::<Vec<_>>();
            items[item_index] = UserContent::ToolResult(replacement);
            *content = OneOrMany::many(items).expect("原始用户消息至少包含一项");
        }
        stats.budget_projected_results += 1;
        remaining_bytes = remaining_bytes.saturating_sub(replacement_bytes);
    }
    stats.tool_result_bytes_after = projected.iter().map(message_tool_result_bytes).sum();
    (projected, stats)
}

fn token_bytes(tokens: u64) -> usize {
    usize::try_from(tokens.saturating_mul(4)).unwrap_or(usize::MAX)
}

fn message_tool_result_bytes(message: &Message) -> usize {
    match message {
        Message::User { content } => content
            .iter()
            .filter_map(|item| match item {
                UserContent::ToolResult(result) => Some(tool_result_bytes(result)),
                _ => None,
            })
            .sum(),
        _ => 0,
    }
}

fn normalize_tool_pairs(history: &[Message], current_prompt: Option<&Message>, stats: &mut HistoryProjectionStats) -> Vec<Message> {
    let call_ids = history
        .iter()
        .filter_map(|message| match message {
            Message::Assistant { content, .. } => Some(content),
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|item| match item {
            AssistantContent::ToolCall(call) => Some(call.id.as_str()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    // Rig 将当前轮 ToolResult 作为 prompt 单独传入；配对判断必须同时看到它，
    // 否则会为 history 末尾的 ToolCall 误补一条 synthetic result。
    let result_ids = history
        .iter()
        .chain(current_prompt)
        .filter_map(|message| match message {
            Message::User { content } => Some(content),
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|item| match item {
            UserContent::ToolResult(result) => Some(result.id.as_str()),
            _ => None,
        })
        .collect::<HashSet<_>>();

    let mut normalized = Vec::with_capacity(history.len());
    for message in history {
        match message {
            Message::Assistant { content, .. } => {
                normalized.push(message.clone());
                let missing_results = content
                    .iter()
                    .filter_map(|item| match item {
                        AssistantContent::ToolCall(call) if !result_ids.contains(call.id.as_str()) => Some(call),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if !missing_results.is_empty() {
                    let synthetic_count = missing_results.len();
                    let synthetic = missing_results
                        .into_iter()
                        .map(|call| {
                            UserContent::ToolResult(ToolResult {
                                id: call.id.clone(),
                                call_id: call.call_id.clone(),
                                content: OneOrMany::one(ToolResultContent::text(
                                    json!({ "status": "aborted", "reason": "tool result unavailable" }).to_string(),
                                )),
                            })
                        })
                        .collect::<Vec<_>>();
                    normalized.push(Message::User { content: OneOrMany::many(synthetic).expect("缺失结果至少包含一项") });
                    stats.synthetic_results += synthetic_count;
                }
            }
            Message::User { content } => {
                let mut items = Vec::new();
                for item in content.iter() {
                    match item {
                        UserContent::ToolResult(result) if !call_ids.contains(result.id.as_str()) => {
                            stats.orphan_results_removed += 1;
                        }
                        _ => items.push(item.clone()),
                    }
                }
                if !items.is_empty() {
                    normalized.push(Message::User { content: OneOrMany::many(items).expect("保留内容至少包含一项") });
                }
            }
            _ => normalized.push(message.clone()),
        }
    }
    normalized
}

fn budgeted_tool_result(tool_name: &str, result: &ToolResult, remaining_bytes: usize, max_tool_result_tokens: u64) -> ToolResult {
    let original_bytes = tool_result_bytes(result);
    let digest = serde_json::from_str::<Value>(&tool_result_digest(tool_name, result, None, false))
        .unwrap_or_else(|error| json!({ "summary": error.to_string() }));
    let receipt = digest.get("facts").cloned().unwrap_or_else(|| json!({}));
    let reference = json!({
        "tool": tool_name,
        "toolCallId": result.call_id.as_deref().unwrap_or(&result.id),
        "source": "original_tool_result",
    });
    let candidates = [
        json!({ "truncated": true, "originalBytes": original_bytes, "maxBytes": token_bytes(max_tool_result_tokens), "reference": reference.clone(), "digest": digest }),
        json!({ "truncated": true, "reference": reference.clone(), "facts": receipt }),
        json!({ "truncated": true, "originalBytes": original_bytes, "reference": reference.clone() }),
        json!({ "truncated": true, "reference": reference.clone() }),
        json!({ "truncated": true, "toolCallId": result.call_id.as_deref().unwrap_or(&result.id) }),
        Value::String(String::new()),
    ];
    let content = candidates
        .into_iter()
        .find(|candidate| candidate.to_string().len() <= remaining_bytes)
        .unwrap_or_else(|| json!(null));
    ToolResult {
        id: result.id.clone(),
        call_id: result.call_id.clone(),
        content: OneOrMany::one(ToolResultContent::text(content.to_string())),
    }
}

pub(super) fn state_digest_with_limits(
    history: &[Message],
    _max_tool_result_tokens: u64,
    _max_tool_history_tokens: u64,
    max_state_digest_tokens: u64,
) -> String {
    // Facts used for recovery are collected from the original audit history.
    // A size projection may replace a large result with a short digest before
    // the operationId or a pending status can be recovered.
    let calls = tool_call_facts(history);
    let mut operations = BTreeMap::new();
    let mut operation_ids = Vec::new();
    let mut pending_operation_ids = Vec::new();
    let mut reads = BTreeMap::new();
    let mut canvas = Map::new();
    let mut latest_plan = None;
    let mut latest_operation = None;
    let mut latest_error = None;
    // A previous compaction replaces the raw messages with a summary. Carry
    // its structured digest forward before scanning newly available results;
    // otherwise operation IDs disappear at the next compaction boundary.
    for digest in embedded_state_digests(history) {
        if let Some(ids) = digest.get("operationIds").and_then(Value::as_array) {
            for id in ids.iter().filter_map(Value::as_str) {
                if !operation_ids.iter().any(|known| known == id) {
                    operation_ids.push(id.to_string());
                }
            }
        }
        if let Some(ids) = digest.get("pendingOperationIds").and_then(Value::as_array) {
            for id in ids.iter().filter_map(Value::as_str) {
                if !pending_operation_ids.iter().any(|known| known == id) {
                    pending_operation_ids.push(id.to_string());
                }
            }
        }
        if let Some(items) = digest.get("operations").and_then(Value::as_array) {
            for item in items {
                if let Some(operation_id) = item.get("operationId").and_then(Value::as_str) {
                    operations.entry(operation_id.to_string()).or_insert_with(|| item.clone());
                }
            }
        }
        if let Some(items) = digest.get("documentAndWorkspaceReads").and_then(Value::as_array) {
            for item in items {
                let key = serde_json::to_string(item).unwrap_or_default();
                reads.insert(key, item.clone());
            }
        }
        if let Some(object) = digest.get("canvas").and_then(Value::as_object) {
            canvas.extend(object.iter().map(|(key, value)| (key.clone(), value.clone())));
        }
        latest_plan = digest
            .get("latestPlan")
            .cloned()
            .filter(|value| !value.is_null())
            .or(latest_plan);
        latest_operation = digest
            .get("latestOperation")
            .cloned()
            .filter(|value| !value.is_null())
            .or(latest_operation);
        latest_error = digest
            .get("latestStructuredError")
            .cloned()
            .filter(|value| !value.is_null())
            .or(latest_error);
    }
    for message in history {
        let Message::User { content } = message else { continue };
        for item in content.iter() {
            let UserContent::ToolResult(result) = item else { continue };
            let Some((tool_name, arguments)) = calls.get(&result.id) else { continue };
            let raw = tool_result_text(result);
            let parsed = serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!({ "summary": truncate_chars(raw.trim(), 700) }));
            let mut facts = Map::new();
            collect_fact_fields(&parsed, &mut facts, 0);
            if let Some(operation_id) = facts.get("operationId").and_then(Value::as_str) {
                operations.insert(operation_id.to_string(), Value::Object(facts.clone()));
                if !operation_ids.iter().any(|known| known == operation_id) {
                    operation_ids.push(operation_id.to_string());
                }
                let terminal = facts
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| matches!(status, "applied" | "failed" | "rejected" | "error" | "cancelled" | "completed"));
                if terminal {
                    pending_operation_ids.retain(|known| known != operation_id);
                } else if !pending_operation_ids.iter().any(|known| known == operation_id) {
                    pending_operation_ids.push(operation_id.to_string());
                }
                latest_operation = Some(Value::Object(facts.clone()));
            }
            if is_stateful_read(tool_name) {
                let read = if tool_name == "inspect_current_page" {
                    compact_inspect_read_fact(arguments, &parsed, &facts)
                } else if tool_name == "resolve_component_specs" {
                    compact_component_spec_read_fact(arguments, &parsed, &facts)
                } else {
                    compact_read_fact(tool_name, arguments, &facts)
                };
                let key = format!("{tool_name}:{}", serde_json::to_string(arguments).unwrap_or_default());
                reads.insert(key, read);
            }
            if tool_name == "update_plan" {
                latest_plan = Some(bound_json_value(arguments, token_bytes(max_state_digest_tokens) / 2));
            }
            for key in ["managerId", "revision", "resultRevision", "workspaceRevision", "componentBindings"] {
                if let Some(value) = facts.get(key) {
                    canvas.insert(key.to_string(), value.clone());
                }
            }
            if facts.get("code").is_some()
                || facts
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| matches!(status, "failed" | "rejected" | "error"))
            {
                latest_error = Some(Value::Object(facts));
            }
        }
    }
    if operations.is_empty() && reads.is_empty() && canvas.is_empty() && latest_plan.is_none() && latest_error.is_none() {
        "无可提取的历史事实；以当前任务上下文和实时 Tool 结果为准。".to_string()
    } else {
        let digest = json!({
            "canvas": canvas,
            "latestOperation": latest_operation,
            "operationIds": operation_ids,
            "pendingOperationIds": pending_operation_ids,
            "operations": operations.into_values().rev().collect::<Vec<_>>(),
            "documentAndWorkspaceReads": reads.into_values().take(32).collect::<Vec<_>>(),
            "latestPlan": latest_plan,
            "latestStructuredError": latest_error,
        })
        .to_string();
        bound_digest(&digest, max_state_digest_tokens)
    }
}

fn embedded_state_digests(history: &[Message]) -> Vec<Value> {
    history
        .iter()
        .filter_map(|message| match message {
            Message::User { content } => content.iter().find_map(|item| match item {
                UserContent::Text(text) if text.text.starts_with("[Agent context summary]") => {
                    let marker = "\n\n[Agent state digest]\n";
                    let digest = text.text.split_once(marker)?.1.split_once("\n\n[Execution budget]")?.0;
                    serde_json::from_str::<Value>(digest).ok()
                }
                _ => None,
            }),
            _ => None,
        })
        .collect()
}

fn bound_digest(value: &str, max_tokens: u64) -> String {
    let max_bytes = token_bytes(max_tokens);
    if value.len() <= max_bytes {
        return value.to_string();
    }
    if let Ok(parsed) = serde_json::from_str::<Value>(value) {
        let references = json!({
            "truncated": true,
            "operationIds": parsed.get("operationIds").cloned().unwrap_or_else(|| json!([])),
            "pendingOperationIds": parsed.get("pendingOperationIds").cloned().unwrap_or_else(|| json!([])),
        })
        .to_string();
        if references.len() <= max_bytes {
            return references;
        }
        // Operation references are replay-safety data. If even their compact
        // form cannot fit, mark the window unrecoverable so the caller stops
        // before dispatching a request that would silently lose facts.
        return json!({
            "truncated": true,
            "unrecoverable": true,
            "reason": "operation references exceed digest budget",
            "operationIds": parsed.get("operationIds").cloned().unwrap_or_else(|| json!([])),
            "pendingOperationIds": parsed.get("pendingOperationIds").cloned().unwrap_or_else(|| json!([])),
        })
        .to_string();
    }
    json!({
        "truncated": true,
        "summary": truncate_utf8_bytes(value, max_bytes.saturating_sub(64) / 2),
    })
    .to_string()
}

fn bound_json_value(value: &Value, max_bytes: usize) -> Value {
    let encoded = serde_json::to_string(value).unwrap_or_default();
    if encoded.len() <= max_bytes {
        return value.clone();
    }
    json!({
        "truncated": true,
        "originalBytes": encoded.len(),
        "summary": truncate_utf8_bytes(&encoded, max_bytes.saturating_sub(64)),
    })
}

fn tool_call_facts(history: &[Message]) -> HashMap<String, (String, Value)> {
    history
        .iter()
        .filter_map(|message| match message {
            Message::Assistant { content, .. } => Some(content),
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|content| match content {
            AssistantContent::ToolCall(call) => Some((call.id.clone(), (call.function.name.clone(), call.function.arguments.clone()))),
            _ => None,
        })
        .collect()
}

fn tool_result_text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| match content {
            ToolResultContent::Text(text) => Some(text.text.as_str()),
            ToolResultContent::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn is_stateful_read(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "resolve_component_specs"
            | "get_component_data_contract"
            | "inspect_current_page"
            | "list_workspace_files"
            | "search_workspace_files"
            | "read_workspace_file"
    )
}

fn compact_read_fact(tool_name: &str, arguments: &Value, facts: &Map<String, Value>) -> Value {
    let mut compact = Map::from_iter([
        ("toolName".to_string(), Value::String(tool_name.to_string())),
        ("arguments".to_string(), compact_read_arguments(arguments)),
    ]);
    for key in [
        "sourceVersion",
        "corpusVersion",
        "catalogVersion",
        "workspaceRevision",
        "revision",
        "componentType",
        "section",
        "startLine",
        "endLine",
        "startOffset",
        "nextStartOffset",
        "nextOffset",
        "page",
        "pageSize",
        "nextPage",
        "pageToken",
        "nextPageToken",
        "cursor",
        "nextCursor",
        "sourceType",
        "path",
        "status",
        "hasMore",
    ] {
        if let Some(value) = facts.get(key) {
            compact.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(compact)
}

fn compact_component_spec_read_fact(arguments: &Value, result: &Value, facts: &Map<String, Value>) -> Value {
    let mut compact = Map::from_iter([
        ("toolName".to_string(), Value::String("resolve_component_specs".to_string())),
        ("arguments".to_string(), compact_component_spec_arguments(arguments)),
    ]);
    if let Some(value) = facts.get("corpusVersion") {
        compact.insert("corpusVersion".to_string(), value.clone());
    }
    let specs = result
        .get("specs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|spec| {
            let object = spec.as_object()?;
            let component_type = object.get("componentType").and_then(Value::as_str)?.trim();
            if component_type.is_empty() || !is_complete_component_spec(object) {
                return None;
            }
            Some(json!({ "componentType": component_type, "complete": true }))
        })
        .collect::<Vec<_>>();
    if !specs.is_empty() {
        compact.insert("specs".to_string(), Value::Array(specs));
    }
    Value::Object(compact)
}

fn compact_component_spec_arguments(arguments: &Value) -> Value {
    let Some(arguments) = arguments.as_object() else {
        return Value::Null;
    };
    let mut compact = Map::new();
    for key in ["componentTypes", "intent"] {
        if let Some(value) = arguments.get(key) {
            compact.insert(key.to_string(), bound_json_value(value, 2_000));
        }
    }
    Value::Object(compact)
}

fn is_complete_component_spec(object: &Map<String, Value>) -> bool {
    let configurable = object
        .get("configurable")
        .and_then(Value::as_object)
        .is_some_and(|value| !value.is_empty());
    let example = object
        .get("example")
        .and_then(Value::as_object)
        .is_some_and(|value| !value.is_empty());
    let valid_data_requirements = object.get("dataRequirements").map_or(true, Value::is_string);
    let valid_constraints = object.get("constraints").map_or(true, Value::is_array);
    configurable && example && valid_data_requirements && valid_constraints
}

/// 组件快照是模型上下文中最大的回执之一。后续验证只需知道查询范围、实际
/// 命中的组件和分页情况，不能把 layer/config/style/data 原文再次带回模型。
fn compact_inspect_read_fact(arguments: &Value, result: &Value, facts: &Map<String, Value>) -> Value {
    let mut compact = compact_read_fact("inspect_current_page", arguments, facts)
        .as_object()
        .cloned()
        .unwrap_or_default();
    let Some(arguments) = arguments.as_object() else {
        return Value::Object(compact);
    };
    for key in ["componentIds", "fields"] {
        if let Some(values) = arguments.get(key).and_then(Value::as_array) {
            let values = values
                .iter()
                .filter_map(Value::as_str)
                .take(100)
                .map(|value| Value::String(value.to_string()))
                .collect::<Vec<_>>();
            compact.insert(
                match key {
                    "componentIds" => "requestedComponentIds".to_string(),
                    _ => "requestedFields".to_string(),
                },
                Value::Array(values),
            );
        }
    }
    for key in ["allPage", "offset", "limit"] {
        if let Some(value) = arguments.get(key) {
            compact.insert(key.to_string(), value.clone());
        }
    }
    if let Some(components) = result.get("components").and_then(Value::as_array) {
        let actual_component_ids = components
            .iter()
            .filter_map(|component| component.get("id").and_then(Value::as_str))
            .take(100)
            .map(|id| Value::String(id.to_string()))
            .collect::<Vec<_>>();
        let actual_component_types = components
            .iter()
            .filter_map(|component| component.get("type").and_then(Value::as_str))
            .take(100)
            .map(|component_type| Value::String(component_type.to_string()))
            .collect::<Vec<_>>();
        compact.insert("actualComponentIds".to_string(), Value::Array(actual_component_ids));
        compact.insert("actualComponentTypes".to_string(), Value::Array(actual_component_types));
        compact.insert("actualComponentCount".to_string(), json!(components.len()));
    }
    if let Some(value) = result.get("totalComponents") {
        compact.insert("totalComponents".to_string(), value.clone());
    }
    for key in ["actualComponentIds", "actualComponentTypes", "actualComponentCount", "totalComponents"] {
        if let Some(value) = facts.get(key) {
            compact.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(compact)
}

fn compact_read_arguments(arguments: &Value) -> Value {
    let Some(arguments) = arguments.as_object() else {
        return Value::Null;
    };
    let mut compact = Map::new();
    for key in [
        "componentType",
        "componentTypes",
        "componentIds",
        "section",
        "intent",
        "fields",
        "startLine",
        "lineCount",
        "startOffset",
        "nextStartOffset",
        "path",
        "query",
        "offset",
        "limit",
        "page",
        "pageSize",
        "pageToken",
        "cursor",
        "sourceType",
    ] {
        if let Some(value) = arguments.get(key) {
            compact.insert(
                key.to_string(),
                match value {
                    Value::String(value) => Value::String(truncate_chars(value, 200)),
                    value => bound_json_value(value, 2_000),
                },
            );
        }
    }
    Value::Object(compact)
}

fn bound_message(
    message: &Message,
    call_names: &HashMap<String, String>,
    max_tool_result_bytes: usize,
    stats: &mut HistoryProjectionStats,
) -> Message {
    let Message::User { content } = message else {
        return message.clone();
    };
    let items = content
        .iter()
        .map(|item| match item {
            UserContent::ToolResult(result) if tool_result_bytes(result) > max_tool_result_bytes => {
                let tool_name = call_names.get(&result.id).map(String::as_str).unwrap_or("unknown");
                let original_bytes = tool_result_bytes(result);
                let digest = tool_result_digest(tool_name, result, None, false);
                let digest = serde_json::from_str::<Value>(&digest).unwrap_or_else(|_| Value::String(digest));
                stats.projected_results += 1;
                UserContent::ToolResult(ToolResult {
                    id: result.id.clone(),
                    call_id: result.call_id.clone(),
                    content: OneOrMany::one(ToolResultContent::text(
                        json!({
                            "truncated": true,
                            "originalBytes": original_bytes,
                            "maxBytes": max_tool_result_bytes,
                            "reference": {
                                "tool": tool_name,
                                "toolCallId": result.call_id.as_deref().unwrap_or(&result.id),
                                "source": "original_tool_result",
                            },
                            "digest": digest,
                        })
                        .to_string(),
                    )),
                })
            }
            _ => item.clone(),
        })
        .collect::<Vec<_>>();
    Message::User { content: OneOrMany::many(items).expect("原始用户消息至少包含一项") }
}

fn tool_result_bytes(result: &ToolResult) -> usize {
    result
        .content
        .iter()
        .map(|content| match content {
            ToolResultContent::Text(text) => text.text.len(),
            ToolResultContent::Image(_) => 0,
        })
        .sum()
}

fn tool_call_names(history: &[Message]) -> HashMap<String, String> {
    history
        .iter()
        .filter_map(|message| match message {
            Message::Assistant { content, .. } => Some(content),
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|content| match content {
            AssistantContent::ToolCall(call) => Some((call.id.clone(), call.function.name.clone())),
            _ => None,
        })
        .collect()
}

pub(super) fn tool_result_digest(tool_name: &str, result: &ToolResult, latest_version: Option<u64>, duplicate: bool) -> String {
    let raw = result
        .content
        .iter()
        .filter_map(|content| match content {
            ToolResultContent::Text(text) => Some(text.text.as_str()),
            ToolResultContent::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let parsed = serde_json::from_str::<Value>(&raw).ok();
    let mut facts = Map::new();
    if let Some(value) = parsed.as_ref() {
        collect_fact_fields(value, &mut facts, 0);
        if tool_name == "inspect_current_page" {
            collect_inspect_fact_fields(value, &mut facts);
        } else if tool_name == "resolve_component_specs" {
            let specs = value
                .get("specs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|spec| {
                    let object = spec.as_object()?;
                    let component_type = object.get("componentType").and_then(Value::as_str)?.trim();
                    if component_type.is_empty() || !is_complete_component_spec(object) {
                        return None;
                    }
                    Some(json!({ "componentType": component_type, "complete": true }))
                })
                .collect::<Vec<_>>();
            if !specs.is_empty() {
                facts.insert("specs".to_string(), Value::Array(specs));
            }
        }
    }
    if facts.is_empty() && !raw.trim().is_empty() {
        facts.insert("summary".to_string(), Value::String(truncate_chars(raw.trim(), 700)));
    }
    let version = parsed.as_ref().and_then(find_version);
    let stale = version.zip(latest_version).is_some_and(|(version, latest)| version < latest);
    json!({
        "tool": tool_name,
        "sourceVersion": version,
        "stale": stale,
        "sameAsPrevious": duplicate,
        "facts": facts
    })
    .to_string()
}

fn collect_fact_fields(value: &Value, facts: &mut Map<String, Value>, depth: usize) {
    if depth > 7 || facts.len() >= 32 {
        return;
    }
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if is_fact_key(key) && is_bounded_fact_value(child) {
                    facts.entry(key.clone()).or_insert_with(|| child.clone());
                }
                collect_fact_fields(child, facts, depth + 1);
            }
        }
        Value::Array(items) => {
            for child in items.iter().take(12) {
                collect_fact_fields(child, facts, depth + 1);
            }
        }
        _ => {}
    }
}

fn collect_inspect_fact_fields(value: &Value, facts: &mut Map<String, Value>) {
    let Some(components) = value.get("components").and_then(Value::as_array) else {
        return;
    };
    let component_ids = components
        .iter()
        .filter_map(|component| component.get("id").and_then(Value::as_str))
        .take(100)
        .map(|id| Value::String(id.to_string()))
        .collect::<Vec<_>>();
    let component_types = components
        .iter()
        .filter_map(|component| component.get("type").and_then(Value::as_str))
        .take(100)
        .map(|component_type| Value::String(component_type.to_string()))
        .collect::<Vec<_>>();
    facts.insert("actualComponentIds".to_string(), Value::Array(component_ids));
    facts.insert("actualComponentTypes".to_string(), Value::Array(component_types));
    facts.insert("actualComponentCount".to_string(), json!(components.len()));
    if let Some(total) = value.get("totalComponents") {
        facts.insert("totalComponents".to_string(), total.clone());
    }
}

fn is_bounded_fact_value(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => true,
        Value::String(value) => value.chars().count() <= 1_000,
        Value::Array(items) => {
            items.len() <= 100
                && items.iter().all(|item| match item {
                    Value::String(value) => value.chars().count() <= 1_000,
                    value => !value.is_array() && !value.is_object(),
                })
        }
        Value::Object(object) => {
            object.len() <= 100
                && object.values().all(|value| match value {
                    Value::String(value) => value.chars().count() <= 1_000,
                    Value::Null | Value::Bool(_) | Value::Number(_) => true,
                    _ => false,
                })
        }
    }
}

fn is_fact_key(key: &str) -> bool {
    matches!(
        key,
        "status"
            | "code"
            | "message"
            | "operationId"
            | "kind"
            | "revision"
            | "resultRevision"
            | "workspaceRevision"
            | "corpusVersion"
            | "sourceVersion"
            | "catalogVersion"
            | "managerId"
            | "componentId"
            | "componentIds"
            | "componentType"
            | "componentBindings"
            | "configuredComponentCount"
            | "defaultConfigComponentCount"
            | "actualComponentIds"
            | "actualComponentTypes"
            | "actualComponentCount"
            | "totalComponents"
            | "missingComponentIds"
            | "missingComponentTypes"
            | "path"
            | "section"
            | "startLine"
            | "endLine"
            | "sourceType"
            | "total"
            | "totalEntries"
            | "totalMatches"
            | "offset"
            | "nextOffset"
            | "hasMore"
            | "truncated"
            | "retryHint"
    )
}

fn find_version(value: &Value) -> Option<u64> {
    ["resultRevision", "workspaceRevision", "revision"]
        .into_iter()
        .find_map(|key| find_field(value, key).and_then(Value::as_u64))
}

fn find_field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(object) => object
            .get(key)
            .or_else(|| object.values().find_map(|child| find_field(child, key))),
        Value::Array(items) => items.iter().find_map(|child| find_field(child, key)),
        _ => None,
    }
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        value.to_string()
    } else {
        value.chars().take(limit).collect::<String>() + "…"
    }
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
#[path = "context_history_tests.rs"]
mod tests;
