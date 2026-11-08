/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 面向 Agent 的完整组件规格组装。
//!
//! Markdown 仍是唯一权威来源；这里仅把完整配置、数据要求、组件特有限制
//! 和最小提交示例整理为稳定的机器可读契约。

use serde::Serialize;
use serde_json::{json, Map, Value};

use super::{
    add_mapping_examples, documents, first_json_block, json_blocks, minimum_static_data_example, required_mapping_fields, ComponentDocument,
    ComponentDocumentSection,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FullComponentSpec {
    component_type: &'static str,
    configurable: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_requirements: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    constraints: Vec<String>,
    example: Value,
}

impl FullComponentSpec {
    pub(crate) fn configurable(&self) -> &Value {
        &self.configurable
    }
}

pub(crate) fn full_component_spec(component_type: &str, intent: &str) -> Option<FullComponentSpec> {
    let doc = documents().get(component_type.trim())?;
    let configurable_section = doc.sections.get(&ComponentDocumentSection::ConfigurableItems)?;
    let mut configurable = first_json_block(configurable_section)?;
    let requirements = doc.sections.get(&ComponentDocumentSection::DataFormatRequirements)?;
    let mapping_fields = required_mapping_fields(requirements);
    add_mapping_schema(&mut configurable, &mapping_fields)?;

    Some(FullComponentSpec {
        component_type: doc.component_type,
        configurable,
        data_requirements: doc.supports_data_input.then(|| requirements.clone()),
        constraints: component_constraints(doc, configurable_section),
        example: minimum_example(doc, intent, requirements, &mapping_fields)?,
    })
}

#[cfg(test)]
pub(crate) fn full_component_spec_types() -> Vec<&'static str> {
    let mut types = documents().keys().copied().collect::<Vec<_>>();
    types.sort_unstable();
    types
}

fn add_mapping_schema(configurable: &mut Value, mapping_fields: &[String]) -> Option<()> {
    let style = configurable.get_mut("style")?.as_object_mut()?;
    for field in mapping_fields {
        style
            .entry(field.clone())
            .or_insert_with(|| json!("string，必须与实际数据记录的属性名一致"));
    }
    Some(())
}

fn minimum_example(doc: &ComponentDocument, intent: &str, requirements: &str, mapping_fields: &[String]) -> Option<Value> {
    let mut style = Map::new();
    let data = doc
        .supports_data_input
        .then(|| minimum_static_data_example(requirements))
        .flatten();
    if doc.supports_data_input {
        add_mapping_examples(&mut style, mapping_fields, data.as_ref());
    } else if let Some(example_style) = minimum_documented_style(doc) {
        style = example_style;
    }

    let mut config = Map::new();
    if let Some(data) = data.clone() {
        config.insert("data".to_string(), data);
    }
    if !style.is_empty() {
        config.insert("style".to_string(), Value::Object(style));
    }

    match intent {
        "create" => Some(json!({ "config": config })),
        "patch" => {
            let mut patch = Map::new();
            if let Some(style) = config.remove("style") {
                patch.insert("stylePatch".to_string(), json!({ "style": style }));
            }
            if let Some(data) = config.remove("data") {
                patch.insert("dataPatch".to_string(), data);
            }
            Some(Value::Object(patch))
        }
        _ => None,
    }
}

fn minimum_documented_style(doc: &ComponentDocument) -> Option<Map<String, Value>> {
    doc.sections
        .get(&ComponentDocumentSection::Notes)
        .into_iter()
        .flat_map(|notes| json_blocks(notes))
        .filter_map(|value| {
            value
                .get("stylePatch")
                .and_then(|patch| patch.get("style"))
                .or_else(|| value.get("config").and_then(|config| config.get("style")))
                .and_then(Value::as_object)
                .cloned()
        })
        .min_by_key(|style| serde_json::to_vec(style).map_or(usize::MAX, |bytes| bytes.len()))
}

fn component_constraints(doc: &ComponentDocument, configurable: &str) -> Vec<String> {
    let mut constraints = configurable_prose(configurable);
    if let Some(notes) = doc.sections.get(&ComponentDocumentSection::Notes) {
        constraints.extend(note_sections(notes));
    }
    constraints.retain(|constraint| !constraint.trim().is_empty());
    constraints
}

fn configurable_prose(section: &str) -> Vec<String> {
    let mut prose = Vec::new();
    let mut outside = true;
    let mut current = String::new();
    for line in section.lines() {
        if line.trim_start().starts_with("```json") {
            outside = false;
            continue;
        }
        if !outside && line.trim() == "```" {
            outside = true;
            continue;
        }
        if outside {
            current.push_str(line);
            current.push('\n');
        }
    }
    for sentence in current.trim().split('。').map(str::trim) {
        if sentence.contains("false")
            || sentence.contains("null")
            || sentence.contains("只能")
            || sentence.contains("不得")
            || sentence.contains("不包含")
        {
            prose.push(format!("{sentence}。"));
        }
    }
    prose
}

fn note_sections(notes: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut title: Option<&str> = None;
    let mut body = Vec::new();
    let flush = |sections: &mut Vec<String>, title: Option<&str>, body: &mut Vec<&str>| {
        let Some(title) = title else {
            body.clear();
            return;
        };
        if title != "通用限制" && title != "参考依据" && !title.contains("示例") {
            let text = body.join("\n").trim().to_string();
            if !text.is_empty() {
                sections.push(format!("{title}：\n{text}"));
            }
        }
        body.clear();
    };
    for line in notes.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("**") && trimmed.ends_with("**") && trimmed.len() > 4 {
            flush(&mut sections, title, &mut body);
            title = Some(trimmed.trim_matches('*').trim());
        } else if title.is_some() && !trimmed.starts_with("```json") && trimmed != "```" {
            body.push(line);
        }
    }
    flush(&mut sections, title, &mut body);
    sections
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::component_docs::component_documents_response;
    use crate::backend::ai_driven::domain::validation_data_contract::is_component_data_field_mapping_key;
    use crate::backend::ai_driven::domain::validation_evidence::AgentValidationEvidence;

    #[test]
    fn every_document_builds_create_and_patch_specs() {
        for component_type in documents().keys() {
            for intent in ["create", "patch"] {
                let spec = full_component_spec(component_type, intent).unwrap_or_else(|| panic!("{component_type} {intent} 完整规格不可生成"));
                assert!(spec.configurable["style"].as_object().is_some_and(|style| !style.is_empty()));
                assert!(spec.example.as_object().is_some_and(|example| !example.is_empty()));
                assert_eq!(spec.data_requirements.is_some(), documents()[component_type].supports_data_input);
            }
        }
    }

    #[test]
    fn area_keeps_nested_fields_mappings_and_visibility_rules() {
        let spec = full_component_spec("AntdBaseArea", "create").unwrap();
        assert_eq!(spec.configurable["style"]["xAxis"], "<g2plot-axis>");
        assert!(spec.configurable["style"]["xField"].is_string());
        assert!(spec.configurable["style"]["yField"].is_string());
        assert!(spec
            .constraints
            .iter()
            .any(|constraint| constraint.contains("false") && constraint.contains("null")));
    }

    #[test]
    fn group_column_keeps_legend_exclusions_in_constraints() {
        let spec = full_component_spec("AntdGroupColumn", "create").unwrap();
        assert!(spec.constraints.iter().any(|constraint| {
            constraint.contains("legend")
                && constraint.contains("不包含")
                && constraint.contains("title")
                && constraint.contains("itemValue")
                && constraint.contains("background")
        }));
    }

    #[test]
    fn every_data_example_passes_the_backend_data_validator() {
        let evidence = AgentValidationEvidence::default();
        let response = component_documents_response(documents().keys().copied());
        evidence.record_component_documents(&response).unwrap();
        for (component_type, doc) in documents() {
            if !doc.supports_data_input {
                continue;
            }
            let spec = full_component_spec(component_type, "create").unwrap();
            let data = &spec.example["config"]["data"];
            evidence
                .validate_page_component_data(component_type, data)
                .unwrap_or_else(|error| panic!("{component_type} 示例未通过数据校验：{}", error.message));
        }
    }

    #[test]
    fn every_patch_style_example_passes_the_backend_style_validator() {
        let evidence = AgentValidationEvidence::default();
        let response = component_documents_response(documents().keys().copied());
        evidence.record_component_documents(&response).unwrap();
        for component_type in documents().keys() {
            let spec = full_component_spec(component_type, "patch").unwrap();
            let Some(style) = spec.example["stylePatch"]["style"].as_object() else {
                continue;
            };
            let ordinary = style
                .iter()
                .filter(|(key, _)| !is_component_data_field_mapping_key(key))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Map<_, _>>();
            if ordinary.is_empty() {
                continue;
            }
            evidence
                .validate_style_patch(component_type, &ordinary)
                .unwrap_or_else(|error| panic!("{component_type} 示例未通过样式校验：{}", error.message));
        }
    }
}
