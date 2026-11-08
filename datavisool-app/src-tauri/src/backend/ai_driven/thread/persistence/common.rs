/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Repository 共用的作用域校验、行转换和 Timeline 序号分配。

use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, Transaction};

use crate::backend::ai_driven::thread::state::{
    AgentItem, AgentTaskRunRef, AgentThread, AgentThreadSummary, AgentTimelineEvent, AgentTurn, AgentTurnInput, ItemStatus, ThreadStatus,
    ThreadTitleSource, TurnInputStatus, TurnStatus,
};

use super::super::error::ThreadRepositoryError;

pub(crate) async fn ensure_scope(
    transaction: &mut Transaction<'_, Sqlite>,
    project_id: i64,
    thread_id: &str,
    turn_id: Option<&str>,
    run_id: Option<&str>,
    item_id: Option<&str>,
) -> Result<(), ThreadRepositoryError> {
    let thread_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM ai_agent_thread thread \
         JOIN project active_project ON active_project.id = thread.project_id AND active_project.deleted = 0 \
         WHERE thread.project_id = ?1 AND thread.thread_id = ?2",
    )
    .bind(project_id)
    .bind(thread_id)
    .fetch_one(&mut **transaction)
    .await?;
    if thread_exists == 0 {
        return Err(ThreadRepositoryError::ScopeMismatch { entity: "thread", key: thread_id.to_string() });
    }
    let Some(turn_id) = turn_id else {
        return Ok(());
    };
    let turn_exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM ai_agent_turn WHERE project_id = ?1 AND thread_id = ?2 AND turn_id = ?3")
        .bind(project_id)
        .bind(thread_id)
        .bind(turn_id)
        .fetch_one(&mut **transaction)
        .await?;
    if turn_exists == 0 {
        return Err(ThreadRepositoryError::ScopeMismatch { entity: "turn", key: turn_id.to_string() });
    }
    if let Some(run_id) = run_id {
        let run_exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM ai_agent_task_run WHERE turn_id = ?1 AND run_id = ?2")
            .bind(turn_id)
            .bind(run_id)
            .fetch_one(&mut **transaction)
            .await?;
        if run_exists == 0 {
            return Err(ThreadRepositoryError::ScopeMismatch { entity: "task run", key: run_id.to_string() });
        }
    }
    if let Some(item_id) = item_id {
        let item_exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM ai_agent_item WHERE turn_id = ?1 AND item_id = ?2")
            .bind(turn_id)
            .bind(item_id)
            .fetch_one(&mut **transaction)
            .await?;
        if item_exists == 0 {
            return Err(ThreadRepositoryError::ScopeMismatch { entity: "item", key: item_id.to_string() });
        }
    }
    Ok(())
}

pub(crate) async fn allocate_event_sequences(
    transaction: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
    run_id: &str,
    created_at: i64,
) -> Result<(u64, u64), ThreadRepositoryError> {
    let thread_sequence = sqlx::query_scalar::<_, i64>(
        "UPDATE ai_agent_thread SET next_event_sequence = next_event_sequence + 1 \
         WHERE thread_id = ?1 RETURNING next_event_sequence - 1",
    )
    .bind(thread_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "thread", key: thread_id.to_string() })?;
    let run_sequence = sqlx::query_scalar::<_, i64>(
        "UPDATE ai_agent_task_run SET next_event_sequence = next_event_sequence + 1, updated_at = MAX(updated_at, ?1) \
         WHERE run_id = ?2 RETURNING next_event_sequence - 1",
    )
    .bind(created_at)
    .bind(run_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "task run", key: run_id.to_string() })?;
    Ok((to_u64(run_sequence, "run event sequence")?, to_u64(thread_sequence, "thread event sequence")?))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_event(
    transaction: &mut Transaction<'_, Sqlite>,
    event_id: &str,
    thread_id: &str,
    turn_id: &str,
    item_id: Option<&str>,
    run_id: &str,
    sequence: u64,
    thread_sequence: u64,
    kind: &str,
    payload: &Value,
    dedupe_key: &str,
    schema_version: u32,
    created_at: i64,
) -> Result<AgentTimelineEvent, ThreadRepositoryError> {
    let payload_json = serde_json::to_string(payload)?;
    sqlx::query(
        "INSERT INTO ai_agent_task_event (\
            id, thread_id, turn_id, item_id, run_id, sequence, thread_sequence, kind, payload_json, dedupe_key, schema_version, created_at\
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )
    .bind(event_id)
    .bind(thread_id)
    .bind(turn_id)
    .bind(item_id)
    .bind(run_id)
    .bind(to_i64(sequence, "run event sequence")?)
    .bind(to_i64(thread_sequence, "thread event sequence")?)
    .bind(kind)
    .bind(payload_json)
    .bind(dedupe_key)
    .bind(i64::from(schema_version))
    .bind(created_at)
    .execute(&mut **transaction)
    .await?;
    Ok(AgentTimelineEvent {
        event_id: event_id.to_string(),
        thread_id: thread_id.to_string(),
        turn_id: turn_id.to_string(),
        item_id: item_id.map(str::to_string),
        run_id: run_id.to_string(),
        sequence,
        thread_sequence,
        kind: kind.to_string(),
        payload: payload.clone(),
        dedupe_key: dedupe_key.to_string(),
        schema_version,
        created_at,
    })
}

pub(crate) async fn find_deduplicated_event(
    transaction: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
    dedupe_key: &str,
) -> Result<Option<AgentTimelineEvent>, ThreadRepositoryError> {
    sqlx::query(
        "SELECT id, thread_id, turn_id, item_id, run_id, sequence, thread_sequence, kind, payload_json, dedupe_key, schema_version, created_at \
         FROM ai_agent_task_event WHERE thread_id = ?1 AND dedupe_key = ?2",
    )
    .bind(thread_id)
    .bind(dedupe_key)
    .fetch_optional(&mut **transaction)
    .await?
    .map(event_from_row)
    .transpose()
}

pub(crate) fn thread_from_row(row: SqliteRow) -> Result<AgentThread, ThreadRepositoryError> {
    Ok(AgentThread {
        thread_id: row.try_get("thread_id")?,
        project_id: row.try_get("project_id")?,
        title: row.try_get("title")?,
        title_source: parse_thread_title_source(row.try_get::<String, _>("title_source")?)?,
        preview: row.try_get("preview")?,
        status: parse_thread_status(row.try_get::<String, _>("status")?)?,
        create_request_id: row.try_get("create_request_id")?,
        row_version: to_u64(row.try_get("row_version")?, "thread row version")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        last_activity_at: row.try_get("last_activity_at")?,
        archived_at: row.try_get("archived_at")?,
    })
}

pub(crate) fn thread_summary_from_row(row: SqliteRow) -> Result<AgentThreadSummary, ThreadRepositoryError> {
    let runtime_status: Option<String> = row.try_get("runtime_status")?;
    Ok(AgentThreadSummary {
        thread: AgentThread {
            thread_id: row.try_get("thread_id")?,
            project_id: row.try_get("project_id")?,
            title: row.try_get("title")?,
            title_source: parse_thread_title_source(row.try_get::<String, _>("title_source")?)?,
            preview: row.try_get("preview")?,
            status: parse_thread_status(row.try_get::<String, _>("status")?)?,
            create_request_id: row.try_get("create_request_id")?,
            row_version: to_u64(row.try_get("row_version")?, "thread row version")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            last_activity_at: row.try_get("last_activity_at")?,
            archived_at: row.try_get("archived_at")?,
        },
        runtime_status: runtime_status.unwrap_or_else(|| "idle".to_string()),
        active_turn_id: row.try_get("active_turn_id")?,
        current_run_id: row.try_get("current_run_id")?,
    })
}

pub(crate) fn turn_from_row(row: SqliteRow) -> Result<AgentTurn, ThreadRepositoryError> {
    Ok(AgentTurn {
        turn_id: row.try_get("turn_id")?,
        project_id: row.try_get("project_id")?,
        thread_id: row.try_get("thread_id")?,
        turn_sequence: to_u64(row.try_get("turn_sequence")?, "turn sequence")?,
        request_id: row.try_get("request_id")?,
        status: parse_turn_status(row.try_get::<String, _>("status")?)?,
        request: decode_json(row.try_get::<String, _>("request_json")?)?,
        input_summary: row.try_get("input_summary")?,
        model_config_id: row.try_get("model_config_id")?,
        runtime_model_id: row.try_get("runtime_model_id")?,
        provider: row.try_get("provider")?,
        entry_digest: row.try_get("entry_digest")?,
        usage: decode_optional_json(row.try_get("usage_json")?)?,
        limits: row
            .try_get::<Option<String>, _>("limits_json")
            .ok()
            .flatten()
            .map(|value| {
                let limits: crate::backend::ai_driven::task::AgentExecutionLimitsSnapshot = serde_json::from_str(&value)?;
                serde_json::to_value(limits)
            })
            .transpose()?,
        last_artifact_id: row.try_get("last_artifact_id")?,
        last_artifact_revision: optional_u64(row.try_get("last_artifact_revision")?, "artifact revision")?,
        error_code: row.try_get("error_code")?,
        error_message: row.try_get("error_message")?,
        row_version: to_u64(row.try_get("row_version")?, "turn row version")?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        updated_at: row.try_get("updated_at")?,
        ended_at: row.try_get("ended_at")?,
    })
}

pub(crate) fn item_from_row(row: SqliteRow) -> Result<AgentItem, ThreadRepositoryError> {
    Ok(AgentItem {
        item_id: row.try_get("item_id")?,
        turn_id: row.try_get("turn_id")?,
        item_sequence: to_u64(row.try_get("item_sequence")?, "item sequence")?,
        source_run_id: row.try_get("source_run_id")?,
        logical_key: row.try_get("logical_key")?,
        item_type: row.try_get("item_type")?,
        status: parse_item_status(row.try_get::<String, _>("status")?)?,
        schema_version: to_u32(row.try_get("schema_version")?, "item schema version")?,
        payload: decode_json(row.try_get::<String, _>("payload_json")?)?,
        last_event_sequence: to_u64(row.try_get("last_event_sequence")?, "item last event sequence")?,
        row_version: to_u64(row.try_get("row_version")?, "item row version")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        completed_at: row.try_get("completed_at")?,
    })
}

pub(crate) fn event_from_row(row: SqliteRow) -> Result<AgentTimelineEvent, ThreadRepositoryError> {
    Ok(AgentTimelineEvent {
        event_id: row.try_get("id")?,
        thread_id: row.try_get("thread_id")?,
        turn_id: row.try_get("turn_id")?,
        item_id: row.try_get("item_id")?,
        run_id: row.try_get("run_id")?,
        sequence: to_u64(row.try_get("sequence")?, "run event sequence")?,
        thread_sequence: to_u64(row.try_get("thread_sequence")?, "thread event sequence")?,
        kind: row.try_get("kind")?,
        payload: decode_json(row.try_get::<String, _>("payload_json")?)?,
        dedupe_key: row.try_get("dedupe_key")?,
        schema_version: to_u32(row.try_get("schema_version")?, "event schema version")?,
        created_at: row.try_get("created_at")?,
    })
}

pub(crate) fn turn_input_from_row(row: SqliteRow) -> Result<AgentTurnInput, ThreadRepositoryError> {
    Ok(AgentTurnInput {
        input_id: row.try_get("input_id")?,
        turn_id: row.try_get("turn_id")?,
        user_item_id: row.try_get("user_item_id")?,
        content_digest: row.try_get("content_digest")?,
        status: parse_turn_input_status(row.try_get::<String, _>("status")?)?,
        row_version: to_u64(row.try_get("row_version")?, "turn input row version")?,
        created_at: row.try_get("created_at")?,
        consumed_at: row.try_get("consumed_at")?,
        abandoned_at: row.try_get("abandoned_at")?,
    })
}

pub(crate) fn run_from_row(row: SqliteRow) -> Result<AgentTaskRunRef, ThreadRepositoryError> {
    Ok(AgentTaskRunRef {
        task_id: row.try_get("task_id")?,
        run_id: row.try_get("run_id")?,
        request_id: row.try_get("request_id")?,
        entry_digest: row.try_get("entry_digest")?,
        project_id: row.try_get("project_id")?,
        thread_id: row.try_get("thread_id")?,
        turn_id: row.try_get("turn_id")?,
        attempt_no: to_u32(row.try_get("attempt_no")?, "attempt number")?,
        client_session_id: row.try_get("client_session_id")?,
        status: row.try_get("status")?,
        task_type: row.try_get("task_type")?,
        limits: decode_json(row.try_get::<String, _>("limits_json")?)?,
        pending: decode_optional_json(row.try_get("pending_json")?)?,
        usage: decode_optional_json(row.try_get("usage_json")?)?,
        last_artifact_id: row.try_get("last_artifact_id")?,
        last_artifact_revision: optional_u64(row.try_get("last_artifact_revision")?, "artifact revision")?,
        error_code: row.try_get("error_code")?,
        error_message: row.try_get("error_message")?,
        project_snapshot_ref: row.try_get("project_snapshot_ref")?,
        rig_version: row.try_get("rig_version")?,
        rig_snapshot_ref: row.try_get("rig_snapshot_ref")?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        updated_at: row.try_get("updated_at")?,
        ended_at: row.try_get("ended_at")?,
    })
}

pub(super) fn to_i64(value: u64, field: &str) -> Result<i64, ThreadRepositoryError> {
    i64::try_from(value).map_err(|_| ThreadRepositoryError::InvalidRecord(format!("{field} exceeds SQLite INTEGER")))
}

pub(super) fn to_u64(value: i64, field: &str) -> Result<u64, ThreadRepositoryError> {
    u64::try_from(value).map_err(|_| ThreadRepositoryError::InvalidRecord(format!("{field} must not be negative")))
}

pub(super) fn to_u32(value: i64, field: &str) -> Result<u32, ThreadRepositoryError> {
    u32::try_from(value).map_err(|_| ThreadRepositoryError::InvalidRecord(format!("{field} is out of range")))
}

fn optional_u64(value: Option<i64>, field: &str) -> Result<Option<u64>, ThreadRepositoryError> {
    value.map(|value| to_u64(value, field)).transpose()
}

fn decode_json(value: String) -> Result<Value, ThreadRepositoryError> {
    Ok(serde_json::from_str(&value)?)
}

fn decode_optional_json(value: Option<String>) -> Result<Option<Value>, ThreadRepositoryError> {
    value.map(|value| decode_json(value)).transpose()
}

fn parse_thread_status(value: String) -> Result<ThreadStatus, ThreadRepositoryError> {
    ThreadStatus::from_str(&value).ok_or_else(|| ThreadRepositoryError::InvalidRecord(format!("unknown thread status: {value}")))
}

fn parse_thread_title_source(value: String) -> Result<ThreadTitleSource, ThreadRepositoryError> {
    ThreadTitleSource::from_str(&value).ok_or_else(|| ThreadRepositoryError::InvalidRecord(format!("unknown title source: {value}")))
}

fn parse_turn_status(value: String) -> Result<TurnStatus, ThreadRepositoryError> {
    TurnStatus::from_str(&value).ok_or_else(|| ThreadRepositoryError::InvalidRecord(format!("unknown turn status: {value}")))
}

fn parse_item_status(value: String) -> Result<ItemStatus, ThreadRepositoryError> {
    ItemStatus::from_str(&value).ok_or_else(|| ThreadRepositoryError::InvalidRecord(format!("unknown item status: {value}")))
}

fn parse_turn_input_status(value: String) -> Result<TurnInputStatus, ThreadRepositoryError> {
    TurnInputStatus::from_str(&value).ok_or_else(|| ThreadRepositoryError::InvalidRecord(format!("unknown turn input status: {value}")))
}
