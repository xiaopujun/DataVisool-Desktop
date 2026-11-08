/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 基于任务内已读目录返回组件的完整配置规格。

use rig_core::tool::{Tool, ToolFailure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::backend::ai_driven::component_docs::{
    common_fragments_for_configurables, component_docs_corpus_version, component_documents_response,
    full_spec::full_component_spec, has_component_document,
};
use crate::backend::ai_driven::domain::validation_evidence::{AgentValidationEvidence, EvidenceViolation};

use super::error::AgentToolError;
use super::output::{serialized_len, COMPONENT_SPEC_OUTPUT_BUDGET_BYTES};
use super::policy::AgentToolEffect;
use super::runtime::AgentToolRuntime;

pub(crate) const RESOLVE_COMPONENT_SPECS_TOOL: &str = "resolve_component_specs";
const MAX_COMPONENT_TYPES: usize = 10;

#[derive(Clone)]
pub(crate) struct ResolveComponentSpecsTool(pub(crate) AgentToolRuntime);

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum ComponentSpecIntent {
    Create,
    Patch,
}

impl ComponentSpecIntent {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Patch => "patch",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResolveComponentSpecsArgs {
    component_types: Vec<String>,
    intent: ComponentSpecIntent,

}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComponentCatalogItem {
    component_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComponentCatalogResponse {
    components: Vec<ComponentCatalogItem>,
}

impl Tool for ResolveComponentSpecsTool {
    const NAME: &'static str = RESOLVE_COMPONENT_SPECS_TOOL;
    type Error = AgentToolError;
    type Args = ResolveComponentSpecsArgs;
    type Output = Value;

    fn description(&self) -> String {
        format!(
            "按明确 componentTypes 一次返回每个组件的完整可配置项、数据要求、组件限制、最小有效示例，以及 configurable.style 引用的 G2Plot 公共片段 commonFragments。用 style 中的完整 <g2plot-…> 标签关联片段；生成配置时按片段和组件限制解释，但提交具体字段和值，绝不提交标签。componentTypes 单次 1–{MAX_COMPONENT_TYPES} 个并自动去重；intent 必须为 create 或 patch。创建使用 example.config，修改使用 example.stylePatch/dataPatch。不得提交函数值或文档未声明字段；修改已有数据前仍须读取 get_component_data_contract。"
        )
    }

    fn parameters(&self) -> Value {
        parameters_schema()
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0.context().authorize(Self::NAME, AgentToolEffect::Read, false)?;
        resolve_specs(self.0.validation_evidence(), args)
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

fn parameters_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "componentTypes": {
                "type": "array",
                "description": format!("目录中的明确组件类型，单次 1–{MAX_COMPONENT_TYPES} 个，重复项自动去重"),
                "items": { "type": "string", "minLength": 1 },
                "minItems": 1,
                "maxItems": MAX_COMPONENT_TYPES
            },
            "intent": { "type": "string", "enum": ["create", "patch"] }
        },
        "required": ["componentTypes", "intent"],
        "additionalProperties": false
    })
}

fn resolve_specs(evidence: &AgentValidationEvidence, mut args: ResolveComponentSpecsArgs) -> Result<Value, AgentToolError> {
    normalize_component_types(&mut args.component_types)?;
    let raw_catalog = evidence.component_catalog_for_specs().map_err(evidence_tool_error)?;
    let catalog = serde_json::from_value::<ComponentCatalogResponse>(raw_catalog)
        .map_err(|error| AgentToolError::validation("agent.evidence.component_catalog_invalid", format!("组件目录返回值无效：{error}")))?;
    let selected = select_components(&catalog.components, &args.component_types)?;
    let undocumented = selected
        .iter()
        .filter(|component| !has_component_document(&component.component_type))
        .map(|component| component.component_type.as_str())
        .collect::<Vec<_>>();
    if !undocumented.is_empty() {
        return Err(AgentToolError::invalid(
            "agent.component_specs.document_unavailable",
            format!("组件缺少可用配置规格：{}", undocumented.join(", ")),
        ));
    }

    let documents = component_documents_response(selected.iter().map(|component| component.component_type.as_str()));
    evidence.record_component_documents(&documents).map_err(evidence_tool_error)?;
    let intent = args.intent.as_str();
    let specs = selected
        .iter()
        .map(|component| {
            full_component_spec(&component.component_type, intent).ok_or_else(|| {
                AgentToolError::invalid(
                    "agent.component_specs.document_unavailable",
                    format!("组件缺少可解析的完整规格：{}", component.component_type),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let common_fragments = common_fragments_for_configurables(specs.iter().map(|spec| spec.configurable()))
        .map_err(|error| {
            AgentToolError::invalid(
                "agent.component_specs.common_fragments_invalid",
                format!("公共 G2Plot 配置片段不完整：{error}"),
            )
        })?;
    let output = json!({
        "corpusVersion": component_docs_corpus_version(),
        "specs": specs,
        "commonFragments": common_fragments,
    });
    finalize_output(evidence, output, intent, selected.iter().map(|component| component.component_type.as_str()))
}

fn finalize_output<'a>(
    evidence: &AgentValidationEvidence,
    output: Value,
    intent: &str,
    component_types: impl IntoIterator<Item = &'a str>,
) -> Result<Value, AgentToolError> {
    let bytes = serialized_len(&output)?;
    if bytes > COMPONENT_SPEC_OUTPUT_BUDGET_BYTES {
        return Err(AgentToolError::invalid(
            "agent.component_specs.output_limit_exceeded",
            format!(
                "完整组件规格响应为 {bytes} 字节，超过 {} 字节上限；请减少 componentTypes 后重试",
                COMPONENT_SPEC_OUTPUT_BUDGET_BYTES
            ),
        ));
    }
    for component_type in component_types {
        evidence.record_complete_component_spec(component_type, intent, component_docs_corpus_version());
    }
    Ok(output)
}

fn normalize_component_types(component_types: &mut Vec<String>) -> Result<(), AgentToolError> {
    for component_type in component_types.iter_mut() {
        *component_type = component_type.trim().to_string();
    }
    component_types.retain(|component_type| !component_type.is_empty());
    let mut seen = std::collections::HashSet::new();
    component_types.retain(|component_type| seen.insert(component_type.clone()));
    if component_types.is_empty() {
        return Err(AgentToolError::invalid("agent.component_specs.component_types_required", count_error(0)));
    }
    if component_types.len() > MAX_COMPONENT_TYPES {
        return Err(AgentToolError::invalid(
            "agent.component_specs.component_types_limit_exceeded",
            count_error(component_types.len()),
        ));
    }
    Ok(())
}

fn count_error(actual: usize) -> String {
    json!({
        "fieldPath": "componentTypes",
        "minItems": 1,
        "maxItems": MAX_COMPONENT_TYPES,
        "actualItems": actual,
    })
    .to_string()
}

fn select_components(catalog: &[ComponentCatalogItem], requested: &[String]) -> Result<Vec<ComponentCatalogItem>, AgentToolError> {
    let mut selected = Vec::with_capacity(requested.len());
    let mut missing = Vec::new();
    for component_type in requested {
        if let Some(component) = catalog.iter().find(|component| component.component_type == *component_type) {
            selected.push(component.clone());
        } else {
            missing.push(component_type.as_str());
        }
    }
    if missing.is_empty() {
        Ok(selected)
    } else {
        Err(AgentToolError::invalid(
            "agent.component_specs.component_unavailable",
            format!("组件不在当前可信目录中：{}", missing.join(", ")),
        ))
    }
}

fn evidence_tool_error(error: EvidenceViolation) -> AgentToolError {
    AgentToolError::validation(error.code, error.message)
}

#[cfg(test)]
mod tests;
