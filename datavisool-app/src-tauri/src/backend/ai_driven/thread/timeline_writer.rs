/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Item 当前态与 Event 历史的统一事务写入器。

use sqlx::SqlitePool;

use super::error::ThreadRepositoryError;
use super::id::{new_event_id, new_item_id};
use super::lifecycle::ensure_item_transition;
use super::persistence::{allocate_event_sequences, ensure_scope, event_from_row, find_deduplicated_event, insert_event, item_from_row};
use super::state::{AgentItem, AgentTimelineEvent, AppendTimelineEventInput, ApplyItemEventInput, StartItemInput, TimelineWriteResult};
use super::usage::{aggregate_thread_usage_in_transaction, with_thread_usage};

const ITEM_SELECT: &str = concat!(
    "SELECT item_id, turn_id, item_sequence, source_run_id, logical_key, item_type, status, schema_version, payload_json, ",
    "last_event_sequence, row_version, created_at, updated_at, completed_at FROM ai_agent_item "
);

#[derive(Clone)]
pub(crate) struct TimelineWriter {
    pool: SqlitePool,
}

impl TimelineWriter {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub(crate) async fn start_item(&self, input: StartItemInput) -> Result<TimelineWriteResult, ThreadRepositoryError> {
        validate_start(&input)?;
        let mut transaction = self.pool.begin().await?;
        ensure_scope(&mut transaction, input.project_id, &input.thread_id, Some(&input.turn_id), Some(&input.run_id), None).await?;
        if let Some(event) = find_deduplicated_event(&mut transaction, &input.thread_id, &input.dedupe_key).await? {
            let item_id = event
                .item_id
                .as_deref()
                .ok_or_else(|| ThreadRepositoryError::InvalidRecord("item.started Event is missing item_id".to_string()))?;
            let item = find_item(&mut transaction, &input.turn_id, item_id)
                .await?
                .ok_or_else(|| ThreadRepositoryError::InvalidRecord("item.started Event references a missing Item".to_string()))?;
            ensure_started_item_matches(&item, &event, &input)?;
            transaction.commit().await?;
            return Ok(TimelineWriteResult { event, item: Some(item), deduplicated: true });
        }

        let by_id = match input.item_id.as_deref() {
            Some(item_id) => find_item(&mut transaction, &input.turn_id, item_id).await?,
            None => None,
        };
        let by_logical = find_item_by_logical_key(&mut transaction, &input.turn_id, input.logical_key.as_deref()).await?;
        if let Some(item) = resolve_existing_item(by_id, by_logical)? {
            let event = find_started_event(&mut transaction, &input.turn_id, &item.item_id)
                .await?
                .ok_or_else(|| ThreadRepositoryError::InvalidRecord("existing Item is missing its item.started Event".to_string()))?;
            ensure_started_item_matches(&item, &event, &input)?;
            transaction.commit().await?;
            return Ok(TimelineWriteResult { event, item: Some(item), deduplicated: true });
        }
        let item_id = input.item_id.unwrap_or_else(new_item_id);
        let item_sequence = sqlx::query_scalar::<_, i64>(
            "UPDATE ai_agent_turn SET next_item_sequence = next_item_sequence + 1, row_version = row_version + 1, updated_at = MAX(updated_at, ?1) \
             WHERE turn_id = ?2 AND status IN ('created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation') \
             RETURNING next_item_sequence - 1",
        )
        .bind(input.created_at)
        .bind(&input.turn_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| ThreadRepositoryError::InvalidTransition("terminal Turn cannot create Item".to_string()))?;
        let item_sequence =
            u64::try_from(item_sequence).map_err(|_| ThreadRepositoryError::InvalidRecord("item sequence must not be negative".to_string()))?;
        let (run_sequence, thread_sequence) = allocate_event_sequences(&mut transaction, &input.thread_id, &input.run_id, input.created_at).await?;
        let payload_json = serde_json::to_string(&input.payload)?;
        let completed_at = input.status.is_terminal().then_some(input.created_at);
        sqlx::query(
            "INSERT INTO ai_agent_item (\
                item_id, turn_id, item_sequence, source_run_id, logical_key, item_type, status, schema_version, payload_json, \
                last_event_sequence, row_version, created_at, updated_at, completed_at\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?11, ?12)",
        )
        .bind(&item_id)
        .bind(&input.turn_id)
        .bind(i64::try_from(item_sequence).unwrap_or(i64::MAX))
        .bind(&input.run_id)
        .bind(input.logical_key.as_deref())
        .bind(&input.item_type)
        .bind(input.status.as_str())
        .bind(i64::from(input.schema_version))
        .bind(payload_json)
        .bind(i64::try_from(thread_sequence).unwrap_or(i64::MAX))
        .bind(input.created_at)
        .bind(completed_at)
        .execute(&mut *transaction)
        .await?;
        let event = insert_event(
            &mut transaction,
            &new_event_id(),
            &input.thread_id,
            &input.turn_id,
            Some(&item_id),
            &input.run_id,
            run_sequence,
            thread_sequence,
            "item.started",
            &input.event_payload,
            &input.dedupe_key,
            input.schema_version,
            input.created_at,
        )
        .await?;
        if input.status.is_terminal() {
            touch_thread_activity(&mut transaction, &input.thread_id, input.created_at).await?;
        }
        let item = find_item(&mut transaction, &input.turn_id, &item_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "item", key: item_id })?;
        transaction.commit().await?;
        Ok(TimelineWriteResult { event, item: Some(item), deduplicated: false })
    }

    pub(crate) async fn apply_item_event(&self, input: ApplyItemEventInput) -> Result<TimelineWriteResult, ThreadRepositoryError> {
        validate_apply(&input)?;
        let mut transaction = self.pool.begin().await?;
        ensure_scope(
            &mut transaction,
            input.project_id,
            &input.thread_id,
            Some(&input.turn_id),
            Some(&input.run_id),
            Some(&input.item_id),
        )
        .await?;
        if let Some(event) = find_deduplicated_event(&mut transaction, &input.thread_id, &input.dedupe_key).await? {
            ensure_event_matches(
                &event,
                &input.turn_id,
                &input.run_id,
                Some(&input.item_id),
                &input.event_kind,
                &input.event_payload,
                input.schema_version,
            )?;
            let item = find_item(&mut transaction, &input.turn_id, &input.item_id).await?;
            transaction.commit().await?;
            return Ok(TimelineWriteResult { event, item, deduplicated: true });
        }
        let current = find_item(&mut transaction, &input.turn_id, &input.item_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "item", key: input.item_id.clone() })?;
        if current.row_version != input.expected_row_version {
            return Err(ThreadRepositoryError::conflict("agent.item.version_conflict", "Item 已被其他事件更新"));
        }
        if current.status.is_terminal() {
            return Err(ThreadRepositoryError::InvalidTransition("terminal Item cannot be modified".to_string()));
        }
        ensure_item_transition(current.status, input.status)?;
        if input.status.is_terminal() != input.completed_at.is_some() {
            return Err(ThreadRepositoryError::InvalidRecord(
                "completed_at must be present exactly when Item becomes terminal".to_string(),
            ));
        }
        let (run_sequence, thread_sequence) = allocate_event_sequences(&mut transaction, &input.thread_id, &input.run_id, input.updated_at).await?;
        let result = sqlx::query(
            "UPDATE ai_agent_item SET status = ?1, schema_version = ?2, payload_json = ?3, last_event_sequence = ?4, \
                 row_version = row_version + 1, updated_at = ?5, completed_at = ?6 \
             WHERE turn_id = ?7 AND item_id = ?8 AND row_version = ?9 \
               AND status NOT IN ('completed','failed','cancelled','declined')",
        )
        .bind(input.status.as_str())
        .bind(i64::from(input.schema_version))
        .bind(serde_json::to_string(&input.payload)?)
        .bind(i64::try_from(thread_sequence).unwrap_or(i64::MAX))
        .bind(input.updated_at)
        .bind(input.completed_at)
        .bind(&input.turn_id)
        .bind(&input.item_id)
        .bind(i64::try_from(input.expected_row_version).unwrap_or(i64::MAX))
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(ThreadRepositoryError::conflict("agent.item.concurrent_update", "Item 并发更新失败"));
        }
        let event = insert_event(
            &mut transaction,
            &new_event_id(),
            &input.thread_id,
            &input.turn_id,
            Some(&input.item_id),
            &input.run_id,
            run_sequence,
            thread_sequence,
            &input.event_kind,
            &input.event_payload,
            &input.dedupe_key,
            input.schema_version,
            input.updated_at,
        )
        .await?;
        if input.status.is_terminal() {
            touch_thread_activity(&mut transaction, &input.thread_id, input.updated_at).await?;
        }
        let item = find_item(&mut transaction, &input.turn_id, &input.item_id).await?;
        transaction.commit().await?;
        Ok(TimelineWriteResult { event, item, deduplicated: false })
    }

    pub(crate) async fn append_event(&self, input: AppendTimelineEventInput) -> Result<TimelineWriteResult, ThreadRepositoryError> {
        validate_append(&input)?;
        let mut transaction = self.pool.begin().await?;
        ensure_scope(
            &mut transaction,
            input.project_id,
            &input.thread_id,
            Some(&input.turn_id),
            Some(&input.run_id),
            input.item_id.as_deref(),
        )
        .await?;
        if let Some(event) = find_deduplicated_event(&mut transaction, &input.thread_id, &input.dedupe_key).await? {
            ensure_append_event_matches(
                &event,
                &input.turn_id,
                &input.run_id,
                input.item_id.as_deref(),
                &input.kind,
                &input.payload,
                input.schema_version,
            )?;
            transaction.commit().await?;
            return Ok(TimelineWriteResult { event, item: None, deduplicated: true });
        }
        let (run_sequence, thread_sequence) = allocate_event_sequences(&mut transaction, &input.thread_id, &input.run_id, input.created_at).await?;
        let payload = if let Some(usage) = input.payload.get("usage") {
            let thread_usage =
                aggregate_thread_usage_in_transaction(&mut transaction, input.project_id, &input.thread_id, Some((&input.turn_id, usage))).await?;
            with_thread_usage(&input.payload, thread_usage)
        } else {
            input.payload.clone()
        };
        let event = insert_event(
            &mut transaction,
            &new_event_id(),
            &input.thread_id,
            &input.turn_id,
            input.item_id.as_deref(),
            &input.run_id,
            run_sequence,
            thread_sequence,
            &input.kind,
            &payload,
            &input.dedupe_key,
            input.schema_version,
            input.created_at,
        )
        .await?;
        transaction.commit().await?;
        Ok(TimelineWriteResult { event, item: None, deduplicated: false })
    }
}

async fn find_item(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    turn_id: &str,
    item_id: &str,
) -> Result<Option<AgentItem>, ThreadRepositoryError> {
    let query = format!("{ITEM_SELECT} WHERE turn_id = ?1 AND item_id = ?2");
    sqlx::query(&query)
        .bind(turn_id)
        .bind(item_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(item_from_row)
        .transpose()
}

async fn find_item_by_logical_key(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    turn_id: &str,
    logical_key: Option<&str>,
) -> Result<Option<AgentItem>, ThreadRepositoryError> {
    let Some(logical_key) = logical_key else {
        return Ok(None);
    };
    let query = format!("{ITEM_SELECT} WHERE turn_id = ?1 AND logical_key = ?2");
    sqlx::query(&query)
        .bind(turn_id)
        .bind(logical_key)
        .fetch_optional(&mut **transaction)
        .await?
        .map(item_from_row)
        .transpose()
}

async fn find_started_event(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    turn_id: &str,
    item_id: &str,
) -> Result<Option<AgentTimelineEvent>, ThreadRepositoryError> {
    sqlx::query(
        "SELECT id, thread_id, turn_id, item_id, run_id, sequence, thread_sequence, kind, payload_json, dedupe_key, schema_version, created_at \
         FROM ai_agent_task_event WHERE turn_id = ?1 AND item_id = ?2 AND kind = 'item.started' \
         ORDER BY thread_sequence ASC LIMIT 1",
    )
    .bind(turn_id)
    .bind(item_id)
    .fetch_optional(&mut **transaction)
    .await?
    .map(event_from_row)
    .transpose()
}

fn resolve_existing_item(by_id: Option<AgentItem>, by_logical: Option<AgentItem>) -> Result<Option<AgentItem>, ThreadRepositoryError> {
    match (by_id, by_logical) {
        (Some(by_id), Some(by_logical)) if by_id.item_id != by_logical.item_id => Err(ThreadRepositoryError::conflict(
            "agent.item.idempotency_conflict",
            "itemId 与 logicalKey 已分别绑定到不同 Item",
        )),
        (Some(item), _) | (None, Some(item)) => Ok(Some(item)),
        (None, None) => Ok(None),
    }
}

fn ensure_started_item_matches(item: &AgentItem, event: &AgentTimelineEvent, input: &StartItemInput) -> Result<(), ThreadRepositoryError> {
    let logical_matches = input
        .logical_key
        .as_deref()
        .is_some_and(|logical_key| item.logical_key.as_deref() == Some(logical_key));
    let item_id_matches = input.item_id.as_deref() == Some(item.item_id.as_str());
    let dedupe_matches = event.dedupe_key == input.dedupe_key;
    let explicit_logical_conflict = input.logical_key.is_some() && !logical_matches;
    let matches = !explicit_logical_conflict
        && (item_id_matches || logical_matches || dedupe_matches)
        && item.turn_id == input.turn_id
        && item.source_run_id.as_deref() == Some(input.run_id.as_str())
        && item.item_type == input.item_type
        && (item.row_version > 1 || (item.status == input.status && item.payload == input.payload))
        && event.thread_id == input.thread_id
        && event.turn_id == input.turn_id
        && event.run_id == input.run_id
        && event.item_id.as_deref() == Some(item.item_id.as_str())
        && event.kind == "item.started"
        && event.payload == input.event_payload
        && event.schema_version == input.schema_version;
    if matches {
        Ok(())
    } else {
        Err(ThreadRepositoryError::conflict(
            "agent.item.idempotency_conflict",
            "itemId 或 logicalKey 已绑定到不一致的 Item 创建事实",
        ))
    }
}

async fn touch_thread_activity(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: &str,
    updated_at: i64,
) -> Result<(), ThreadRepositoryError> {
    sqlx::query("UPDATE ai_agent_thread SET updated_at = MAX(updated_at, ?1), last_activity_at = MAX(last_activity_at, ?1) WHERE thread_id = ?2")
        .bind(updated_at)
        .bind(thread_id)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn ensure_event_matches(
    event: &AgentTimelineEvent,
    turn_id: &str,
    run_id: &str,
    item_id: Option<&str>,
    kind: &str,
    payload: &serde_json::Value,
    schema_version: u32,
) -> Result<(), ThreadRepositoryError> {
    if event.turn_id == turn_id
        && event.run_id == run_id
        && event.item_id.as_deref() == item_id
        && event.kind == kind
        && event.payload == *payload
        && event.schema_version == schema_version
    {
        Ok(())
    } else {
        Err(ThreadRepositoryError::conflict(
            "agent.event.dedupe_conflict",
            format!("dedupeKey {} 已绑定到不同事件", event.dedupe_key),
        ))
    }
}

fn ensure_append_event_matches(
    event: &AgentTimelineEvent,
    turn_id: &str,
    run_id: &str,
    item_id: Option<&str>,
    kind: &str,
    payload: &serde_json::Value,
    schema_version: u32,
) -> Result<(), ThreadRepositoryError> {
    let mut persisted_payload = event.payload.clone();
    if let Some(usage) = persisted_payload.get_mut("usage").and_then(serde_json::Value::as_object_mut) {
        usage.remove("threadUsage");
    }
    let mut input_payload = payload.clone();
    if let Some(usage) = input_payload.get_mut("usage").and_then(serde_json::Value::as_object_mut) {
        usage.remove("threadUsage");
    }
    if event.turn_id == turn_id
        && event.run_id == run_id
        && event.item_id.as_deref() == item_id
        && event.kind == kind
        && persisted_payload == input_payload
        && event.schema_version == schema_version
    {
        Ok(())
    } else {
        Err(ThreadRepositoryError::conflict(
            "agent.event.dedupe_conflict",
            format!("dedupeKey {} 已绑定到不同事件", event.dedupe_key),
        ))
    }
}

fn validate_start(input: &StartItemInput) -> Result<(), ThreadRepositoryError> {
    validate_common(&input.thread_id, &input.turn_id, &input.run_id, &input.dedupe_key)?;
    if input.item_type.trim().is_empty() {
        return Err(ThreadRepositoryError::InvalidRecord("item_type must not be blank".to_string()));
    }
    if input.schema_version == 0 {
        return Err(ThreadRepositoryError::InvalidRecord("schema_version must be positive".to_string()));
    }
    if input.status.is_terminal() {
        return Err(ThreadRepositoryError::InvalidRecord(
            "start_item requires a non-terminal status; append the terminal Item event separately".to_string(),
        ));
    }
    Ok(())
}

fn validate_apply(input: &ApplyItemEventInput) -> Result<(), ThreadRepositoryError> {
    validate_common(&input.thread_id, &input.turn_id, &input.run_id, &input.dedupe_key)?;
    if input.item_id.trim().is_empty() || input.event_kind.trim().is_empty() {
        return Err(ThreadRepositoryError::InvalidRecord("item_id and event_kind must not be blank".to_string()));
    }
    if input.schema_version == 0 {
        return Err(ThreadRepositoryError::InvalidRecord("schema_version must be positive".to_string()));
    }
    Ok(())
}

fn validate_append(input: &AppendTimelineEventInput) -> Result<(), ThreadRepositoryError> {
    validate_common(&input.thread_id, &input.turn_id, &input.run_id, &input.dedupe_key)?;
    if input.kind.trim().is_empty() || input.schema_version == 0 {
        return Err(ThreadRepositoryError::InvalidRecord("event kind and schema_version are required".to_string()));
    }
    Ok(())
}

fn validate_common(thread_id: &str, turn_id: &str, run_id: &str, dedupe_key: &str) -> Result<(), ThreadRepositoryError> {
    if [thread_id, turn_id, run_id, dedupe_key]
        .iter()
        .any(|value| value.trim().is_empty())
    {
        Err(ThreadRepositoryError::InvalidRecord("timeline scope and dedupe_key must not be blank".to_string()))
    } else {
        Ok(())
    }
}
