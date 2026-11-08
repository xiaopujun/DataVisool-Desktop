/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 模型调用错误类型。

/// 模型调用层统一错误结构。
#[derive(Clone, Debug)]
pub(crate) struct ModelInvocationError {
    /// 稳定错误码。
    pub code: String,
    /// 发生错误的 provider；配置解析阶段可能为空。
    pub provider: Option<String>,
    /// 上游 HTTP 状态码；本地校验错误没有该字段。
    pub status_code: Option<u16>,
    /// 该错误是否适合由调用方重试。
    pub retryable: bool,
    /// 面向日志和上层状态机的简短错误原因。
    pub message: String,
}

impl ModelInvocationError {
    /// 构造本地参数或配置校验错误。
    pub(crate) fn validation(message: impl Into<String>) -> Self {
        Self {
            code: "model_invocation.failed".to_string(),
            provider: None,
            status_code: None,
            retryable: false,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ModelInvocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.provider, self.status_code) {
            (Some(provider), Some(status)) => write!(f, "{provider}({status}): {}", self.message),
            (Some(provider), None) => write!(f, "{provider}: {}", self.message),
            (None, Some(status)) => write!(f, "model invocation({status}): {}", self.message),
            (None, None) => write!(f, "model invocation: {}", self.message),
        }
    }
}

impl std::error::Error for ModelInvocationError {}
