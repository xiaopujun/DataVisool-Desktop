/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 模型配置解析器，负责补齐厂商调用配置。

use crate::backend::ai_driven::model::find_ai_driven_provider_model;
use crate::backend::ai_driven::model_invocation::types::{ModelInvocationError, ModelRuntimeCapabilities, ModelRuntimeConfig};
use crate::backend::ai_driven::provider_model_catalog::{ProviderCatalog, ProviderKind};

use super::store::ModelConfigStore;

/// 模型配置解析器。
pub(crate) struct ModelConfigResolver;

impl ModelConfigResolver {
    /// 创建模型配置解析器。
    pub(crate) fn new() -> Self {
        Self
    }

    /// 根据模型配置记录 id 和实际模型型号补齐运行配置。
    pub(crate) async fn resolve(
        &self,
        store: &dyn ModelConfigStore,
        model_config_id: &str,
        runtime_model_id: &str,
        capabilities: Option<&ModelRuntimeCapabilities>,
    ) -> Result<ModelRuntimeConfig, ModelInvocationError> {
        let config_id = parse_i64_id(model_config_id, "model_config_id")?;
        let model_name = normalize_required(runtime_model_id, "runtime_model_id")?;
        let stored = store.load_model_config(config_id).await?;
        let provider_kind = ProviderCatalog::parse_provider(&stored.provider)
            .ok_or_else(|| ModelInvocationError::validation(format!("不支持的模型厂商：{}", stored.provider.trim())))?;
        let provider = provider_kind.key().to_string();
        let base_url = stored
            .base_url
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .unwrap_or_else(|| ProviderCatalog::default_base_url(provider_kind).to_string());
        let supports_image_input = capabilities
            .and_then(|item| item.supports_image_input)
            .unwrap_or_else(|| ProviderCatalog::infer_image_support(provider_kind, &model_name));
        let context_window_tokens = find_ai_driven_provider_model(&provider, &model_name)
            .map_err(ModelInvocationError::validation)?
            .map(|model| model.context_window_tokens);
        // Kimi 和 Gemini 3.6/3.5 系列不允许传递温度参数，统一交由服务端处理。
        let temperature = if matches!(provider_kind, ProviderKind::Kimi | ProviderKind::Gemini) {
            None
        } else if matches!(provider_kind, ProviderKind::Glm) {
            normalize_temperature(stored.temperature).map(|value| value.min(1.0))
        } else {
            normalize_temperature(stored.temperature)
        };

        Ok(ModelRuntimeConfig {
            provider_family: ProviderCatalog::provider_family(provider_kind).to_string(),
            provider,
            model_name,
            base_url,
            api_key: stored.api_key,
            supports_image_input,
            temperature,
            max_tokens: normalize_max_tokens(stored.max_tokens),
            context_window_tokens,
        })
    }
}

/// 解析正整数 id。
fn parse_i64_id(value: &str, field: &str) -> Result<i64, ModelInvocationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ModelInvocationError::validation(format!("{field} 不能为空")));
    }
    value
        .parse::<i64>()
        .map_err(|e| ModelInvocationError::validation(format!("{field} 不合法：{e}")))
}

/// 读取必填字符串。
fn normalize_required(value: &str, field: &str) -> Result<String, ModelInvocationError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        Err(ModelInvocationError::validation(format!("{field} 不能为空")))
    } else {
        Ok(value)
    }
}

/// 规整温度参数。
fn normalize_temperature(value: f64) -> Option<f32> {
    if value.is_finite() {
        Some(value.clamp(0.0, 2.0) as f32)
    } else {
        None
    }
}

/// 规整最大 token 参数。
fn normalize_max_tokens(value: Option<i64>) -> Option<u32> {
    value.and_then(|item| if item > 0 { Some(item as u32) } else { None })
}

#[cfg(test)]
mod tests {
    use super::super::{ModelConfigStoreFuture, StoredModelConfig};
    use super::*;

    struct FakeModelConfigStore {
        config: StoredModelConfig,
    }

    impl ModelConfigStore for FakeModelConfigStore {
        fn load_model_config(&self, config_id: i64) -> ModelConfigStoreFuture<'_> {
            Box::pin(async move {
                assert_eq!(config_id, 7);
                Ok(self.config.clone())
            })
        }
    }

    #[tokio::test]
    async fn resolve_uses_store_without_desktop_state() {
        let store = FakeModelConfigStore {
            config: StoredModelConfig {
                provider: "deepseek".to_string(),
                base_url: None,
                api_key: Some("test-key".to_string()),
                temperature: 3.5,
                max_tokens: Some(-1),
            },
        };

        let resolved = ModelConfigResolver::new()
            .resolve(&store, "7", "deepseek-chat", Some(&ModelRuntimeCapabilities { supports_image_input: Some(true) }))
            .await
            .expect("config should resolve");

        assert_eq!(resolved.provider, "deepseek");
        assert_eq!(resolved.model_name, "deepseek-chat");
        assert_eq!(resolved.api_key, Some("test-key".to_string()));
        assert_eq!(resolved.supports_image_input, true);
        assert_eq!(resolved.temperature, Some(2.0));
        assert_eq!(resolved.max_tokens, None);
        assert_eq!(resolved.context_window_tokens, None);
        assert!(!resolved.base_url.is_empty());
    }

    #[tokio::test]
    async fn resolves_context_window_from_agent_model_catalog() {
        let store = FakeModelConfigStore {
            config: StoredModelConfig { provider: "mimo".to_string(), base_url: None, api_key: None, temperature: 0.2, max_tokens: None },
        };

        let resolved = ModelConfigResolver::new()
            .resolve(&store, "7", "mimo-v2.5-pro", None)
            .await
            .expect("catalog model should resolve");

        assert_eq!(resolved.context_window_tokens, Some(1_000_000));
        assert_eq!(resolved.max_tokens, None);
    }

    #[tokio::test]
    async fn omits_kimi_temperature_to_use_the_server_default() {
        let store = FakeModelConfigStore {
            config: StoredModelConfig { provider: "kimi".to_string(), base_url: None, api_key: None, temperature: 0.7, max_tokens: None },
        };

        let resolved = ModelConfigResolver::new()
            .resolve(&store, "7", "kimi-k3", None)
            .await
            .expect("Kimi config should resolve");

        assert_eq!(resolved.base_url, "https://api.moonshot.cn/v1");
        assert_eq!(resolved.temperature, None);
        assert_eq!(resolved.context_window_tokens, Some(1_000_000));
    }

    #[tokio::test]
    async fn omits_gemini_temperature_for_interactions_api() {
        let store = FakeModelConfigStore {
            config: StoredModelConfig {
                provider: "gemini".to_string(),
                base_url: None,
                api_key: Some("test-key".to_string()),
                temperature: 0.7,
                max_tokens: None,
            },
        };

        let resolved = ModelConfigResolver::new()
            .resolve(&store, "7", "gemini-3.6-flash", None)
            .await
            .expect("Gemini config should resolve");

        assert_eq!(resolved.base_url, "https://generativelanguage.googleapis.com");
        assert_eq!(resolved.temperature, None);
        assert!(resolved.supports_image_input);
        assert_eq!(resolved.context_window_tokens, Some(1_048_576));
    }

    #[tokio::test]
    async fn resolves_grok_responses_runtime_config() {
        let store = FakeModelConfigStore {
            config: StoredModelConfig {
                provider: "grok".to_string(),
                base_url: None,
                api_key: Some("test-key".to_string()),
                temperature: 0.7,
                max_tokens: None,
            },
        };

        let resolved = ModelConfigResolver::new()
            .resolve(&store, "7", "grok-4.6", None)
            .await
            .expect("Grok config should resolve");

        assert_eq!(resolved.provider, "grok");
        assert_eq!(resolved.provider_family, "xai-responses");
        assert_eq!(resolved.base_url, "https://api.x.ai/v1");
        assert_eq!(resolved.temperature, Some(0.7));
        assert!(resolved.supports_image_input);
        assert_eq!(resolved.context_window_tokens, Some(500_000));
    }

    #[tokio::test]
    async fn clamps_glm_temperature_to_the_supported_range() {
        let store = FakeModelConfigStore {
            config: StoredModelConfig { provider: "glm".to_string(), base_url: None, api_key: None, temperature: 1.8, max_tokens: None },
        };

        let resolved = ModelConfigResolver::new()
            .resolve(&store, "7", "glm-5.2", None)
            .await
            .expect("GLM config should resolve");

        assert_eq!(resolved.base_url, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(resolved.temperature, Some(1.0));
        assert_eq!(resolved.context_window_tokens, Some(1_000_000));
    }

    #[tokio::test]
    async fn resolves_minimax_openai_compatible_runtime_config() {
        let store = FakeModelConfigStore {
            config: StoredModelConfig {
                provider: "minimax".to_string(),
                base_url: None,
                api_key: Some("test-key".to_string()),
                temperature: 1.0,
                max_tokens: None,
            },
        };

        let resolved = ModelConfigResolver::new()
            .resolve(&store, "7", "MiniMax-M3", None)
            .await
            .expect("MiniMax config should resolve");

        assert_eq!(resolved.provider, "minimax");
        assert_eq!(resolved.provider_family, "openai-compatible");
        assert_eq!(resolved.base_url, "https://api.minimaxi.com/v1");
        assert!(resolved.supports_image_input);
        assert_eq!(resolved.context_window_tokens, Some(1_000_000));
    }

    #[tokio::test]
    async fn resolves_doubao_openai_compatible_runtime_config() {
        let store = FakeModelConfigStore {
            config: StoredModelConfig {
                provider: "doubao".to_string(),
                base_url: None,
                api_key: Some("test-key".to_string()),
                temperature: 0.7,
                max_tokens: None,
            },
        };

        let resolved = ModelConfigResolver::new()
            .resolve(&store, "7", "doubao-seed-2-0-lite-260215", None)
            .await
            .expect("Doubao config should resolve");

        assert_eq!(resolved.provider, "doubao");
        assert_eq!(resolved.provider_family, "openai-compatible");
        assert_eq!(resolved.base_url, "https://ark.cn-beijing.volces.com/api/v3");
        assert!(resolved.supports_image_input);
        assert_eq!(resolved.context_window_tokens, Some(256_000));
    }

    #[tokio::test]
    async fn resolves_hunyuan_openai_compatible_runtime_config() {
        let store = FakeModelConfigStore {
            config: StoredModelConfig {
                provider: "hunyuan".to_string(),
                base_url: None,
                api_key: Some("test-key".to_string()),
                temperature: 0.7,
                max_tokens: None,
            },
        };

        let resolved = ModelConfigResolver::new()
            .resolve(&store, "7", "hunyuan-vision", None)
            .await
            .expect("Hunyuan config should resolve");

        assert_eq!(resolved.provider, "hunyuan");
        assert_eq!(resolved.provider_family, "openai-compatible");
        assert_eq!(resolved.base_url, "https://api.hunyuan.cloud.tencent.com/v1");
        assert!(resolved.supports_image_input);
        assert_eq!(resolved.context_window_tokens, Some(32_000));
    }
}
