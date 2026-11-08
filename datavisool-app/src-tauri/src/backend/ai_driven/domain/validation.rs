/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 真实画布动作的边界校验。

use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceTerminalValidationIssue;

use super::artifact::{ComponentConfigPatch, PageDraftComponent};
use super::validation_data_contract::is_component_data_field_mapping_key;
use super::validation_evidence::{AgentValidationEvidence, EvidenceViolation};

#[derive(Clone, Debug, Default)]
pub(crate) struct AgentArtifactValidationContext {
    pub validation_evidence: AgentValidationEvidence,
}

pub(crate) fn validate_page_operation(
    components: &[PageDraftComponent],
    context: &AgentArtifactValidationContext,
) -> Vec<WorkspaceTerminalValidationIssue> {
    let mut issues = Vec::new();
    if components.is_empty() {
        issues.push(issue("agent.operation.empty_page", "创建组件列表不能为空", Some("/components")));
        return issues;
    }
    let mut ids = HashSet::new();
    for (index, component) in components.iter().enumerate() {
        let path = format!("/components/{index}");
        if component.id.trim().is_empty() || !ids.insert(component.id.trim()) {
            issues.push(issue("agent.operation.invalid_draft_component_id", "草稿组件 ID 不能为空或重复", Some(&format!("{path}/id"))));
        }
        if component.component_type.trim().is_empty() {
            issues.push(issue("agent.operation.component_type_required", "组件 type 不能为空", Some(&format!("{path}/type"))));
        } else if let Err(error) = context
            .validation_evidence
            .validate_page_component_type(component.component_type.trim())
        {
            issues.push(evidence_issue(error, &format!("{path}/type")));
        }
        if !component.x.is_finite()
            || !component.y.is_finite()
            || !component.width.is_finite()
            || !component.height.is_finite()
            || component.width <= 0.0
            || component.height <= 0.0
        {
            issues.push(issue("agent.operation.invalid_component_bounds", "组件坐标必须有限且宽高大于 0", Some(&path)));
        }
        if component.x < 0.0 || component.y < 0.0 {
            issues.push(issue("agent.operation.negative_component_position", "组件坐标不能为负数", Some(&path)));
        }
        scan_data_value(&Value::Object(component.config.clone()), &format!("{path}/config"), &mut issues);
        if let Some(data) = component.config.get("data") {
            if let Err(error) = context
                .validation_evidence
                .validate_page_component_data(component.component_type.trim(), data)
            {
                issues.push(evidence_issue(error, &format!("{path}/config/data")));
            }
        }
    }
    issues
}

pub(crate) fn validate_component_patch_operation(
    patches: &[ComponentConfigPatch],
    context: &AgentArtifactValidationContext,
) -> Vec<WorkspaceTerminalValidationIssue> {
    let mut issues = Vec::new();
    if patches.is_empty() {
        issues.push(issue("agent.operation.empty_component_patches", "组件补丁不能为空", Some("/patches")));
    }
    let mut seen = HashSet::new();
    for (index, patch) in patches.iter().enumerate() {
        validate_target(&patch.component_id, &patch.component_type, index, &mut seen, &mut issues);
        if patch.style_patch.is_none() && patch.data_patch.is_none() && patch.layer_patch.is_none() && patch.filter_patch.is_none() {
            issues.push(issue(
                "agent.operation.empty_component_patch",
                "组件补丁必须包含 stylePatch、dataPatch、layerPatch 或 filterPatch",
                Some(&format!("/patches/{index}")),
            ));
        }
        if let Some(style_patch) = &patch.style_patch {
            let path = format!("/patches/{index}/stylePatch/style");
            if style_patch.style.is_empty() {
                issues.push(issue("agent.operation.empty_style_patch", "stylePatch.style 不能为空", Some(&path)));
            } else {
                scan_safe_value(&Value::Object(style_patch.style.clone()), &path, &mut issues);
                let ordinary_style = style_patch
                    .style
                    .iter()
                    .filter(|(key, _)| !is_component_data_field_mapping_key(key))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<Map<_, _>>();
                if !ordinary_style.is_empty() {
                    if let Err(error) = context
                        .validation_evidence
                        .validate_style_patch(&patch.component_type, &ordinary_style)
                    {
                        issues.push(evidence_issue(error, &format!("/patches/{index}/stylePatch")));
                    }
                }
            }
        }
        if let Some(data_patch) = &patch.data_patch {
            let path = format!("/patches/{index}/dataPatch");
            if data_patch.is_empty() {
                issues.push(issue("agent.operation.empty_data_patch", "dataPatch 不能为空", Some(&path)));
            } else {
                scan_data_object(data_patch, &path, &mut issues);
            }
        }
        let has_field_mapping_patch = patch
            .style_patch
            .as_ref()
            .is_some_and(|style_patch| style_patch.style.keys().any(|key| is_component_data_field_mapping_key(key)));
        if patch.data_patch.is_some() || has_field_mapping_patch {
            if let Err(error) = context.validation_evidence.validate_component_data_change(
                &patch.component_id,
                &patch.component_type,
                patch.data_patch.as_ref(),
                patch.style_patch.as_ref().map(|style_patch| &style_patch.style),
            ) {
                issues.push(evidence_issue(error, &format!("/patches/{index}")));
            }
        }
        if let Some(layer) = &patch.layer_patch {
            let path = format!("/patches/{index}/layerPatch");
            let empty = layer.x.is_none() && layer.y.is_none() && layer.width.is_none() && layer.height.is_none() && layer.name.is_none();
            if empty {
                issues.push(issue("agent.operation.empty_layer_patch", "layerPatch 不能为空", Some(&path)));
            }
            if layer.x.is_some_and(|value| !value.is_finite() || value < 0.0)
                || layer.y.is_some_and(|value| !value.is_finite() || value < 0.0)
                || layer.width.is_some_and(|value| !value.is_finite() || value <= 0.0)
                || layer.height.is_some_and(|value| !value.is_finite() || value <= 0.0)
            {
                issues.push(issue("agent.operation.invalid_layer_patch", "坐标必须非负，宽高必须大于 0，且数值必须有限", Some(&path)));
            }
            if layer
                .name
                .as_ref()
                .is_some_and(|name| name.trim().is_empty() || name.chars().count() > 200)
            {
                issues.push(issue("agent.operation.invalid_layer_name", "组件名称必须为 1–200 个字符", Some(&format!("{path}/name"))));
            }
        }
        if let Some(filter) = &patch.filter_patch {
            let path = format!("/patches/{index}/filterPatch");
            let values = [filter.blur, filter.brightness, filter.contrast, filter.opacity, filter.saturate, filter.hue_rotate];
            if filter.enable.is_none() && values.iter().all(Option::is_none) {
                issues.push(issue("agent.operation.empty_filter_patch", "filterPatch 不能为空", Some(&path)));
            }
            if values.into_iter().flatten().any(|value| !value.is_finite())
                || filter.blur.is_some_and(|value| !(0.0..=100.0).contains(&value))
                || filter.brightness.is_some_and(|value| !(0.0..=10.0).contains(&value))
                || filter.contrast.is_some_and(|value| !(0.0..=10.0).contains(&value))
                || filter.opacity.is_some_and(|value| !(0.0..=1.0).contains(&value))
                || filter.saturate.is_some_and(|value| !(0.0..=10.0).contains(&value))
                || filter.hue_rotate.is_some_and(|value| !(-360.0..=360.0).contains(&value))
            {
                issues.push(issue("agent.operation.invalid_filter_patch", "滤镜参数超出安全范围", Some(&path)));
            }
        }
    }
    issues
}

fn validate_target(
    component_id: &str,
    component_type: &str,
    index: usize,
    seen: &mut HashSet<String>,
    issues: &mut Vec<WorkspaceTerminalValidationIssue>,
) {
    let component_id = component_id.trim();
    let component_type = component_type.trim();
    if component_id.is_empty() || component_type.is_empty() {
        issues.push(issue(
            "agent.operation.invalid_component_target",
            "componentId 和 componentType 不能为空",
            Some(&format!("/patches/{index}")),
        ));
        return;
    }
    if !seen.insert(component_id.to_owned()) {
        issues.push(issue(
            "agent.operation.duplicate_component_target",
            "同一组件不能重复写入",
            Some(&format!("/patches/{index}/componentId")),
        ));
    }
}

fn scan_data_object(object: &Map<String, Value>, path: &str, issues: &mut Vec<WorkspaceTerminalValidationIssue>) {
    for (key, value) in object {
        let normalized = key.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "__proto__" | "constructor" | "prototype") {
            issues.push(issue("agent.operation.dangerous_data_key", format!("禁止危险字段：{key}"), Some(&format!("{path}/{key}"))));
        }
        if is_sensitive_key(&normalized) {
            issues.push(issue("agent.operation.sensitive_data_key", format!("禁止敏感字段：{key}"), Some(&format!("{path}/{key}"))));
        }
        scan_data_value(value, &format!("{path}/{key}"), issues);
    }
}

fn scan_data_value(value: &Value, path: &str, issues: &mut Vec<WorkspaceTerminalValidationIssue>) {
    match value {
        Value::Object(object) => scan_data_object(object, path, issues),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                scan_data_value(item, &format!("{path}/{index}"), issues);
            }
        }
        Value::String(text) => {
            if has_invalid_expression(text) {
                issues.push(issue("agent.operation.invalid_data_expression", "禁止函数或表达式", Some(path)));
            }
            if path.ends_with("/url") && (has_sensitive_query(text) || has_url_userinfo(text)) {
                issues.push(issue("agent.operation.sensitive_url_credentials", "URL 不能包含认证信息", Some(path)));
            }
        }
        _ => {}
    }
}

fn scan_safe_value(value: &Value, path: &str, issues: &mut Vec<WorkspaceTerminalValidationIssue>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let normalized = key.trim().to_ascii_lowercase();
                if matches!(normalized.as_str(), "__proto__" | "constructor" | "prototype") {
                    issues.push(issue("agent.operation.dangerous_config_key", format!("禁止危险字段：{key}"), Some(&format!("{path}/{key}"))));
                }
                scan_safe_value(child, &format!("{path}/{key}"), issues);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                scan_safe_value(child, &format!("{path}/{index}"), issues);
            }
        }
        _ => {}
    }
}

fn is_sensitive_key(normalized: &str) -> bool {
    normalized.contains("password")
        || normalized.contains("token")
        || normalized.contains("secret")
        || normalized.contains("authorization")
        || normalized.contains("cookie")
        || normalized.contains("credential")
        || normalized == "key"
        || normalized.ends_with("key")
        || normalized.contains("-key")
        || normalized.contains("_key")
}

fn has_invalid_expression(value: &str) -> bool {
    let value = value.trim();
    value == "undefined"
        || value == "NaN"
        || value == "Infinity"
        || value.starts_with("function ")
        || value.starts_with("async function ")
        || value.contains("=>")
        || value.contains("${")
        || value.contains("#{")
}

fn has_sensitive_query(value: &str) -> bool {
    let Some((_, query_with_hash)) = value.split_once('?') else {
        return false;
    };
    query_with_hash
        .split('#')
        .next()
        .unwrap_or_default()
        .split('&')
        .filter_map(|part| part.split('=').next())
        .map(|key| key.trim().to_ascii_lowercase())
        .any(|key| is_sensitive_key(&key))
}

fn has_url_userinfo(value: &str) -> bool {
    let Some((_, after_scheme)) = value.split_once("://") else {
        return false;
    };
    after_scheme
        .split(['/', '?', '#'])
        .next()
        .is_some_and(|authority| authority.contains('@'))
}

fn issue(code: impl Into<String>, message: impl Into<String>, path: Option<&str>) -> WorkspaceTerminalValidationIssue {
    let issue = WorkspaceTerminalValidationIssue::new(code, message);
    match path {
        Some(path) => issue.with_path(path),
        None => issue,
    }
}

fn evidence_issue(error: EvidenceViolation, base_path: &str) -> WorkspaceTerminalValidationIssue {
    let path = error.path.as_deref().map_or_else(
        || base_path.to_string(),
        |relative| {
            let relative = relative.replace('.', "/").replace('[', "/").replace(']', "");
            format!("{base_path}/{}", relative.trim_start_matches('/'))
        },
    );
    WorkspaceTerminalValidationIssue::new(error.code, error.message).with_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_patch_requires_at_least_one_target() {
        let issues = validate_component_patch_operation(&[], &AgentArtifactValidationContext::default());
        assert!(issues
            .iter()
            .any(|issue| issue.code == "agent.operation.empty_component_patches"));
    }
}
