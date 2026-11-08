/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 模型运行配置类型。

use serde::Deserialize;

/// 模型运行配置，全部由后端解析补齐。
#[derive(Clone, Debug)]
pub(crate) struct ModelRuntimeConfig {
    /// 标准 provider key，只允许 openai/deepseek/mimo/anthropic。
    pub provider: String,
    /// Provider 协议族，当前为 openai-compatible 或 anthropic。
    pub provider_family: String,
    /// 实际传给厂商的模型名。
    pub model_name: String,
    /// 厂商 API 基础地址。
    pub base_url: String,
    /// 解密后的 API Key。
    pub api_key: Option<String>,
    /// 当前模型是否支持图片输入。
    pub supports_image_input: bool,
    /// 模型温度参数。
    pub temperature: Option<f32>,
    /// 模型最大输出 token 数。
    pub max_tokens: Option<u32>,
    /// 模型完整上下文窗口；未知的自定义型号使用 Agent 的保守软阈值压缩。
    pub context_window_tokens: Option<u64>,
}

/// 前端已知的模型能力标记。
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelRuntimeCapabilities {
    /// 当前模型是否支持图片输入。
    pub supports_image_input: Option<bool>,
}
