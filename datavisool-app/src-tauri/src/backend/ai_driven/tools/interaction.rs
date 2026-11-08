/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Rig 使用的类型化项目上下文与前端交互 Tool。

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use rig_core::tool::{Tool, ToolFailure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::error::AgentToolError;
use super::output::{fit_model_output, ModelOutputBudget};
use super::policy::AgentToolEffect;
use super::runtime::AgentToolRuntime;
use crate::backend::ai_driven::domain::validation_evidence::EvidenceViolation;

// Keep this re-export while callers migrate to `tools::task_intent`.

pub(crate) const INSPECT_CURRENT_PAGE_TOOL: &str = "inspect_current_page";
/// 暂不向 Agent 注册，保留供组件库规模扩大后恢复。
#[allow(dead_code)]
pub(crate) const SEARCH_AVAILABLE_COMPONENTS_TOOL: &str = "search_available_components";
pub(crate) const REQUEST_USER_INPUT_TOOL: &str = "request_user_input";

const REQUEST_USER_INPUT_DESCRIPTION: &str =
    "仅在缺少不可推断的用户决策时提出 1–3 个结构化问题；每题提供 2–3 个互斥选项，前端会自动提供自定义输入。该工具会暂停当前步骤并等待用户回答；调用时不得同时发起其他工具，依赖用户回答的动作必须在下一步骤调用。";

const FRONTEND_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub(crate) struct InspectCurrentPageTool(pub(crate) AgentToolRuntime);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum ComponentSnapshotField {
    Layer,
    Config,
    Base,
    Style,
    Data,
    Filter,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InspectCurrentPageArgs {
    #[serde(default)]
    component_ids: Vec<String>,
    fields: Vec<ComponentSnapshotField>,
    #[serde(default)]
    all_page: bool,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_page_component_limit")]
    limit: usize,
}

impl Tool for InspectCurrentPageTool {
    const NAME: &'static str = INSPECT_CURRENT_PAGE_TOOL;
    type Error = AgentToolError;
    type Args = InspectCurrentPageArgs;
    type Output = Value;

    fn description(&self) -> String {
        "按组件 ID 读取有限快照；修改全部页面组件时使用 allPage=true 和 offset/limit 分页读完整清单。只请求真正需要的字段。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "componentIds": { "type": "array", "items": { "type": "string" }, "maxItems": 10, "default": [] },
                "allPage": { "type": "boolean", "default": false, "description": "true 时分页读取当前页面全部组件" },
                "offset": { "type": "integer", "minimum": 0, "default": 0 },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 50 },
                "fields": {
                    "type": "array",
                    "items": { "type": "string", "enum": ["layer", "config", "base", "style", "data", "filter"] },
                    "minItems": 1
                }
            },
            "required": ["fields"],
            "additionalProperties": false
        })
    }

    async fn call(&self, mut args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0.context().authorize(Self::NAME, AgentToolEffect::Read, false)?;
        normalize_inspect_args(&mut args)?;
        let frontend = self.0.frontend()?;
        let response: Value = frontend
            .call("getComponentSnapshots", args, Some(FRONTEND_TIMEOUT), self.0.context())
            .await?;
        frontend.record_canvas_snapshot_response(&response)?;
        fit_model_output(response, ModelOutputBudget::Snapshot, "减少 componentIds 或 fields，并使用 offset/limit 分页")
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct SearchAvailableComponentsTool(pub(crate) AgentToolRuntime);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SearchAvailableComponentsArgs {
    #[serde(default)]
    queries: Option<Vec<String>>,
    #[serde(default)]
    limit_per_query: Option<usize>,
}

#[derive(Debug, Eq, PartialEq)]
struct NormalizedComponentSearchArgs {
    queries: Vec<String>,
    limit_per_query: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComponentCatalogItem {
    component_type: String,
    component_name: String,
    #[serde(default)]
    categorize: Option<String>,
    #[serde(default)]
    sub_categorize: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
    #[serde(default)]
    has_optimization_doc: bool,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
#[serde(rename_all = "camelCase")]
struct ComponentCatalogResponse {
    catalog_version: String,
    components: Vec<ComponentCatalogItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComponentSearchResult {
    catalog_version: String,
    components: Vec<ComponentSearchItem>,
    unmatched_queries: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComponentSearchItem {
    component_type: String,
    component_name: String,
    matched_queries: Vec<String>,
}

impl Tool for SearchAvailableComponentsTool {
    const NAME: &'static str = SEARCH_AVAILABLE_COMPONENTS_TOOL;
    type Error = AgentToolError;
    type Args = SearchAvailableComponentsArgs;
    type Output = Value;

    fn description(&self) -> String {
        "按一个或多个关键词检索设计器已注册且允许 AI 使用的组件类型；组件按 componentType 全局去重，并附带命中的关键词。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "queries": {
                    "type": "array",
                    "items": { "type": "string", "minLength": 1 },
                    "minItems": 1,
                    "maxItems": 50,
                    "description": "按类型、名称或分类筛选，最多 50 个关键词；重复关键词会自动合并"
                },
                "limitPerQuery": { "type": "integer", "minimum": 1, "maximum": 5, "default": 5 }
            },
            "required": ["queries"],
            "additionalProperties": false
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0
            .context()
            .authorize(Self::NAME, AgentToolEffect::FrontendInteraction, false)?;
        let normalized_args = normalize_component_search_args(args)?;
        let raw_response: Value = self
            .0
            .frontend()?
            .call("getComponentCatalog", json!({}), Some(FRONTEND_TIMEOUT), self.0.context())
            .await?;
        self.0
            .validation_evidence()
            .record_component_catalog(&raw_response)
            .map_err(evidence_tool_error)?;
        let response = serde_json::from_value::<ComponentCatalogResponse>(raw_response)
            .map_err(|error| AgentToolError::validation("agent.evidence.component_catalog_invalid", format!("组件目录返回值无效：{error}")))?;
        let (components, unmatched_queries) =
            search_component_catalog(&response.components, &normalized_args.queries, normalized_args.limit_per_query);
        let output = serde_json::to_value(ComponentSearchResult { catalog_version: response.catalog_version, components, unmatched_queries })
            .map_err(|error| AgentToolError::internal("agent.tool.serialize_failed", error.to_string()))?;
        fit_model_output(output, ModelOutputBudget::Catalog, "使用更具体的 queries 或减少 limitPerQuery")
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

#[derive(Clone)]
pub(crate) struct RequestUserInputTool(pub(crate) AgentToolRuntime);

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AskUserArgs {
    questions: Vec<UserInputQuestion>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UserInputQuestion {
    header: String,
    id: String,
    question: String,
    options: Vec<UserInputOption>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UserInputOption {
    label: String,
    description: String,
}

impl Tool for RequestUserInputTool {
    const NAME: &'static str = REQUEST_USER_INPUT_TOOL;
    type Error = AgentToolError;
    type Args = AskUserArgs;
    type Output = Value;

    fn description(&self) -> String {
        REQUEST_USER_INPUT_DESCRIPTION.to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array", "minItems": 1, "maxItems": 3,
                    "items": {
                        "type": "object",
                        "properties": {
                            "header": {"type": "string", "minLength": 1, "maxLength": 12},
                            "id": {"type": "string", "pattern": "^[a-z][a-z0-9_]{0,63}$"},
                            "question": {"type": "string", "minLength": 1, "maxLength": 300},
                            "options": {
                                "type": "array", "minItems": 2, "maxItems": 3,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": {"type": "string", "minLength": 1, "maxLength": 40},
                                        "description": {"type": "string", "minLength": 1, "maxLength": 160}
                                    },
                                    "required": ["label", "description"],
                                    "additionalProperties": false
                                }
                            }
                        },
                        "required": ["header", "id", "question", "options"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["questions"],
            "additionalProperties": false
        })
    }

    async fn call(&self, mut args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0
            .context()
            .authorize(Self::NAME, AgentToolEffect::FrontendInteraction, false)?;
        normalize_user_questions(&mut args.questions)?;
        self.0.frontend()?.call("userInput", args, None, self.0.context()).await
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

fn normalize_user_questions(questions: &mut [UserInputQuestion]) -> Result<(), AgentToolError> {
    if questions.is_empty() || questions.len() > 3 {
        return Err(AgentToolError::invalid("agent.user_input.question_count_invalid", "questions 数量必须在 1–3"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for question in questions {
        question.header = required_text(std::mem::take(&mut question.header), "header")?;
        question.id = required_text(std::mem::take(&mut question.id), "id")?;
        question.question = required_text(std::mem::take(&mut question.question), "question")?;
        if question.header.chars().count() > 12 || question.question.chars().count() > 300 {
            return Err(AgentToolError::invalid("agent.user_input.question_too_long", "header 或 question 超出长度限制"));
        }
        if !question.id.chars().enumerate().all(|(index, ch)| {
            (index == 0 && ch.is_ascii_lowercase()) || (index > 0 && (ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'))
        }) || question.id.len() > 64
            || !ids.insert(question.id.clone())
        {
            return Err(AgentToolError::invalid("agent.user_input.question_id_invalid", "question.id 必须是唯一 snake_case 标识"));
        }
        if question.options.len() < 2 || question.options.len() > 3 {
            return Err(AgentToolError::invalid("agent.user_input.option_count_invalid", "每题 options 数量必须在 2–3"));
        }
        let mut labels = std::collections::BTreeSet::new();
        for option in &mut question.options {
            option.label = required_text(std::mem::take(&mut option.label), "option.label")?;
            option.description = required_text(std::mem::take(&mut option.description), "option.description")?;
            if option.label.chars().count() > 40 || option.description.chars().count() > 160 || !labels.insert(option.label.clone()) {
                return Err(AgentToolError::invalid("agent.user_input.option_invalid", "选项必须唯一且符合长度限制"));
            }
        }
    }
    Ok(())
}

const MAX_COMPONENT_SEARCH_QUERIES: usize = 50;
const MAX_COMPONENT_SEARCH_RESULTS_PER_QUERY: usize = 5;

fn default_catalog_limit() -> usize {
    MAX_COMPONENT_SEARCH_RESULTS_PER_QUERY
}

fn normalize_component_search_args(args: SearchAvailableComponentsArgs) -> Result<NormalizedComponentSearchArgs, AgentToolError> {
    let queries = args.queries.unwrap_or_default();
    let mut normalized_queries = Vec::with_capacity(queries.len());
    let mut seen_queries = HashSet::with_capacity(queries.len());
    for query in queries {
        let query = query.trim().to_string();
        if !query.is_empty() && seen_queries.insert(query.to_lowercase()) {
            normalized_queries.push(query);
        }
    }
    if normalized_queries.is_empty() {
        return Err(AgentToolError::invalid("agent.tool.empty_list", "queries 去空后不能为空"));
    }
    if normalized_queries.len() > MAX_COMPONENT_SEARCH_QUERIES {
        return Err(AgentToolError::invalid("agent.tool.too_many_queries", "queries 去重后单次不能超过 50 个"));
    }
    let queries = normalized_queries;
    let limit_per_query = args.limit_per_query.unwrap_or_else(default_catalog_limit);
    if !(1..=MAX_COMPONENT_SEARCH_RESULTS_PER_QUERY).contains(&limit_per_query) {
        return Err(AgentToolError::invalid("agent.tool.catalog_limit_invalid", "limitPerQuery 必须在 1–5"));
    }

    Ok(NormalizedComponentSearchArgs { queries, limit_per_query })
}

fn default_page_component_limit() -> usize {
    50
}

fn normalize_inspect_args(args: &mut InspectCurrentPageArgs) -> Result<(), AgentToolError> {
    if args.all_page {
        args.component_ids.clear();
        args.fields = vec![ComponentSnapshotField::Layer];
        args.limit = args.limit.clamp(1, 50);
    } else {
        normalize_texts(&mut args.component_ids, "componentIds")?;
        if args.component_ids.len() > 10 {
            return Err(AgentToolError::invalid("agent.tool.too_many_components", "componentIds 单次不能超过 10 个"));
        }
        args.offset = 0;
    }
    if args.fields.is_empty() {
        return Err(AgentToolError::invalid("agent.tool.empty_fields", "fields 不能为空"));
    }
    Ok(())
}

fn normalize_texts(values: &mut Vec<String>, field: &str) -> Result<(), AgentToolError> {
    let mut normalized = Vec::new();
    for value in values.drain(..) {
        let value = value.trim().to_string();
        if !value.is_empty() && !normalized.contains(&value) {
            normalized.push(value);
        }
    }
    if normalized.is_empty() {
        return Err(AgentToolError::invalid("agent.tool.empty_list", format!("{field} 不能为空")));
    }
    *values = normalized;
    Ok(())
}

fn required_text(value: String, field: &str) -> Result<String, AgentToolError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        Err(AgentToolError::invalid("agent.tool.empty_text", format!("{field} 不能为空")))
    } else {
        Ok(value)
    }
}

fn evidence_tool_error(error: EvidenceViolation) -> AgentToolError {
    AgentToolError::validation(error.code, error.message)
}

fn search_component_catalog(catalog: &[ComponentCatalogItem], queries: &[String], limit_per_query: usize) -> (Vec<ComponentSearchItem>, Vec<String>) {
    let mut components = Vec::<ComponentSearchItem>::new();
    let mut component_indexes = HashMap::<String, usize>::new();
    let mut unmatched_queries = Vec::new();

    for query in queries {
        let normalized_query = query.to_lowercase();
        let matching_components = catalog
            .iter()
            .filter(|component| catalog_item_matches(component, &normalized_query))
            .take(limit_per_query)
            .collect::<Vec<_>>();
        if matching_components.is_empty() {
            unmatched_queries.push(query.clone());
            continue;
        }
        for component in matching_components {
            if let Some(index) = component_indexes.get(&component.component_type).copied() {
                if !components[index].matched_queries.contains(query) {
                    components[index].matched_queries.push(query.clone());
                }
            } else {
                component_indexes.insert(component.component_type.clone(), components.len());
                components.push(ComponentSearchItem {
                    component_type: component.component_type.clone(),
                    component_name: component.component_name.clone(),
                    matched_queries: vec![query.clone()],
                });
            }
        }
    }

    (components, unmatched_queries)
}

fn catalog_item_matches(item: &ComponentCatalogItem, query: &str) -> bool {
    [
        Some(item.component_type.as_str()),
        Some(item.component_name.as_str()),
        item.categorize.as_deref(),
        item.sub_categorize.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_page_inspection_is_forced_to_a_thin_paginated_list() {
        let mut args: InspectCurrentPageArgs = serde_json::from_value(json!({
            "componentIds": ["ignored"],
            "allPage": true,
            "offset": 50,
            "limit": 100,
            "fields": ["config", "style", "data"]
        }))
        .unwrap();

        normalize_inspect_args(&mut args).unwrap();

        assert!(args.component_ids.is_empty());
        assert_eq!(args.fields, vec![ComponentSnapshotField::Layer]);
        assert_eq!(args.limit, 50);
        assert_eq!(args.offset, 50);
    }

    #[test]
    fn empty_component_catalog_query_is_rejected() {
        let error = required_text("  ".to_string(), "query").unwrap_err();
        assert_eq!(error.code, "agent.tool.empty_text");
    }

    #[test]
    fn component_search_deduplicates_components_and_aggregates_matched_queries() {
        let catalog: Vec<ComponentCatalogItem> = serde_json::from_value(json!([
            { "componentType": "BaseButton", "componentName": "按钮", "categorize": "基础" },
            { "componentType": "IconButton", "componentName": "图标按钮", "categorize": "基础" },
            { "componentType": "BaseText", "componentName": "文本", "categorize": "基础" }
        ]))
        .unwrap();
        let queries = vec!["button".to_string(), "按钮".to_string(), "文本".to_string(), "缺失".to_string()];

        let (components, unmatched_queries) = search_component_catalog(&catalog, &queries, 2);

        assert_eq!(components.len(), 3);
        assert_eq!(components[0].component_type, "BaseButton");
        assert_eq!(components[0].matched_queries, vec!["button", "按钮"]);
        assert_eq!(components[1].component_type, "IconButton");
        assert_eq!(components[1].matched_queries, vec!["button", "按钮"]);
        assert_eq!(components[2].component_type, "BaseText");
        assert_eq!(components[2].matched_queries, vec!["文本"]);
        assert_eq!(unmatched_queries, vec!["缺失"]);

        let result = serde_json::to_value(ComponentSearchResult { catalog_version: "v1".to_string(), components, unmatched_queries }).unwrap();
        let component = result["components"][0].as_object().unwrap();
        assert_eq!(component.len(), 3);
        assert!(component.contains_key("componentType"));
        assert!(component.contains_key("componentName"));
        assert!(component.contains_key("matchedQueries"));
        assert!(!component.contains_key("categorize"));
        assert_eq!(result["unmatchedQueries"], json!(["缺失"]));
    }

    #[test]
    fn component_search_rejects_conflicts_and_invalid_boundaries() {
        assert!(serde_json::from_value::<SearchAvailableComponentsArgs>(json!({"query":"button"})).is_err());

        for payload in [json!({ "queries": [] }), json!({ "queries": ["  "] }), json!({})] {
            let args: SearchAvailableComponentsArgs = serde_json::from_value(payload).unwrap();
            assert!(normalize_component_search_args(args).is_err());
        }

        let fifty_queries = (1..=50).map(|index| index.to_string()).collect::<Vec<_>>();
        let allowed: SearchAvailableComponentsArgs = serde_json::from_value(json!({ "queries": fifty_queries })).unwrap();
        assert_eq!(normalize_component_search_args(allowed).unwrap().queries.len(), 50);

        let fifty_one_queries = (1..=51).map(|index| index.to_string()).collect::<Vec<_>>();
        let too_many_queries: SearchAvailableComponentsArgs = serde_json::from_value(json!({ "queries": fifty_one_queries })).unwrap();
        assert_eq!(normalize_component_search_args(too_many_queries).unwrap_err().code, "agent.tool.too_many_queries");

        let duplicate_queries: SearchAvailableComponentsArgs = serde_json::from_value(json!({
            "queries": [" Button ", "button", "文本"]
        }))
        .unwrap();
        assert_eq!(
            normalize_component_search_args(duplicate_queries).unwrap().queries,
            vec!["Button".to_string(), "文本".to_string()]
        );

        for payload in [
            json!({ "queries": ["button"], "limitPerQuery": 0 }),
            json!({ "queries": ["button"], "limitPerQuery": 6 }),
        ] {
            let args: SearchAvailableComponentsArgs = serde_json::from_value(payload).unwrap();
            assert_eq!(normalize_component_search_args(args).unwrap_err().code, "agent.tool.catalog_limit_invalid");
        }
    }

    #[test]
    fn interaction_tool_descriptions_define_step_boundaries() {
        assert!(REQUEST_USER_INPUT_DESCRIPTION.contains("暂停当前步骤并等待用户回答"));
        assert!(REQUEST_USER_INPUT_DESCRIPTION.contains("不得同时发起其他工具"));
        assert!(REQUEST_USER_INPUT_DESCRIPTION.contains("依赖用户回答的动作必须在下一步骤调用"));
    }
}
