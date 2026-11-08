/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 驱动运行设置接口。

use serde_json::Value;

use crate::backend::ai_driven::settings::{AiDrivenRuntimeSettings, AiDrivenRuntimeSettingsStore};
use crate::backend::types::{err_with_detail, ok, HttpResponse};
use crate::db::DbState;

pub(crate) async fn get(db: &DbState) -> Result<HttpResponse, String> {
    let settings = AiDrivenRuntimeSettingsStore::new(db.pool.clone()).load().await?;
    Ok(ok(serde_json::json!({
        "settings": settings,
        "defaults": AiDrivenRuntimeSettings::default(),
    })))
}

pub(crate) async fn save(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let settings = match serde_json::from_value::<AiDrivenRuntimeSettings>(body) {
        Ok(settings) => settings,
        Err(error) => return Ok(err_with_detail(400, "AI 运行设置格式错误", error.to_string())),
    };
    if let Err(error) = settings.validate() {
        return Ok(err_with_detail(400, "AI 运行设置不合法", error));
    }
    AiDrivenRuntimeSettingsStore::new(db.pool.clone()).save(&settings).await?;
    Ok(ok(serde_json::to_value(settings).map_err(|error| error.to_string())?))
}
