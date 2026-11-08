/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 前端异步交互能力门面。

mod api;
mod error;
mod registry;
mod types;

use tauri::State;

pub(crate) use api::{FrontendEffectApi, FrontendEffectOptions};
pub(crate) use error::FrontendEffectError;
pub(crate) use registry::FrontendEffectRegistry;
pub(crate) use types::FrontendEffectResponse;

/// 前端完成一次异步交互后调用的 Tauri command。
#[tauri::command]
pub(crate) fn datavisool_complete_frontend_effect(
    frontend_effects: State<'_, FrontendEffectRegistry>,
    response: FrontendEffectResponse,
) -> Result<(), String> {
    let effect_id = response.effect_id.clone();
    frontend_effects.complete(response).map_err(|error| {
        tracing::warn!(
            target: "desktop_backend::frontend_effect",
            effect_id = %effect_id,
            error = %error,
            "frontend effect complete failed"
        );
        error.to_string()
    })
}
