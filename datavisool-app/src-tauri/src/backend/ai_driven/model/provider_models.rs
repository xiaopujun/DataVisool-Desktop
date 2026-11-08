/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI驱动可用模型型号静态配置加载与查询。
//!
//! 配置数据来自同目录下的 `provider_models.json`，用于给前端提供模型厂商及其
//! 可选具体型号，不读取用户数据库配置。

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::backend::ai_driven::provider_model_catalog::ProviderCatalog;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
/// 单个具体模型型号的静态描述。
pub(crate) struct AiDrivenProviderModel {
    /// 模型接口调用时使用的型号 id。
    pub model_name: String,
    /// 前端展示用模型名称。
    pub label: String,
    /// 是否为当前厂商下的默认型号。
    pub is_default: bool,
    /// 该型号是否支持图片理解输入。
    pub supports_image_input: bool,
    /// 模型可用的完整上下文窗口 token 数。
    pub context_window_tokens: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
/// 单个模型厂商及其可用型号列表。
pub(crate) struct AiDrivenProviderModelsProvider {
    /// 模型厂商标识。
    pub provider: String,
    /// 前端展示用模型厂商名称。
    pub provider_label: String,
    /// 厂商协议族，用于前端按协议区分调用适配。
    pub provider_family: String,
    /// 当前厂商下可选的具体模型型号。
    pub models: Vec<AiDrivenProviderModel>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
/// `provider_models.json` 的文件级结构。
struct AiDrivenProviderModelsFile {
    /// 文件中对外提供的模型厂商配置数组。
    providers: Vec<AiDrivenProviderModelsProvider>,
}

static AI_DRIVEN_PROVIDER_MODELS: OnceLock<Result<AiDrivenProviderModelsFile, String>> = OnceLock::new();

/// 懒加载 AI驱动模型型号配置。
///
/// 解析结果会被缓存；解析失败也会缓存错误，避免每次请求重复解析同一份静态文件。
fn load_ai_driven_provider_models() -> Result<&'static AiDrivenProviderModelsFile, String> {
    AI_DRIVEN_PROVIDER_MODELS
        .get_or_init(|| {
            let catalog: AiDrivenProviderModelsFile = serde_json::from_str(include_str!("provider_models.json"))
                .map_err(|e| format!("aiDriven.providerModels: parse provider models failed: {e}"))?;
            validate_provider_models_catalog(&catalog)?;
            Ok(catalog)
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// 返回 AI驱动静态配置中的 `providers` 数组。
pub(crate) fn list_ai_driven_provider_model_providers() -> Result<Vec<AiDrivenProviderModelsProvider>, String> {
    load_ai_driven_provider_models().map(|catalog| catalog.providers.clone())
}

/// 按厂商和型号查询 AI驱动静态模型配置。
pub(crate) fn find_ai_driven_provider_model(provider: &str, model_name: &str) -> Result<Option<&'static AiDrivenProviderModel>, String> {
    let provider = provider.trim();
    let model_name = model_name.trim();
    let catalog = load_ai_driven_provider_models()?;
    Ok(catalog
        .providers
        .iter()
        .find(|item| item.provider == provider)
        .and_then(|item| item.models.iter().find(|model| model.model_name == model_name)))
}

/// 校验静态模型列表和后端 ProviderCatalog 的 provider 元信息一致。
fn validate_provider_models_catalog(catalog: &AiDrivenProviderModelsFile) -> Result<(), String> {
    for provider in &catalog.providers {
        let kind = ProviderCatalog::parse_provider(&provider.provider)
            .ok_or_else(|| format!("aiDriven.providerModels: unsupported provider {}", provider.provider))?;

        if provider.provider != kind.key() {
            return Err(format!("aiDriven.providerModels: provider key {} should be {}", provider.provider, kind.key()));
        }

        let expected_family = ProviderCatalog::provider_family(kind);
        if provider.provider_family != expected_family {
            return Err(format!(
                "aiDriven.providerModels: provider {} family {} should be {}",
                provider.provider, provider.provider_family, expected_family
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_models_json_matches_provider_model_catalog() {
        let providers = list_ai_driven_provider_model_providers().unwrap();
        let deprecated_mimo_v2_models = ["mimo-v2-pro", "mimo-v2-omni", "mimo-v2-flash", "mimo-v2-tts"];

        assert!(!providers.is_empty());
        for provider in providers {
            let kind = ProviderCatalog::parse_provider(&provider.provider).unwrap();
            assert_eq!(provider.provider, kind.key());
            assert_eq!(provider.provider_family, ProviderCatalog::provider_family(kind));
            for model in provider.models {
                assert!(model.context_window_tokens > 0, "model {} must declare its context window", model.model_name);
                assert!(
                    !deprecated_mimo_v2_models.contains(&model.model_name.as_str()),
                    "deprecated MiMo model {} must not be selectable",
                    model.model_name
                );
            }
        }
    }

    #[test]
    fn finds_gpt_5_5_image_capability_from_static_catalog() {
        let model = find_ai_driven_provider_model("openai", "gpt-5.5").unwrap().unwrap();

        assert!(model.supports_image_input);
        assert_eq!(model.context_window_tokens, 1_050_000);
    }

    #[test]
    fn returns_none_for_model_missing_from_static_catalog() {
        assert!(find_ai_driven_provider_model("openai", "custom-model").unwrap().is_none());
    }
}
