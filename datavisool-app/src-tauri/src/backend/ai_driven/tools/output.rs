/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 外部模型可见 Tool 结果的统一脱敏、裁剪与降级投影。

use serde_json::{json, Map, Value};

use super::error::AgentToolError;

const MAX_DEPTH: usize = 12;
const MAX_ARRAY_ITEMS: usize = 32;
const MAX_COMPONENT_ITEMS: usize = 100;
const MAX_COMPONENT_SEARCH_QUERY_ITEMS: usize = 50;
const MAX_STRING_CHARS: usize = 2_000;
pub(super) const CATALOG_OUTPUT_BUDGET_BYTES: usize = 6_000;
/// 完整组件规格专用硬上限；超过时必须失败，禁止走通用裁剪降级。
pub(super) const COMPONENT_SPEC_OUTPUT_BUDGET_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy)]
pub(super) enum ModelOutputBudget {
    Catalog,
    Snapshot,
    Workspace,
}

impl ModelOutputBudget {
    const fn bytes(self) -> usize {
        match self {
            Self::Catalog => CATALOG_OUTPUT_BUDGET_BYTES,
            Self::Snapshot | Self::Workspace => 16_000,
        }
    }
}

/// 结果过大时返回保留关键事实的投影，不再让整个 Tool 调用失败。
pub(super) fn fit_model_output(mut value: Value, budget: ModelOutputBudget, retry_hint: &str) -> Result<Value, AgentToolError> {
    let original_bytes = serialized_len(&value)?;
    let mut truncation_count = 0usize;
    sanitize_value(&mut value, None, 0, &mut truncation_count);
    if serialized_len(&value)? > budget.bytes() {
        compact_value(&mut value, 0, 12, 512, 6, &mut truncation_count);
    }
    if serialized_len(&value)? > budget.bytes() {
        compact_value(&mut value, 0, 4, 160, 4, &mut truncation_count);
    }
    if serialized_len(&value)? > budget.bytes() {
        value = fact_envelope(&value);
        truncation_count = truncation_count.saturating_add(1);
    }
    if truncation_count > 0 || original_bytes > budget.bytes() {
        attach_metadata(&mut value, original_bytes, truncation_count.max(1), retry_hint);
    }
    Ok(value)
}

pub(super) fn serialized_len(value: &Value) -> Result<usize, AgentToolError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| AgentToolError::internal("agent.tool.serialize_failed", error.to_string()))
}

fn sanitize_value(value: &mut Value, parent_key: Option<&str>, depth: usize, truncation_count: &mut usize) {
    if depth >= MAX_DEPTH {
        *value = Value::String("[truncated: depth limit]".to_string());
        *truncation_count = truncation_count.saturating_add(1);
        return;
    }
    if parent_key.is_some_and(is_sensitive_key) {
        *value = Value::String("[redacted]".to_string());
        return;
    }
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                sanitize_value(child, Some(key), depth + 1, truncation_count);
            }
        }
        Value::Array(items) => {
            let limit = match parent_key {
                Some("components") => MAX_COMPONENT_ITEMS,
                Some("matchedQueries" | "unmatchedQueries") => MAX_COMPONENT_SEARCH_QUERY_ITEMS,
                _ => MAX_ARRAY_ITEMS,
            };
            if items.len() > limit {
                items.truncate(limit);
                *truncation_count = truncation_count.saturating_add(1);
            }
            for child in items {
                sanitize_value(child, parent_key, depth + 1, truncation_count);
            }
        }
        Value::String(text) => {
            if parent_key.is_some_and(is_url_key) || text.contains("://") {
                *text = redact_url_credentials(text);
            }
            if text.chars().count() > MAX_STRING_CHARS {
                *text = truncate_chars(text, MAX_STRING_CHARS);
                *truncation_count = truncation_count.saturating_add(1);
            }
        }
        _ => {}
    }
}

fn compact_value(value: &mut Value, depth: usize, array_limit: usize, string_limit: usize, depth_limit: usize, truncation_count: &mut usize) {
    if depth >= depth_limit {
        if value.is_array() || value.is_object() {
            *value = Value::String("[truncated: projection depth]".to_string());
            *truncation_count = truncation_count.saturating_add(1);
        }
        return;
    }
    match value {
        Value::Object(object) => {
            for child in object.values_mut() {
                compact_value(child, depth + 1, array_limit, string_limit, depth_limit, truncation_count);
            }
        }
        Value::Array(items) => {
            if items.len() > array_limit {
                items.truncate(array_limit);
                *truncation_count = truncation_count.saturating_add(1);
            }
            for child in items {
                compact_value(child, depth + 1, array_limit, string_limit, depth_limit, truncation_count);
            }
        }
        Value::String(text) if text.chars().count() > string_limit => {
            *text = truncate_chars(text, string_limit);
            *truncation_count = truncation_count.saturating_add(1);
        }
        _ => {}
    }
}

fn fact_envelope(value: &Value) -> Value {
    let mut facts = Map::new();
    collect_facts(value, &mut facts, 0);
    json!({ "facts": facts, "content": "[truncated: use a narrower query to fetch details]" })
}

fn collect_facts(value: &Value, facts: &mut Map<String, Value>, depth: usize) {
    if depth > 8 || facts.len() >= 16 {
        return;
    }
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if is_fact_key(key) && !child.is_object() && !child.is_array() {
                    facts.entry(key.clone()).or_insert_with(|| compact_fact(child));
                }
                collect_facts(child, facts, depth + 1);
            }
        }
        Value::Array(items) => {
            for child in items.iter().take(8) {
                collect_facts(child, facts, depth + 1);
            }
        }
        _ => {}
    }
}

fn compact_fact(value: &Value) -> Value {
    match value {
        Value::String(text) if text.chars().count() > 160 => Value::String(truncate_chars(text, 160)),
        _ => value.clone(),
    }
}

fn attach_metadata(value: &mut Value, original_bytes: usize, truncation_count: usize, retry_hint: &str) {
    let metadata = [
        ("truncated", Value::Bool(true)),
        ("truncationCount", Value::from(truncation_count)),
        ("originalBytes", Value::from(original_bytes)),
        ("retryHint", Value::String(retry_hint.to_string())),
    ];
    if let Some(object) = value.as_object_mut() {
        object.extend(metadata.into_iter().map(|(key, value)| (key.to_string(), value)));
    } else {
        let content = std::mem::take(value);
        *value = json!({
            "content": content,
            "truncated": true,
            "truncationCount": truncation_count,
            "originalBytes": original_bytes,
            "retryHint": retry_hint
        });
    }
}

fn is_fact_key(key: &str) -> bool {
    matches!(
        key,
        "status"
            | "code"
            | "message"
            | "operationId"
            | "revision"
            | "resultRevision"
            | "workspaceRevision"
            | "catalogVersion"
            | "total"
            | "totalEntries"
            | "totalMatches"
            | "offset"
            | "nextOffset"
            | "hasMore"
            | "componentId"
            | "componentType"
            | "path"
    )
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase().replace(['-', '_'], "");
    key == "headers"
        || key == "key"
        || key.ends_with("key")
        || key.contains("password")
        || key.contains("passwd")
        || key.contains("token")
        || key.contains("secret")
        || key.contains("authorization")
        || key.contains("cookie")
        || key.contains("credential")
        || key.contains("base64")
}

fn is_url_key(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase().replace(['-', '_'], "");
    key.ends_with("url") || key.ends_with("uri") || key.ends_with("endpoint")
}

fn redact_url_credentials(value: &str) -> String {
    let mut value = value.to_string();
    if let Some(scheme_end) = value.find("://") {
        let authority_start = scheme_end + 3;
        let authority_end = value[authority_start..]
            .find(['/', '?', '#'])
            .map_or(value.len(), |offset| authority_start + offset);
        if let Some(at) = value[authority_start..authority_end].rfind('@') {
            value.replace_range(authority_start..authority_start + at + 1, "[redacted]@");
        }
    }
    let (base, fragment) = value
        .split_once('#')
        .map_or((value.as_str(), None), |(base, fragment)| (base, Some(fragment)));
    let base = sanitize_url_part(base);
    fragment.map_or(base.clone(), |fragment| format!("{base}#{}", sanitize_url_part(fragment)))
}

fn sanitize_url_part(value: &str) -> String {
    let Some((base, query)) = value.split_once('?') else {
        return value.to_string();
    };
    let query = query
        .split('&')
        .map(|part| {
            let (key, raw_value) = part.split_once('=').unwrap_or((part, ""));
            if is_sensitive_key(key) {
                format!("{key}=[redacted]")
            } else if raw_value.is_empty() {
                key.to_string()
            } else {
                format!("{key}={raw_value}")
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{query}")
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secrets_and_returns_bounded_projection() {
        let value = json!({
            "operationId": "op-1",
            "components": (0..80).map(|index| json!({
                "componentId": format!("c-{index}"),
                "apiKey": "secret-key",
                "apiUrl": "https://user:password@example.com/data?token=secret",
                "note": "x".repeat(2_500)
            })).collect::<Vec<_>>()
        });

        let output = fit_model_output(value, ModelOutputBudget::Catalog, "减少组件数量").unwrap();
        let encoded = output.to_string();
        assert!(encoded.len() <= 6_500);
        assert!(encoded.contains("op-1"));
        assert!(output["truncated"].as_bool().unwrap_or(false));
        assert!(!encoded.contains("secret-key"));
        assert!(!encoded.contains("user:password"));
        assert!(!encoded.contains("token=secret"));
    }

    #[test]
    fn keeps_up_to_fifty_component_search_queries() {
        let queries = (1..=50).map(|index| format!("q{index}")).collect::<Vec<_>>();
        let value = json!({
            "components": [{
                "componentType": "BaseText",
                "componentName": "基础文本",
                "matchedQueries": queries
            }],
            "unmatchedQueries": (1..=50).map(|index| format!("missing{index}")).collect::<Vec<_>>()
        });

        let output = fit_model_output(value, ModelOutputBudget::Catalog, "减少关键词").unwrap();

        assert_eq!(output["components"][0]["matchedQueries"].as_array().unwrap().len(), 50);
        assert_eq!(output["unmatchedQueries"].as_array().unwrap().len(), 50);
        assert!(output.get("truncated").is_none());
    }

    #[test]
    fn keeps_one_hundred_component_catalog_entries() {
        let value = json!({
            "catalogVersion": "component-catalog/v3:100",
            "components": (1..=100)
                .map(|index| json!({ "componentType": format!("Component{index}"), "componentName": format!("组件{index}") }))
                .collect::<Vec<_>>(),
        });

        let output = fit_model_output(value, ModelOutputBudget::Catalog, "组件目录已截断").unwrap();

        assert_eq!(output["components"].as_array().map(Vec::len), Some(100));
        assert!(output.get("truncated").is_none());
    }
}
