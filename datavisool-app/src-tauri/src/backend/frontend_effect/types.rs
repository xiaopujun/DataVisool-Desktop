/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 前端异步交互协议类型。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Rust 请求前端执行一次交互动作的事件载荷。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrontendEffectRequest {
    /// 本次前端交互唯一 id，用于把响应路由回对应 Future。
    pub effect_id: String,
    /// Rust 为本次请求计算的作用域与参数绑定；前端回执必须原样回传。
    pub binding: FrontendEffectBinding,
    /// 本次交互类型。
    pub kind: FrontendEffectKind,
    /// 工具名或用户输入场景名。
    pub name: String,
    /// 前端执行工具或渲染输入面板需要的业务参数。
    pub payload: Value,
}

/// Rust 侧不再等待某个 Effect 时通知前端停止未完成交互。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrontendEffectCancelRequest {
    /// 要停止的 Effect id。
    pub effect_id: String,
    /// 稳定停止原因，例如 task_cancelled、timeout 或 request_dropped。
    pub reason: String,
}

/// Rust 请求前端执行的交互类型。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum FrontendEffectKind {
    /// 调用前端工具并等待返回结果。
    ToolCall,
    /// 请求用户补充输入并等待提交。
    UserInput,
}

/// 前端完成一次交互后回传给 Rust 的响应。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrontendEffectResponse {
    /// 对应请求中的 effect id。
    pub effect_id: String,
    /// 必须与请求中的 binding 完全匹配，防止跨任务或跨参数伪造回执。
    pub binding: FrontendEffectBinding,
    /// 前端是否成功完成交互。
    pub ok: bool,
    /// 成功时返回的业务结果。
    pub data: Option<Value>,
    /// 失败或用户取消时返回的错误信息。
    pub error: Option<FrontendEffectErrorPayload>,
}

/// Frontend Effect 的不可变绑定信息。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrontendEffectBinding {
    pub request_hash: String,
    pub tool_name: String,
    pub tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

/// 前端交互失败时回传的错误载荷。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrontendEffectErrorPayload {
    /// 稳定错误码，例如 user_cancelled 或 tool_failed。
    pub code: String,
    /// 给日志和上层错误提示使用的错误消息。
    pub message: String,
}
