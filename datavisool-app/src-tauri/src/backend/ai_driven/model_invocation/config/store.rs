/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 模型配置数据源边界。

use std::future::Future;
use std::pin::Pin;

use sqlx::Row;

use crate::app_state::AppState;
use crate::backend::ai_driven::model_invocation::types::ModelInvocationError;
use crate::db::DbState;

use super::api_key_secret::decrypt_api_key_if_needed;

/// 模型配置读取异步结果。
pub(crate) type ModelConfigStoreFuture<'a> = Pin<Box<dyn Future<Output = Result<StoredModelConfig, ModelInvocationError>> + Send + 'a>>;

/// 模型配置数据源。
pub(crate) trait ModelConfigStore: Send + Sync {
    /// 按模型配置记录 id 读取可用于模型调用的配置。
    fn load_model_config(&self, config_id: i64) -> ModelConfigStoreFuture<'_>;
}

/// 已从存储层读取的模型配置。
#[derive(Clone, Debug)]
pub(crate) struct StoredModelConfig {
    /// 模型厂商标识。
    pub provider: String,
    /// 用户配置的 API 基础地址。
    pub base_url: Option<String>,
    /// 可直接用于调用的 API Key。
    pub api_key: Option<String>,
    /// 模型温度参数。
    pub temperature: f64,
    /// 模型最大输出 token 数。
    pub max_tokens: Option<i64>,
}

/// 基于桌面端数据库的模型配置数据源。
pub(crate) struct DbModelConfigStore<'a> {
    db: &'a DbState,
    app_state: &'a AppState,
}

impl<'a> DbModelConfigStore<'a> {
    /// 创建真实数据库模型配置数据源。
    pub(crate) fn new(db: &'a DbState, app_state: &'a AppState) -> Self {
        Self { db, app_state }
    }
}

impl ModelConfigStore for DbModelConfigStore<'_> {
    fn load_model_config(&self, config_id: i64) -> ModelConfigStoreFuture<'_> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT provider, base_url, api_key, temperature, max_tokens \
                 FROM ai_model \
                 WHERE id = ?1 AND enabled = 1 AND deleted = 0 \
                 LIMIT 1",
            )
            .bind(config_id)
            .fetch_optional(&self.db.pool)
            .await
            .map_err(|e| ModelInvocationError::validation(format!("查询 AI 模型配置失败：{e}")))?
            .ok_or_else(|| ModelInvocationError::validation("AI 模型配置不存在或未启用"))?;

            let provider: String = row
                .try_get("provider")
                .map_err(|e| ModelInvocationError::validation(format!("读取 provider 失败：{e}")))?;
            let base_url: Option<String> = row
                .try_get("base_url")
                .map_err(|e| ModelInvocationError::validation(format!("读取 base_url 失败：{e}")))?;
            let api_key: Option<String> = row
                .try_get("api_key")
                .map_err(|e| ModelInvocationError::validation(format!("读取 api_key 失败：{e}")))?;
            let temperature: f64 = row
                .try_get("temperature")
                .map_err(|e| ModelInvocationError::validation(format!("读取 temperature 失败：{e}")))?;
            let max_tokens: Option<i64> = row
                .try_get("max_tokens")
                .map_err(|e| ModelInvocationError::validation(format!("读取 max_tokens 失败：{e}")))?;

            Ok(StoredModelConfig {
                provider,
                base_url,
                api_key: decrypt_api_key_if_needed(self.app_state, api_key.as_deref())?,
                temperature,
                max_tokens,
            })
        })
    }
}
