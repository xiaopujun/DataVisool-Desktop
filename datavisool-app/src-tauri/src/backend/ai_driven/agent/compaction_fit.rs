/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use super::*;

/// 将摘要后的历史收缩到可发送窗口；保持用户要求、摘要和完整 Tool 配对。
/// 无法容纳必要内容时返回 None，让调用方停止而非发送已知超容量请求。
pub(super) fn fit_compacted_history(
    prompt: &Message,
    compacted: Vec<Message>,
    source: &[Message],
    plan: &AgentContextPlan,
    tool_schema_bytes: &HashMap<String, usize>,
    threshold_tokens: u64,
    max_tool_result_tokens: u64,
    max_tool_history_tokens: u64,
) -> Option<Vec<Message>> {
    if estimate_request_tokens(prompt, &compacted, plan, tool_schema_bytes) <= threshold_tokens && prompt_pair_is_present(&compacted, prompt) {
        return Some(compacted);
    }

    let requirements = preserved_user_requirements(source);
    let notices = preserved_runtime_notices(&compacted);
    let summary = compacted
        .iter()
        .filter(|message| is_compaction_message(message))
        .cloned()
        .collect::<Vec<_>>();
    let recent_start = recent_complete_suffix_start_with_prompt(&compacted, prompt);
    // Keep the bounded recovery journal alongside the summary. It contains
    // operation IDs and pending references that the summary provider may omit;
    // dropping it here would make later compactions lose already-applied work.
    let essential = essential_tool_history(&compacted[..recent_start]);
    let recent = compacted[recent_start..].to_vec();
    let protected_prefix_len = requirements.len() + notices.len() + summary.len() + essential.len();
    let mut candidate = requirements.clone();
    candidate.extend(notices);
    candidate.extend(summary);
    candidate.extend(essential);
    candidate.extend(recent);
    candidate = project_tool_history_with_budget(&candidate, Some(prompt), max_tool_result_tokens, max_tool_history_tokens.min(threshold_tokens)).0;
    if estimate_request_tokens(prompt, &candidate, plan, tool_schema_bytes) <= threshold_tokens && prompt_pair_is_present(&candidate, prompt) {
        return Some(candidate);
    }

    // Drop the oldest complete suffix group while retaining the summary and
    // original requirements. `drop` is allowed only when both call/result sets
    // remain equal, so a multi-call assistant message is never split.
    let prefix_len = protected_prefix_len.min(candidate.len());
    let prefix = candidate[..prefix_len.min(candidate.len())].to_vec();
    let suffix = candidate[prefix.len()..].to_vec();
    for drop in 1..=suffix.len() {
        let tail = suffix[drop..].to_vec();
        if !tool_pairs_are_closed(&tail, Some(prompt)) {
            continue;
        }
        let mut trimmed = prefix.clone();
        trimmed.extend(tail);
        let trimmed =
            project_tool_history_with_budget(&trimmed, Some(prompt), max_tool_result_tokens, max_tool_history_tokens.min(threshold_tokens)).0;
        if estimate_request_tokens(prompt, &trimmed, plan, tool_schema_bytes) <= threshold_tokens && prompt_pair_is_present(&trimmed, prompt) {
            return Some(trimmed);
        }
    }

    // A single oversized result can still occupy the whole recent suffix. Keep
    // only requirements and the compaction summary; the digest retains the
    // operation and component facts needed for a subsequent re-read.
    let minimal = project_tool_history_with_budget(&prefix, Some(prompt), max_tool_result_tokens, max_tool_history_tokens.min(threshold_tokens)).0;
    (estimate_request_tokens(prompt, &minimal, plan, tool_schema_bytes) <= threshold_tokens && prompt_pair_is_present(&minimal, prompt))
        .then_some(minimal)
}

fn is_compaction_message(message: &Message) -> bool {
    matches!(
        message,
        Message::User { content }
            if content.iter().any(|item| matches!(item, UserContent::Text(text) if text.text.starts_with("[Agent context summary]")))
    )
}

fn recent_complete_suffix_start_with_prompt(history: &[Message], prompt: &Message) -> usize {
    let mut start = history.len().saturating_sub(RECENT_MESSAGE_COUNT);
    while start > 0 {
        if tool_pairs_are_closed(&history[start..], Some(prompt)) {
            break;
        }
        start -= 1;
    }
    start
}

fn tool_pairs_are_closed(history: &[Message], prompt: Option<&Message>) -> bool {
    let calls = tool_call_ids(history).into_iter().map(str::to_string).collect::<HashSet<_>>();
    let mut results = tool_result_ids(history).into_iter().map(str::to_string).collect::<HashSet<_>>();
    if let Some(prompt) = prompt {
        results.extend(tool_result_ids(std::slice::from_ref(prompt)).into_iter().map(str::to_string));
    }
    calls == results
}

fn prompt_pair_is_present(history: &[Message], prompt: &Message) -> bool {
    current_prompt_tool_result_id(prompt).map_or(true, |id| tool_call_ids(history).contains(id))
}

pub(super) fn fit_summary_history(
    history: &[Message],
    prompt: &Message,
    context_capacity_tokens: u64,
    summary_output_tokens: u64,
    max_tool_result_tokens: u64,
    max_tool_history_tokens: u64,
) -> Option<Vec<Message>> {
    let input_budget = context_capacity_tokens.saturating_sub(summary_output_tokens);
    let recent_start = recent_complete_suffix_start_with_prompt(history, prompt);
    let mut candidate = preserved_user_requirements(&history[..recent_start]);
    candidate.extend(essential_tool_history(&history[..recent_start]));
    candidate.extend_from_slice(&history[recent_start..]);
    candidate.push(prompt.clone());
    candidate = project_tool_history_with_budget(&candidate, None, max_tool_result_tokens, max_tool_history_tokens.min(input_budget)).0;
    if summary_request_tokens(&candidate, summary_output_tokens) <= context_capacity_tokens {
        return Some(candidate);
    }

    // Old prose has no replay value after the summary request has been bounded;
    // keep requirements, Tool calls/results, and the most recent complete suffix.
    let required = preserved_user_requirements(history);
    let mut relevant = required.clone();
    relevant.extend(candidate.into_iter().filter(|message| match message {
        Message::Assistant { content, .. } => content.iter().any(|item| matches!(item, AssistantContent::ToolCall(_))),
        Message::User { content } => content.iter().any(|item| matches!(item, UserContent::ToolResult(_))),
        Message::System { .. } => false,
    }));
    relevant = project_tool_history_with_budget(&relevant, None, max_tool_result_tokens, max_tool_history_tokens.min(input_budget)).0;
    if summary_request_tokens(&relevant, summary_output_tokens) <= context_capacity_tokens {
        return Some(relevant);
    }

    let prefix_len = required.len().min(relevant.len());
    let prefix = relevant[..prefix_len].to_vec();
    let suffix = relevant[prefix_len..].to_vec();
    for drop in 1..=suffix.len() {
        let tail = suffix[drop..].to_vec();
        if !tool_pairs_are_closed(&tail, None) {
            continue;
        }
        let mut trimmed = prefix.clone();
        trimmed.extend(tail);
        let trimmed = project_tool_history_with_budget(&trimmed, None, max_tool_result_tokens, max_tool_history_tokens.min(input_budget)).0;
        if summary_request_tokens(&trimmed, summary_output_tokens) <= context_capacity_tokens {
            return Some(trimmed);
        }
    }
    None
}

pub(super) fn summary_request_tokens(history: &[Message], max_output_tokens: u64) -> u64 {
    let request_bytes = serde_json::to_vec(&(history, Message::user(COMPACTION_PROMPT))).map_or(0, |bytes| bytes.len());
    let output_bytes = token_bytes(max_output_tokens);
    ((request_bytes.saturating_add(output_bytes)) as u64).saturating_add(3) / 4
}
