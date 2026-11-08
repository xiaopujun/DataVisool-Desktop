/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Rust 调用方使用的前端异步交互 API。

use std::time::Duration;

use rand::RngCore;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;
use tokio::sync::watch;

use super::error::FrontendEffectError;
use super::registry::FrontendEffectRegistry;
use super::types::{FrontendEffectBinding, FrontendEffectCancelRequest, FrontendEffectKind, FrontendEffectRequest, FrontendEffectResponse};
use crate::backend::utils::sha256_hex;

/// 单次前端交互的等待配置。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FrontendEffectOptions {
    /// 本次交互最大等待时间；None 表示不主动超时。
    pub timeout: Option<Duration>,
}

/// 对调用方暴露的普通异步前端交互 API。
pub(crate) struct FrontendEffectApi<'a> {
    /// 等待前端响应的进程内注册表。
    registry: &'a FrontendEffectRegistry,
}

impl<'a> FrontendEffectApi<'a> {
    /// 创建绑定到指定注册表的前端交互 API。
    pub(crate) fn new(registry: &'a FrontendEffectRegistry) -> Self {
        Self { registry }
    }

    /// 调用前端工具，并使用调用方传入的等待配置。
    pub(crate) async fn call_tool_with_options<TReq, TResp>(
        &self,
        app_handle: &tauri::AppHandle,
        tool_name: impl Into<String>,
        request: TReq,
        options: FrontendEffectOptions,
    ) -> Result<TResp, FrontendEffectError>
    where
        TReq: Serialize + Send,
        TResp: DeserializeOwned + Send,
    {
        self.request(app_handle, FrontendEffectKind::ToolCall, tool_name.into(), request, options, None)
            .await
    }

    /// 调用前端工具，并在等待期间监听外部取消信号。
    pub(crate) async fn call_tool_with_cancel<TReq, TResp>(
        &self,
        app_handle: &tauri::AppHandle,
        tool_name: impl Into<String>,
        request: TReq,
        options: FrontendEffectOptions,
        cancel_rx: Option<watch::Receiver<bool>>,
    ) -> Result<TResp, FrontendEffectError>
    where
        TReq: Serialize + Send,
        TResp: DeserializeOwned + Send,
    {
        self.request(app_handle, FrontendEffectKind::ToolCall, tool_name.into(), request, options, cancel_rx)
            .await
    }

    /// 发送前端交互请求，并等待前端通过 command 回传结果。
    async fn request<TReq, TResp>(
        &self,
        app_handle: &tauri::AppHandle,
        kind: FrontendEffectKind,
        name: String,
        request: TReq,
        options: FrontendEffectOptions,
        cancel_rx: Option<watch::Receiver<bool>>,
    ) -> Result<TResp, FrontendEffectError>
    where
        TReq: Serialize + Send,
        TResp: DeserializeOwned + Send,
    {
        let effect_id = create_effect_id();
        let payload = serde_json::to_value(request).map_err(FrontendEffectError::EncodeRequest)?;
        let binding = effect_binding(&effect_id, &name, &payload)?;
        let receiver = self.registry.register(effect_id.clone(), binding.clone())?;
        let mut lifecycle = EffectLifecycleGuard::new(app_handle.clone(), self.registry.clone(), effect_id.clone());
        let event = FrontendEffectRequest { effect_id: effect_id.clone(), binding, kind, name, payload };

        if let Err(error) = app_handle.emit("datavisool:frontend-effect-request", event) {
            self.registry.remove(&effect_id);
            lifecycle.disarm();
            return Err(FrontendEffectError::EmitFailed(error.to_string()));
        }

        let response = match self.await_response(&effect_id, receiver, options, cancel_rx).await {
            Ok(response) => {
                lifecycle.disarm();
                response
            }
            Err(error) => {
                lifecycle.cancel(effect_cancel_reason(&error));
                return Err(error);
            }
        };
        decode_frontend_response(response)
    }

    /// 等待指定 effect 的前端响应，并在超时或取消时清理注册表。
    async fn await_response(
        &self,
        effect_id: &str,
        receiver: tokio::sync::oneshot::Receiver<FrontendEffectResponse>,
        options: FrontendEffectOptions,
        cancel_rx: Option<watch::Receiver<bool>>,
    ) -> Result<FrontendEffectResponse, FrontendEffectError> {
        let response_future = async {
            match options.timeout {
                Some(timeout) => tokio::time::timeout(timeout, receiver)
                    .await
                    .map_err(|_| FrontendEffectError::Timeout)?
                    .map_err(|_| FrontendEffectError::ResponseChannelClosed),
                None => receiver.await.map_err(|_| FrontendEffectError::ResponseChannelClosed),
            }
        };

        let Some(mut cancel_rx) = cancel_rx else {
            return response_future.await;
        };
        if *cancel_rx.borrow() {
            return Err(FrontendEffectError::Cancelled(effect_id.to_string()));
        }

        tokio::select! {
            _ = cancel_rx.changed() => {
                Err(FrontendEffectError::Cancelled(effect_id.to_string()))
            }
            response = response_future => response,
        }
    }
}

fn effect_cancel_reason(error: &FrontendEffectError) -> &'static str {
    match error {
        FrontendEffectError::Timeout => "timeout",
        FrontendEffectError::Cancelled(_) => "task_cancelled",
        _ => "request_failed",
    }
}

/// 即使调用方直接 drop Future，也同步清理 registry 并通知前端。
struct EffectLifecycleGuard {
    app_handle: tauri::AppHandle,
    registry: FrontendEffectRegistry,
    effect_id: String,
    active: bool,
}

impl EffectLifecycleGuard {
    fn new(app_handle: tauri::AppHandle, registry: FrontendEffectRegistry, effect_id: String) -> Self {
        Self { app_handle, registry, effect_id, active: true }
    }

    fn cancel(&mut self, reason: &str) {
        if !self.active {
            return;
        }
        self.registry.remove(&self.effect_id);
        let _ = self.app_handle.emit(
            "datavisool:frontend-effect-cancel",
            FrontendEffectCancelRequest { effect_id: self.effect_id.clone(), reason: reason.to_string() },
        );
        self.active = false;
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for EffectLifecycleGuard {
    fn drop(&mut self) {
        self.cancel("request_dropped");
    }
}

/// 把前端原始响应解码成调用方期望的类型。
fn decode_frontend_response<TResp>(response: FrontendEffectResponse) -> Result<TResp, FrontendEffectError>
where
    TResp: DeserializeOwned + Send,
{
    if !response.ok {
        return Err(FrontendEffectError::FrontendFailed(response.error));
    }

    let data = response.data.ok_or(FrontendEffectError::MissingResponseData)?;
    serde_json::from_value(data).map_err(FrontendEffectError::DecodeResponse)
}

/// 使用系统随机数生成不可预测的 effect id，避免可猜测的回执路由键。
fn create_effect_id() -> String {
    let mut bytes = [0_u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("frontend-effect-{}", bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>())
}

fn effect_binding(effect_id: &str, tool_name: &str, payload: &Value) -> Result<FrontendEffectBinding, FrontendEffectError> {
    let bytes = serde_json::to_vec(payload).map_err(FrontendEffectError::EncodeRequest)?;
    let scope = payload.get("agentScope").and_then(Value::as_object);
    Ok(FrontendEffectBinding {
        request_hash: format!("sha256:{}", sha256_hex(&bytes)),
        tool_name: tool_name.to_string(),
        tool_call_id: effect_id.to_string(),
        task_id: scope
            .and_then(|scope| scope.get("taskId"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        run_id: scope
            .and_then(|scope| scope.get("runId"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        turn_id: scope
            .and_then(|scope| scope.get("turnId"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::decode_frontend_response;
    use crate::backend::frontend_effect::types::{FrontendEffectBinding, FrontendEffectErrorPayload, FrontendEffectResponse};

    fn binding() -> FrontendEffectBinding {
        FrontendEffectBinding {
            request_hash: "hash".to_string(),
            tool_name: "tool".to_string(),
            tool_call_id: "effect".to_string(),
            task_id: None,
            run_id: None,
            turn_id: None,
        }
    }

    #[derive(Debug, Deserialize, Eq, PartialEq)]
    struct TestPayload {
        /// 测试响应值。
        value: i32,
    }

    #[test]
    fn decodes_success_response_data() {
        let result: TestPayload = decode_frontend_response(FrontendEffectResponse {
            effect_id: "effect-1".to_string(),
            binding: binding(),
            ok: true,
            data: Some(json!({ "value": 7 })),
            error: None,
        })
        .expect("response should decode");

        assert_eq!(result, TestPayload { value: 7 });
    }

    #[test]
    fn converts_frontend_error_to_api_error() {
        let error = decode_frontend_response::<TestPayload>(FrontendEffectResponse {
            effect_id: "effect-1".to_string(),
            binding: binding(),
            ok: false,
            data: None,
            error: Some(FrontendEffectErrorPayload { code: "user_cancelled".to_string(), message: "用户取消".to_string() }),
        })
        .expect_err("frontend error should fail");

        assert!(error.to_string().contains("user_cancelled"));
    }
}
