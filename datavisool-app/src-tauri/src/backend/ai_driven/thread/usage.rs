/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Thread 范围 Token 用量聚合。每个 Turn 只贡献一份累计快照，Run/resume 不单独求和。

use serde::Serialize;
use serde_json::Value;
use sqlx::{Sqlite, SqlitePool, Transaction};

use super::error::ThreadRepositoryError;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
}

#[derive(Default)]
struct UsageAccumulator {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    cached_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
}

impl UsageAccumulator {
    fn add(&mut self, snapshot: &Value) {
        let input = number(snapshot, "inputTokens");
        let output = number(snapshot, "outputTokens");
        let total = number(snapshot, "totalTokens").max(input.saturating_add(output));
        self.input_tokens = self.input_tokens.saturating_add(input);
        self.output_tokens = self.output_tokens.saturating_add(output);
        self.total_tokens = self.total_tokens.saturating_add(total);
        add_optional(&mut self.cached_input_tokens, snapshot, "cachedInputTokens");
        add_optional(&mut self.cache_creation_input_tokens, snapshot, "cacheCreationInputTokens");
    }

    fn finish(self) -> ThreadTokenUsage {
        ThreadTokenUsage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            total_tokens: self.total_tokens,
            cached_input_tokens: self.cached_input_tokens,
            cache_creation_input_tokens: self.cache_creation_input_tokens,
        }
    }
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default()
}

fn add_optional(total: &mut Option<u64>, snapshot: &Value, key: &str) {
    if let Some(value) = snapshot.get(key).and_then(Value::as_u64) {
        *total = Some(total.unwrap_or_default().saturating_add(value));
    }
}

/// 在同一 SQLite 快照中读取会话合计与当前 Run 的最新 runtime 用量。
pub(crate) async fn thread_usage_snapshot(
    pool: &SqlitePool,
    project_id: i64,
    thread_id: &str,
    turn_id: &str,
    run_id: &str,
) -> Result<(ThreadTokenUsage, Option<Value>), ThreadRepositoryError> {
    let mut transaction = pool.begin().await?;
    let aggregate = aggregate_thread_usage_in_transaction(&mut transaction, project_id, thread_id, None).await?;
    let current = latest_run_usage_snapshot_in_transaction(&mut transaction, project_id, thread_id, turn_id, run_id).await?;
    transaction.commit().await?;
    Ok((aggregate, current))
}

async fn latest_run_usage_snapshot_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    project_id: i64,
    thread_id: &str,
    turn_id: &str,
    run_id: &str,
) -> Result<Option<Value>, ThreadRepositoryError> {
    let row = sqlx::query(
        "SELECT event.payload_json FROM ai_agent_task_event event \
         JOIN ai_agent_turn turn ON turn.turn_id = event.turn_id AND turn.thread_id = event.thread_id \
         WHERE turn.project_id = ?1 AND turn.thread_id = ?2 AND turn.turn_id = ?3 \
           AND event.run_id = ?4 \
           AND json_type(event.payload_json, '$.usage') = 'object' \
         ORDER BY event.thread_sequence DESC LIMIT 1",
    )
    .bind(project_id)
    .bind(thread_id)
    .bind(turn_id)
    .bind(run_id)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let payload_json: String = sqlx::Row::try_get(&row, "payload_json")?;
    let payload: Value = serde_json::from_str(&payload_json)?;
    Ok(payload.get("usage").cloned())
}

/// 写 usage Event 时，用本次事件中的快照覆盖当前 Turn 的旧快照。
pub(crate) async fn aggregate_thread_usage_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    project_id: i64,
    thread_id: &str,
    current_turn: Option<(&str, &Value)>,
) -> Result<ThreadTokenUsage, ThreadRepositoryError> {
    let turns = sqlx::query(
        "SELECT turn_id, status, usage_json FROM ai_agent_turn \
         WHERE project_id = ?1 AND thread_id = ?2 ORDER BY turn_sequence ASC",
    )
    .bind(project_id)
    .bind(thread_id)
    .fetch_all(&mut **transaction)
    .await?;
    let mut total = UsageAccumulator::default();

    for turn in turns {
        let turn_id: String = sqlx::Row::try_get(&turn, "turn_id")?;
        if let Some((current_turn_id, snapshot)) = current_turn {
            if turn_id == current_turn_id {
                total.add(snapshot);
                continue;
            }
        }

        let status: String = sqlx::Row::try_get(&turn, "status")?;
        let stored_usage: Option<String> = sqlx::Row::try_get(&turn, "usage_json")?;
        let stored_usage = stored_usage.and_then(|value| serde_json::from_str::<Value>(&value).ok());
        let snapshot = if is_terminal_turn(&status) {
            match stored_usage {
                Some(snapshot) => Some(snapshot),
                None => latest_usage_event(transaction, thread_id, &turn_id).await?,
            }
        } else {
            latest_usage_event(transaction, thread_id, &turn_id).await?.or(stored_usage)
        };
        if let Some(snapshot) = snapshot {
            total.add(&snapshot);
        }
    }

    Ok(total.finish())
}

fn is_terminal_turn(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled" | "budgetExhausted" | "interrupted" | "declined")
}

async fn latest_usage_event(
    transaction: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
    turn_id: &str,
) -> Result<Option<Value>, ThreadRepositoryError> {
    let rows = sqlx::query(
        "SELECT payload_json FROM ai_agent_task_event WHERE thread_id = ?1 AND turn_id = ?2 \
         AND json_type(payload_json, '$.usage') = 'object' ORDER BY thread_sequence DESC LIMIT 1",
    )
    .bind(thread_id)
    .bind(turn_id)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(row) = rows else {
        return Ok(None);
    };
    let payload_json: String = sqlx::Row::try_get(&row, "payload_json")?;
    let payload: Value = serde_json::from_str(&payload_json)?;
    Ok(payload.get("usage").cloned())
}

/// 给事件原有的 runtime 用量添加展示用会话快照。
pub(crate) fn with_thread_usage(payload: &Value, thread_usage: ThreadTokenUsage) -> Value {
    let mut payload = payload.clone();
    if let Some(usage) = payload.get_mut("usage").and_then(Value::as_object_mut) {
        usage.insert("threadUsage".to_string(), serde_json::to_value(thread_usage).unwrap_or(Value::Null));
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::thread::{
        AppendTimelineEventInput, CreateThreadInput, CreateTurnInput, ListThreadEventsQuery, SqliteThreadRepository, ThreadService, TimelineWriter,
    };
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn database() -> (ThreadService, SqlitePool) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect test database");
        sqlx::raw_sql("CREATE TABLE project (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);")
            .execute(&pool)
            .await
            .expect("create project table");
        for migration in [
            include_str!("../../../../migrations_desktop/1014_desktop_ai_thread_turn_item.sql"),
            include_str!("../../../../migrations_desktop/1016_desktop_ai_turn_verifying.sql"),
            include_str!("../../../../migrations_desktop/1017_desktop_ai_working_checkpoint.sql"),
            include_str!("../../../../migrations_desktop/1019_desktop_ai_budget_exhausted.sql"),
        ] {
            sqlx::raw_sql(migration).execute(&pool).await.expect("apply Thread migration");
        }
        sqlx::query("INSERT INTO project (id, name, deleted) VALUES (1, 'project', 0)")
            .execute(&pool)
            .await
            .expect("insert project");
        (ThreadService::new(pool.clone()), pool)
    }

    async fn create_turn(
        service: &ThreadService,
        thread_id: &str,
        suffix: &str,
        created_at: i64,
    ) -> crate::backend::ai_driven::thread::CreateTurnResult {
        service
            .create_turn(CreateTurnInput {
                project_id: 1,
                thread_id: thread_id.to_string(),
                request_id: format!("request-{suffix}"),
                request: json!({"prompt": suffix}),
                input_summary: format!("prompt {suffix}"),
                model_config_id: "model-config".to_string(),
                runtime_model_id: "runtime-model".to_string(),
                provider: Some("test".to_string()),
                entry_digest: format!("sha256:{suffix}"),
                client_session_id: "client-session".to_string(),
                task_type: "general".to_string(),
                limits: json!({"maxModelCalls": 8}),
                project_snapshot_ref: None,
                rig_version: Some("test-rig".to_string()),
                rig_snapshot_ref: Some("rig:snapshot".to_string()),
                user_item_payload: json!({"text": suffix}),
                attachments: Vec::new(),
                created_at,
            })
            .await
            .expect("create Turn")
    }

    async fn set_terminal_usage(pool: &SqlitePool, turn_id: &str, status: &str, usage: Value, ended_at: i64) {
        sqlx::query("UPDATE ai_agent_turn SET status = ?1, usage_json = ?2, ended_at = ?3, updated_at = ?3 WHERE turn_id = ?4")
            .bind(status)
            .bind(serde_json::to_string(&usage).expect("serialize usage"))
            .bind(ended_at)
            .bind(turn_id)
            .execute(pool)
            .await
            .expect("set terminal Turn usage");
    }

    fn usage_event(turn_id: &str, run_id: &str, payload: Value, dedupe_key: &str, created_at: i64) -> AppendTimelineEventInput {
        AppendTimelineEventInput {
            project_id: 1,
            thread_id: String::new(),
            turn_id: turn_id.to_string(),
            run_id: run_id.to_string(),
            item_id: None,
            kind: "agent.model_call.completed".to_string(),
            payload,
            dedupe_key: dedupe_key.to_string(),
            schema_version: 1,
            created_at,
        }
    }

    #[test]
    fn sums_turn_snapshots_and_keeps_optional_cache_totals() {
        let mut total = UsageAccumulator::default();
        total.add(&json!({"inputTokens": 100, "outputTokens": 20, "totalTokens": 120, "cachedInputTokens": 5}));
        total.add(&json!({"inputTokens": 40, "outputTokens": 10, "totalTokens": 50, "cacheCreationInputTokens": 3}));

        assert_eq!(
            total.finish(),
            ThreadTokenUsage {
                input_tokens: 140,
                output_tokens: 30,
                total_tokens: 170,
                cached_input_tokens: Some(5),
                cache_creation_input_tokens: Some(3),
            }
        );
    }

    #[test]
    fn snapshot_totals_fall_back_to_input_plus_output() {
        let mut total = UsageAccumulator::default();
        total.add(&json!({"inputTokens": 12, "outputTokens": 8, "totalTokens": 0}));
        assert_eq!(total.finish().total_tokens, 20);
    }

    #[test]
    fn attaches_thread_usage_without_changing_the_original_payload_elsewhere() {
        let payload = json!({"stepIndex": 2, "usage": {"inputTokens": 10}});
        let enriched = with_thread_usage(&payload, ThreadTokenUsage { input_tokens: 10, ..Default::default() });
        assert_eq!(payload["usage"].get("threadUsage"), None);
        assert_eq!(enriched["usage"]["threadUsage"]["inputTokens"], 10);
    }

    #[tokio::test]
    async fn aggregates_one_snapshot_per_turn_and_persists_deduplicated_event_usage() {
        let (service, pool) = database().await;
        let thread = service
            .create_thread(CreateThreadInput { project_id: 1, create_request_id: "create-usage-thread".to_string(), title: None, created_at: 1 })
            .await
            .expect("create Thread");
        let failed = create_turn(&service, &thread.thread_id, "failed", 2).await;
        set_terminal_usage(
            &pool,
            &failed.turn.turn_id,
            "failed",
            json!({"inputTokens": 100, "outputTokens": 50, "totalTokens": 150, "cachedInputTokens": 4}),
            3,
        )
        .await;
        let cancelled = create_turn(&service, &thread.thread_id, "cancelled", 4).await;
        set_terminal_usage(
            &pool,
            &cancelled.turn.turn_id,
            "cancelled",
            json!({"inputTokens": 80, "outputTokens": 20, "totalTokens": 100}),
            5,
        )
        .await;
        let budget_exhausted = create_turn(&service, &thread.thread_id, "budget", 6).await;
        set_terminal_usage(
            &pool,
            &budget_exhausted.turn.turn_id,
            "budgetExhausted",
            json!({"inputTokens": 40, "outputTokens": 5, "totalTokens": 45, "cacheCreationInputTokens": 2}),
            7,
        )
        .await;
        let resumed_turn = create_turn(&service, &thread.thread_id, "resumed", 8).await;
        sqlx::query(
            "INSERT INTO ai_agent_task_run (task_id, run_id, request_id, turn_id, attempt_no, client_session_id, status, task_type, limits_json, created_at, updated_at) \
             VALUES ('task-resumed', 'run-resumed', 'request-resumed-run', ?1, 2, 'client-session', 'created', 'general', '{}', 9, 9)",
        )
        .bind(&resumed_turn.turn.turn_id)
        .execute(&pool)
        .await
        .expect("create resumed Run");

        let writer = TimelineWriter::new(pool.clone());
        let old_snapshot = json!({"stepIndex": 1, "usage": {"inputTokens": 200, "outputTokens": 100, "totalTokens": 300, "cachedInputTokens": 10}});
        let mut first_event = usage_event(&resumed_turn.turn.turn_id, &resumed_turn.run.run_id, old_snapshot.clone(), "usage-first", 10);
        first_event.thread_id = thread.thread_id.clone();
        let first = writer
            .append_event(first_event.clone())
            .await
            .expect("append first usage Event");
        assert_eq!(first.event.payload["usage"]["threadUsage"]["inputTokens"], 420);

        let duplicate = writer.append_event(first_event).await.expect("deduplicate usage Event");
        assert!(duplicate.deduplicated);
        assert_eq!(duplicate.event.payload, first.event.payload, "dedupe keeps the first persisted threadUsage snapshot");

        let (before_resume_usage, latest_usage) = thread_usage_snapshot(&pool, 1, &thread.thread_id, &resumed_turn.turn.turn_id, "run-resumed")
            .await
            .expect("read new Run before its first usage Event");
        assert_eq!(before_resume_usage.input_tokens, 420, "session aggregation still includes earlier Run usage");
        assert_eq!(latest_usage, None, "new Run has no runtime usage event yet");

        let new_snapshot = json!({"stepIndex": 2, "usage": {"inputTokens": 250, "outputTokens": 110, "totalTokens": 360, "cachedInputTokens": 12}});
        let mut resumed_event = usage_event(&resumed_turn.turn.turn_id, "run-resumed", new_snapshot, "usage-after-resume", 11);
        resumed_event.thread_id = thread.thread_id.clone();
        let resumed = writer.append_event(resumed_event).await.expect("append resumed usage Event");
        assert_eq!(resumed.event.payload["usage"]["threadUsage"]["inputTokens"], 470);
        assert_eq!(resumed.event.payload["usage"]["threadUsage"]["outputTokens"], 185);
        assert_eq!(resumed.event.payload["usage"]["threadUsage"]["totalTokens"], 655);
        assert_eq!(resumed.event.payload["usage"]["threadUsage"]["cachedInputTokens"], 16);
        let (after_resume_usage, latest_usage) = thread_usage_snapshot(&pool, 1, &thread.thread_id, &resumed_turn.turn.turn_id, "run-resumed")
            .await
            .expect("read current resumed Run usage");
        assert_eq!(after_resume_usage.input_tokens, 470, "session aggregation includes the resumed Run snapshot once");
        assert_eq!(
            latest_usage.expect("usage event exists")["inputTokens"],
            250,
            "current runtime snapshot is scoped to the resumed Run"
        );

        let malformed_event = json!({"usage": null});
        let mut malformed = usage_event(&resumed_turn.turn.turn_id, "run-resumed", malformed_event, "usage-null", 12);
        malformed.thread_id = thread.thread_id.clone();
        writer.append_event(malformed).await.expect("append usage with invalid shape");
        let (_, latest_usage) = thread_usage_snapshot(&pool, 1, &thread.thread_id, &resumed_turn.turn.turn_id, "run-resumed")
            .await
            .expect("ignore invalid usage shape");
        assert_eq!(latest_usage.expect("previous object usage remains visible")["inputTokens"], 250);

        sqlx::query("UPDATE ai_agent_turn SET status = 'failed', usage_json = NULL, ended_at = 13, updated_at = 13 WHERE turn_id = ?1")
            .bind(&resumed_turn.turn.turn_id)
            .execute(&pool)
            .await
            .expect("set terminal Turn without run usage snapshot");
        let (aggregate, _) = thread_usage_snapshot(&pool, 1, &thread.thread_id, &resumed_turn.turn.turn_id, "run-resumed")
            .await
            .expect("aggregate Thread usage");
        assert_eq!(aggregate.input_tokens, 470);
        assert_eq!(aggregate.output_tokens, 185);
        assert_eq!(aggregate.total_tokens, 655);
        assert_eq!(aggregate.cached_input_tokens, Some(16));
        assert_eq!(aggregate.cache_creation_input_tokens, Some(2));
        assert_eq!(
            thread_usage_snapshot(&pool, 2, &thread.thread_id, &resumed_turn.turn.turn_id, "run-resumed")
                .await
                .expect("isolate project")
                .0
                .total_tokens,
            0
        );

        let events = SqliteThreadRepository::new(pool.clone())
            .list_thread_events(ListThreadEventsQuery { project_id: 1, thread_id: thread.thread_id, after_thread_sequence: 0, limit: 100 })
            .await
            .expect("replay Thread Events");
        let replayed = events
            .data
            .iter()
            .find(|event| event.dedupe_key == "usage-first")
            .expect("persisted usage Event");
        assert_eq!(replayed.payload, first.event.payload, "event replay returns the persisted enrichment unchanged");
    }
}
