/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 前端异步交互错误类型。

use std::error::Error;
use std::fmt::{Display, Formatter};

use super::types::FrontendEffectErrorPayload;

/// 前端异步交互 API 的统一错误。
#[derive(Debug)]
pub(crate) enum FrontendEffectError {
    /// 生成了重复 effect id。
    DuplicateEffectId(String),
    /// Tauri 事件发送失败。
    EmitFailed(String),
    /// 等待期间响应 channel 被关闭。
    ResponseChannelClosed,
    /// 前端显式返回失败。
    FrontendFailed(Option<FrontendEffectErrorPayload>),
    /// 前端返回成功但缺少 data。
    MissingResponseData,
    /// 请求载荷序列化失败。
    EncodeRequest(serde_json::Error),
    /// 响应 data 无法反序列化为调用方类型。
    DecodeResponse(serde_json::Error),
    /// 前端回传了不存在的 effect id。
    UnknownEffectId(String),
    /// 回执未携带原始请求的任务和参数绑定。
    BindingMismatch(String),
    /// Rust 等待方已被丢弃。
    WaiterDropped,
    /// 前端交互等待超时。
    Timeout,
    /// 等待期间收到 AI Agent 取消信号。
    Cancelled(String),
    /// 当前运行上下文缺少前端交互能力。
    Unavailable(String),
}

impl Display for FrontendEffectError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateEffectId(effect_id) => {
                write!(formatter, "前端交互 effect_id 重复：{effect_id}")
            }
            Self::EmitFailed(message) => write!(formatter, "前端交互事件发送失败：{message}"),
            Self::ResponseChannelClosed => write!(formatter, "前端交互响应通道已关闭"),
            Self::FrontendFailed(Some(payload)) => {
                write!(formatter, "前端交互失败：{}: {}", payload.code, payload.message)
            }
            Self::FrontendFailed(None) => write!(formatter, "前端交互失败：缺少错误详情"),
            Self::MissingResponseData => write!(formatter, "前端交互成功响应缺少 data"),
            Self::EncodeRequest(error) => write!(formatter, "前端交互请求序列化失败：{error}"),
            Self::DecodeResponse(error) => write!(formatter, "前端交互响应反序列化失败：{error}"),
            Self::UnknownEffectId(effect_id) => {
                write!(formatter, "前端交互 effect_id 不存在：{effect_id}")
            }
            Self::BindingMismatch(effect_id) => write!(formatter, "前端交互回执绑定不匹配：{effect_id}"),
            Self::WaiterDropped => write!(formatter, "前端交互等待方已释放"),
            Self::Timeout => write!(formatter, "前端交互等待超时"),
            Self::Cancelled(stage) => write!(formatter, "AI Agent 任务已取消：{stage}"),
            Self::Unavailable(message) => write!(formatter, "前端交互能力不可用：{message}"),
        }
    }
}

impl Error for FrontendEffectError {
    /// 返回底层错误来源。
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::EncodeRequest(error) | Self::DecodeResponse(error) => Some(error),
            _ => None,
        }
    }
}
