/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 编译期嵌入的组件 AI 资料库；这是组件优化说明的唯一运行时来源。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::backend::ai_driven::domain::validation_data_contract::is_component_data_field_mapping_key;

include!(concat!(env!("OUT_DIR"), "/component_optimization_docs.rs"));

pub(crate) mod full_spec;
mod g2plot_fragments;
mod search;

pub(crate) use search::{ComponentDocumentSearchError, ComponentDocumentSearchErrorKind};

static COMPONENT_DOCUMENTS: OnceLock<HashMap<&'static str, ComponentDocument>> = OnceLock::new();
static G2PLOT_COMMON_FRAGMENTS: OnceLock<Result<BTreeMap<String, Value>, String>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ComponentDocumentSection {
    ComponentInfo,
    DataFormatRequirements,
    ConfigurableItems,
    ValidationRules,
    Notes,
}

impl ComponentDocumentSection {
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::ComponentInfo => "componentInfo",
            Self::DataFormatRequirements => "dataFormatRequirements",
            Self::ConfigurableItems => "configurableItems",
            Self::ValidationRules => "validationRules",
            Self::Notes => "notes",
        }
    }

    const fn heading(self) -> &'static str {
        match self {
            Self::ComponentInfo => "组件信息",
            Self::DataFormatRequirements => "数据格式要求",
            Self::ConfigurableItems => "可配置项",
            Self::ValidationRules => "校验规则",
            Self::Notes => "注意事项",
        }
    }

    const ALL: [Self; 5] = [Self::ComponentInfo, Self::DataFormatRequirements, Self::ConfigurableItems, Self::ValidationRules, Self::Notes];
}

#[derive(Debug)]
struct ComponentDocument {
    component_type: &'static str,
    content: &'static str,
    sections: HashMap<ComponentDocumentSection, String>,
    supports_data_input: bool,
}

pub(crate) fn has_component_document(component_type: &str) -> bool {
    documents().contains_key(component_type)
}

pub(crate) fn component_docs_corpus_version() -> &'static str {
    COMPONENT_DOCS_CORPUS_VERSION
}

/// Returns the full JSON objects referenced by the selected component configurables.
///
/// Fragment parsing is cached for the process lifetime; references are collected from
/// `configurable.style` only, deduplicated, and emitted in stable label order.
pub(crate) fn common_fragments_for_configurables<'a>(
    configurables: impl IntoIterator<Item = &'a Value>,
) -> Result<BTreeMap<String, Value>, String> {
    let fragments = G2PLOT_COMMON_FRAGMENTS
        .get_or_init(|| g2plot_fragments::parse_fragments(EMBEDDED_G2PLOT_COMMON_FRAGMENTS))
        .as_ref()
        .map_err(Clone::clone)?;
    let mut references = BTreeSet::new();
    for configurable in configurables {
        references.extend(g2plot_fragments::collect_style_references(configurable)?);
    }
    references
        .into_iter()
        .map(|label| {
            fragments
                .get(&label)
                .cloned()
                .map(|fragment| (label.clone(), fragment))
                .ok_or_else(|| format!("未找到公共配置片段：{label}"))
        })
        .collect()
}

pub(crate) fn component_data_requirements(component_type: &str) -> Option<(&'static str, bool)> {
    let doc = documents().get(component_type.trim())?;
    let requirements = doc.sections.get(&ComponentDocumentSection::DataFormatRequirements)?.as_str();
    Some((requirements, doc.supports_data_input))
}

pub(crate) fn component_documents_response<'a>(component_types: impl IntoIterator<Item = &'a str>) -> Value {
    let mut docs = Vec::new();
    let mut missing_component_types = Vec::new();
    let mut seen = HashSet::new();
    for component_type in component_types {
        let component_type = component_type.trim();
        if component_type.is_empty() || !seen.insert(component_type.to_string()) {
            continue;
        }
        match documents().get(component_type) {
            Some(doc) => docs.push(json!({
                "componentType": doc.component_type,
                "sourcePath": format!("component-optimization/{}.md", doc.component_type),
                "content": doc.content,
                "dataFormatRequirements": doc.sections[&ComponentDocumentSection::DataFormatRequirements],
                "supportsDataInput": doc.supports_data_input,
            })),
            None => missing_component_types.push(component_type.to_string()),
        }
    }
    json!({ "docs": docs, "missingComponentTypes": missing_component_types })
}

fn documents() -> &'static HashMap<&'static str, ComponentDocument> {
    COMPONENT_DOCUMENTS.get_or_init(|| {
        EMBEDDED_COMPONENT_DOCS
            .iter()
            .map(|&(component_type, content, _, _)| {
                let sections = ComponentDocumentSection::ALL
                    .into_iter()
                    .map(|section| {
                        let content = extract_level_two_section(content, section.heading())
                            .unwrap_or_else(|| panic!("组件 {component_type} 缺少“{}”章节", section.heading()));
                        (section, content)
                    })
                    .collect::<HashMap<_, _>>();
                let data_requirements = &sections[&ComponentDocumentSection::DataFormatRequirements];
                let supports_data_input = reads_data_patch_support(data_requirements);
                (component_type, ComponentDocument { component_type, content, sections, supports_data_input })
            })
            .collect()
    })
}

fn component_document_location(component_type: &str, physical_line: u64) -> Option<(ComponentDocumentSection, usize)> {
    let doc = documents().get(component_type)?;
    let target = usize::try_from(physical_line).ok()?.checked_sub(1)?;
    let lines = doc.content.lines().collect::<Vec<_>>();
    let mut headings = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if let Some(section) = ComponentDocumentSection::ALL
            .into_iter()
            .find(|section| line.trim() == format!("## {}", section.heading()))
        {
            headings.push((index, section));
        }
    }
    for (heading_index, (heading_line, section)) in headings.iter().copied().enumerate() {
        let next_heading = headings.get(heading_index + 1).map(|(line, _)| *line).unwrap_or(lines.len());
        if target < heading_line || target >= next_heading {
            continue;
        }
        let mut start = heading_line.saturating_add(1);
        while start < next_heading && lines[start].trim().is_empty() {
            start += 1;
        }
        let mut end = next_heading;
        while end > start && lines[end - 1].trim().is_empty() {
            end -= 1;
        }
        let total_lines = end.saturating_sub(start).max(1);
        let section_line = if target <= start {
            1
        } else {
            target.saturating_sub(start).saturating_add(1).min(total_lines)
        };
        return Some((section, section_line));
    }
    // 标题前的文档元信息归入首个组件信息章节，保证所有合法命中都有可读取上下文。
    (target < headings.first()?.0).then_some((ComponentDocumentSection::ComponentInfo, 1))
}

fn extract_level_two_section(content: &str, heading: &str) -> Option<String> {
    let marker = format!("## {heading}");
    let mut collecting = false;
    let mut lines = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == marker {
            collecting = true;
            continue;
        }
        if collecting && trimmed.starts_with("## ") {
            break;
        }
        if collecting {
            lines.push(line);
        }
    }
    let content = lines.join("\n").trim().to_string();
    (!content.is_empty()).then_some(content)
}

fn reads_data_patch_support(requirements: &str) -> bool {
    if requirements.contains("该组件不支持通过 `dataPatch` 修改数据。") {
        return false;
    }
    requirements.contains("该组件支持通过 `dataPatch` 修改数据。")
}

fn first_json_block(content: &str) -> Option<Value> {
    json_blocks(content).into_iter().next()
}

fn json_blocks(content: &str) -> Vec<Value> {
    content
        .split("```json")
        .skip(1)
        .filter_map(|rest| rest.split_once("```").map(|(json, _)| json.trim()))
        .filter_map(|json| serde_json::from_str(json).ok())
        .collect()
}

fn required_mapping_fields(requirements: &str) -> Vec<String> {
    let mut fields = requirements
        .match_indices("style.")
        .filter_map(|(index, _)| {
            let tail = &requirements[index + "style.".len()..];
            let field = tail
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect::<String>();
            is_component_data_field_mapping_key(&field).then_some(field)
        })
        .collect::<Vec<_>>();
    fields.sort();
    fields.dedup();
    fields
}

fn add_mapping_examples(style: &mut Map<String, Value>, fields: &[String], data: Option<&Value>) {
    let keys = data
        .and_then(|data| data.get("staticData"))
        .and_then(|data| data.get("value"))
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(Value::as_object)
        .map(|row| row.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    for (index, field) in fields
        .iter()
        .filter(|field| is_component_data_field_mapping_key(field))
        .enumerate()
    {
        let fallback = if field == "yField" || field == "weightField" {
            keys.get(1).or_else(|| keys.first())
        } else {
            keys.get(index).or_else(|| keys.first())
        };
        style
            .entry(field.clone())
            .or_insert_with(|| Value::String(fallback.cloned().unwrap_or_else(|| "field".to_string())));
    }
}

fn minimum_static_data_example(requirements: &str) -> Option<Value> {
    for marker in ["静态数据示例：`", "最小数据示例：`"] {
        let Some((_, tail)) = requirements.split_once(marker) else { continue };
        let json = tail.split_once('`')?.0;
        let value = serde_json::from_str::<Value>(json).ok()?;
        if value.get("sourceType").is_some() {
            return Some(value);
        }
        return Some(json!({ "sourceType": "static", "staticData": { "value": value } }));
    }
    let value = if requirements.contains("对象数组") {
        json!([{ "category": "示例", "value": 1 }])
    } else {
        json!(1)
    };
    Some(json!({ "sourceType": "static", "staticData": { "value": value } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_component_named_documents_as_the_only_store() {
        assert_eq!(documents().len(), 48);
        assert!(has_component_document("AntdBaseLine"));
        assert!(has_component_document("BaseText"));
        assert!(has_component_document("DateTimePicker"));
        assert!(has_component_document("ScreenReference"));
        assert!(!has_component_document("Unknown"));
    }

    #[test]
    fn evidence_response_keeps_full_trusted_document() {
        let response = component_documents_response(["BaseText", "Unknown"]);
        assert_eq!(response["docs"].as_array().unwrap().len(), 1);
        assert_eq!(response["missingComponentTypes"], json!(["Unknown"]));
        assert!(response["docs"][0]["content"]
            .as_str()
            .unwrap()
            .contains("component-style-doc/v1"));
    }

    #[test]
    fn missing_or_malformed_common_fragment_reference_fails() {
        let missing = json!({ "style": { "axis": "<g2plot-this-fragment-does-not-exist>" } });
        let error = common_fragments_for_configurables([&missing]).unwrap_err();
        assert!(error.contains("<g2plot-this-fragment-does-not-exist>"));

        let malformed = json!({ "style": { "axis": "<g2plot-axis> suffix" } });
        let error = common_fragments_for_configurables([&malformed]).unwrap_err();
        assert!(error.contains("标签必须完整匹配"));
    }

    #[test]
    fn every_document_follows_the_component_style_protocol() {
        let expected_headings = ["组件信息", "数据格式要求", "可配置项", "校验规则", "注意事项"];
        for &(component_type, markdown, _, _) in EMBEDDED_COMPONENT_DOCS {
            let headings = markdown
                .lines()
                .filter_map(|line| line.trim().strip_prefix("## "))
                .collect::<Vec<_>>();
            assert_eq!(headings, expected_headings, "{component_type}: 二级章节不符合协议");

            let configurable = extract_level_two_section(markdown, "可配置项").unwrap();
            let configurable_schema = json_blocks(&configurable).into_iter().next().unwrap();
            let configurable_object = configurable_schema.as_object().unwrap();
            assert_eq!(configurable_object.keys().collect::<Vec<_>>(), vec!["style"]);
            assert!(!configurable_object["style"].as_object().unwrap().is_empty());

            let blocks = json_blocks(markdown);
            let style_schema = blocks
                .iter()
                .find(|block| block["schemaVersion"] == "component-style-doc/v1")
                .unwrap_or_else(|| panic!("{component_type}: 缺少 component-style-doc/v1"));
            let allowed_paths = style_schema["allowedStylePaths"].as_array().unwrap();
            assert!(!allowed_paths.is_empty(), "{component_type}: allowedStylePaths 不能为空");
            assert!(allowed_paths
                .iter()
                .all(|entry| entry["path"].as_str().is_some_and(|path| path.starts_with("style."))));
            for block in blocks.iter().filter(|block| block.get("stylePatch").is_some()) {
                let style_patch = block["stylePatch"].as_object().unwrap();
                assert_eq!(style_patch.keys().collect::<Vec<_>>(), vec!["style"]);
                assert!(!style_patch["style"].as_object().unwrap().is_empty());
            }

            let requirements = extract_level_two_section(markdown, "数据格式要求").unwrap();
            let support_count = requirements.matches("该组件支持通过 `dataPatch` 修改数据。").count()
                + requirements.matches("该组件不支持通过 `dataPatch` 修改数据。").count();
            assert_eq!(support_count, 1, "{component_type}: 数据能力必须明确声明一次");

            let component_info = extract_level_two_section(markdown, "组件信息").unwrap();
            let component_info = json_blocks(&component_info).into_iter().next().unwrap();
            assert_eq!(component_info["componentType"], component_type);
            assert!(component_info["componentName"]
                .as_str()
                .is_some_and(|value| !value.trim().is_empty()));
            assert!(component_info["description"]
                .as_str()
                .is_some_and(|value| !value.trim().is_empty()));
            assert!(component_info["nativeCapabilities"]
                .as_str()
                .is_some_and(|value| !value.trim().is_empty() && value.chars().count() <= 120));

            if component_type.starts_with("Antd") && !matches!(component_type, "AntdGauge" | "AntdLiquid" | "AntdRingProgress") {
                assert!(requirements.contains("属性名不固定") || requirements.contains("字段名不固定"));
                assert!(
                    requirements.contains("style.xField") || requirements.contains("style.angleField") || requirements.contains("style.wordField")
                );
                assert!(requirements.contains("映射"));
                assert!(requirements.contains("保持一致") || requirements.contains("必须与"));
            }
        }
    }

    fn json_blocks(markdown: &str) -> Vec<Value> {
        markdown
            .split("```json")
            .skip(1)
            .filter_map(|rest| rest.split_once("```").map(|(json, _)| json.trim()))
            .map(|json| serde_json::from_str(json).unwrap_or_else(|error| panic!("无效 JSON 代码块：{error}")))
            .collect()
    }
}
