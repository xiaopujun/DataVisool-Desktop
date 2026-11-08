/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! G2Plot 公共配置片段的严格解析与组件引用提取。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// 解析 Markdown 中成对包围的 G2Plot 配置片段。
///
/// 标签必须单独占一行，且格式为 `<g2plot-[a-z0-9-]+>`。片段内容必须是
/// JSON 对象；重复、嵌套、错配或未闭合标签都会返回错误。
pub(crate) fn parse_fragments(source: &str) -> Result<BTreeMap<String, Value>, String> {
    let mut fragments = BTreeMap::new();
    let mut open_fragment: Option<(String, Vec<&str>)> = None;

    for (line_index, line) in source.lines().enumerate() {
        let candidate = is_boundary_candidate(line);
        if !candidate {
            if let Some((_, lines)) = &mut open_fragment {
                lines.push(line);
            }
            continue;
        }

        let line_number = line_index + 1;
        let tag = parse_tag(line).ok_or_else(|| {
            format!("第 {line_number} 行的 G2Plot 边界标签格式错误或未单独占行：{line:?}")
        })?;

        match open_fragment.take() {
            None => open_fragment = Some((tag, Vec::new())),
            Some((open_tag, content_lines)) if open_tag == tag => {
                let json_text = content_lines.join("\n");
                let value = serde_json::from_str::<Value>(json_text.trim())
                    .map_err(|error| format!("片段 {tag} 的 JSON 无效：{error}"))?;
                let object = value
                    .as_object()
                    .ok_or_else(|| format!("片段 {tag} 必须是 JSON 对象"))?;
                let value = Value::Object(object.clone());
                if fragments.insert(tag.clone(), value).is_some() {
                    return Err(format!("片段 {tag} 被重复定义"));
                }
            }
            Some((open_tag, _)) => {
                return Err(format!(
                    "第 {line_number} 行的标签 {tag} 与未闭合片段 {open_tag} 不匹配"
                ));
            }
        }
    }

    if let Some((tag, _)) = open_fragment {
        return Err(format!("片段 {tag} 缺少配对边界标签"));
    }
    if fragments.is_empty() {
        return Err("未找到 G2Plot 公共配置片段".to_string());
    }
    Ok(fragments)
}

/// 从 `configurable.style` 的字符串值中提取完整的 G2Plot 标签。
///
/// 对象键、style 外的内容和非字符串值均不作为引用。以标签前缀开头但格式
/// 错误的字符串会报错，防止拼写错误被静默忽略；普通说明文字不作为引用。
pub(crate) fn collect_style_references(configurable: &Value) -> Result<BTreeSet<String>, String> {
    let style = configurable
        .get("style")
        .and_then(Value::as_object)
        .ok_or_else(|| "configurable.style 必须是 JSON 对象".to_string())?;
    let mut references = BTreeSet::new();
    collect_value_references(&Value::Object(style.clone()), &mut references)?;
    Ok(references)
}

fn collect_value_references(value: &Value, references: &mut BTreeSet<String>) -> Result<(), String> {
    match value {
        Value::String(text) => {
            if let Some(tag) = parse_tag(text) {
                references.insert(tag);
            } else if has_suspected_tag_prefix(text) {
                return Err(format!("G2Plot 标签必须完整匹配且单独占值：{text:?}"));
            }
        }
        Value::Object(object) => {
            for child in object.values() {
                collect_value_references(child, references)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_value_references(child, references)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn has_suspected_tag_prefix(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    lower.starts_with("<g2plot") || lower.starts_with("g2plot-")
}

fn is_boundary_candidate(line: &str) -> bool {
    let trimmed = line.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    (trimmed.starts_with('<') && lower.contains("g2plot")) || lower.starts_with("g2plot-")
}

fn parse_tag(text: &str) -> Option<String> {
    let inner = text.strip_prefix("<g2plot-")?.strip_suffix('>')?;
    if inner.is_empty()
        || !inner
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return None;
    }
    Some(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const COMMON_SOURCE: &str = include_str!("../../../../resources/ai/G2Plot公共配置分块说明.md");

    #[test]
    fn parses_the_repository_fragment_source() {
        let fragments = parse_fragments(COMMON_SOURCE).unwrap();
        assert!(fragments.contains_key("<g2plot-canvas>"));
        assert!(fragments.contains_key("<g2plot-axis-extended>"));
        assert!(fragments["<g2plot-canvas>"].get("padding").is_some());
    }

    #[test]
    fn rejects_invalid_fragment_boundaries_and_json() {
        for source in [
            "<g2plot-axis>\n{}\n<g2plot-other>",
            "<g2plot-axis>\n{}",
            "<g2plot-axis> suffix\n{}\n<g2plot-axis>",
            "<G2Plot-axis>\n{}\n<G2Plot-axis>",
            "<g2plot-axis>\n\n<g2plot-axis>",
            "<g2plot-axis>\n{oops}\n<g2plot-axis>",
            "<g2plot-axis>\n[]\n<g2plot-axis>",
            "<g2plot-axis>\n{\"a\":1}\n<g2plot-axis>\n<g2plot-axis>\n{\"b\":2}\n<g2plot-axis>",
        ] {
            assert!(parse_fragments(source).is_err(), "expected error for {source:?}");
        }
    }

    #[test]
    fn accepts_an_empty_json_object_fragment() {
        let fragments = parse_fragments("<g2plot-axis>\n{}\n<g2plot-axis>").unwrap();
        assert_eq!(fragments["<g2plot-axis>"], json!({}));
    }

    #[test]
    fn extracts_only_nested_style_string_values_and_deduplicates() {
        let configurable = json!({
            "description": "<g2plot-outside>",
            "style": {
                "axis": "<g2plot-axis>",
                "nested": ["<g2plot-axis>", {"label": "<g2plot-label>"}],
                "note": "说明见 <g2plot-axis>",
                "other": 3
            }
        });
        assert_eq!(
            collect_style_references(&configurable).unwrap(),
            BTreeSet::from(["<g2plot-axis>".to_string(), "<g2plot-label>".to_string()])
        );
    }

    #[test]
    fn rejects_malformed_or_partial_style_labels() {
        for label in [
            "<g2plot-axis> suffix",
            " <g2plot-axis>",
            "<G2Plot-axis>",
            "<g2plot-axis",
            "g2plot-axis",
            "<g2plot-Axis>",
        ] {
            let configurable = json!({"style": {"axis": label}});
            assert!(collect_style_references(&configurable).is_err(), "expected error for {label:?}");
        }
    }

    #[test]
    fn ignores_non_reference_style_descriptions() {
        let configurable = json!({
            "style": {
                "description": "说明见 <g2plot-axis>",
                "text": "G2Plot axis settings"
            }
        });
        assert!(collect_style_references(&configurable).unwrap().is_empty());
    }
}
