/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! security 相关接口。
//!
//! 负责向前端暴露本地后端公钥，供前端加密提交敏感请求体。

use crate::backend::crypto;
use crate::backend::types::{ok, HttpResponse};

/// 返回本地后端 RSA 公钥。
pub(crate) fn get_encrypt_key() -> HttpResponse {
    ok(serde_json::json!({ "key": crypto::server_public_key_pem() }))
}
