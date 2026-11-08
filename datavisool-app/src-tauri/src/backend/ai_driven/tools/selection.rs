/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 用户受控组件选择 Tool。

use std::collections::{HashMap, HashSet};

use rig_core::tool::{Tool, ToolFailure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::error::AgentToolError;
use super::policy::AgentToolEffect;
use super::runtime::AgentToolRuntime;

pub(crate) const REQUEST_COMPONENT_SELECTION_TOOL: &str = "request_component_selection";
const MAX_SELECTION_CANDIDATES: usize = 30;

#[derive(Clone)]
pub(crate) struct RequestComponentSelectionTool(pub(crate) AgentToolRuntime);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ComponentSelectionCandidate {
    component_type: String,
    display_name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    default_selected: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RequestComponentSelectionArgs {
    message: String,
    candidates: Vec<ComponentSelectionCandidate>,
    #[serde(default = "default_min_selected")]
    min_selected: usize,
    #[serde(default)]
    max_selected: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrontendComponentSelectionResult {
    confirmed: bool,
    #[serde(default)]
    selected_component_types: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelectedComponent {
    component_type: String,
    display_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComponentSelectionResult {
    confirmed: bool,
    selected_components: Vec<SelectedComponent>,
}

impl Tool for RequestComponentSelectionTool {
    const NAME: &'static str = REQUEST_COMPONENT_SELECTION_TOOL;
    type Error = AgentToolError;
    type Args = RequestComponentSelectionArgs;
    type Output = ComponentSelectionResult;

    fn description(&self) -> String {
        "向用户展示有限候选组件并等待受控选择；取消时 confirmed=false。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": { "type": "string", "minLength": 1 },
                "candidates": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_SELECTION_CANDIDATES,
                    "items": {
                        "type": "object",
                        "properties": {
                            "componentType": { "type": "string", "minLength": 1 },
                            "displayName": { "type": "string", "minLength": 1 },
                            "description": { "type": "string" },
                            "defaultSelected": { "type": "boolean", "default": false }
                        },
                        "required": ["componentType", "displayName"],
                        "additionalProperties": false
                    }
                },
                "minSelected": { "type": "integer", "minimum": 0, "default": 1 },
                "maxSelected": { "type": "integer", "minimum": 1 }
            },
            "required": ["message", "candidates"],
            "additionalProperties": false
        })
    }

    async fn call(&self, mut args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0
            .context()
            .authorize(Self::NAME, AgentToolEffect::FrontendInteraction, false)?;
        normalize_selection_request(&mut args)?;
        self.canonicalize_candidates(&mut args).await?;
        let candidates = args.candidates.clone();
        let min_selected = args.min_selected;
        let max_selected = args.max_selected.unwrap_or(candidates.len());
        let response = self
            .0
            .frontend()?
            .call("requestComponentSelection", args, None, self.0.context())
            .await?;
        resolve_component_selection(response, &candidates, min_selected, max_selected)
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

impl RequestComponentSelectionTool {
    async fn canonicalize_candidates(&self, args: &mut RequestComponentSelectionArgs) -> Result<(), AgentToolError> {
        let catalog: Value = self
            .0
            .frontend()?
            .call("getComponentCatalog", json!({}), Some(std::time::Duration::from_secs(30)), self.0.context())
            .await?;
        self.0
            .validation_evidence()
            .record_component_catalog(&catalog)
            .map_err(|error| AgentToolError::validation(error.code, error.message))?;
        let canonical_names = catalog
            .get("components")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| {
                Some((
                    item.get("componentType")?.as_str()?.trim().to_string(),
                    item.get("componentName")?.as_str()?.trim().to_string(),
                ))
            })
            .collect::<HashMap<_, _>>();
        for candidate in &mut args.candidates {
            self.0
                .validation_evidence()
                .validate_page_component_type(&candidate.component_type)
                .map_err(|error| AgentToolError::validation(error.code, error.message))?;
            candidate.display_name = canonical_names
                .get(&candidate.component_type)
                .filter(|name| !name.is_empty())
                .cloned()
                .ok_or_else(|| {
                    AgentToolError::validation(
                        "agent.selection.candidate_not_in_catalog",
                        format!("组件不在当前权威目录中：{}", candidate.component_type),
                    )
                })?;
            candidate.description = None;
        }
        Ok(())
    }
}

fn normalize_selection_request(args: &mut RequestComponentSelectionArgs) -> Result<(), AgentToolError> {
    args.message = required_text(std::mem::take(&mut args.message), "message")?;
    if args.candidates.is_empty() || args.candidates.len() > MAX_SELECTION_CANDIDATES {
        return Err(AgentToolError::invalid(
            "agent.selection.candidate_count_invalid",
            format!("candidates 数量必须在 1..={MAX_SELECTION_CANDIDATES}"),
        ));
    }
    let mut seen = HashSet::new();
    for candidate in &mut args.candidates {
        candidate.component_type = required_text(std::mem::take(&mut candidate.component_type), "componentType")?;
        candidate.display_name = required_text(std::mem::take(&mut candidate.display_name), "displayName")?;
        candidate.description = candidate
            .description
            .take()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        if !seen.insert(candidate.component_type.clone()) {
            return Err(AgentToolError::invalid(
                "agent.selection.duplicate_candidate",
                format!("候选组件类型重复：{}", candidate.component_type),
            ));
        }
    }
    let max_selected = args.max_selected.unwrap_or(args.candidates.len());
    if max_selected == 0 || max_selected > args.candidates.len() || args.min_selected > max_selected {
        return Err(AgentToolError::invalid("agent.selection.range_invalid", "minSelected/maxSelected 超出候选组件范围"));
    }
    if args.candidates.iter().filter(|candidate| candidate.default_selected).count() > max_selected {
        return Err(AgentToolError::invalid("agent.selection.defaults_invalid", "默认选择数量超过 maxSelected"));
    }
    Ok(())
}

fn resolve_component_selection(
    response: FrontendComponentSelectionResult,
    candidates: &[ComponentSelectionCandidate],
    min_selected: usize,
    max_selected: usize,
) -> Result<ComponentSelectionResult, AgentToolError> {
    if !response.confirmed {
        return Ok(ComponentSelectionResult { confirmed: false, selected_components: Vec::new() });
    }
    let candidate_map = candidates
        .iter()
        .map(|candidate| (candidate.component_type.as_str(), candidate))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut selected_components = Vec::new();
    for component_type in response.selected_component_types {
        let component_type = component_type.trim();
        let candidate = candidate_map.get(component_type).ok_or_else(|| {
            AgentToolError::validation("agent.selection.untrusted_result", format!("前端返回了候选列表之外的组件：{component_type}"))
        })?;
        if seen.insert(component_type.to_string()) {
            selected_components
                .push(SelectedComponent { component_type: candidate.component_type.clone(), display_name: candidate.display_name.clone() });
        }
    }
    if selected_components.len() < min_selected || selected_components.len() > max_selected {
        return Err(AgentToolError::validation("agent.selection.result_count_invalid", "前端返回的组件选择数量超出允许范围"));
    }
    Ok(ComponentSelectionResult { confirmed: true, selected_components })
}

fn required_text(value: String, field: &str) -> Result<String, AgentToolError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        Err(AgentToolError::invalid("agent.tool.empty_text", format!("{field} 不能为空")))
    } else {
        Ok(value)
    }
}

const fn default_min_selected() -> usize {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate() -> ComponentSelectionCandidate {
        ComponentSelectionCandidate {
            component_type: "BaseText".to_string(),
            display_name: "文本".to_string(),
            description: None,
            default_selected: true,
        }
    }

    #[test]
    fn rejects_values_outside_server_candidates() {
        let error = resolve_component_selection(
            FrontendComponentSelectionResult { confirmed: true, selected_component_types: vec!["Unknown".to_string()] },
            &[candidate()],
            1,
            1,
        )
        .unwrap_err();
        assert_eq!(error.code, "agent.selection.untrusted_result");
    }

    #[test]
    fn cancelled_selection_is_empty() {
        let result = resolve_component_selection(
            FrontendComponentSelectionResult { confirmed: false, selected_component_types: vec!["BaseText".to_string()] },
            &[candidate()],
            1,
            1,
        )
        .unwrap();
        assert!(!result.confirmed);
        assert!(result.selected_components.is_empty());
    }
}
