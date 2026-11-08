/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 具体 Rig Client 和 CompletionModel 工厂。

use std::fmt;
use std::time::Duration;

use rig_core::client::CompletionClient;
use rig_core::http_client::ReqwestClient;
use rig_core::providers::{anthropic, deepseek, gemini, openai, xai, xiaomimimo};

use crate::backend::ai_driven::settings::DEFAULT_MODEL_REQUEST_TIMEOUT_SECONDS;

use super::config::{RigModelConfig, RigModelConfigError, RigProviderKind};

pub(crate) type OpenAiRigModel = openai::completion::CompletionModel<ReqwestClient>;
pub(crate) type AnthropicRigModel = anthropic::completion::CompletionModel<ReqwestClient>;
pub(crate) type DeepSeekRigModel = deepseek::CompletionModel<ReqwestClient>;
pub(crate) type MimoRigModel = openai::completion::GenericCompletionModel<xiaomimimo::XiaomiMimoExt, ReqwestClient>;
pub(crate) type GeminiRigModel = gemini::interactions_api::InteractionsCompletionModel<ReqwestClient>;
pub(crate) type GrokRigModel = xai::CompletionModel<ReqwestClient>;

/// 多类 Provider 的统一枚举，避免 Rig 的泛型模型类型扩散到业务层。
pub(crate) enum RigCompletionModel {
    OpenAi(OpenAiRigModel),
    Anthropic(AnthropicRigModel),
    DeepSeek(DeepSeekRigModel),
    Mimo(MimoRigModel),
    Gemini(GeminiRigModel),
    Grok(GrokRigModel),
}

/// 已构建的 Rig 模型及其非敏感运行元信息。
pub(crate) struct BuiltRigModel {
    provider: RigProviderKind,
    model_name: String,
    base_url: String,
    model: RigCompletionModel,
}

impl BuiltRigModel {
    pub(crate) const fn provider(&self) -> RigProviderKind {
        self.provider
    }

    pub(crate) fn model_name(&self) -> &str {
        &self.model_name
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn model(&self) -> &RigCompletionModel {
        &self.model
    }

    pub(crate) fn into_model(self) -> RigCompletionModel {
        self.model
    }
}

/// 使用统一 HTTP 超时构建具体 Rig 模型。
#[derive(Clone)]
pub(crate) struct RigModelFactory {
    http_client: ReqwestClient,
}

impl RigModelFactory {
    pub(crate) fn new() -> Result<Self, RigModelFactoryError> {
        Self::with_request_timeout(Duration::from_secs(DEFAULT_MODEL_REQUEST_TIMEOUT_SECONDS))
    }

    pub(crate) fn with_request_timeout(request_timeout: Duration) -> Result<Self, RigModelFactoryError> {
        let http_client = ReqwestClient::builder()
            .timeout(request_timeout)
            .build()
            .map_err(|error| RigModelFactoryError::HttpClientBuild(error.to_string()))?;
        Ok(Self { http_client })
    }

    /// 直接接收现有运行配置字段，供迁移期调用方使用。
    pub(crate) fn build_from_runtime_parts(
        &self,
        provider: &str,
        api_key: Option<&str>,
        base_url: &str,
        model_name: &str,
    ) -> Result<BuiltRigModel, RigModelFactoryError> {
        let config = RigModelConfig::try_from_runtime_parts(provider, api_key, base_url, model_name)?;
        self.build(config)
    }

    /// 构建具体 Rig Client，并立即绑定运行时模型名。
    pub(crate) fn build(&self, config: RigModelConfig) -> Result<BuiltRigModel, RigModelFactoryError> {
        let provider = config.provider();
        let model_name = config.model_name().to_string();
        let (model, base_url) = match provider {
            RigProviderKind::OpenAi => self.build_openai(&config)?,
            RigProviderKind::Qwen => self.build_openai(&config)?,
            RigProviderKind::Kimi => self.build_openai(&config)?,
            RigProviderKind::Glm => self.build_openai(&config)?,
            RigProviderKind::MiniMax => self.build_openai(&config)?,
            RigProviderKind::Doubao => self.build_openai(&config)?,
            RigProviderKind::Hunyuan => self.build_openai(&config)?,
            RigProviderKind::Gemini => self.build_gemini(&config)?,
            RigProviderKind::Grok => self.build_grok(&config)?,
            RigProviderKind::Anthropic => self.build_anthropic(&config)?,
            RigProviderKind::DeepSeek => self.build_deepseek(&config)?,
            RigProviderKind::Mimo => self.build_mimo(&config)?,
        };

        Ok(BuiltRigModel { provider, model_name, base_url, model })
    }

    fn build_openai(&self, config: &RigModelConfig) -> Result<(RigCompletionModel, String), RigModelFactoryError> {
        // 现有 OpenAI 链路使用 Chat Completions；显式选择 CompletionsClient，避免切换到 Rig 默认 Responses API。
        let client = openai::CompletionsClient::builder()
            .api_key(config.api_key())
            .base_url(config.base_url())
            .http_client(self.http_client.clone())
            .build()
            .map_err(|error| client_build_error(config.provider(), error))?;
        let base_url = client.base_url().to_string();
        Ok((RigCompletionModel::OpenAi(client.completion_model(config.model_name())), base_url))
    }

    fn build_gemini(&self, config: &RigModelConfig) -> Result<(RigCompletionModel, String), RigModelFactoryError> {
        let client = gemini::InteractionsClient::builder()
            .api_key(config.api_key())
            .base_url(config.base_url())
            .http_client(self.http_client.clone())
            .build()
            .map_err(|error| client_build_error(RigProviderKind::Gemini, error))?;
        let base_url = client.base_url().to_string();

        Ok((
            RigCompletionModel::Gemini(gemini::interactions_api::InteractionsCompletionModel::with_model(client, config.model_name())),
            base_url,
        ))
    }

    fn build_grok(&self, config: &RigModelConfig) -> Result<(RigCompletionModel, String), RigModelFactoryError> {
        let client = xai::Client::builder()
            .api_key(config.api_key())
            .base_url(config.base_url())
            .http_client(self.http_client.clone())
            .build()
            .map_err(|error| client_build_error(RigProviderKind::Grok, error))?;
        let base_url = client.base_url().to_string();
        Ok((RigCompletionModel::Grok(xai::CompletionModel::new(client, config.model_name())), base_url))
    }

    fn build_anthropic(&self, config: &RigModelConfig) -> Result<(RigCompletionModel, String), RigModelFactoryError> {
        let client = anthropic::Client::builder()
            .api_key(config.api_key())
            .base_url(config.base_url())
            .http_client(self.http_client.clone())
            .build()
            .map_err(|error| client_build_error(config.provider(), error))?;
        let base_url = client.base_url().to_string();
        Ok((RigCompletionModel::Anthropic(client.completion_model(config.model_name())), base_url))
    }

    fn build_deepseek(&self, config: &RigModelConfig) -> Result<(RigCompletionModel, String), RigModelFactoryError> {
        let client = deepseek::Client::builder()
            .api_key(config.api_key())
            .base_url(config.base_url())
            .http_client(self.http_client.clone())
            .build()
            .map_err(|error| client_build_error(config.provider(), error))?;
        let base_url = client.base_url().to_string();
        Ok((RigCompletionModel::DeepSeek(client.completion_model(config.model_name())), base_url))
    }

    fn build_mimo(&self, config: &RigModelConfig) -> Result<(RigCompletionModel, String), RigModelFactoryError> {
        let client = xiaomimimo::Client::builder()
            .api_key(config.api_key())
            .base_url(config.base_url())
            .http_client(self.http_client.clone())
            .build()
            .map_err(|error| client_build_error(config.provider(), error))?;
        let base_url = client.base_url().to_string();
        Ok((RigCompletionModel::Mimo(client.completion_model(config.model_name())), base_url))
    }
}

fn client_build_error(provider: RigProviderKind, error: impl fmt::Display) -> RigModelFactoryError {
    RigModelFactoryError::ClientBuild { provider, message: error.to_string() }
}

#[derive(Debug)]
pub(crate) enum RigModelFactoryError {
    InvalidConfig(RigModelConfigError),
    HttpClientBuild(String),
    ClientBuild { provider: RigProviderKind, message: String },
}

impl fmt::Display for RigModelFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(error) => error.fmt(formatter),
            Self::HttpClientBuild(message) => write!(formatter, "创建 Rig HTTP Client 失败：{message}"),
            Self::ClientBuild { provider, message } => write!(formatter, "创建 {} Rig Client 失败：{message}", provider.as_str()),
        }
    }
}

impl std::error::Error for RigModelFactoryError {}

impl From<RigModelConfigError> for RigModelFactoryError {
    fn from(value: RigModelConfigError) -> Self {
        Self::InvalidConfig(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_all_provider_models_with_custom_base_urls() {
        let factory = RigModelFactory::new().expect("HTTP client should build");
        let cases = [
            ("openai", "gpt-test", "https://openai-compatible.example/v1", RigProviderKind::OpenAi),
            ("qwen", "qwen3.7-plus", "https://dashscope.aliyuncs.com/compatible-mode/v1", RigProviderKind::Qwen),
            ("kimi", "kimi-k3", "https://api.moonshot.cn/v1", RigProviderKind::Kimi),
            ("glm", "glm-5.2", "https://open.bigmodel.cn/api/paas/v4", RigProviderKind::Glm),
            ("minimax", "MiniMax-M3", "https://api.minimaxi.com/v1", RigProviderKind::MiniMax),
            ("doubao", "doubao-seed-2-0-lite-260215", "https://ark.cn-beijing.volces.com/api/v3", RigProviderKind::Doubao),
            ("hunyuan", "hunyuan-turbos-latest", "https://api.hunyuan.cloud.tencent.com/v1", RigProviderKind::Hunyuan),
            ("gemini", "gemini-3.6-flash", "https://generativelanguage.googleapis.com", RigProviderKind::Gemini),
            ("grok", "grok-4.6", "https://api.x.ai/v1", RigProviderKind::Grok),
            ("anthropic", "claude-test", "https://anthropic-compatible.example/v1/messages", RigProviderKind::Anthropic),
            ("deepseek", "deepseek-test", "https://deepseek-compatible.example/v1", RigProviderKind::DeepSeek),
            ("mimo", "mimo-test", "https://mimo-compatible.example/v1", RigProviderKind::Mimo),
        ];

        for (provider, model_name, base_url, expected_provider) in cases {
            let built = factory
                .build_from_runtime_parts(provider, Some("test-key"), base_url, model_name)
                .expect("provider model should build");

            assert_eq!(built.provider(), expected_provider);
            assert_eq!(built.model_name(), model_name);
            match expected_provider {
                RigProviderKind::Anthropic => assert_eq!(built.base_url(), "https://anthropic-compatible.example"),
                _ => assert_eq!(built.base_url(), base_url),
            }
            assert!(matches!(
                (expected_provider, built.model()),
                (RigProviderKind::OpenAi, RigCompletionModel::OpenAi(_))
                    | (RigProviderKind::Qwen, RigCompletionModel::OpenAi(_))
                    | (RigProviderKind::Kimi, RigCompletionModel::OpenAi(_))
                    | (RigProviderKind::Glm, RigCompletionModel::OpenAi(_))
                    | (RigProviderKind::MiniMax, RigCompletionModel::OpenAi(_))
                    | (RigProviderKind::Doubao, RigCompletionModel::OpenAi(_))
                    | (RigProviderKind::Hunyuan, RigCompletionModel::OpenAi(_))
                    | (RigProviderKind::Gemini, RigCompletionModel::Gemini(_))
                    | (RigProviderKind::Grok, RigCompletionModel::Grok(_))
                    | (RigProviderKind::Anthropic, RigCompletionModel::Anthropic(_))
                    | (RigProviderKind::DeepSeek, RigCompletionModel::DeepSeek(_))
                    | (RigProviderKind::Mimo, RigCompletionModel::Mimo(_))
            ));
        }
    }

    #[test]
    fn rejects_invalid_api_key_before_model_creation() {
        let factory = RigModelFactory::new().unwrap();
        let error = factory
            .build_from_runtime_parts("openai", Some("line-one\nline-two"), "https://example.com/v1", "model")
            .err()
            .expect("invalid header value should fail");

        assert!(matches!(error, RigModelFactoryError::ClientBuild { provider: RigProviderKind::OpenAi, .. }));
    }
}
