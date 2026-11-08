/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 组件数据说明书证据和数据补丁硬校验。

use serde_json::{Map, Value};

use super::validation_evidence::EvidenceViolation;

const DATA_CONTRACT_VERSION: &str = "component-data-contract/v1";
const DATA_FIELD_MAPPING_KEYS: &[&str] = &[
    "xField",
    "yField",
    "seriesField",
    "angleField",
    "colorField",
    "sizeField",
    "shapeField",
    "wordField",
    "weightField",
    "groupField",
];

#[derive(Clone, Debug)]
pub(super) struct ComponentDataContractEvidence {
    pub(super) component_type: String,
    pub(super) source_type: String,
    pub(super) supports_data_input: bool,
    value_example: Value,
    field_mapping: Map<String, Value>,
}

pub(crate) fn is_component_data_field_mapping_key(key: &str) -> bool {
    DATA_FIELD_MAPPING_KEYS.contains(&key)
}

pub(super) fn validate_data_support_statement(requirements: &str, supports_data_input: bool) -> Result<(), EvidenceViolation> {
    let expected = if supports_data_input {
        "该组件支持通过 `dataPatch` 修改数据。"
    } else {
        "该组件不支持通过 `dataPatch` 修改数据。"
    };
    if requirements.contains(expected) {
        Ok(())
    } else {
        Err(evidence_error(
            "agent.evidence.data_requirements_invalid",
            format!("数据格式要求与 supportsDataInput 不一致，缺少声明：{expected}"),
        ))
    }
}

pub(super) fn parse_component_data_contract(
    response: &Value,
    expected_source_type: &str,
) -> Result<Vec<(String, ComponentDataContractEvidence, Value)>, EvidenceViolation> {
    let version = required_text_field(response, "schemaVersion")?;
    if version != DATA_CONTRACT_VERSION {
        return Err(evidence_error("agent.evidence.data_contract_invalid", format!("不支持的组件数据说明书版本：{version}")));
    }
    let source_contract = response
        .get("sourceContract")
        .and_then(Value::as_object)
        .ok_or_else(|| evidence_error("agent.evidence.data_contract_invalid", "数据说明书缺少 sourceContract 对象"))?;
    let source_type = required_text_map_field(source_contract, "sourceType")?;
    if source_type != expected_source_type {
        return Err(evidence_error(
            "agent.evidence.data_contract_invalid",
            format!("数据说明书 sourceType 与查询参数不一致：{source_type}"),
        ));
    }
    let expected_branch = data_source_branch(source_type)?;
    let branch = required_text_map_field(source_contract, "branch")?;
    if branch != expected_branch || !source_contract.get("structure").is_some_and(Value::is_object) {
        return Err(evidence_error("agent.evidence.data_contract_invalid", "数据说明书的数据源分支或 structure 无效"));
    }

    let components = response
        .get("components")
        .and_then(Value::as_array)
        .ok_or_else(|| evidence_error("agent.evidence.data_contract_invalid", "数据说明书缺少 components 数组"))?;
    components
        .iter()
        .map(|component| parse_component_contract(component, source_type))
        .collect()
}

pub(super) fn validate_component_data_change(
    contract: &ComponentDataContractEvidence,
    component_id: &str,
    component_type: &str,
    data_patch: Option<&Map<String, Value>>,
    style_patch: Option<&Map<String, Value>>,
) -> Result<(), EvidenceViolation> {
    if contract.component_type != component_type {
        return Err(EvidenceViolation::new(
            "agent.operation.component_data_contract_type_mismatch",
            format!("组件 {component_id} 的数据说明书类型为 {}，动作参数类型为 {component_type}", contract.component_type),
            None::<String>,
        ));
    }
    if !contract.supports_data_input {
        return Err(data_error(format!("组件 {component_id} 不支持 dataPatch 或数据字段映射修改"), "dataPatch"));
    }

    let mapping_changed = style_patch.is_some_and(|style| style.keys().any(|key| is_component_data_field_mapping_key(key)));
    let mut final_value = contract.value_example.clone();
    let mut validate_mapping = mapping_changed && data_patch.is_none();
    if let Some(data_patch) = data_patch {
        let source_type = validate_data_source_config(data_patch)?;
        if source_type != contract.source_type {
            return Err(EvidenceViolation::new(
                "agent.operation.component_data_contract_source_mismatch",
                format!("dataPatch.sourceType={source_type} 与已读取的说明书 sourceType={} 不一致", contract.source_type),
                Some("sourceType"),
            ));
        }
        if source_type == "static" {
            final_value = data_patch
                .get("staticData")
                .and_then(Value::as_object)
                .and_then(|branch| branch.get("value"))
                .cloned()
                .unwrap_or(Value::Null);
            validate_mapping = true;
        }
    }

    let mut final_mapping = contract.field_mapping.clone();
    if let Some(style_patch) = style_patch {
        for key in DATA_FIELD_MAPPING_KEYS {
            if let Some(value) = style_patch.get(*key) {
                final_mapping.insert((*key).to_string(), value.clone());
            }
        }
    }
    validate_field_mapping_object(&final_mapping)?;
    if validate_mapping {
        validate_field_mapping_against_data(&final_mapping, &final_value)?;
    }
    Ok(())
}

pub(super) fn validate_page_component_data(component_type: &str, supports_data_input: bool, data: &Value) -> Result<(), EvidenceViolation> {
    if !supports_data_input {
        return Err(data_error(format!("组件 {component_type} 不支持 config.data"), "config.data"));
    }
    let object = data
        .as_object()
        .ok_or_else(|| data_error("config.data 必须是普通对象", "config.data"))?;
    validate_data_source_config(object).map(|_| ())
}

fn parse_component_contract(component: &Value, source_type: &str) -> Result<(String, ComponentDataContractEvidence, Value), EvidenceViolation> {
    let component_id = required_text_field(component, "componentId")?;
    let component_type = required_text_field(component, "componentType")?;
    let requirements = required_text_field(component, "dataFormatRequirements")?;
    let supports_data_input = component
        .get("supportsDataInput")
        .and_then(Value::as_bool)
        .ok_or_else(|| evidence_error("agent.evidence.data_contract_invalid", "返回值缺少布尔字段 supportsDataInput"))?;
    validate_data_support_statement(requirements, supports_data_input)?;
    let current_data = component
        .get("currentData")
        .and_then(Value::as_object)
        .ok_or_else(|| evidence_error("agent.evidence.data_contract_invalid", format!("组件 {component_id} 缺少 currentData")))?;
    let value_example = current_data.get("valueExample").cloned().unwrap_or(Value::Null);
    let field_mapping = component
        .get("fieldMapping")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| evidence_error("agent.evidence.data_contract_invalid", format!("组件 {component_id} 缺少 fieldMapping")))?;
    validate_field_mapping_object(&field_mapping)?;
    Ok((
        component_id.to_string(),
        ComponentDataContractEvidence {
            component_type: component_type.to_string(),
            source_type: source_type.to_string(),
            supports_data_input,
            value_example,
            field_mapping,
        },
        component.clone(),
    ))
}

fn validate_field_mapping_object(mapping: &Map<String, Value>) -> Result<(), EvidenceViolation> {
    for (key, value) in mapping {
        if !DATA_FIELD_MAPPING_KEYS.contains(&key.as_str()) {
            return Err(evidence_error("agent.evidence.data_contract_invalid", format!("数据说明书包含不支持的字段映射：{key}")));
        }
        if !value.as_str().is_some_and(|text| !text.trim().is_empty()) {
            return Err(data_error(format!("字段映射 {key} 必须是非空字符串"), key));
        }
    }
    Ok(())
}

fn validate_field_mapping_against_data(mapping: &Map<String, Value>, data: &Value) -> Result<(), EvidenceViolation> {
    if mapping.is_empty() {
        return Ok(());
    }
    let items: Vec<&Map<String, Value>> = match data {
        Value::Array(values) if values.is_empty() => return Ok(()),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                value
                    .as_object()
                    .ok_or_else(|| data_error("有字段映射的组件数据必须是对象数组", "staticData.value"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Value::Object(object) => vec![object],
        _ => {
            return Err(data_error("当前数据不是对象或对象数组，无法与字段映射匹配", "staticData.value"));
        }
    };
    for (mapping_key, data_key) in mapping {
        let data_key = data_key.as_str().expect("field mapping was validated");
        for (index, item) in items.iter().enumerate() {
            if !item.contains_key(data_key) {
                return Err(data_error(
                    format!("字段映射 {mapping_key}={data_key} 与最终数据不一致，第 {} 项缺少该属性", index + 1),
                    mapping_key,
                ));
            }
        }
    }
    Ok(())
}

fn validate_data_source_config(data: &Map<String, Value>) -> Result<String, EvidenceViolation> {
    let source_type = data
        .get("sourceType")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| data_error("dataPatch.sourceType 必须是非空字符串", "sourceType"))?;
    let branch = data_source_branch(source_type)?;
    for key in data.keys() {
        if key != "sourceType" && key != branch {
            return Err(data_error(format!("sourceType={source_type} 时不能写入字段：{key}"), key));
        }
    }
    let branch_value = data
        .get(branch)
        .and_then(Value::as_object)
        .ok_or_else(|| data_error(format!("sourceType={source_type} 时 {branch} 必须是对象"), branch))?;
    if branch_value.is_empty() {
        return Err(data_error(format!("dataPatch.{branch} 不能是空对象"), branch));
    }
    for (field, value) in branch_value {
        validate_data_source_field(source_type, field, value)?;
    }
    if source_type == "static" && !branch_value.contains_key("value") {
        return Err(data_error("静态数据必须写入 staticData.value", "staticData.value"));
    }
    Ok(source_type.to_string())
}

fn data_source_branch(source_type: &str) -> Result<&'static str, EvidenceViolation> {
    match source_type {
        "static" => Ok("staticData"),
        "api" => Ok("apiData"),
        "database" => Ok("database"),
        "commonApi" => Ok("commonApi"),
        "globalVariable" => Ok("globalVariable"),
        "webSocket" => Ok("webSocket"),
        "mqtt" => Ok("mqtt"),
        _ => Err(data_error(format!("不支持的 sourceType：{source_type}"), "sourceType")),
    }
}

fn validate_data_source_field(source_type: &str, field: &str, value: &Value) -> Result<(), EvidenceViolation> {
    let valid = match (source_type, field) {
        ("static", "value") => true,
        ("api", "url") | ("api", "method") => value.is_string(),
        ("api", "headers") | ("api", "params") => value.is_object(),
        ("api", "autoFlush") => value.is_boolean(),
        ("api", "frequency") => value.is_number(),
        ("commonApi", "commonApiId") => value.is_string(),
        ("commonApi", "autoFlush") => value.is_boolean(),
        ("commonApi", "frequency") => value.is_number(),
        ("database", "targetDb") | ("database", "sql") => value.is_string(),
        ("database", "autoFlush") => value.is_boolean(),
        ("database", "frequency") => value.is_number(),
        ("globalVariable", "gvId") => value.is_string(),
        ("webSocket", "url") | ("webSocket", "protocols") => value.is_string(),
        ("mqtt", "host") | ("mqtt", "path") | ("mqtt", "clientId") | ("mqtt", "username") | ("mqtt", "topics") => value.is_string(),
        ("mqtt", "port") | ("mqtt", "connectTimeout") | ("mqtt", "keepalive") | ("mqtt", "reconnectPeriod") | ("mqtt", "protocolVersion") => {
            value.is_number()
        }
        ("mqtt", "clean") => value.is_boolean(),
        ("mqtt", "qos") => value.as_u64().is_some_and(|qos| qos <= 2),
        _ => false,
    };
    if source_type == "api" && field == "method" {
        let method = value.as_str().unwrap_or_default();
        if !matches!(method, "get" | "post" | "put" | "delete") {
            return Err(data_error("apiData.method 必须是 get/post/put/delete", format!("apiData.{field}")));
        }
    }
    if valid {
        Ok(())
    } else {
        let branch = data_source_branch(source_type)?;
        Err(data_error(format!("dataPatch.{branch}.{field} 不在允许范围内或值类型不正确"), format!("{branch}.{field}")))
    }
}

fn required_text_field<'a>(value: &'a Value, field: &str) -> Result<&'a str, EvidenceViolation> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| evidence_error("agent.evidence.data_contract_invalid", format!("返回值缺少非空字段 {field}")))
}

fn required_text_map_field<'a>(value: &'a Map<String, Value>, field: &str) -> Result<&'a str, EvidenceViolation> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| evidence_error("agent.evidence.data_contract_invalid", format!("返回值缺少非空字段 {field}")))
}

fn evidence_error(code: &'static str, message: impl Into<String>) -> EvidenceViolation {
    EvidenceViolation::new(code, message, None::<String>)
}

fn data_error(message: impl Into<String>, path: impl Into<String>) -> EvidenceViolation {
    EvidenceViolation::new("agent.operation.data_patch_not_allowed", message, Some(path))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn static_contract_response(supports_data_input: bool) -> Value {
        let requirements = if supports_data_input {
            "该组件支持通过 `dataPatch` 修改数据。数据项属性与字段映射必须一致。"
        } else {
            "该组件不支持通过 `dataPatch` 修改数据。"
        };
        json!({
            "schemaVersion": "component-data-contract/v1",
            "sourceContract": {
                "sourceType": "static",
                "branch": "staticData",
                "structure": {"sourceType": "static", "staticData": {"value": "由组件决定"}}
            },
            "components": [{
                "componentId": "chart-1",
                "componentType": "BaseChart",
                "supportsDataInput": supports_data_input,
                "dataFormatRequirements": requirements,
                "currentData": {"valueExample": [{"year": "2025", "value": 10, "category": "A"}]},
                "fieldMapping": {"xField": "year", "yField": "value", "seriesField": "category"}
            }]
        })
    }

    #[test]
    fn static_patch_requires_exact_new_structure() {
        let parsed = parse_component_data_contract(&static_contract_response(true), "static").unwrap();
        let contract = &parsed[0].1;
        let valid = serde_json::from_value(json!({
            "sourceType": "static",
            "staticData": {"value": [{"time": "2026", "amount": 20, "kind": "B"}]}
        }))
        .unwrap();
        let mapping = serde_json::from_value(json!({
            "xField": "time",
            "yField": "amount",
            "seriesField": "kind"
        }))
        .unwrap();
        validate_component_data_change(contract, "chart-1", "BaseChart", Some(&valid), Some(&mapping)).unwrap();

        let old_shape = serde_json::from_value(json!({"sourceType": "static", "value": []})).unwrap();
        assert!(validate_component_data_change(contract, "chart-1", "BaseChart", Some(&old_shape), None).is_err());
        let mismatched = serde_json::from_value(json!({
            "sourceType": "static",
            "staticData": {"value": [{"time": "2026", "amount": 20, "kind": "B"}]}
        }))
        .unwrap();
        assert!(validate_component_data_change(contract, "chart-1", "BaseChart", Some(&mismatched), None).is_err());
    }

    #[test]
    fn source_contract_and_component_capability_are_enforced() {
        let response = static_contract_response(false);
        assert!(parse_component_data_contract(&response, "api").is_err());
        let parsed = parse_component_data_contract(&response, "static").unwrap();
        let patch = serde_json::from_value(json!({"sourceType": "static", "staticData": {"value": []}})).unwrap();
        assert!(validate_component_data_change(&parsed[0].1, "chart-1", "BaseChart", Some(&patch), None).is_err());
        assert!(validate_page_component_data("BaseChart", false, &json!({"sourceType": "static", "staticData": {"value": []}})).is_err());
    }

    #[test]
    fn dynamic_source_does_not_validate_unknown_runtime_rows() {
        let response = json!({
            "schemaVersion": "component-data-contract/v1",
            "sourceContract": {"sourceType": "api", "branch": "apiData", "structure": {"sourceType": "api", "apiData": {}}},
            "components": [{
                "componentId": "chart-1",
                "componentType": "BaseChart",
                "supportsDataInput": true,
                "dataFormatRequirements": "该组件支持通过 `dataPatch` 修改数据。",
                "currentData": {"valueExample": null},
                "fieldMapping": {"xField": "year", "yField": "value"}
            }]
        });
        let parsed = parse_component_data_contract(&response, "api").unwrap();
        let patch = serde_json::from_value(json!({
            "sourceType": "api",
            "apiData": {"url": "https://example.com/data", "method": "get"}
        }))
        .unwrap();
        validate_component_data_change(&parsed[0].1, "chart-1", "BaseChart", Some(&patch), None).unwrap();
    }
}
