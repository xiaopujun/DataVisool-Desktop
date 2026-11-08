/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 驱动运行设置及 SQLite 持久化。

use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::agent::{AgentContextLimits, AgentExecutionLimits};

const KV_CONFIG_KEY: &str = "ai_driven_runtime_settings";
pub(crate) const DEFAULT_MODEL_REQUEST_TIMEOUT_SECONDS: u64 = 120;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AgentModelRequestSettings {
    pub timeout_seconds: u64,
}

impl Default for AgentModelRequestSettings {
    fn default() -> Self {
        Self { timeout_seconds: DEFAULT_MODEL_REQUEST_TIMEOUT_SECONDS }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AgentContextSettings {
    pub max_active_context_tokens: u64,
    pub compact_at_percent: u8,
    pub max_tool_result_tokens: u64,
    pub max_tool_history_tokens: u64,
    pub max_state_digest_tokens: u64,
    pub compact_summary_tokens: u64,
}

impl Default for AgentContextSettings {
    fn default() -> Self {
        let defaults = AgentContextLimits::default();
        Self {
            max_active_context_tokens: defaults.max_active_context_tokens,
            compact_at_percent: defaults.compact_at_percent,
            max_tool_result_tokens: defaults.max_tool_result_tokens,
            max_tool_history_tokens: defaults.max_tool_history_tokens,
            max_state_digest_tokens: defaults.max_state_digest_tokens,
            compact_summary_tokens: defaults.compact_summary_tokens,
        }
    }
}

impl AgentContextSettings {
    pub(crate) fn to_limits(&self) -> AgentContextLimits {
        AgentContextLimits {
            max_active_context_tokens: self.max_active_context_tokens,
            compact_at_percent: self.compact_at_percent,
            max_tool_result_tokens: self.max_tool_result_tokens,
            max_tool_history_tokens: self.max_tool_history_tokens.min(self.max_active_context_tokens),
            max_state_digest_tokens: self.max_state_digest_tokens.min(self.max_active_context_tokens),
            compact_summary_tokens: self.compact_summary_tokens,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AgentExecutionSettings {
    pub token_budget_enabled: bool,
    pub call_budget_enabled: bool,
    pub duration_budget_enabled: bool,
    pub max_model_calls: u64,
    pub max_tool_calls: u64,
    pub max_write_calls: u64,
    pub max_total_tokens: u64,
    pub max_duration_seconds: u64,
    pub max_tool_duration_seconds: u64,
    pub max_consecutive_failures: u64,
    pub max_frontend_interactions: u64,
    pub max_same_read_repeats: u64,
    pub max_completion_continuations: u64,
}

impl Default for AgentExecutionSettings {
    fn default() -> Self {
        let defaults = AgentExecutionLimits::default();
        Self {
            token_budget_enabled: false,
            call_budget_enabled: false,
            duration_budget_enabled: false,
            max_model_calls: defaults.max_model_calls,
            max_tool_calls: defaults.max_tool_calls,
            max_write_calls: defaults.max_write_calls,
            max_total_tokens: defaults.max_total_tokens,
            max_duration_seconds: defaults.max_duration.as_secs(),
            max_tool_duration_seconds: defaults.max_tool_duration.as_secs(),
            max_consecutive_failures: defaults.max_consecutive_failures,
            max_frontend_interactions: defaults.max_frontend_interactions,
            max_same_read_repeats: defaults.max_same_read_repeats,
            max_completion_continuations: defaults.max_completion_continuations,
        }
    }
}

impl AgentExecutionSettings {
    pub(crate) fn to_limits(&self) -> AgentExecutionLimits {
        AgentExecutionLimits {
            token_budget_enabled: self.token_budget_enabled,
            call_budget_enabled: self.call_budget_enabled,
            duration_budget_enabled: self.duration_budget_enabled,
            max_model_calls: self.max_model_calls,
            max_tool_calls: self.max_tool_calls,
            max_write_calls: self.max_write_calls,
            max_total_tokens: self.max_total_tokens,
            max_duration: Duration::from_secs(self.max_duration_seconds),
            max_tool_duration: Duration::from_secs(self.max_tool_duration_seconds),
            max_consecutive_failures: self.max_consecutive_failures,
            max_frontend_interactions: self.max_frontend_interactions,
            max_same_read_repeats: self.max_same_read_repeats,
            max_completion_continuations: self.max_completion_continuations,
            workspace_files: Default::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiDrivenRuntimeSettings {
    pub schema_version: u16,
    pub model_request: AgentModelRequestSettings,
    pub context: AgentContextSettings,
    pub execution: AgentExecutionSettings,
}

impl Default for AiDrivenRuntimeSettings {
    fn default() -> Self {
        Self {
            schema_version: 2,
            model_request: Default::default(),
            context: Default::default(),
            execution: Default::default(),
        }
    }
}

impl<'de> Deserialize<'de> for AiDrivenRuntimeSettings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value.as_object().ok_or_else(|| D::Error::custom("运行设置必须是对象"))?;
        if object.get("schemaVersion").and_then(|value| value.as_u64()) != Some(2) {
            return Err(D::Error::custom("不支持的运行设置版本"));
        }
        let read = |field: &str| object.get(field).cloned();
        Ok(Self {
            schema_version: 2,
            model_request: read("modelRequest")
                .map(serde_json::from_value)
                .transpose()
                .map_err(D::Error::custom)?
                .unwrap_or_default(),
            context: read("context")
                .map(serde_json::from_value)
                .transpose()
                .map_err(D::Error::custom)?
                .unwrap_or_default(),
            execution: read("execution")
                .map(serde_json::from_value)
                .transpose()
                .map_err(D::Error::custom)?
                .unwrap_or_default(),
        })
    }
}

impl AiDrivenRuntimeSettings {
    pub(crate) fn model_request_timeout(&self) -> Duration {
        Duration::from_secs(self.model_request.timeout_seconds)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_range("模型请求超时", self.model_request.timeout_seconds, 30, 3_600)?;
        validate_range("活动上下文长度", self.context.max_active_context_tokens, 4_096, 2_000_000)?;
        validate_range("上下文压缩比例", u64::from(self.context.compact_at_percent), 50, 95)?;
        validate_range("工具结果 Token 上限", self.context.max_tool_result_tokens, 512, 100_000)?;
        validate_range("工具历史 Token 上限", self.context.max_tool_history_tokens, 2_048, 1_000_000)?;
        validate_range("状态摘要 Token 上限", self.context.max_state_digest_tokens, 512, 32_768)?;
        validate_range("压缩摘要 Token 上限", self.context.compact_summary_tokens, 512, 32_768)?;
        if self.context.compact_summary_tokens > self.context.max_active_context_tokens {
            return Err("压缩摘要 Token 上限不能超过活动上下文长度".to_string());
        }
        if self.context.max_tool_result_tokens > self.context.max_active_context_tokens {
            return Err("工具结果 Token 上限不能超过活动上下文长度".to_string());
        }
        if self.context.max_tool_history_tokens < self.context.max_tool_result_tokens {
            return Err("工具历史 Token 上限不能低于单个工具结果上限".to_string());
        }

        let execution = &self.execution;
        validate_range("模型调用次数", execution.max_model_calls, 1, 1_000)?;
        validate_range("工具调用次数", execution.max_tool_calls, 1, 5_000)?;
        validate_range("写入调用次数", execution.max_write_calls, 1, 1_000)?;
        validate_range("累计 Token 上限", execution.max_total_tokens, 10_000, 10_000_000)?;
        validate_range("任务最长时间", execution.max_duration_seconds, 60, 86_400)?;
        validate_range("单次工具最长时间", execution.max_tool_duration_seconds, 1, 14_400)?;
        validate_range("连续失败次数", execution.max_consecutive_failures, 1, 100)?;
        validate_range("前端交互次数", execution.max_frontend_interactions, 1, 500)?;
        validate_range("相同读取重复次数", execution.max_same_read_repeats, 1, 20)?;
        validate_range("完成续跑次数", execution.max_completion_continuations, 1, 100)?;
        if execution.max_write_calls > execution.max_tool_calls {
            return Err("写入调用次数不能超过工具调用次数".to_string());
        }
        if execution.duration_budget_enabled && execution.max_tool_duration_seconds > execution.max_duration_seconds {
            return Err("单次工具最长时间不能超过任务最长时间".to_string());
        }
        Ok(())
    }
}

pub(crate) struct AiDrivenRuntimeSettingsStore {
    pool: SqlitePool,
}

impl AiDrivenRuntimeSettingsStore {
    /// 使用给定连接池创建运行设置存储。
    pub(crate) fn new(pool: SqlitePool) -> Self {
        // 保存数据库连接池，后续读写均复用同一连接池。
        Self { pool }
    }

    /// 读取设置；仅缺失记录使用默认值，损坏配置明确报错以免丢失预算。
    pub(crate) async fn load(&self) -> Result<AiDrivenRuntimeSettings, String> {
        // 1. 按固定配置键读取持久化的 JSON 文本；记录不存在与查询失败需要区分处理。
        let stored = sqlx::query_scalar::<_, String>("SELECT v FROM kv_config WHERE k = ?1")
            .bind(KV_CONFIG_KEY)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| error.to_string())?;

        // 2. 首次使用尚未保存配置时，直接返回安全的默认设置。
        let Some(stored) = stored else {
            return Ok(AiDrivenRuntimeSettings::default());
        };

        // 3. 解析当前版本设置，损坏配置不能静默切换为不限额。
        let settings = serde_json::from_str::<AiDrivenRuntimeSettings>(&stored).map_err(|error| format!("已保存的 AI 设置无法解析：{error}"))?;
        settings.validate()?;

        // 4. 仅返回已成功解析且验证通过的持久化设置。
        Ok(settings)
    }

    /// 校验并以 upsert 方式持久化运行设置。
    pub(crate) async fn save(&self, settings: &AiDrivenRuntimeSettings) -> Result<(), String> {
        // 1. 写入前先校验，避免将超范围或互相冲突的限制持久化。
        settings.validate()?;

        // 2. 将结构化设置序列化为 kv_config 可存储的 JSON 文本。
        let value = serde_json::to_string(settings).map_err(|error| error.to_string())?;

        // 3. 以配置键执行 upsert：不存在时插入，已存在时覆盖为最新值。
        sqlx::query(
            "INSERT INTO kv_config (k, v) VALUES (?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        )
        .bind(KV_CONFIG_KEY)
        .bind(value)
        .execute(&self.pool)
        .await
        .map_err(|error| error.to_string())?;

        // 4. 数据库执行成功后返回空结果，表示设置已持久化。
        Ok(())
    }
}

fn validate_range(label: &str, value: u64, min: u64, max: u64) -> Result<(), String> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!("{label}必须在 {min} 到 {max} 之间"))
    }
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn store() -> AiDrivenRuntimeSettingsStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE kv_config (k TEXT PRIMARY KEY NOT NULL, v TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        AiDrivenRuntimeSettingsStore::new(pool)
    }

    #[tokio::test]
    async fn missing_record_uses_defaults() {
        let store = store().await;
        assert_eq!(store.load().await.unwrap(), AiDrivenRuntimeSettings::default());
    }

    #[tokio::test]
    async fn corrupt_saved_budget_is_not_replaced_with_unlimited_defaults() {
        let store = store().await;
        for value in ["invalid-json", r#"{"execution":{"maxTotalTokens":0}}"#, r#"{"schemaVersion":3}"#] {
            sqlx::query("INSERT OR REPLACE INTO kv_config (k, v) VALUES (?1, ?2)")
                .bind(KV_CONFIG_KEY)
                .bind(value)
                .execute(&store.pool)
                .await
                .unwrap();
            assert!(store.load().await.is_err(), "{value}");
        }
    }

    #[tokio::test]
    async fn settings_round_trip_through_database() {
        let store = store().await;
        let mut settings = AiDrivenRuntimeSettings::default();
        settings.model_request.timeout_seconds = 420;
        settings.context.max_active_context_tokens = 256_000;
        settings.execution.max_duration_seconds = 7_200;
        store.save(&settings).await.unwrap();
        assert_eq!(store.load().await.unwrap(), settings);
    }

    #[test]
    fn partial_settings_fill_missing_fields_with_defaults() {
        let settings: AiDrivenRuntimeSettings = serde_json::from_str(r#"{"schemaVersion":2,"modelRequest":{"timeoutSeconds":300}}"#).unwrap();
        assert_eq!(settings.model_request.timeout_seconds, 300);
        assert_eq!(settings.context, AgentContextSettings::default());
        assert_eq!(settings.execution, AgentExecutionSettings::default());
    }

    #[test]
    fn rejects_unsafe_relationships() {
        let mut settings = AiDrivenRuntimeSettings::default();
        settings.execution.max_write_calls = settings.execution.max_tool_calls + 1;
        assert!(settings.validate().unwrap_err().contains("写入调用次数"));
    }

    #[test]
    fn rejects_tool_history_budget_smaller_than_single_result_budget() {
        let mut settings = AiDrivenRuntimeSettings::default();
        settings.context.max_tool_history_tokens = settings.context.max_tool_result_tokens - 1;

        assert!(settings.validate().unwrap_err().contains("工具历史 Token 上限"));
    }

    #[test]
    fn workspace_file_limits_are_not_configurable() {
        let settings = AiDrivenRuntimeSettings::default();
        assert_eq!(settings.execution.to_limits().workspace_files, Default::default());
    }

    #[test]
    fn lower_global_limits_do_not_reintroduce_ready_phase_limits() {
        let mut settings = AiDrivenRuntimeSettings::default();
        settings.execution.max_model_calls = 4;
        settings.execution.max_tool_calls = 10;
        settings.execution.max_write_calls = 10;
        settings.execution.max_total_tokens = 10_000;

        settings.validate().unwrap();
        assert!(!settings.execution.to_limits().token_budget_enabled);
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;

    #[test]
    fn rejects_invalid_or_future_schema_versions() {
        for version in ["null", "\"2\"", "-1", "1.5", "0", "3"] {
            let raw = format!("{{\"schemaVersion\":{version}}}");
            assert!(serde_json::from_str::<AiDrivenRuntimeSettings>(&raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn rejects_legacy_settings() {
        assert!(serde_json::from_str::<AiDrivenRuntimeSettings>(r#"{"schemaVersion":1,"execution":{}}"#).is_err());
    }

    #[test]
    fn new_settings_default_to_automatic_context_management() {
        let settings = AiDrivenRuntimeSettings::default();
        settings.validate().unwrap();
        assert!(!settings.execution.token_budget_enabled && !settings.execution.call_budget_enabled && !settings.execution.duration_budget_enabled);
    }
}
