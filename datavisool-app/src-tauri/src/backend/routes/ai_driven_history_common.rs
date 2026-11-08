/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 历史接口共用的作用域解析与错误映射。

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::backend::ai_driven::thread::ThreadRepositoryError;
use crate::backend::types::{err, err_with_detail, HttpResponse};

pub(super) fn parse_body<T: DeserializeOwned>(body: Value, label: &str) -> Result<T, HttpResponse> {
    serde_json::from_value(body).map_err(|error| err_with_detail(400, format!("{label} 请求格式错误"), error.to_string()))
}

pub(super) fn parse_project_id(value: &str) -> Result<i64, HttpResponse> {
    let normalized = value.trim();
    let project_id = normalized
        .parse::<i64>()
        .map_err(|_| err(400, "请求参数错误：projectId 必须是正整数"))?;
    if project_id <= 0 {
        return Err(err(400, "请求参数错误：projectId 必须是正整数"));
    }
    Ok(project_id)
}

pub(super) fn required_trimmed(value: &str, field: &str) -> Result<String, HttpResponse> {
    let normalized = value.trim();
    if normalized.is_empty() {
        Err(err(400, format!("请求参数错误：{field} 不能为空")))
    } else {
        Ok(normalized.to_string())
    }
}

pub(super) fn repository_error(error: ThreadRepositoryError) -> HttpResponse {
    let code = error.stable_code();
    match error {
        ThreadRepositoryError::NotFound { .. } => err_with_detail(404, "记录不存在", code),
        ThreadRepositoryError::ScopeMismatch { .. } => err_with_detail(403, "无权访问此记录", code),
        ThreadRepositoryError::Conflict { message, .. } | ThreadRepositoryError::InvalidTransition(message) => err_with_detail(409, message, code),
        ThreadRepositoryError::InvalidRecord(message) => err_with_detail(400, message, code),
        ThreadRepositoryError::Sql(_) | ThreadRepositoryError::Json(_) => err_with_detail(500, "AI 历史记录读写失败", code),
    }
}

pub(super) fn with_string_project_id<T: Serialize>(value: T, project_id: i64) -> Value {
    let mut value = serde_json::to_value(value).unwrap_or(Value::Null);
    if let Value::Object(object) = &mut value {
        object.insert("projectId".to_string(), Value::String(project_id.to_string()));
    }
    value
}
