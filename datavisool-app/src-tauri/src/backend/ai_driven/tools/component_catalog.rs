/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 面向 Agent 的精简组件目录 Tool。

use std::time::Duration;

use rig_core::tool::{Tool, ToolFailure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::error::AgentToolError;
use super::output::{fit_model_output, ModelOutputBudget, CATALOG_OUTPUT_BUDGET_BYTES};
use super::policy::AgentToolEffect;
use super::runtime::AgentToolRuntime;
use crate::backend::ai_driven::domain::validation_evidence::EvidenceViolation;

pub(crate) const GET_COMPONENT_CATALOG_TOOL: &str = "get_component_catalog";

const FRONTEND_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_COMPONENT_CATALOG_ITEMS: usize = 100;

#[derive(Clone)]
pub(crate) struct GetComponentCatalogTool(pub(crate) AgentToolRuntime);

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetComponentCatalogArgs {}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComponentCatalogItem {
    component_type: String,
    component_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComponentCatalogResponse {
    catalog_version: String,
    components: Vec<ComponentCatalogItem>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComponentCatalogEntry {
    component_type: String,
    component_name: String,
}

impl Tool for GetComponentCatalogTool {
    const NAME: &'static str = GET_COMPONENT_CATALOG_TOOL;
    type Error = AgentToolError;
    type Args = GetComponentCatalogArgs;
    type Output = Value;

    fn description(&self) -> String {
        "组件类型不明确时，可读取当前设计器可供 AI 使用的全部组件类型和名称；单个任务最多读取一次。已有明确 componentType 时可直接调用 resolve_component_specs 获取权威规格，无需先读取目录。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0
            .context()
            .authorize(Self::NAME, AgentToolEffect::FrontendInteraction, false)?;
        let raw_response: Value = self
            .0
            .frontend()?
            .call("getComponentCatalog", json!({}), Some(FRONTEND_TIMEOUT), self.0.context())
            .await?;
        self.0
            .validation_evidence()
            .record_component_catalog(&raw_response)
            .map_err(evidence_tool_error)?;
        let catalog = serde_json::from_value::<ComponentCatalogResponse>(raw_response)
            .map_err(|error| AgentToolError::validation("agent.evidence.component_catalog_invalid", format!("组件目录返回值无效：{error}")))?;
        fit_model_output(
            project_catalog_for_model(catalog.catalog_version, catalog.components),
            ModelOutputBudget::Catalog,
            "组件目录已截断；请使用已返回的 componentType 调用 resolve_component_specs",
        )
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

fn project_catalog_for_model(catalog_version: String, components: Vec<ComponentCatalogItem>) -> Value {
    let total = components.len();
    let entries = components
        .into_iter()
        .map(|component| ComponentCatalogEntry { component_type: component.component_type, component_name: component.component_name })
        .collect::<Vec<_>>();
    let complete = catalog_result(&catalog_version, &entries, total, false);
    if entries.len() <= MAX_COMPONENT_CATALOG_ITEMS && serialized_len(&complete) <= CATALOG_OUTPUT_BUDGET_BYTES {
        return complete;
    }

    let mut returned = Vec::with_capacity(MAX_COMPONENT_CATALOG_ITEMS);
    for entry in entries.into_iter().take(MAX_COMPONENT_CATALOG_ITEMS) {
        let mut candidate = returned.clone();
        candidate.push(entry);
        if serialized_len(&catalog_result(&catalog_version, &candidate, total, true)) > CATALOG_OUTPUT_BUDGET_BYTES {
            break;
        }
        returned = candidate;
    }
    catalog_result(&catalog_version, &returned, total, true)
}

fn catalog_result(catalog_version: &str, components: &[ComponentCatalogEntry], total: usize, truncated: bool) -> Value {
    let mut result = json!({
        "catalogVersion": catalog_version,
        "components": components,
    });
    if truncated {
        result["total"] = Value::from(total);
        result["returned"] = Value::from(components.len());
        result["hasMore"] = Value::Bool(true);
    }
    result
}

fn serialized_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

fn evidence_tool_error(error: EvidenceViolation) -> AgentToolError {
    AgentToolError::validation(error.code, error.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(index: usize) -> ComponentCatalogItem {
        ComponentCatalogItem { component_type: format!("Component{index}"), component_name: format!("组件{index}") }
    }

    #[test]
    fn model_catalog_keeps_all_current_components_and_only_selection_fields() {
        let catalog = project_catalog_for_model("component-catalog/v3:51".to_string(), (1..=51).map(component).collect());

        assert_eq!(catalog["catalogVersion"], "component-catalog/v3:51");
        assert_eq!(catalog["components"].as_array().map(Vec::len), Some(51));
        assert!(catalog.get("total").is_none());
        assert!(catalog["components"].as_array().unwrap().iter().all(|component| {
            component
                .as_object()
                .is_some_and(|entry| entry.len() == 2 && entry.contains_key("componentType") && entry.contains_key("componentName"))
        }));
    }

    #[test]
    fn model_catalog_reports_incomplete_results_when_item_limit_is_exceeded() {
        let catalog = project_catalog_for_model("component-catalog/v3:101".to_string(), (1..=101).map(component).collect());

        assert_eq!(catalog["total"], 101);
        assert_eq!(catalog["returned"], 100);
        assert_eq!(catalog["hasMore"], true);
        assert_eq!(catalog["components"].as_array().map(Vec::len), Some(100));
    }

    #[test]
    fn rejects_any_tool_arguments() {
        assert!(serde_json::from_value::<GetComponentCatalogArgs>(json!({})).is_ok());
        assert!(serde_json::from_value::<GetComponentCatalogArgs>(json!({"componentType": "BaseText"})).is_err());
    }
}
