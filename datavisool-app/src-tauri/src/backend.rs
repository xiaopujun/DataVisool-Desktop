/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 本地后端（Tauri Command）总入口。
//!
//! 该模块是前端和 Rust 本地后端之间的桥接层，负责接收前端发起的类 HTTP 请求，
//! 将请求包装为统一的 `HttpRequest`，并分发给 `backend/router.rs` 中的具体业务路由。
//! 它不直接承载业务 SQL 或资源处理逻辑，只负责请求生命周期管理、日志记录、取消信号挂接、
//! 异常兜底和统一 `HttpResponse` 返回。
//!
//! 请求处理链路：
//! 1. 前端通过 Tauri command 调用 `datavisool_http_request`。
//! 2. 本模块生成本地 request id，记录请求元信息，并订阅可选的取消信号。
//! 3. 请求交给 `router::dispatch` 按 method/url 分发到具体 handler。
//! 4. handler 返回统一响应后，本模块按响应码记录完成、取消或业务错误日志。
//! 5. 如果路由层抛出未预期错误，本模块将其转换为 500 响应，避免 Rust 错误直接泄漏到前端。
//!
//! 为什么要拆分：
//! - 原来的 `backend.rs` 同时承担了：协议类型、工具函数、路由分发、项目 SQL、若干占位接口；
//!   随着接口增多会迅速失控（可读性差、改动容易互相影响）。
//!
//! 当前拆分后的职责边界：
//! - `backend/types.rs`：前后端桥接协议（HttpRequest/HttpResponse）+ ok/err 统一返回
//! - `backend/utils.rs`：与业务无关的小工具（例如从 URL 解析 id）
//! - `backend/router.rs`：method/url 分发 + 参数抽取（薄层，不写 SQL）
//! - `backend/routes/*`：按资源/能力划分的业务处理器（写 SQL/组装 data）
//!
//! 扩展方式（新增一个 API）：
//! 1) 在 `backend/routes/xxx.rs` 写一个 handler（最好按资源维度命名文件）
//! 2) 在 `backend/router.rs` 增加一条 method/url 分发规则指向该 handler

mod ai_driven;
mod ai_model_catalog;
mod ai_model_provider;
mod crypto;
pub(crate) mod frontend_effect;
mod request_cancel;
mod router;
mod routes;
mod types;
mod utils;

pub(crate) use ai_driven::cleanup_expired_agent_attachments;
pub(crate) use ai_driven::thread::ThreadService;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tauri::{AppHandle, State};

use crate::app_state::AppState;
use crate::backend::types::err_with_detail;
use crate::db::DbState;
use crate::logging;

pub(crate) use frontend_effect::FrontendEffectRegistry;
pub(crate) use frontend_effect::{FrontendEffectApi, FrontendEffectError, FrontendEffectOptions};
pub(crate) use request_cancel::HttpRequestCancelRegistry;
pub use types::{HttpRequest, HttpResponse};

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// 前端统一 HTTP 桥接入口。
///
/// 该命令接收前端组装的 `HttpRequest`，记录请求日志、挂接取消信号，然后交给路由层分发。
#[tauri::command]
pub async fn datavisool_http_request(
    app_handle: AppHandle,
    db_state: State<'_, DbState>,
    app_state: State<'_, AppState>,
    request_cancels: State<'_, HttpRequestCancelRegistry>,
    frontend_effects: State<'_, FrontendEffectRegistry>,
    req: HttpRequest,
) -> Result<HttpResponse, String> {
    let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let method = req.method.to_uppercase();
    let url = req.url.clone();
    let header_count = req.headers.as_ref().map_or(0, |headers| headers.len());
    let body_bytes = req.body.as_ref().map_or(0, |body| body.to_string().len());
    let started_at = Instant::now();
    let request_cancel_id = req.request_id;
    // 如果前端传入 requestId，则为本次请求订阅对应取消信号。
    let request_cancel_rx = request_cancel_id.map(|request_id| request_cancels.subscribe_or_register(request_id));

    if request_cancel_rx.as_ref().is_some_and(|cancel_rx| *cancel_rx.borrow()) {
        if let Some(request_id) = request_cancel_id {
            request_cancels.unregister(request_id);
        }
        tracing::info!(
            target: "desktop_backend::request",
            request_id,
            method = %method,
            url = %url,
            "desktop backend request cancelled before dispatch"
        );
        return Ok(crate::backend::types::err(499, "请求已取消"));
    }

    tracing::info!(
        target: "desktop_backend::request",
        request_id,
        method = %method,
        url = %url,
        header_count,
        body_bytes,
        "desktop backend request started"
    );

    // 路由层只返回统一 HttpResponse；未捕获的 Rust 错误在这里转成 500 业务响应。
    let res = match router::dispatch(&app_handle, db_state, app_state, frontend_effects, req, request_cancel_rx).await {
        Ok(res) => res,
        Err(e) => {
            let elapsed_ms = started_at.elapsed().as_millis() as u64;

            tracing::error!(
                target: "desktop_backend::request",
                request_id,
                method = %method,
                url = %url,
                elapsed_ms,
                error = %e,
                backtrace = %logging::capture_backtrace(),
                "desktop backend request failed unexpectedly"
            );

            err_with_detail(500, "请求处理失败，请稍后重试。如仍失败请检查日志或配置", e)
        }
    };

    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if let Some(request_id) = request_cancel_id {
        request_cancels.unregister(request_id);
    }

    // 按响应 code 分级记录日志，便于定位取消、业务错误和正常请求。
    if res.code == 499 {
        tracing::info!(
            target: "desktop_backend::request",
            request_id,
            method = %method,
            url = %url,
            response_code = res.code,
            elapsed_ms,
            message = %res.msg,
            "desktop backend request cancelled"
        );
    } else if res.code >= 400 {
        tracing::warn!(
            target: "desktop_backend::request",
            request_id,
            method = %method,
            url = %url,
            response_code = res.code,
            elapsed_ms,
            message = %res.msg,
            "desktop backend request completed with business error"
        );
    } else {
        tracing::info!(
            target: "desktop_backend::request",
            request_id,
            method = %method,
            url = %url,
            response_code = res.code,
            elapsed_ms,
            "desktop backend request completed"
        );
    }

    Ok(res)
}

/// 预注册一个可取消请求 id。
#[tauri::command]
pub fn datavisool_register_http_request(request_cancels: State<'_, HttpRequestCancelRegistry>, request_id: u64) {
    let _ = request_cancels.subscribe_or_register(request_id);
}

/// 标记指定请求为已取消。
#[tauri::command]
pub fn datavisool_cancel_http_request(request_cancels: State<'_, HttpRequestCancelRegistry>, request_id: u64) {
    let _ = request_cancels.cancel(request_id);
}

/// 注销指定请求 id，清理取消信号注册表。
#[tauri::command]
pub fn datavisool_unregister_http_request(request_cancels: State<'_, HttpRequestCancelRegistry>, request_id: u64) {
    request_cancels.unregister(request_id);
}
