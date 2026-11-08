/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 模型配置页使用的 provider 元数据工具。
//!
//! 这里只保留配置管理需要的 provider 归一化、展示名和协议族判断，不包含任何模型调用逻辑。

/// 将用户输入或历史数据中的 provider 名称归一化为内部标准 key。
/// TODO(legacy-compat): 停止读取旧 provider 配置后，移除仅供旧数据使用的别名；保留当前输入规范化。
pub(crate) fn normalize_provider_key(value: &str) -> Option<&'static str> {
    let normalized = value.trim().to_lowercase();
    if normalized.is_empty() {
        return None;
    }

    match normalized.as_str() {
        "openai" => Some("openai"),
        "qwen" | "dashscope" => Some("qwen"),
        "kimi" | "moonshot" => Some("kimi"),
        "glm" | "zhipu" | "bigmodel" => Some("glm"),
        "minimax" | "mini-max" => Some("minimax"),
        "doubao" | "volcengine" | "ark" => Some("doubao"),
        "hunyuan" | "tencent" | "tencent-hunyuan" => Some("hunyuan"),
        "gemini" | "google" | "google-gemini" => Some("gemini"),
        "grok" | "xai" | "x-ai" => Some("grok"),
        "deepseek" => Some("deepseek"),
        "mimo" | "xiaomi" | "xiaomi-mimo" => Some("mimo"),
        "anthropic" | "claude" => Some("anthropic"),
        "ollama" => Some("ollama"),
        "compatible" | "openai-compatible" | "custom" => Some("compatible"),
        _ => None,
    }
}

/// 返回 provider 的前端展示名。
pub(crate) fn provider_display_name(provider: &str) -> String {
    match normalize_provider_key(provider) {
        Some("openai") => "OpenAI".to_string(),
        Some("qwen") => "千问 Qwen".to_string(),
        Some("kimi") => "Kimi".to_string(),
        Some("glm") => "智谱 GLM".to_string(),
        Some("minimax") => "MiniMax".to_string(),
        Some("doubao") => "豆包（火山方舟）".to_string(),
        Some("hunyuan") => "腾讯混元".to_string(),
        Some("gemini") => "Google Gemini".to_string(),
        Some("grok") => "xAI Grok".to_string(),
        Some("deepseek") => "DeepSeek".to_string(),
        Some("mimo") => "小米 MiMo".to_string(),
        Some("anthropic") => "Anthropic".to_string(),
        Some("ollama") => "Ollama".to_string(),
        Some("compatible") => "OpenAI 兼容".to_string(),
        None => {
            let trimmed = provider.trim();
            if trimmed.is_empty() {
                "自定义".to_string()
            } else {
                trimmed.to_string()
            }
        }
        _ => "OpenAI 兼容".to_string(),
    }
}

/// 返回 provider 所属协议族，仅用于配置页展示和持久化字段兼容。
pub(crate) fn provider_family(provider: &str) -> &'static str {
    match normalize_provider_key(provider) {
        Some("anthropic") => "anthropic",
        Some("gemini") => "gemini",
        Some("grok") => "xai-responses",
        Some("openai") | Some("qwen") | Some("kimi") | Some("glm") | Some("minimax") | Some("doubao") | Some("hunyuan") | Some("deepseek")
        | Some("mimo") | Some("ollama") | Some("compatible") => "openai-compatible",
        None => "openai-compatible",
        _ => "openai-compatible",
    }
}

/// 归一化后用于数据库持久化的 provider key。
pub(crate) fn normalize_provider_key_for_storage(provider: &str) -> String {
    match normalize_provider_key(provider) {
        Some(item) => item.to_string(),
        None => {
            let trimmed = provider.trim();
            if trimmed.is_empty() {
                "compatible".to_string()
            } else {
                trimmed.to_lowercase()
            }
        }
    }
}
