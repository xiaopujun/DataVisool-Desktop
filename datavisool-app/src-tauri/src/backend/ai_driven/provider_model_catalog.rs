/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 模型 Provider 元信息目录。

/// 当前后端支持的模型厂商。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderKind {
    /// OpenAI。
    OpenAi,
    /// 千问。
    Qwen,
    /// Kimi。
    Kimi,
    /// 智谱 GLM。
    Glm,
    /// MiniMax。
    MiniMax,
    /// 豆包（火山方舟）。
    Doubao,
    /// 腾讯混元。
    Hunyuan,
    /// Google Gemini。
    Gemini,
    /// xAI Grok。
    Grok,
    /// DeepSeek。
    DeepSeek,
    /// 小米 MiMo。
    Mimo,
    /// Anthropic。
    Anthropic,
}

impl ProviderKind {
    /// 返回稳定 provider key。
    pub(crate) fn key(self) -> &'static str {
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
            Self::DeepSeek => "deepseek",
            Self::Mimo => "mimo",
            Self::Anthropic => "anthropic",
        }
    }
}

/// Provider 元信息查询入口。
pub(crate) struct ProviderCatalog;

impl ProviderCatalog {
    /// 解析 provider key。
    pub(crate) fn parse_provider(provider: &str) -> Option<ProviderKind> {
        match provider.trim() {
            "openai" => Some(ProviderKind::OpenAi),
            "qwen" => Some(ProviderKind::Qwen),
            "kimi" | "moonshot" => Some(ProviderKind::Kimi),
            "glm" | "zhipu" | "bigmodel" => Some(ProviderKind::Glm),
            "minimax" | "mini-max" => Some(ProviderKind::MiniMax),
            "doubao" | "volcengine" | "ark" => Some(ProviderKind::Doubao),
            "hunyuan" | "tencent" | "tencent-hunyuan" => Some(ProviderKind::Hunyuan),
            "gemini" | "google" | "google-gemini" => Some(ProviderKind::Gemini),
            "grok" | "xai" | "x-ai" => Some(ProviderKind::Grok),
            "deepseek" => Some(ProviderKind::DeepSeek),
            "mimo" => Some(ProviderKind::Mimo),
            "anthropic" => Some(ProviderKind::Anthropic),
            _ => None,
        }
    }

    /// 根据 provider 返回协议族。
    pub(crate) fn provider_family(provider: ProviderKind) -> &'static str {
        match provider {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Gemini => "gemini",
            ProviderKind::Grok => "xai-responses",
            _ => "openai-compatible",
        }
    }

    /// 根据 provider 返回默认 base_url。
    pub(crate) fn default_base_url(provider: ProviderKind) -> &'static str {
        match provider {
            ProviderKind::OpenAi => "https://api.openai.com/v1",
            ProviderKind::Qwen => "https://dashscope.aliyuncs.com/compatible-mode/v1",
            ProviderKind::Kimi => "https://api.moonshot.cn/v1",
            ProviderKind::Glm => "https://open.bigmodel.cn/api/paas/v4",
            ProviderKind::MiniMax => "https://api.minimaxi.com/v1",
            ProviderKind::Doubao => "https://ark.cn-beijing.volces.com/api/v3",
            ProviderKind::Hunyuan => "https://api.hunyuan.cloud.tencent.com/v1",
            ProviderKind::Gemini => "https://generativelanguage.googleapis.com",
            ProviderKind::Grok => "https://api.x.ai/v1",
            ProviderKind::DeepSeek => "https://api.deepseek.com",
            ProviderKind::Mimo => "https://api.xiaomimimo.com/v1",
            ProviderKind::Anthropic => "https://api.anthropic.com",
        }
    }

    /// 根据 provider 和模型名兜底判断是否支持图片。
    pub(crate) fn infer_image_support(provider: ProviderKind, model_name: &str) -> bool {
        let model = model_name.to_ascii_lowercase();
        match provider {
            ProviderKind::Anthropic => true,
            ProviderKind::Qwen => {
                matches!(model.as_str(), "qwen3.8-max" | "qwen3.7-plus" | "qwen3.7-flash")
                    || model.contains("vl")
                    || model.contains("vision")
                    || model.contains("omni")
            }
            ProviderKind::Kimi => matches!(model.as_str(), "kimi-k3" | "kimi-k2.7-code" | "kimi-k2.7-code-highspeed" | "kimi-k2.6"),
            ProviderKind::Glm => model == "glm-5v-turbo",
            ProviderKind::MiniMax => model == "minimax-m3",
            ProviderKind::Doubao => model.starts_with("doubao-seed-2-0-lite-") || model.starts_with("doubao-seed-2-0-pro-"),
            ProviderKind::Hunyuan => model.contains("vision"),
            ProviderKind::Gemini => matches!(model.as_str(), "gemini-3.6-flash" | "gemini-3.5-flash-lite"),
            ProviderKind::Grok => model == "grok-4.6",
            ProviderKind::DeepSeek => model.contains("vl") || model.contains("vision"),
            ProviderKind::Mimo => model == "mimo-v2.5" || model.contains("vision") || model.contains("vl"),
            ProviderKind::OpenAi => {
                model.contains("gpt-4o") || model.contains("gpt-4.1") || model.contains("vision") || model.contains("o3") || model.contains("o4")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_provider_keys() {
        assert_eq!(ProviderCatalog::parse_provider("openai"), Some(ProviderKind::OpenAi));
        assert_eq!(ProviderCatalog::parse_provider("qwen"), Some(ProviderKind::Qwen));
        assert_eq!(ProviderCatalog::parse_provider("kimi"), Some(ProviderKind::Kimi));
        assert_eq!(ProviderCatalog::parse_provider("moonshot"), Some(ProviderKind::Kimi));
        assert_eq!(ProviderCatalog::parse_provider("glm"), Some(ProviderKind::Glm));
        assert_eq!(ProviderCatalog::parse_provider("bigmodel"), Some(ProviderKind::Glm));
        assert_eq!(ProviderCatalog::parse_provider("minimax"), Some(ProviderKind::MiniMax));
        assert_eq!(ProviderCatalog::parse_provider("doubao"), Some(ProviderKind::Doubao));
        assert_eq!(ProviderCatalog::parse_provider("volcengine"), Some(ProviderKind::Doubao));
        assert_eq!(ProviderCatalog::parse_provider("hunyuan"), Some(ProviderKind::Hunyuan));
        assert_eq!(ProviderCatalog::parse_provider("tencent"), Some(ProviderKind::Hunyuan));
        assert_eq!(ProviderCatalog::parse_provider("gemini"), Some(ProviderKind::Gemini));
        assert_eq!(ProviderCatalog::parse_provider("google"), Some(ProviderKind::Gemini));
        assert_eq!(ProviderCatalog::parse_provider("grok"), Some(ProviderKind::Grok));
        assert_eq!(ProviderCatalog::parse_provider("xai"), Some(ProviderKind::Grok));
        assert_eq!(ProviderCatalog::parse_provider("deepseek"), Some(ProviderKind::DeepSeek));
        assert_eq!(ProviderCatalog::parse_provider("mimo"), Some(ProviderKind::Mimo));
        assert_eq!(ProviderCatalog::parse_provider("anthropic"), Some(ProviderKind::Anthropic));
        assert_eq!(ProviderCatalog::parse_provider("unknown"), None);
    }

    #[test]
    fn returns_provider_runtime_metadata() {
        assert_eq!(ProviderCatalog::provider_family(ProviderKind::OpenAi), "openai-compatible");
        assert_eq!(ProviderCatalog::default_base_url(ProviderKind::Qwen), "https://dashscope.aliyuncs.com/compatible-mode/v1");
        assert_eq!(ProviderCatalog::default_base_url(ProviderKind::Kimi), "https://api.moonshot.cn/v1");
        assert_eq!(ProviderCatalog::default_base_url(ProviderKind::Glm), "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(ProviderCatalog::default_base_url(ProviderKind::MiniMax), "https://api.minimaxi.com/v1");
        assert_eq!(ProviderCatalog::default_base_url(ProviderKind::Doubao), "https://ark.cn-beijing.volces.com/api/v3");
        assert_eq!(ProviderCatalog::default_base_url(ProviderKind::Hunyuan), "https://api.hunyuan.cloud.tencent.com/v1");
        assert_eq!(ProviderCatalog::provider_family(ProviderKind::Gemini), "gemini");
        assert_eq!(ProviderCatalog::default_base_url(ProviderKind::Gemini), "https://generativelanguage.googleapis.com");
        assert_eq!(ProviderCatalog::provider_family(ProviderKind::Grok), "xai-responses");
        assert_eq!(ProviderCatalog::default_base_url(ProviderKind::Grok), "https://api.x.ai/v1");
        assert_eq!(ProviderCatalog::provider_family(ProviderKind::Anthropic), "anthropic");
        assert_eq!(ProviderCatalog::default_base_url(ProviderKind::DeepSeek), "https://api.deepseek.com");
    }

    #[test]
    fn preserves_existing_image_support_inference() {
        assert!(ProviderCatalog::infer_image_support(ProviderKind::Anthropic, "claude-sonnet"));
        assert!(ProviderCatalog::infer_image_support(ProviderKind::DeepSeek, "deepseek-vl"));
        assert!(ProviderCatalog::infer_image_support(ProviderKind::Mimo, "mimo-v2.5"));
        assert!(ProviderCatalog::infer_image_support(ProviderKind::OpenAi, "gpt-4o"));
        assert!(ProviderCatalog::infer_image_support(ProviderKind::Qwen, "qwen3.8-max"));
        assert!(ProviderCatalog::infer_image_support(ProviderKind::Kimi, "kimi-k3"));
        assert!(ProviderCatalog::infer_image_support(ProviderKind::Glm, "glm-5v-turbo"));
        assert!(ProviderCatalog::infer_image_support(ProviderKind::MiniMax, "MiniMax-M3"));
        assert!(!ProviderCatalog::infer_image_support(ProviderKind::MiniMax, "MiniMax-M2.7"));
        assert!(ProviderCatalog::infer_image_support(ProviderKind::Doubao, "doubao-seed-2-0-lite-260215"));
        assert!(!ProviderCatalog::infer_image_support(ProviderKind::Doubao, "doubao-seed-2-0-code-preview-260215"));
        assert!(ProviderCatalog::infer_image_support(ProviderKind::Hunyuan, "hunyuan-vision"));
        assert!(!ProviderCatalog::infer_image_support(ProviderKind::Hunyuan, "hunyuan-turbos-latest"));
        assert!(ProviderCatalog::infer_image_support(ProviderKind::Gemini, "gemini-3.6-flash"));
        assert!(ProviderCatalog::infer_image_support(ProviderKind::Grok, "grok-4.6"));
        assert!(!ProviderCatalog::infer_image_support(ProviderKind::Glm, "glm-5.2"));
        assert!(!ProviderCatalog::infer_image_support(ProviderKind::DeepSeek, "deepseek-chat"));
    }
}
