/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 模型目录加载与查询。
//!
//! 目录数据来自同目录下的 `ai_model_catalog.json`，包含 provider、模型能力、
//! 默认 token 等前端配置项。这里统一做懒加载和 provider 归一化查询。

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::backend::ai_model_provider::normalize_provider_key_for_storage;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
/// 单个模型在目录中的静态能力描述。
pub(crate) struct AiModelCatalogModel {
    pub model_name: String,
    pub label: String,
    pub default_max_tokens: Option<i64>,
    pub supports_image_input: bool,
    pub supports_tool_calling: bool,
    pub supports_reasoning_content: bool,
    pub is_default: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
/// 单个 provider 的目录配置。
pub(crate) struct AiModelCatalogProvider {
    pub provider: String,
    pub provider_label: String,
    pub provider_family: String,
    pub models: Vec<AiModelCatalogModel>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
/// `ai_model_catalog.json` 的文件级结构。
struct AiModelCatalogFile {
    providers: Vec<AiModelCatalogProvider>,
}

static AI_MODEL_CATALOG: OnceLock<Result<AiModelCatalogFile, String>> = OnceLock::new();

/// 懒加载模型目录。
///
/// 解析结果会被缓存；解析失败也会缓存错误，避免每次请求重复解析同一份静态文件。
fn load_ai_model_catalog() -> Result<&'static AiModelCatalogFile, String> {
    AI_MODEL_CATALOG
        .get_or_init(|| {
            serde_json::from_str(include_str!("ai_model_catalog.json")).map_err(|e| format!("aiModel.catalog: parse catalog failed: {e}"))
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// 返回所有 provider 目录配置，供前端初始化模型选择面板。
pub(crate) fn list_ai_model_catalog_providers() -> Result<Vec<AiModelCatalogProvider>, String> {
    load_ai_model_catalog().map(|catalog| catalog.providers.clone())
}

#[allow(dead_code)]
/// 按 provider 查找目录配置。
///
/// 输入会先走统一 provider key 归一化，兼容历史别名和大小写差异。
pub(crate) fn find_ai_model_catalog_provider(provider: &str) -> Result<Option<AiModelCatalogProvider>, String> {
    let normalized_provider = normalize_provider_key_for_storage(provider);
    load_ai_model_catalog().map(|catalog| {
        catalog
            .providers
            .iter()
            .find(|item| item.provider == normalized_provider)
            .cloned()
    })
}

#[allow(dead_code)]
/// 按 provider + model_name 查找单个模型配置。
pub(crate) fn resolve_ai_model_catalog_model(provider: &str, model_name: &str) -> Result<Option<AiModelCatalogModel>, String> {
    let Some(provider_catalog) = find_ai_model_catalog_provider(provider)? else {
        return Ok(None);
    };

    Ok(provider_catalog
        .models
        .into_iter()
        .find(|item| item.model_name == model_name.trim()))
}
