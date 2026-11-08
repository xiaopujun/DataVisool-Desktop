/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 本模块只放“跨路由通用”的请求/响应结构，以及统一的成功/失败响应构造器。
//!
//! 拆分原则：
//! - 路由函数只关心“业务数据”和“DB 操作”，不重复组装 `{code,msg,data}`。
//! - `HttpRequest/HttpResponse` 作为前后端桥接协议类型，应集中在一处，便于后续演进字段。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::utils::sanitize_error_message;

/// 前端收到的统一响应结构。
#[derive(Debug, Serialize)]
pub struct HttpResponse {
    pub code: i32,
    pub msg: String,
    pub data: Value,
}

/// 前端通过 Tauri command 传入的类 HTTP 请求结构。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpRequest {
    pub url: String,
    pub method: String,
    pub headers: Option<std::collections::HashMap<String, String>>,
    pub body: Option<Value>,
    #[serde(default)]
    pub request_id: Option<u64>,
}

/// 构造成功响应。
pub(crate) fn ok(data: Value) -> HttpResponse {
    HttpResponse { code: 200, msg: "操作成功".to_string(), data }
}

/// 构造失败响应。
pub(crate) fn err(code: i32, msg: impl Into<String>) -> HttpResponse {
    HttpResponse { code, msg: msg.into(), data: Value::Null }
}

/// 构造带安全明细的失败响应。
pub(crate) fn err_with_detail(code: i32, user_msg: impl Into<String>, detail: impl AsRef<str>) -> HttpResponse {
    let user_msg = user_msg.into();
    let detail = sanitize_error_message(detail.as_ref());
    if detail.trim().is_empty() {
        err(code, user_msg)
    } else {
        err(code, format!("{user_msg}（{detail}）"))
    }
}

/// 将字符串参数解析成 i64，并统一返回安全的 400 响应。
pub(crate) fn parse_i64_param(raw: &str, field: &str) -> Result<i64, HttpResponse> {
    raw.parse::<i64>()
        .map_err(|e| err_with_detail(400, format!("请求参数错误：{field} 不合法"), e.to_string()))
}

/// 将字符串列表解析成 i64 列表，并统一返回首个非法值对应的 400 响应。
pub(crate) fn parse_i64_list_param(raw_list: &[String], field: &str) -> Result<Vec<i64>, HttpResponse> {
    let mut parsed = Vec::with_capacity(raw_list.len());
    for raw in raw_list {
        parsed.push(parse_i64_param(raw, field)?);
    }
    Ok(parsed)
}

/// 要求参数存在且能解析成 i64，否则返回统一的 400 响应。
pub(crate) fn parse_required_i64_param(raw: Option<&str>, field: &str) -> Result<i64, HttpResponse> {
    let Some(raw) = raw else {
        return Err(err(400, format!("请求参数错误：缺少 {field}")));
    };
    parse_i64_param(raw, field)
}
