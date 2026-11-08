/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! systemAuth 相关接口。
//!
//! 这类接口属于“运行环境能力探测/授权占位”，不涉及 DB。
//! 后续如果引入真实授权体系（例如本地 license、用户表），可以把逻辑替换为 DB/配置读取。

use serde_json::Value;

use crate::backend::types::{ok, HttpResponse};

/// 返回当前桌面端是否具备本地授权能力。
pub(crate) fn has_auth() -> HttpResponse {
    ok(Value::Bool(true))
}

/// 返回本地授权类型标识。
pub(crate) fn get_auth_code() -> HttpResponse {
    ok(Value::String("LOCAL".to_string()))
}
