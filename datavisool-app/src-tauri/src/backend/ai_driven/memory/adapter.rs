/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Rig 消息与项目稳定会话消息的单一转换边界。

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rig_core::completion::{AssistantContent, Message};
use rig_core::memory::{ConversationMemory, MemoryError};
use rig_core::message::{ToolResultContent, UserContent};
use rig_core::wasm_compat::WasmBoxedFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::conversation_store::{
    ConversationMemoryBackend, ConversationMemoryError, ConversationMessage, ConversationRole, ConversationSummary, ListConversationMessages,
    NewConversationMessage, SaveConversationSummary,
};
use super::policy::{ConversationWindow, ConversationWindowPolicy};

const STORED_MESSAGE_SCHEMA_VERSION: u16 = 2;
const STORED_SUMMARY_SCHEMA_VERSION: u16 = 1;
const AUTO_SUMMARY_MAX_CHARS: usize = 4_096;
const AUTO_SUMMARY_PAGE_SIZE: u32 = 256;
const AUTO_SUMMARY_MESSAGE_CHARS: usize = 512;
const AUTO_SUMMARY_MAX_TOOL_NAMES: usize = 32;

const STORED_MESSAGE_MAX_CHARS: usize = 8_192;
const STORED_TOOL_RESULT_MAX_CHARS: usize = 1_024;

/// 项目拥有的消息信封。内容只保存稳定文本摘要，不保存 Rig 序列化块。
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredMessageEnvelope {
    schema_version: u16,
    message: StoredMessage,
}

/// Rig 的富媒体内容块只在此转换层解析；数据库只认识这个项目版本化信封。
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum StoredMessage {
    User { text: String },
    Assistant { text: String },
    Tool { text: String },
}

/// 项目自有的历史摘要信封；不保存 Rig 类型或完整消息 JSON。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredConversationSummary {
    schema_version: u16,
    kind: &'static str,
    text: String,
}

/// 绑定单个项目和窗口策略的 Rig ConversationMemory。
#[derive(Clone)]
pub(crate) struct ConversationMemoryAdapter {
    project_id: i64,
    backend: Arc<dyn ConversationMemoryBackend>,
    policy: ConversationWindowPolicy,
    task_scope: Option<TaskMemoryScope>,
}

#[derive(Clone, Debug)]
struct TaskMemoryScope {
    thread_id: String,
    turn_id: String,
}

impl ConversationMemoryAdapter {
    pub(crate) fn new(project_id: i64, backend: Arc<dyn ConversationMemoryBackend>, policy: ConversationWindowPolicy) -> Self {
        Self { project_id, backend, policy, task_scope: None }
    }

    pub(crate) fn with_turn_scope(mut self, thread_id: String, turn_id: String) -> Self {
        self.task_scope = Some(TaskMemoryScope { thread_id, turn_id });
        self
    }

    fn validate_conversation_key(&self, conversation_id: &str) -> Result<(), MemoryError> {
        if conversation_id.trim().is_empty() {
            return Err(MemoryError::Policy("conversation_id must not be blank".to_string()));
        }
        if let Some(scope) = &self.task_scope {
            let expected = super::task_adapter::task_conversation_key(&scope.turn_id);
            if conversation_id != expected {
                return Err(MemoryError::Policy(format!("task memory key must be {expected}, got {conversation_id}")));
            }
        }
        Ok(())
    }

    async fn load_compacted_window(&self, conversation_id: &str) -> Result<ConversationWindow, ConversationMemoryError> {
        if let Some(scope) = &self.task_scope {
            return self
                .backend
                .load_turn_window(self.project_id, scope.thread_id.clone(), scope.turn_id.clone(), self.policy)
                .await;
        }
        let window = self
            .backend
            .load_window(self.project_id, conversation_id.to_string(), self.policy)
            .await?;
        if !window.was_truncated {
            return Ok(window);
        }

        let summarized_until = window.summary.as_ref().map_or(0, |summary| summary.summarized_until_sequence);
        let through_sequence = window
            .truncated_through_sequence
            .ok_or_else(|| ConversationMemoryError::InvalidRecord("truncated conversation window is missing its compaction boundary".to_string()))?;
        if through_sequence <= summarized_until {
            return Err(ConversationMemoryError::InvalidRecord(
                "truncated conversation window did not advance beyond the summary watermark".to_string(),
            ));
        }

        let mut builder =
            AutoSummaryBuilder::new(window.summary.as_ref(), summarized_until.saturating_add(1), through_sequence, summary_char_limit(self.policy));
        let mut cursor = summarized_until;
        while cursor < through_sequence {
            let page = self
                .backend
                .list_messages(ListConversationMessages {
                    project_id: self.project_id,
                    thread_id: conversation_id.to_string(),
                    after_sequence: cursor,
                    through_sequence: Some(through_sequence),
                    limit: AUTO_SUMMARY_PAGE_SIZE,
                })
                .await?;
            let next_cursor = page
                .last()
                .map(|message| message.sequence)
                .ok_or_else(|| ConversationMemoryError::InvalidRecord(format!("conversation history is missing messages after sequence {cursor}")))?;
            if next_cursor <= cursor {
                return Err(ConversationMemoryError::InvalidRecord("conversation compaction cursor did not advance".to_string()));
            }
            for message in page {
                builder.push_message(message)?;
            }
            cursor = next_cursor;
        }

        let summary_text = builder.finish();
        let estimated_tokens = estimate_text_tokens(&summary_text);
        let content = serde_json::to_value(StoredConversationSummary {
            schema_version: STORED_SUMMARY_SCHEMA_VERSION,
            kind: "conversationHistorySummary",
            text: summary_text,
        })?;
        self.backend
            .save_summary(SaveConversationSummary {
                project_id: self.project_id,
                thread_id: conversation_id.to_string(),
                content,
                summarized_until_sequence: through_sequence,
                estimated_tokens: Some(estimated_tokens),
                updated_at: summary_now_ms()?,
            })
            .await?;

        self.backend
            .load_window(self.project_id, conversation_id.to_string(), self.policy)
            .await
    }
}

impl ConversationMemory for ConversationMemoryAdapter {
    fn load<'a>(&'a self, conversation_id: &'a str) -> WasmBoxedFuture<'a, Result<Vec<Message>, MemoryError>> {
        Box::pin(async move {
            self.validate_conversation_key(conversation_id)?;
            let window = self
                .load_compacted_window(conversation_id)
                .await
                .map_err(MemoryError::backend)?;
            let mut messages = Vec::with_capacity(window.messages.len() + usize::from(window.summary.is_some()));
            if let Some(summary) = window.summary {
                messages.push(summary_to_rig_message(summary.summarized_until_sequence, &summary.content));
            }
            for message in window.messages {
                messages.push(stored_to_rig_message(message).map_err(MemoryError::backend)?);
            }
            Ok(messages)
        })
    }

    fn append<'a>(&'a self, conversation_id: &'a str, messages: Vec<Message>) -> WasmBoxedFuture<'a, Result<(), MemoryError>> {
        Box::pin(async move {
            self.validate_conversation_key(conversation_id)?;
            let (thread_id, turn_id) = self
                .task_scope
                .as_ref()
                .map(|scope| (scope.thread_id.clone(), Some(scope.turn_id.clone())))
                .unwrap_or_else(|| (conversation_id.to_string(), None));
            let created_at = now_ms()?;
            let mut stored_messages = Vec::with_capacity(messages.len());
            for (index, message) in messages.into_iter().enumerate() {
                let (role, content, artifact_refs) = rig_to_stored_message(message)?;
                stored_messages.push(NewConversationMessage {
                    project_id: self.project_id,
                    // Task 记忆仍归属 Thread Conversation，但轨迹按 Turn 隔离。
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    source_item_id: None,
                    role,
                    content,
                    estimated_tokens: None,
                    artifact_refs,
                    created_at: created_at.saturating_add(index as i64),
                });
            }
            self.backend
                .append_messages(stored_messages)
                .await
                .map_err(MemoryError::backend)?;
            Ok(())
        })
    }

    fn clear<'a>(&'a self, conversation_id: &'a str) -> WasmBoxedFuture<'a, Result<(), MemoryError>> {
        Box::pin(async move {
            self.validate_conversation_key(conversation_id)?;
            if let Some(scope) = &self.task_scope {
                self.backend
                    .clear_turn(self.project_id, scope.thread_id.clone(), scope.turn_id.clone())
                    .await
                    .map_err(MemoryError::backend)
            } else {
                self.backend
                    .clear(self.project_id, conversation_id.to_string())
                    .await
                    .map_err(MemoryError::backend)
            }
        })
    }
}

struct AutoSummaryBuilder {
    start_sequence: u64,
    through_sequence: u64,
    max_chars: usize,
    previous_summary: String,
    dialogue: VecDeque<String>,
    tool_names: BTreeSet<String>,
}

impl AutoSummaryBuilder {
    fn new(previous: Option<&ConversationSummary>, start_sequence: u64, through_sequence: u64, max_chars: usize) -> Self {
        Self {
            start_sequence,
            through_sequence,
            max_chars,
            previous_summary: previous
                .map(|summary| extract_summary_text(&summary.content, AUTO_SUMMARY_MAX_CHARS))
                .unwrap_or_default(),
            dialogue: VecDeque::new(),
            tool_names: BTreeSet::new(),
        }
    }

    fn push_message(&mut self, message: ConversationMessage) -> Result<(), ConversationMemoryError> {
        self.collect_stored_tool_names(&message.content);
        match stored_to_rig_message(message)? {
            Message::User { content } => {
                for item in content {
                    if let UserContent::Text(text) = item {
                        self.push_dialogue("User", &text.text);
                    }
                }
            }
            Message::Assistant { content, .. } => {
                for item in content {
                    match item {
                        AssistantContent::Text(text) => self.push_dialogue("Assistant", &text.text),
                        AssistantContent::ToolCall(call) if self.tool_names.len() < AUTO_SUMMARY_MAX_TOOL_NAMES => {
                            let name = normalize_visible_text(&call.function.name, 64);
                            if !name.is_empty() {
                                self.tool_names.insert(name);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Message::System { .. } => {
                return Err(ConversationMemoryError::InvalidRecord(
                    "system message cannot appear in persisted conversation history".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn collect_stored_tool_names(&mut self, content: &Value) {
        let Some(text) = content.pointer("/message/text").and_then(Value::as_str) else {
            return;
        };
        for section in text.split("[Tool call: ").skip(1) {
            let Some(name) = section.split(']').next() else {
                continue;
            };
            let name = normalize_visible_text(name, 64);
            if !name.is_empty() && self.tool_names.len() < AUTO_SUMMARY_MAX_TOOL_NAMES {
                self.tool_names.insert(name);
            }
        }
    }

    fn push_dialogue(&mut self, role: &str, text: &str) {
        let text = normalize_visible_text(text, AUTO_SUMMARY_MESSAGE_CHARS);
        if text.is_empty() {
            return;
        }
        self.dialogue.push_back(format!("{role}: {text}"));
        while self.dialogue.iter().map(|line| line.chars().count() + 1).sum::<usize>() > AUTO_SUMMARY_MAX_CHARS {
            self.dialogue.pop_front();
        }
    }

    fn finish(self) -> String {
        let mut output = String::new();
        append_bounded(&mut output, &format!("History {}-{}.", self.start_sequence, self.through_sequence), self.max_chars);
        if !self.tool_names.is_empty() {
            append_bounded(&mut output, "\nTools: ", self.max_chars);
            append_bounded(&mut output, &self.tool_names.into_iter().collect::<Vec<_>>().join(", "), self.max_chars);
        }

        let dialogue = self.dialogue.into_iter().collect::<Vec<_>>().join("\n");
        let remaining = self.max_chars.saturating_sub(output.chars().count());
        let previous_budget = if !self.previous_summary.is_empty() && !dialogue.is_empty() {
            remaining / 3
        } else if !self.previous_summary.is_empty() {
            remaining
        } else {
            0
        };
        if previous_budget > 0 {
            let section = format!("\nPrevious: {}", self.previous_summary);
            append_bounded(&mut output, &truncate_chars(&section, previous_budget), self.max_chars);
        }
        if !dialogue.is_empty() {
            append_bounded(&mut output, "\n", self.max_chars);
            let remaining = self.max_chars.saturating_sub(output.chars().count());
            append_bounded(&mut output, &retain_trailing_chars(&dialogue, remaining), self.max_chars);
        }
        output
    }
}

fn rig_to_stored_message(message: Message) -> Result<(ConversationRole, Value, Vec<String>), MemoryError> {
    let mut artifact_refs = BTreeSet::new();
    let (role, message) = match message {
        Message::System { .. } => {
            return Err(MemoryError::Policy(
                "system prompts are factory-owned and must not be persisted in conversation memory".to_string(),
            ))
        }
        Message::User { content } => {
            let role = if content.iter().all(|item| matches!(item, UserContent::ToolResult(_))) {
                ConversationRole::Tool
            } else {
                ConversationRole::User
            };
            let mut sections = Vec::new();
            for item in content {
                match item {
                    UserContent::Text(text) => push_visible_section(&mut sections, &text.text, STORED_MESSAGE_MAX_CHARS),
                    UserContent::ToolResult(result) => {
                        sections.push(summarize_tool_result(&result, &mut artifact_refs));
                    }
                    UserContent::Image(_) => sections.push("[image omitted; use Artifact reference]".to_string()),
                    UserContent::Audio(_) => sections.push("[audio omitted]".to_string()),
                    UserContent::Video(_) => sections.push("[video omitted]".to_string()),
                    UserContent::Document(_) => sections.push("[document omitted; use Artifact reference]".to_string()),
                }
            }
            let text = truncate_chars(&sections.join("\n"), STORED_MESSAGE_MAX_CHARS);
            let stored = if role == ConversationRole::Tool {
                StoredMessage::Tool { text }
            } else {
                StoredMessage::User { text }
            };
            (role, stored)
        }
        Message::Assistant { content, .. } => {
            let mut sections = Vec::new();
            for item in content {
                match item {
                    AssistantContent::Text(text) => push_visible_section(&mut sections, &text.text, STORED_MESSAGE_MAX_CHARS),
                    AssistantContent::ToolCall(call) => {
                        collect_artifact_refs(&call.function.arguments, &mut artifact_refs);
                        let name = normalize_visible_text(&call.function.name, 128);
                        sections.push(format!("[Tool call: {name}]"));
                    }
                    AssistantContent::Reasoning(_) => {}
                    AssistantContent::Image(_) => sections.push("[assistant image omitted]".to_string()),
                }
            }
            let text = truncate_chars(&sections.join("\n"), STORED_MESSAGE_MAX_CHARS);
            (ConversationRole::Assistant, StoredMessage::Assistant { text })
        }
    };
    let content = serde_json::to_value(StoredMessageEnvelope { schema_version: STORED_MESSAGE_SCHEMA_VERSION, message })
        .map_err(|error| MemoryError::Internal(format!("serialize project message envelope: {error}")))?;
    Ok((role, content, artifact_refs.into_iter().collect()))
}

fn stored_to_rig_message(message: ConversationMessage) -> Result<Message, ConversationMemoryError> {
    let schema_version = message
        .content
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| ConversationMemoryError::InvalidRecord("conversation message is missing schemaVersion".to_string()))?;
    if schema_version == u64::from(STORED_MESSAGE_SCHEMA_VERSION) {
        let envelope: StoredMessageEnvelope = serde_json::from_value(message.content)?;
        return match envelope.message {
            StoredMessage::User { text } => {
                require_role(message.role, ConversationRole::User)?;
                Ok(Message::user(text))
            }
            StoredMessage::Assistant { text } => {
                require_role(message.role, ConversationRole::Assistant)?;
                Ok(Message::assistant(text))
            }
            StoredMessage::Tool { text } => {
                require_role(message.role, ConversationRole::Tool)?;
                Ok(Message::user(text))
            }
        };
    }
    Err(ConversationMemoryError::InvalidRecord(format!(
        "unsupported conversation message schema version: {schema_version}"
    )))
}

fn push_visible_section(sections: &mut Vec<String>, text: &str, max_chars: usize) {
    let text = normalize_visible_text(text, max_chars);
    if !text.is_empty() {
        sections.push(text);
    }
}

fn summarize_tool_result(result: &rig_core::message::ToolResult, artifact_refs: &mut BTreeSet<String>) -> String {
    let mut summaries = Vec::new();
    for item in result.content.iter() {
        match item {
            ToolResultContent::Text(text) => {
                if let Ok(value) = serde_json::from_str::<Value>(&text.text) {
                    collect_artifact_refs(&value, artifact_refs);
                    summaries.push(summarize_tool_json(&value));
                } else {
                    push_visible_section(&mut summaries, &text.text, STORED_TOOL_RESULT_MAX_CHARS);
                }
            }
            ToolResultContent::Image(_) => summaries.push("[tool image omitted; use Artifact reference]".to_string()),
        }
    }
    let summary = truncate_chars(&summaries.join("; "), STORED_TOOL_RESULT_MAX_CHARS);
    format!("[Tool result {}: {summary}]", normalize_visible_text(&result.id, 128))
}

fn summarize_tool_json(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let mut fields = Vec::new();
            for key in ["artifactId", "revision", "status", "valid", "code", "draftId", "layerCount"] {
                if let Some(value) = object
                    .get(key)
                    .filter(|value| value.is_string() || value.is_number() || value.is_boolean())
                {
                    fields.push(format!("{key}={value}"));
                }
            }
            if let Some(supplement) = object.get("supplement").and_then(Value::as_str) {
                fields.push(format!("supplement={}", normalize_visible_text(supplement, 512)));
            }
            if fields.is_empty() {
                let keys = object.keys().take(16).cloned().collect::<Vec<_>>().join(",");
                format!("object(keys={keys})")
            } else {
                fields.join(", ")
            }
        }
        Value::Array(items) => format!("array(length={})", items.len()),
        Value::String(text) => normalize_visible_text(text, STORED_TOOL_RESULT_MAX_CHARS),
        Value::Null => "null".to_string(),
        value => value.to_string(),
    }
}

fn collect_artifact_refs(value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if let (Some(artifact_id), Some(revision)) =
                (object.get("artifactId").and_then(Value::as_str), object.get("revision").and_then(Value::as_u64))
            {
                let artifact_id = artifact_id.trim();
                if !artifact_id.is_empty() {
                    refs.insert(format!("{artifact_id}@{revision}"));
                }
            }
            for child in object.values() {
                collect_artifact_refs(child, refs);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_artifact_refs(child, refs);
            }
        }
        _ => {}
    }
}

fn require_role(actual: ConversationRole, expected: ConversationRole) -> Result<(), ConversationMemoryError> {
    if actual != expected {
        return Err(ConversationMemoryError::InvalidRecord(format!(
            "stored message role mismatch: expected {expected:?}, got {actual:?}"
        )));
    }
    Ok(())
}

fn summary_to_rig_message(summarized_until_sequence: u64, content: &Value) -> Message {
    let text = extract_summary_text(content, AUTO_SUMMARY_MAX_CHARS);
    Message::user(format!("[Conversation summary through message {summarized_until_sequence}]\n{text}"))
}

fn extract_summary_text(content: &Value, max_chars: usize) -> String {
    if let Some(text) = content.as_str() {
        return normalize_visible_text(text, max_chars);
    }
    if let Some(text) = content.get("text").and_then(Value::as_str) {
        return normalize_visible_text(text, max_chars);
    }

    let mut fragments = Vec::new();
    collect_summary_fragments(content, None, &mut fragments, max_chars, 0);
    truncate_chars(&fragments.join("; "), max_chars)
}

fn collect_summary_fragments(value: &Value, key: Option<&str>, fragments: &mut Vec<String>, max_chars: usize, depth: u8) {
    if depth > 4 || fragments.len() >= 32 || fragments.iter().map(|item| item.chars().count()).sum::<usize>() >= max_chars {
        return;
    }
    match value {
        Value::String(text) if !key.is_some_and(is_payload_key) => {
            let text = normalize_visible_text(text, AUTO_SUMMARY_MESSAGE_CHARS.min(max_chars));
            if !text.is_empty() {
                fragments.push(text);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_summary_fragments(item, key, fragments, max_chars, depth + 1);
            }
        }
        Value::Object(object) => {
            for (child_key, child) in object {
                collect_summary_fragments(child, Some(child_key), fragments, max_chars, depth + 1);
            }
        }
        _ => {}
    }
}

fn is_payload_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "data" | "base64" | "bytes" | "binary" | "payload" | "arguments" | "input" | "contentjson"
    )
}

fn normalize_visible_text(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let trimmed = text.trim();
    if trimmed.starts_with("data:") || looks_like_embedded_binary(trimmed) {
        return truncate_chars("[binary-like text omitted]", max_chars);
    }
    let normalized = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&normalized, max_chars)
}

fn looks_like_embedded_binary(text: &str) -> bool {
    if text.len() < 256 {
        return false;
    }
    let mut significant = 0_usize;
    let mut base64_chars = 0_usize;
    let mut whitespace = 0_usize;
    for character in text.chars() {
        if character.is_whitespace() {
            whitespace += 1;
            continue;
        }
        significant += 1;
        if character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=') {
            base64_chars += 1;
        }
    }
    significant > 0 && whitespace.saturating_mul(50) < significant && base64_chars.saturating_mul(100) >= significant.saturating_mul(98)
}

fn summary_char_limit(policy: ConversationWindowPolicy) -> usize {
    let policy_chars = policy.max_estimated_tokens.saturating_mul(2);
    usize::try_from(policy_chars)
        .unwrap_or(usize::MAX)
        .min(AUTO_SUMMARY_MAX_CHARS)
        .max(1)
}

fn estimate_text_tokens(text: &str) -> u64 {
    let chars = text.chars().count() as u64;
    if chars == 0 {
        0
    } else {
        (chars + 3) / 4
    }
}

fn append_bounded(output: &mut String, value: &str, max_chars: usize) {
    let remaining = max_chars.saturating_sub(output.chars().count());
    if remaining > 0 {
        output.push_str(&truncate_chars(value, remaining));
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let length = value.chars().count();
    if length <= max_chars {
        return value.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars == 1 {
        return "…".to_string();
    }
    let mut truncated = value.chars().take(max_chars - 1).collect::<String>();
    truncated.push('…');
    truncated
}

fn retain_trailing_chars(value: &str, max_chars: usize) -> String {
    let length = value.chars().count();
    if length <= max_chars {
        return value.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let mut result = String::from("…");
    result.extend(value.chars().skip(length.saturating_sub(max_chars.saturating_sub(1))));
    result
}

fn now_ms() -> Result<i64, MemoryError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| MemoryError::Internal(format!("system clock is before UNIX epoch: {error}")))?;
    i64::try_from(elapsed.as_millis()).map_err(|_| MemoryError::Internal("current timestamp exceeds i64".to_string()))
}

fn summary_now_ms() -> Result<i64, ConversationMemoryError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ConversationMemoryError::InvalidRecord(format!("system clock is before UNIX epoch: {error}")))?;
    i64::try_from(elapsed.as_millis()).map_err(|_| ConversationMemoryError::InvalidRecord("current timestamp exceeds i64".to_string()))
}

#[cfg(test)]
mod tests {
    use rig_core::completion::Message;
    use rig_core::message::{ToolCall, ToolFunction};
    use serde_json::json;

    use super::*;

    #[test]
    fn converts_messages_to_stable_text_without_rig_blocks() {
        let messages = [Message::user("hello"), Message::assistant("done")];

        for original in messages {
            let (role, content, artifact_refs) = rig_to_stored_message(original.clone()).expect("convert to stable message");
            assert_eq!(content.get("schemaVersion").and_then(Value::as_u64), Some(2));
            assert!(content.get("role").is_none(), "project envelope must not use Rig's top-level role tag");
            assert!(artifact_refs.is_empty());
            let restored = stored_to_rig_message(ConversationMessage {
                message_id: "message-1".to_string(),
                conversation_id: "thread-1".to_string(),
                sequence: 1,
                turn_id: None,
                source_item_id: None,
                role,
                content,
                estimated_tokens: 1,
                artifact_refs: vec![],
                created_at: 1,
            })
            .expect("restore Rig message");
            assert_eq!(restored, original);
        }
    }

    #[test]
    fn rejects_pre_rebuild_conversation_message_formats() {
        let error = stored_to_rig_message(ConversationMessage {
            message_id: "legacy-message".to_string(),
            conversation_id: "thread-1".to_string(),
            sequence: 1,
            turn_id: None,
            source_item_id: None,
            role: ConversationRole::User,
            content: json!({"schemaVersion": 1, "message": {"blocks": []}}),
            estimated_tokens: 1,
            artifact_refs: vec![],
            created_at: 1,
        })
        .expect_err("旧 Conversation 格式不应保留兼容分支");
        assert!(matches!(error, ConversationMemoryError::InvalidRecord(_)));
    }

    #[test]
    fn tool_arguments_and_large_results_are_not_persisted_but_artifact_refs_are() {
        let tool_call = Message::from(ToolCall::new(
            "call-1".to_string(),
            ToolFunction::new("read_artifact".to_string(), json!({ "artifactId": "result", "revision": 7, "apiKey": "secret-argument" })),
        ));
        let (_, call_content, call_refs) = rig_to_stored_message(tool_call).unwrap();
        let (_, result_content, result_refs) = rig_to_stored_message(Message::tool_result(
            "call-1",
            json!({
                "artifactId": "result",
                "revision": 7,
                "status": "draft",
                "payload": "secret-result".repeat(2_000)
            })
            .to_string(),
        ))
        .unwrap();

        let persisted = format!("{call_content}{result_content}");
        assert!(!persisted.contains("secret-argument"));
        assert!(!persisted.contains("secret-result"));
        assert_eq!(call_refs, vec!["result@7"]);
        assert_eq!(result_refs, vec!["result@7"]);
    }

    #[test]
    fn rejects_system_prompt_persistence() {
        assert!(matches!(rig_to_stored_message(Message::system("factory-owned")), Err(MemoryError::Policy(_))));
    }
}
