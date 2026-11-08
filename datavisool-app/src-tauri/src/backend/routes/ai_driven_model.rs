/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI驱动模型型号静态配置接口。

use serde_json::json;

use crate::backend::ai_driven::model::list_ai_driven_provider_model_providers;
use crate::backend::types::{err, ok, HttpResponse};

/// 返回 `provider_models.json` 中的 `providers` 属性。
pub(crate) async fn providers() -> Result<HttpResponse, String> {
    match list_ai_driven_provider_model_providers() {
        Ok(providers) => Ok(ok(json!(providers))),
        Err(detail) => Ok(err(500, detail)),
    }
}
