/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Rig Provider 配置边界。

use std::fmt;

/// Rig 适配层支持的模型厂商。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RigProviderKind {
    OpenAi,
    Qwen,
    Kimi,
    Glm,
    MiniMax,
    Doubao,
    Hunyuan,
    Gemini,
    Grok,
    Anthropic,
    DeepSeek,
    Mimo,
}

impl RigProviderKind {
    /// 解析项目稳定的 Provider key。
    pub(crate) fn parse(value: &str) -> Result<Self, RigModelConfigError> {
        match value.trim() {
            "openai" => Ok(Self::OpenAi),
            "qwen" => Ok(Self::Qwen),
            "kimi" => Ok(Self::Kimi),
            "glm" => Ok(Self::Glm),
            "minimax" => Ok(Self::MiniMax),
            "doubao" => Ok(Self::Doubao),
            "hunyuan" => Ok(Self::Hunyuan),
            "gemini" => Ok(Self::Gemini),
            "grok" => Ok(Self::Grok),
            "anthropic" => Ok(Self::Anthropic),
            "deepseek" => Ok(Self::DeepSeek),
            "mimo" => Ok(Self::Mimo),
            other => Err(RigModelConfigError::UnsupportedProvider(other.to_string())),
        }
    }

    /// 返回项目稳定的 Provider key。
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Qwen => "qwen",
            Self::Kimi => "kimi",
            Self::Glm => "glm",
            Self::MiniMax => "minimax",
            Self::Doubao => "doubao",
            Self::Hunyuan => "hunyuan",
            Self::Gemini => "gemini",
            Self::Grok => "grok",
            Self::Anthropic => "anthropic",
            Self::DeepSeek => "deepseek",
            Self::Mimo => "mimo",
        }
    }
}

/// 已校验、可用于创建 Rig Client 的运行配置。
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct RigModelConfig {
    provider: RigProviderKind,
    api_key: String,
    base_url: String,
    model_name: String,
}

impl RigModelConfig {
    /// 从现有模型运行配置的稳定字段创建 Rig 配置。
    pub(crate) fn try_from_runtime_parts(
        provider: &str,
        api_key: Option<&str>,
        base_url: &str,
        model_name: &str,
    ) -> Result<Self, RigModelConfigError> {
        let provider = RigProviderKind::parse(provider)?;
        let api_key = required(api_key.unwrap_or_default(), "api_key")?;
        let base_url = required(base_url, "base_url")?;
        let model_name = required(model_name, "model_name")?;

        Ok(Self { provider, api_key, base_url, model_name })
    }

    pub(crate) const fn provider(&self) -> RigProviderKind {
        self.provider
    }

    pub(crate) fn api_key(&self) -> &str {
        &self.api_key
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn model_name(&self) -> &str {
        &self.model_name
    }
}

/// 配置错误不携带 API Key，避免敏感信息进入日志。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RigModelConfigError {
    UnsupportedProvider(String),
    MissingField(&'static str),
}

impl fmt::Display for RigModelConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedProvider(provider) => write!(formatter, "不支持的 Rig 模型厂商：{provider}"),
            Self::MissingField(field) => write!(formatter, "Rig 模型配置字段为空：{field}"),
        }
    }
}

impl std::error::Error for RigModelConfigError {}

impl fmt::Debug for RigModelConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RigModelConfig")
            .field("provider", &self.provider)
            .field("api_key", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .field("model_name", &self.model_name)
            .finish()
    }
}

fn required(value: &str, field: &'static str) -> Result<String, RigModelConfigError> {
    let value = value.trim();
    if value.is_empty() {
        Err(RigModelConfigError::MissingField(field))
    } else {
        Ok(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_all_supported_runtime_provider_keys() {
        for (provider, expected) in [
            ("openai", RigProviderKind::OpenAi),
            ("qwen", RigProviderKind::Qwen),
            ("kimi", RigProviderKind::Kimi),
            ("glm", RigProviderKind::Glm),
            ("minimax", RigProviderKind::MiniMax),
            ("doubao", RigProviderKind::Doubao),
            ("hunyuan", RigProviderKind::Hunyuan),
            ("gemini", RigProviderKind::Gemini),
            ("grok", RigProviderKind::Grok),
            ("anthropic", RigProviderKind::Anthropic),
            ("deepseek", RigProviderKind::DeepSeek),
            ("mimo", RigProviderKind::Mimo),
        ] {
            let config = RigModelConfig::try_from_runtime_parts(provider, Some(" secret "), " https://provider.example/v1 ", " model-id ")
                .expect("supported provider should convert");

            assert_eq!(config.provider(), expected);
            assert_eq!(config.api_key(), "secret");
            assert_eq!(config.base_url(), "https://provider.example/v1");
            assert_eq!(config.model_name(), "model-id");
        }
    }

    #[test]
    fn rejects_unsupported_provider_and_missing_required_fields() {
        assert_eq!(
            RigModelConfig::try_from_runtime_parts("unknown", Some("key"), "https://example.com", "model").unwrap_err(),
            RigModelConfigError::UnsupportedProvider("unknown".to_string())
        );
        assert_eq!(
            RigModelConfig::try_from_runtime_parts("openai", None, "https://example.com", "model").unwrap_err(),
            RigModelConfigError::MissingField("api_key")
        );
        assert_eq!(
            RigModelConfig::try_from_runtime_parts("openai", Some("key"), " ", "model").unwrap_err(),
            RigModelConfigError::MissingField("base_url")
        );
        assert_eq!(
            RigModelConfig::try_from_runtime_parts("openai", Some("key"), "https://example.com", " ").unwrap_err(),
            RigModelConfigError::MissingField("model_name")
        );
    }

    #[test]
    fn debug_output_redacts_api_key() {
        let config = RigModelConfig::try_from_runtime_parts("openai", Some("top-secret"), "https://example.com", "model").unwrap();
        let debug = format!("{config:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("top-secret"));
    }
}
