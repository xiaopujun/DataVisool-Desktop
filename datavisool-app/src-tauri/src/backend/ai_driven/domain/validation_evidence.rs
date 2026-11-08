/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 后端内置资料与前端只读 Tool 返回的可信业务 Schema 证据。
//!
//! 模型只能看到这些响应，不能直接写入本缓存。动作校验只接受本次任务中
//! 由宿主提供并成功解析的规则，避免把模型自报的字段白名单当成可信依据。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Deserialize;
use serde_json::{Map, Value};

use super::validation_data_contract::{
    parse_component_data_contract, validate_component_data_change, validate_data_support_statement, validate_page_component_data,
    ComponentDataContractEvidence,
};
use crate::backend::ai_driven::component_docs::component_docs_corpus_version;

const STYLE_SCHEMA_VERSION: &str = "component-style-doc/v1";
const CATALOG_VERSION_PREFIX: &str = "component-catalog/v3:";

/// 一次 Agent 运行内共享的只读校验证据。
#[derive(Clone, Debug, Default)]
pub(crate) struct AgentValidationEvidence {
    state: Arc<Mutex<EvidenceState>>,
}

#[derive(Debug, Default)]
struct EvidenceState {
    style_schemas: HashMap<String, ComponentStyleSchema>,
    data_contracts: HashMap<String, ComponentDataContractEvidence>,
    data_capabilities: HashMap<String, bool>,
    component_catalog: Option<HashSet<String>>,
    raw_style_docs: HashMap<String, Value>,
    raw_data_contracts: HashMap<String, Value>,
    raw_component_catalog: Option<Value>,
    component_spec_reads: HashSet<ComponentSpecReadKey>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ComponentSpecReadKey {
    component_type: String,
    intent: String,
    corpus_version: String,
}

/// 可直接转换为终态校验问题的稳定错误。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvidenceViolation {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) path: Option<String>,
}

impl EvidenceViolation {
    pub(super) fn new(code: &'static str, message: impl Into<String>, path: Option<impl Into<String>>) -> Self {
        Self { code, message: message.into(), path: path.map(Into::into) }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComponentStyleSchema {
    schema_version: String,
    component_type: String,
    #[serde(default)]
    forbidden_function_config: bool,
    #[serde(default)]
    allowed_style_paths: Vec<ValueRule>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ValueRule {
    path: String,
    #[serde(rename = "type")]
    value_type: String,
    #[serde(default)]
    min: Option<f64>,
    #[serde(default)]
    max: Option<f64>,
    #[serde(default)]
    enum_values: Option<Vec<Value>>,
    #[serde(default)]
    allow_null: bool,
    #[serde(default)]
    allow_false: bool,
    #[serde(default)]
    min_items: Option<usize>,
    #[serde(default)]
    max_items: Option<usize>,
    #[serde(default)]
    item_object_fields: Vec<ObjectFieldRule>,
    #[serde(default)]
    allow_additional_item_fields: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObjectFieldRule {
    name: String,
    #[serde(rename = "type")]
    value_type: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    min: Option<f64>,
    #[serde(default)]
    max: Option<f64>,
    #[serde(default)]
    enum_values: Option<Vec<Value>>,
    #[serde(default)]
    allow_null: bool,
}

impl AgentValidationEvidence {
    /// 缓存 get_component_document 的原始文档，并提取机器可读样式规则和数据能力。
    pub(crate) fn record_component_documents(&self, response: &Value) -> Result<(), EvidenceViolation> {
        let docs = response
            .get("docs")
            .and_then(Value::as_array)
            .ok_or_else(|| evidence_response_error("agent.evidence.style_docs_invalid", "get_component_document 返回值缺少 docs 数组"))?;
        let mut parsed = Vec::with_capacity(docs.len());
        for doc in docs {
            let component_type = required_text_field(doc, "componentType", "agent.evidence.style_docs_invalid")?;
            let content = required_text_field(doc, "content", "agent.evidence.style_docs_invalid")?;
            let schema = parse_style_schema(content)?;
            if schema.component_type != component_type {
                return Err(evidence_response_error(
                    "agent.evidence.style_docs_invalid",
                    format!("样式文档组件类型不匹配：文档为 {component_type}，Schema 为 {}", schema.component_type),
                ));
            }
            validate_schema_rules(&schema.allowed_style_paths, "style.", style_rule_type_supported, "样式")?;
            let requirements = required_text_field(doc, "dataFormatRequirements", "agent.evidence.style_docs_invalid")?;
            let supports_data_input = required_bool_field(doc, "supportsDataInput", "agent.evidence.style_docs_invalid")?;
            validate_data_support_statement(requirements, supports_data_input)?;
            parsed.push((component_type.to_string(), schema, supports_data_input, doc.clone()));
        }

        let mut state = self.lock();
        for (component_type, schema, supports_data_input, raw) in parsed {
            state.style_schemas.insert(component_type.clone(), schema);
            state.data_capabilities.insert(component_type.clone(), supports_data_input);
            state.raw_style_docs.insert(component_type, raw);
        }
        Ok(())
    }

    /// 缓存 Agent 显式读取的组件数据说明书。
    pub(crate) fn record_component_data_contract(&self, response: &Value, expected_source_type: &str) -> Result<(), EvidenceViolation> {
        let parsed = parse_component_data_contract(response, expected_source_type)?;
        let mut state = self.lock();
        for (component_id, contract, raw) in parsed {
            state
                .data_capabilities
                .insert(contract.component_type.clone(), contract.supports_data_input);
            state.data_contracts.insert(component_id.clone(), contract);
            state.raw_data_contracts.insert(component_id, raw);
        }
        Ok(())
    }

    /// 缓存前端目录读取获得的完整可信组件目录。
    pub(crate) fn record_component_catalog(&self, response: &Value) -> Result<(), EvidenceViolation> {
        let version = required_text_field(response, "catalogVersion", "agent.evidence.component_catalog_invalid")?;
        if !version.starts_with(CATALOG_VERSION_PREFIX) {
            return Err(evidence_response_error(
                "agent.evidence.component_catalog_invalid",
                format!("不支持的组件目录版本：{version}"),
            ));
        }
        let components = response
            .get("components")
            .and_then(Value::as_array)
            .ok_or_else(|| evidence_response_error("agent.evidence.component_catalog_invalid", "组件目录返回值缺少 components 数组"))?;
        let mut catalog = HashSet::with_capacity(components.len());
        for component in components {
            let component_type = required_text_field(component, "componentType", "agent.evidence.component_catalog_invalid")?;
            if !catalog.insert(component_type.to_string()) {
                return Err(evidence_response_error(
                    "agent.evidence.component_catalog_invalid",
                    format!("组件目录包含重复类型：{component_type}"),
                ));
            }
        }
        let mut state = self.lock();
        state.component_catalog = Some(catalog);
        state.raw_component_catalog = Some(response.clone());
        Ok(())
    }

    /// 规格解析使用宿主预加载的可信目录，不要求 Agent 再读取目录。
    pub(crate) fn component_catalog_for_specs(&self) -> Result<Value, EvidenceViolation> {
        let state = self.lock();
        state
            .raw_component_catalog
            .clone()
            .ok_or_else(|| EvidenceViolation::new("agent.evidence.component_catalog_unavailable", "宿主当前可信组件目录不可用", None::<String>))
    }

    /// 校验恢复的模型可见目录回执；规格解析仍使用本轮宿主预加载的可信目录。
    pub(crate) fn validate_restored_component_catalog(response: &Value) -> Result<(), EvidenceViolation> {
        validate_catalog_view(response)
    }

    /// 判断本任务是否已经向 Agent 完整返回指定版本的组件规格。
    ///
    /// 规格证据按组件类型、意图和文档版本精确隔离。调用方只能在最终
    /// 响应成功生成后调用 [`record_complete_component_spec`]，因此缓存中
    /// 不会留下失败、截断或分页中间态。
    pub(crate) fn has_complete_component_spec(&self, component_type: &str, intent: &str, corpus_version: &str) -> bool {
        let Some(key) = component_spec_read_key(component_type, intent, corpus_version) else {
            return false;
        };
        self.lock().component_spec_reads.contains(&key)
    }

    /// 返回指定组件和意图当前记录的完整规格版本。
    ///
    /// 只有当前文档版本才可用于创建校验；旧版本证据不会被当作当前规格
    /// 使用。该方法主要供上下文恢复和诊断使用。
    pub(crate) fn component_spec_corpus_version(&self, component_type: &str, intent: &str) -> Option<String> {
        let current = component_docs_corpus_version();
        self.has_complete_component_spec(component_type, intent, current)
            .then(|| current.to_string())
    }

    pub(crate) fn validate_component_spec_for_create(&self, component_type: &str) -> Result<(), EvidenceViolation> {
        let component_type = component_type.trim();
        let state = self.lock();
        let requires_spec = state.data_capabilities.get(component_type).copied().unwrap_or(false);
        if !requires_spec {
            return Ok(());
        }
        let current_version = component_docs_corpus_version();
        let current_key = component_spec_read_key(component_type, "create", current_version);
        if current_key.as_ref().is_some_and(|key| state.component_spec_reads.contains(key)) {
            return Ok(());
        }

        let has_any_create_spec = state
            .component_spec_reads
            .iter()
            .any(|key| key.component_type == component_type && key.intent == "create");
        if !has_any_create_spec {
            return Err(EvidenceViolation::new(
                "agent.operation.component_spec_required",
                format!("创建数据组件 {component_type} 前必须读取当前版本的完整 create 规格"),
                None::<String>,
            ));
        }
        Err(EvidenceViolation::new(
            "agent.operation.component_spec_incomplete",
            format!("组件 {component_type} 的 create 规格不是当前文档版本 {current_version}"),
            None::<String>,
        ))
    }

    /// 记录一次已经成功形成的完整规格响应。
    pub(crate) fn record_complete_component_spec(&self, component_type: &str, intent: &str, corpus_version: &str) {
        let Some(key) = component_spec_read_key(component_type, intent, corpus_version) else {
            return;
        };
        self.lock().component_spec_reads.insert(key);
    }

    pub(crate) fn validate_style_patch(&self, component_type: &str, style: &Map<String, Value>) -> Result<(), EvidenceViolation> {
        let schema = self.lock().style_schemas.get(component_type).cloned().ok_or_else(|| {
            EvidenceViolation::new(
                "agent.operation.style_schema_required",
                format!("宿主当前未提供组件 {component_type} 的可信样式 Schema"),
                None::<String>,
            )
        })?;
        if schema.forbidden_function_config {
            reject_function_values(&Value::Object(style.clone()), "style")?;
        }
        let rules = rule_map(&schema.allowed_style_paths);
        validate_object("style", "style", style, &rules, ValidationMode::Style)
    }

    /// 校验组件数据或数据字段映射修改。
    pub(crate) fn validate_component_data_change(
        &self,
        component_id: &str,
        component_type: &str,
        data_patch: Option<&Map<String, Value>>,
        style_patch: Option<&Map<String, Value>>,
    ) -> Result<(), EvidenceViolation> {
        let contract = self.lock().data_contracts.get(component_id).cloned().ok_or_else(|| {
            EvidenceViolation::new(
                "agent.operation.component_data_contract_required",
                format!("修改组件 {component_id} 的数据或字段映射前必须先读取数据说明书"),
                None::<String>,
            )
        })?;
        validate_component_data_change(&contract, component_id, component_type, data_patch, style_patch)
    }

    /// 创建组件时校验显式传入的 config.data。
    pub(crate) fn validate_page_component_data(&self, component_type: &str, data: &Value) -> Result<(), EvidenceViolation> {
        let supports_data_input = self.lock().data_capabilities.get(component_type).copied().ok_or_else(|| {
            EvidenceViolation::new(
                "agent.operation.component_data_capability_required",
                format!("宿主当前未提供组件 {component_type} 的数据能力说明"),
                None::<String>,
            )
        })?;
        validate_page_component_data(component_type, supports_data_input, data)
    }

    pub(crate) fn validate_page_component_type(&self, component_type: &str) -> Result<(), EvidenceViolation> {
        let state = self.lock();
        let catalog = state
            .component_catalog
            .as_ref()
            .ok_or_else(|| EvidenceViolation::new("agent.operation.component_catalog_required", "宿主当前可信组件目录不可用", None::<String>))?;
        if catalog.contains(component_type) {
            Ok(())
        } else {
            Err(EvidenceViolation::new(
                "agent.operation.page_component_not_in_catalog",
                format!("组件类型不在可信组件目录中：{component_type}"),
                None::<String>,
            ))
        }
    }

    fn lock(&self) -> MutexGuard<'_, EvidenceState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn validate_catalog_view(response: &Value) -> Result<(), EvidenceViolation> {
    let version = required_text_field(response, "catalogVersion", "agent.evidence.component_catalog_invalid")?;
    if !version.starts_with(CATALOG_VERSION_PREFIX) {
        return Err(evidence_response_error(
            "agent.evidence.component_catalog_invalid",
            format!("不支持的组件目录版本：{version}"),
        ));
    }
    let components = response
        .get("components")
        .and_then(Value::as_array)
        .ok_or_else(|| evidence_response_error("agent.evidence.component_catalog_invalid", "组件目录返回值缺少 components 数组"))?;
    for component in components {
        required_text_field(component, "componentType", "agent.evidence.component_catalog_invalid")?;
    }
    Ok(())
}

fn component_spec_read_key(component_type: &str, intent: &str, corpus_version: &str) -> Option<ComponentSpecReadKey> {
    let component_type = component_type.trim();
    let intent = intent.trim();
    let corpus_version = corpus_version.trim();
    if component_type.is_empty() || intent.is_empty() || corpus_version.is_empty() {
        return None;
    }
    Some(ComponentSpecReadKey {
        component_type: component_type.to_string(),
        intent: intent.to_string(),
        corpus_version: corpus_version.to_string(),
    })
}

#[derive(Clone, Copy)]
enum ValidationMode {
    Style,
}

impl ValidationMode {
    fn code(self) -> &'static str {
        match self {
            Self::Style => "agent.operation.style_patch_not_allowed",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Style => "stylePatch",
        }
    }
}

fn parse_style_schema(content: &str) -> Result<ComponentStyleSchema, EvidenceViolation> {
    for block in fenced_code_blocks(content) {
        let block = strip_json_language(block);
        if !block.contains(STYLE_SCHEMA_VERSION) || !block.contains("\"schemaVersion\"") {
            continue;
        }
        let schema = serde_json::from_str::<ComponentStyleSchema>(block)
            .map_err(|error| evidence_response_error("agent.evidence.style_docs_invalid", format!("解析组件样式 Schema 失败：{error}")))?;
        if schema.schema_version != STYLE_SCHEMA_VERSION {
            return Err(evidence_response_error(
                "agent.evidence.style_docs_invalid",
                format!("不支持的组件样式 Schema 版本：{}", schema.schema_version),
            ));
        }
        return Ok(schema);
    }
    Err(evidence_response_error(
        "agent.evidence.style_docs_invalid",
        "组件说明文档缺少 component-style-doc/v1 JSON Schema 代码块",
    ))
}

fn fenced_code_blocks(content: &str) -> impl Iterator<Item = &str> {
    content
        .split("```")
        .enumerate()
        .filter_map(|(index, block)| (index % 2 == 1).then_some(block))
}

fn strip_json_language(block: &str) -> &str {
    let block = block.trim_start();
    match block.split_once('\n') {
        Some((language, body)) if language.trim().eq_ignore_ascii_case("json") => body.trim(),
        _ => block.trim(),
    }
}

fn validate_schema_rules(rules: &[ValueRule], required_prefix: &str, type_supported: fn(&str) -> bool, label: &str) -> Result<(), EvidenceViolation> {
    if rules.is_empty() {
        return Err(evidence_response_error("agent.evidence.schema_rules_invalid", format!("{label} Schema 的允许路径不能为空")));
    }
    let mut paths = HashSet::with_capacity(rules.len());
    for rule in rules {
        let path = rule.path.trim();
        if path.is_empty() || path != rule.path || !path.starts_with(required_prefix) || !paths.insert(path) {
            return Err(evidence_response_error(
                "agent.evidence.schema_rules_invalid",
                format!("{label} Schema 包含空路径、重复路径或非法路径：{}", rule.path),
            ));
        }
        if !type_supported(&rule.value_type) {
            return Err(evidence_response_error(
                "agent.evidence.schema_rules_invalid",
                format!("{label} Schema 使用了不支持的类型：{}", rule.value_type),
            ));
        }
        if rule.min.zip(rule.max).is_some_and(|(min, max)| min > max) || rule.min_items.zip(rule.max_items).is_some_and(|(min, max)| min > max) {
            return Err(evidence_response_error(
                "agent.evidence.schema_rules_invalid",
                format!("{label} Schema 范围上下界无效：{path}"),
            ));
        }
        if rule.value_type == "objectArray" {
            validate_object_field_rules(&rule.item_object_fields, label, path)?;
        }
    }
    Ok(())
}

fn validate_object_field_rules(fields: &[ObjectFieldRule], label: &str, path: &str) -> Result<(), EvidenceViolation> {
    let mut names = HashSet::with_capacity(fields.len());
    for field in fields {
        let name = field.name.trim();
        if name.is_empty() || name != field.name || !names.insert(name) || is_dangerous_key(name) {
            return Err(evidence_response_error(
                "agent.evidence.schema_rules_invalid",
                format!("{label} Schema 的对象数组字段无效：{path}.{}", field.name),
            ));
        }
        if !matches!(field.value_type.as_str(), "boolean" | "number" | "string" | "array" | "object")
            || field.min.zip(field.max).is_some_and(|(min, max)| min > max)
        {
            return Err(evidence_response_error(
                "agent.evidence.schema_rules_invalid",
                format!("{label} Schema 的对象数组字段规则无效：{path}.{name}"),
            ));
        }
    }
    Ok(())
}

fn style_rule_type_supported(value_type: &str) -> bool {
    matches!(
        value_type,
        "boolean"
            | "number"
            | "string"
            | "color"
            | "colorOrColorArray"
            | "array"
            | "numberOrArray"
            | "numberOrArrayOrAuto"
            | "numberOrString"
            | "string-or-string-array"
            | "null"
    )
}

fn rule_map(rules: &[ValueRule]) -> HashMap<&str, &ValueRule> {
    rules.iter().map(|rule| (rule.path.as_str(), rule)).collect()
}

fn validate_object(
    rule_parent_path: &str,
    display_parent_path: &str,
    object: &Map<String, Value>,
    rules: &HashMap<&str, &ValueRule>,
    mode: ValidationMode,
) -> Result<(), EvidenceViolation> {
    for (key, value) in object {
        if is_dangerous_key(key) {
            return Err(validation_error(mode, format!("{} 包含危险字段：{key}", mode.label()), join_path(display_parent_path, key)));
        }
        let rule_path = join_path(rule_parent_path, key);
        let display_path = join_path(display_parent_path, key);
        if let Value::Object(child) = value {
            let has_descendants = rules.keys().any(|candidate| candidate.starts_with(&format!("{rule_path}.")));
            if has_descendants {
                if child.is_empty() {
                    return Err(validation_error(mode, format!("{}.{display_path} 不能为空对象", mode.label()), display_path));
                }
                validate_object(&rule_path, &display_path, child, rules, mode)?;
                continue;
            }
        }
        if let Value::Array(items) = value {
            let wildcard_parent = format!("{rule_path}[]");
            let has_wildcard_descendants = rules
                .keys()
                .any(|candidate| candidate.starts_with(&format!("{wildcard_parent}.")));
            if has_wildcard_descendants {
                validate_value(&rule_path, &display_path, value, rules, mode)?;
                for (index, item) in items.iter().enumerate() {
                    let child = item.as_object().ok_or_else(|| {
                        validation_error(mode, format!("{}.{display_path}[{index}] 必须是对象", mode.label()), format!("{display_path}[{index}]"))
                    })?;
                    validate_object(&wildcard_parent, &format!("{display_path}[{index}]"), child, rules, mode)?;
                }
                continue;
            }
        }
        validate_value(&rule_path, &display_path, value, rules, mode)?;
    }
    Ok(())
}

fn validate_value(
    rule_path: &str,
    display_path: &str,
    value: &Value,
    rules: &HashMap<&str, &ValueRule>,
    mode: ValidationMode,
) -> Result<(), EvidenceViolation> {
    let rule = rules
        .get(rule_path)
        .ok_or_else(|| validation_error(mode, format!("{}.{display_path} 不在可信 Schema 允许范围内", mode.label()), display_path))?;
    if value.is_null() {
        return if rule.allow_null || rule.value_type == "null" {
            Ok(())
        } else {
            Err(validation_error(mode, format!("{}.{display_path} 不允许为 null", mode.label()), display_path))
        };
    }
    if value == &Value::Bool(false) && rule.allow_false {
        return Ok(());
    }

    match rule.value_type.as_str() {
        "boolean" => {
            ensure_type(value.is_boolean(), mode, display_path, "boolean")?;
            ensure_enum(value, rule.enum_values.as_deref(), mode, display_path)
        }
        "number" => {
            ensure_number(value, rule.min, rule.max, mode, display_path)?;
            ensure_enum(value, rule.enum_values.as_deref(), mode, display_path)
        }
        "string" | "color" => ensure_string(value, rule.enum_values.as_deref(), mode, display_path),
        "array" => ensure_array(value, rule.min_items, rule.max_items, mode, display_path),
        "object" => ensure_type(value.is_object(), mode, display_path, "object"),
        "objectArray" => ensure_object_array(value, rule, mode, display_path),
        "colorOrColorArray" | "string-or-string-array" => ensure_string_or_array(value, mode, display_path),
        "numberOrArray" => ensure_number_or_array(value, rule.min, rule.max, false, mode, display_path),
        "numberOrArrayOrAuto" => ensure_number_or_array(value, rule.min, rule.max, true, mode, display_path),
        "numberOrString" => {
            ensure_type(value.is_number() || value.is_string(), mode, display_path, "number 或 string")?;
            ensure_enum(value, rule.enum_values.as_deref(), mode, display_path)
        }
        "null" => Err(validation_error(mode, format!("{}.{display_path} 只能为 null", mode.label()), display_path)),
        _ => Err(validation_error(mode, format!("{}.{display_path} 使用了不支持的 Schema 类型", mode.label()), display_path)),
    }
}

fn ensure_type(valid: bool, mode: ValidationMode, path: &str, expected: &str) -> Result<(), EvidenceViolation> {
    if valid {
        Ok(())
    } else {
        Err(validation_error(mode, format!("{}.{path} 必须是 {expected}", mode.label()), path))
    }
}

fn ensure_number(value: &Value, min: Option<f64>, max: Option<f64>, mode: ValidationMode, path: &str) -> Result<(), EvidenceViolation> {
    let number = value
        .as_f64()
        .ok_or_else(|| validation_error(mode, format!("{}.{path} 必须是 number", mode.label()), path))?;
    if min.is_some_and(|min| number < min) || max.is_some_and(|max| number > max) {
        return Err(validation_error(mode, format!("{}.{path} 超出允许数值范围", mode.label()), path));
    }
    Ok(())
}

fn ensure_string(value: &Value, enum_values: Option<&[Value]>, mode: ValidationMode, path: &str) -> Result<(), EvidenceViolation> {
    ensure_type(value.is_string(), mode, path, "string")?;
    ensure_enum(value, enum_values, mode, path)
}

fn ensure_enum(value: &Value, enum_values: Option<&[Value]>, mode: ValidationMode, path: &str) -> Result<(), EvidenceViolation> {
    if enum_values.is_some_and(|values| !values.iter().any(|candidate| candidate == value)) {
        Err(validation_error(mode, format!("{}.{path} 不在允许枚举值内", mode.label()), path))
    } else {
        Ok(())
    }
}

fn ensure_array(
    value: &Value,
    min_items: Option<usize>,
    max_items: Option<usize>,
    mode: ValidationMode,
    path: &str,
) -> Result<(), EvidenceViolation> {
    let items = value
        .as_array()
        .ok_or_else(|| validation_error(mode, format!("{}.{path} 必须是 array", mode.label()), path))?;
    ensure_array_bounds(items.len(), min_items, max_items, mode, path)
}

fn ensure_array_bounds(
    len: usize,
    min_items: Option<usize>,
    max_items: Option<usize>,
    mode: ValidationMode,
    path: &str,
) -> Result<(), EvidenceViolation> {
    if min_items.is_some_and(|min| len < min) || max_items.is_some_and(|max| len > max) {
        Err(validation_error(mode, format!("{}.{path} 数组项数超出允许范围", mode.label()), path))
    } else {
        Ok(())
    }
}

fn ensure_string_or_array(value: &Value, mode: ValidationMode, path: &str) -> Result<(), EvidenceViolation> {
    if value.is_string() {
        return Ok(());
    }
    let items = value
        .as_array()
        .ok_or_else(|| validation_error(mode, format!("{}.{path} 必须是 string 或 string[]", mode.label()), path))?;
    if items.iter().all(Value::is_string) {
        Ok(())
    } else {
        Err(validation_error(mode, format!("{}.{path} 数组项必须是 string", mode.label()), path))
    }
}

fn ensure_number_or_array(
    value: &Value,
    min: Option<f64>,
    max: Option<f64>,
    allow_auto: bool,
    mode: ValidationMode,
    path: &str,
) -> Result<(), EvidenceViolation> {
    if allow_auto && value.as_str() == Some("auto") {
        return Ok(());
    }
    if value.is_number() {
        return ensure_number(value, min, max, mode, path);
    }
    let items = value
        .as_array()
        .ok_or_else(|| validation_error(mode, format!("{}.{path} 必须是 number 或 number[]", mode.label()), path))?;
    for (index, item) in items.iter().enumerate() {
        ensure_number(item, min, max, mode, &format!("{path}[{index}]"))?;
    }
    Ok(())
}

fn ensure_object_array(value: &Value, rule: &ValueRule, mode: ValidationMode, path: &str) -> Result<(), EvidenceViolation> {
    let items = value
        .as_array()
        .ok_or_else(|| validation_error(mode, format!("{}.{path} 必须是 object[]", mode.label()), path))?;
    ensure_array_bounds(items.len(), rule.min_items, rule.max_items, mode, path)?;
    let field_rules = rule
        .item_object_fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect::<HashMap<_, _>>();
    for (index, item) in items.iter().enumerate() {
        let object = item
            .as_object()
            .ok_or_else(|| validation_error(mode, format!("{}.{path}[{index}] 必须是对象", mode.label()), format!("{path}[{index}]")))?;
        for required in rule.item_object_fields.iter().filter(|field| field.required) {
            if !object.contains_key(&required.name) {
                return Err(validation_error(
                    mode,
                    format!("{}.{path}[{index}] 缺少必填字段 {}", mode.label(), required.name),
                    format!("{path}[{index}]"),
                ));
            }
        }
        for (field_name, field_value) in object {
            if is_dangerous_key(field_name) {
                return Err(validation_error(
                    mode,
                    format!("{}.{path}[{index}] 包含危险字段：{field_name}", mode.label()),
                    format!("{path}[{index}].{field_name}"),
                ));
            }
            let Some(field_rule) = field_rules.get(field_name.as_str()) else {
                if rule.allow_additional_item_fields {
                    continue;
                }
                return Err(validation_error(
                    mode,
                    format!("{}.{path}[{index}].{field_name} 不在可信 Schema 允许范围内", mode.label()),
                    format!("{path}[{index}].{field_name}"),
                ));
            };
            validate_object_field(field_value, field_rule, mode, &format!("{path}[{index}].{field_name}"))?;
        }
    }
    Ok(())
}

fn validate_object_field(value: &Value, rule: &ObjectFieldRule, mode: ValidationMode, path: &str) -> Result<(), EvidenceViolation> {
    if value.is_null() {
        return if rule.allow_null {
            Ok(())
        } else {
            Err(validation_error(mode, format!("{}.{path} 不允许为 null", mode.label()), path))
        };
    }
    match rule.value_type.as_str() {
        "boolean" => {
            ensure_type(value.is_boolean(), mode, path, "boolean")?;
            ensure_enum(value, rule.enum_values.as_deref(), mode, path)
        }
        "number" => {
            ensure_number(value, rule.min, rule.max, mode, path)?;
            ensure_enum(value, rule.enum_values.as_deref(), mode, path)
        }
        "string" => ensure_string(value, rule.enum_values.as_deref(), mode, path),
        "array" => ensure_type(value.is_array(), mode, path, "array"),
        "object" => ensure_type(value.is_object(), mode, path, "object"),
        _ => Err(validation_error(mode, format!("{}.{path} 使用了不支持的字段类型", mode.label()), path)),
    }
}

fn required_text_field<'a>(value: &'a Value, field: &str, code: &'static str) -> Result<&'a str, EvidenceViolation> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| evidence_response_error(code, format!("返回值缺少非空字段 {field}")))
}

fn required_bool_field(value: &Value, field: &str, code: &'static str) -> Result<bool, EvidenceViolation> {
    value
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| evidence_response_error(code, format!("返回值缺少布尔字段 {field}")))
}

fn evidence_response_error(code: &'static str, message: impl Into<String>) -> EvidenceViolation {
    EvidenceViolation::new(code, message, None::<String>)
}

fn validation_error(mode: ValidationMode, message: impl Into<String>, path: impl Into<String>) -> EvidenceViolation {
    EvidenceViolation::new(mode.code(), message, Some(path))
}

fn join_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

fn is_dangerous_key(key: &str) -> bool {
    matches!(key, "__proto__" | "prototype" | "constructor")
}

fn reject_function_values(value: &Value, path: &str) -> Result<(), EvidenceViolation> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                reject_function_values(child, &join_path(path, key))?;
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                reject_function_values(child, &format!("{path}[{index}]"))?;
            }
        }
        Value::String(text)
            if text.trim_start().starts_with("function ") || text.trim_start().starts_with("async function ") || text.contains("=>") =>
        {
            return Err(validation_error(ValidationMode::Style, format!("stylePatch.{path} 不能包含函数配置"), path));
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
#[path = "validation_evidence_tests.rs"]
mod tests;
