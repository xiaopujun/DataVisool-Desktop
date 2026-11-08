/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 组件数据说明书查询 Tool。

use std::time::Duration;

use rig_core::tool::{Tool, ToolFailure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::backend::ai_driven::component_docs::component_data_requirements;

use super::error::AgentToolError;
use super::output::{fit_model_output, ModelOutputBudget};
use super::policy::AgentToolEffect;
use super::runtime::AgentToolRuntime;

pub(crate) const GET_COMPONENT_DATA_CONTRACT_TOOL: &str = "get_component_data_contract";
const FRONTEND_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub(crate) struct GetComponentDataContractTool(pub(crate) AgentToolRuntime);

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DataSourceTypeArg {
    Static,
    Api,
    Database,
    CommonApi,
    GlobalVariable,
    WebSocket,
    Mqtt,
}

impl DataSourceTypeArg {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Api => "api",
            Self::Database => "database",
            Self::CommonApi => "commonApi",
            Self::GlobalVariable => "globalVariable",
            Self::WebSocket => "webSocket",
            Self::Mqtt => "mqtt",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GetComponentDataContractArgs {
    component_ids: Vec<String>,
    #[serde(default)]
    source_type: Option<DataSourceTypeArg>,
}

impl Tool for GetComponentDataContractTool {
    const NAME: &'static str = GET_COMPONENT_DATA_CONTRACT_TOOL;
    type Error = AgentToolError;
    type Args = GetComponentDataContractArgs;
    type Output = Value;

    fn description(&self) -> String {
        "读取目标组件的完整数据说明书：指定的单个数据源结构、组件专属数据格式、当前脱敏样例和字段映射。默认查询 static；修改组件数据或字段映射前必须调用。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "componentIds": {
                    "type": "array",
                    "items": {"type": "string"},
                    "minItems": 1,
                    "maxItems": 4
                },
                "sourceType": {
                    "type": "string",
                    "enum": ["static", "api", "database", "commonApi", "globalVariable", "webSocket", "mqtt"],
                    "default": "static",
                    "description": "只返回本次准备修改的数据源分支"
                }
            },
            "required": ["componentIds"],
            "additionalProperties": false
        })
    }

    async fn call(&self, mut args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0
            .context()
            .authorize(Self::NAME, AgentToolEffect::FrontendInteraction, false)?;
        normalize_component_ids(&mut args.component_ids)?;
        let source_type = args.source_type.unwrap_or(DataSourceTypeArg::Static);
        let mut response: Value = self
            .0
            .frontend()?
            .call(
                "getComponentDataContract",
                json!({"componentIds": args.component_ids, "sourceType": source_type}),
                Some(FRONTEND_TIMEOUT),
                self.0.context(),
            )
            .await?;
        enrich_component_data_requirements(&mut response)?;
        self.0
            .validation_evidence()
            .record_component_data_contract(&response, source_type.as_str())
            .map_err(|error| AgentToolError::validation(error.code, error.message))?;
        fit_model_output(response, ModelOutputBudget::Snapshot, "减少 componentIds，或分批读取数据说明书")
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

fn enrich_component_data_requirements(response: &mut Value) -> Result<(), AgentToolError> {
    let components = response
        .get_mut("components")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| AgentToolError::validation("agent.component_data_contract.invalid_response", "组件数据说明书缺少 components 数组"))?;
    for component in components {
        let object = component
            .as_object_mut()
            .ok_or_else(|| AgentToolError::validation("agent.component_data_contract.invalid_response", "组件数据说明书包含无效组件"))?;
        let component_type = object
            .get("componentType")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AgentToolError::validation("agent.component_data_contract.invalid_response", "组件数据说明书缺少 componentType"))?
            .to_string();
        let requirements = component_data_requirements(&component_type);
        object.insert("supportsDataInput".to_string(), Value::Bool(requirements.is_some_and(|(_, supports)| supports)));
        object.insert(
            "dataFormatRequirements".to_string(),
            Value::String(requirements.map_or_else(String::new, |(content, _)| content.to_string())),
        );
        let warnings = object.entry("warnings").or_insert_with(|| Value::Array(Vec::new()));
        let warnings = warnings
            .as_array_mut()
            .ok_or_else(|| AgentToolError::validation("agent.component_data_contract.invalid_response", "组件数据说明书 warnings 必须是数组"))?;
        if requirements.is_none() {
            push_warning(warnings, "component_data_format_requirements_missing");
        }
        if !requirements.is_some_and(|(_, supports)| supports) {
            push_warning(warnings, "component_data_patch_not_supported");
        }
    }
    Ok(())
}

fn push_warning(warnings: &mut Vec<Value>, warning: &str) {
    if !warnings.iter().any(|value| value.as_str() == Some(warning)) {
        warnings.push(Value::String(warning.to_string()));
    }
}

fn normalize_component_ids(component_ids: &mut Vec<String>) -> Result<(), AgentToolError> {
    let mut normalized = Vec::new();
    for component_id in component_ids.drain(..) {
        let component_id = component_id.trim().to_string();
        if !component_id.is_empty() && !normalized.contains(&component_id) {
            normalized.push(component_id);
        }
    }
    if normalized.is_empty() || normalized.len() > 4 {
        return Err(AgentToolError::invalid("agent.tool.invalid_component_count", "componentIds 数量必须在 1–4"));
    }
    *component_ids = normalized;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_type_defaults_are_stable() {
        let args: GetComponentDataContractArgs = serde_json::from_value(json!({"componentIds": ["c-1"]})).unwrap();
        assert!(args.source_type.is_none());
        assert_eq!(DataSourceTypeArg::Static.as_str(), "static");
        assert_eq!(DataSourceTypeArg::CommonApi.as_str(), "commonApi");
    }

    #[test]
    fn component_ids_are_bounded_and_deduplicated() {
        let mut ids = vec![" a ".to_string(), "a".to_string(), "b".to_string()];
        normalize_component_ids(&mut ids).unwrap();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn backend_documents_enrich_frontend_runtime_contract() {
        let mut response = json!({
            "components": [{"componentType": "BaseText", "warnings": []}]
        });

        enrich_component_data_requirements(&mut response).unwrap();

        assert_eq!(response["components"][0]["supportsDataInput"], true);
        assert!(response["components"][0]["dataFormatRequirements"]
            .as_str()
            .unwrap()
            .contains("dataPatch"));
    }

    #[test]
    fn missing_backend_document_is_reported_as_unsupported() {
        let mut response = json!({
            "components": [{"componentType": "Unknown", "warnings": []}]
        });

        enrich_component_data_requirements(&mut response).unwrap();

        assert_eq!(response["components"][0]["supportsDataInput"], false);
        assert_eq!(
            response["components"][0]["warnings"],
            json!(["component_data_format_requirements_missing", "component_data_patch_not_supported"])
        );
    }
}
