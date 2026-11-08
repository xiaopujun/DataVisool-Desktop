/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Turn 追加输入 Inbox：持久化接纳、幂等重放与消费 CAS。

use serde_json::{json, Value};
use sqlx::{Sqlite, Transaction};

use crate::backend::ai_driven::thread::error::ThreadRepositoryError;
use crate::backend::ai_driven::thread::id::{new_event_id, new_item_id};
use crate::backend::ai_driven::thread::state::{
    AcceptTurnInput, AcceptedTurnInput, AgentItem, AgentTaskRunRef, AgentTimelineEvent, AgentTurn, AgentTurnInput, ConsumeTurnInput,
    ConsumedTurnInput, TurnInputStatus,
};

use super::{
    allocate_event_sequences, ensure_scope, event_from_row, insert_event, item_from_row, run_from_row, turn_from_row, turn_input_from_row,
    SqliteThreadRepository,
};

const ITEM_SELECT: &str = concat!(
    "SELECT item_id, turn_id, item_sequence, source_run_id, logical_key, item_type, status, schema_version, payload_json, ",
    "last_event_sequence, row_version, created_at, updated_at, completed_at FROM ai_agent_item "
);
const INPUT_SELECT: &str = concat!(
    "SELECT input_id, turn_id, user_item_id, content_digest, status, row_version, created_at, consumed_at, abandoned_at ",
    "FROM ai_agent_turn_input "
);
const TURN_SELECT: &str = concat!(
    "SELECT turn_id, project_id, thread_id, turn_sequence, request_id, status, request_json, COALESCE(input_summary, '') AS input_summary, ",
    "model_config_id, runtime_model_id, provider, entry_digest, usage_json, last_artifact_id, last_artifact_revision, error_code, error_message, ",
    "row_version, created_at, started_at, updated_at, ended_at FROM ai_agent_turn "
);
const RUN_SELECT: &str = concat!(
    "SELECT run.task_id, run.run_id, run.request_id, turn.entry_digest, turn.project_id, turn.thread_id, run.turn_id, run.attempt_no, run.client_session_id, ",
    "run.status, run.task_type, run.limits_json, run.pending_json, run.usage_json, run.last_artifact_id, run.last_artifact_revision, ",
    "run.error_code, run.error_message, run.project_snapshot_ref, run.rig_version, run.rig_snapshot_ref, run.created_at, run.started_at, ",
    "run.updated_at, run.ended_at FROM ai_agent_task_run run JOIN ai_agent_turn turn ON turn.turn_id = run.turn_id "
);

impl SqliteThreadRepository {
    pub(crate) async fn accept_turn_input(&self, input: AcceptTurnInput) -> Result<AcceptedTurnInput, ThreadRepositoryError> {
        validate_accept(&input)?;
        let mut transaction = self.pool.begin().await?;
        if let Some(existing) = find_input(&mut transaction, &input.input_id).await? {
            let replay = replay_existing(&mut transaction, &input, existing).await?;
            transaction.commit().await?;
            return Ok(replay);
        }
        ensure_scope(&mut transaction, input.project_id, &input.thread_id, Some(&input.turn_id), Some(&input.run_id), None).await?;
        ensure_turn_accepts_input(&mut transaction, &input.turn_id).await?;

        let item_id = new_item_id();
        let item_sequence = allocate_item_sequence(&mut transaction, &input.turn_id, input.accepted_at).await?;
        let logical_key = format!("input:{}", input.input_id);
        sqlx::query(
            "INSERT INTO ai_agent_item (\
                item_id, turn_id, item_sequence, source_run_id, logical_key, item_type, status, schema_version, payload_json, \
                last_event_sequence, row_version, created_at, updated_at, completed_at\
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'userMessage', 'completed', ?6, ?7, 0, 1, ?8, ?8, ?8)",
        )
        .bind(&item_id)
        .bind(&input.turn_id)
        .bind(item_sequence)
        .bind(&input.run_id)
        .bind(&logical_key)
        .bind(i64::from(input.schema_version))
        .bind(serde_json::to_string(&input.user_item_payload)?)
        .bind(input.accepted_at)
        .execute(&mut *transaction)
        .await?;

        let started = append_item_event(&mut transaction, &input, &item_id, "item.started", format!("input:{}:item.started", input.input_id)).await?;
        let completed =
            append_item_event(&mut transaction, &input, &item_id, "item.completed", format!("input:{}:item.completed", input.input_id)).await?;
        sqlx::query("UPDATE ai_agent_item SET last_event_sequence = ?1 WHERE item_id = ?2")
            .bind(to_i64(completed.thread_sequence, "thread event sequence")?)
            .bind(&item_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO ai_agent_turn_input (\
                input_id, turn_id, user_item_id, content_digest, status, row_version, created_at, consumed_at, abandoned_at\
             ) VALUES (?1, ?2, ?3, ?4, 'accepted', 1, ?5, NULL, NULL)",
        )
        .bind(&input.input_id)
        .bind(&input.turn_id)
        .bind(&item_id)
        .bind(&input.content_digest)
        .bind(input.accepted_at)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE ai_agent_thread SET updated_at = MAX(updated_at, ?1), last_activity_at = MAX(last_activity_at, ?1) \
             WHERE project_id = ?2 AND thread_id = ?3",
        )
        .bind(input.accepted_at)
        .bind(input.project_id)
        .bind(&input.thread_id)
        .execute(&mut *transaction)
        .await?;

        let accepted = find_input(&mut transaction, &input.input_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "turn input", key: input.input_id.clone() })?;
        let item = find_item(&mut transaction, &input.turn_id, &item_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "item", key: item_id })?;
        transaction.commit().await?;
        Ok(AcceptedTurnInput { input: accepted, user_item: item, events: vec![started, completed], replayed: false })
    }

    pub(crate) async fn list_accepted_turn_inputs(
        &self,
        project_id: i64,
        thread_id: &str,
        turn_id: &str,
        limit: u32,
    ) -> Result<Vec<AgentTurnInput>, ThreadRepositoryError> {
        self.ensure_turn_read_scope(project_id, thread_id, turn_id).await?;
        let query = format!("{INPUT_SELECT} WHERE turn_id = ?1 AND status = 'accepted' ORDER BY created_at ASC, input_id ASC LIMIT ?2");
        sqlx::query(&query)
            .bind(turn_id)
            .bind(i64::from(limit.clamp(1, 200)))
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(turn_input_from_row)
            .collect()
    }

    pub(crate) async fn find_turn_input(
        &self,
        project_id: i64,
        thread_id: &str,
        turn_id: &str,
        input_id: &str,
    ) -> Result<Option<AgentTurnInput>, ThreadRepositoryError> {
        self.ensure_turn_read_scope(project_id, thread_id, turn_id).await?;
        let query = format!("{INPUT_SELECT} WHERE input_id = ?1");
        let input = sqlx::query(&query)
            .bind(input_id)
            .fetch_optional(&self.pool)
            .await?
            .map(turn_input_from_row)
            .transpose()?;
        match input {
            Some(input) if input.turn_id != turn_id => Err(ThreadRepositoryError::ScopeMismatch { entity: "turn input", key: input_id.to_string() }),
            input => Ok(input),
        }
    }

    pub(crate) async fn consume_turn_input(&self, input: ConsumeTurnInput) -> Result<ConsumedTurnInput, ThreadRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        ensure_scope(&mut transaction, input.project_id, &input.thread_id, Some(&input.turn_id), Some(&input.run_id), None).await?;
        let current = find_input(&mut transaction, &input.input_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "turn input", key: input.input_id.clone() })?;
        if current.turn_id != input.turn_id {
            return Err(ThreadRepositoryError::ScopeMismatch { entity: "turn input", key: input.input_id });
        }
        if current.status == TurnInputStatus::Consumed {
            let replayed = consumed_result(&mut transaction, &input, current, true).await?;
            transaction.commit().await?;
            return Ok(replayed);
        }
        if current.status != TurnInputStatus::Accepted {
            return Err(ThreadRepositoryError::InvalidTransition("abandoned Turn input cannot be consumed".to_string()));
        }
        if current.row_version != input.expected_row_version {
            return Err(ThreadRepositoryError::conflict("agent.input.version_conflict", "追加输入已被其他执行器处理"));
        }
        let turn = find_turn(&mut transaction, &input.turn_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "turn", key: input.turn_id.clone() })?;
        if turn.status.is_terminal() || turn.status.as_str() == "created" {
            return Err(ThreadRepositoryError::InvalidTransition(
                "Turn input can only be consumed by a running or waiting Turn".to_string(),
            ));
        }
        let current_run = find_run(&mut transaction, &input.run_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "task run", key: input.run_id.clone() })?;
        let waiting_item = if turn.status.is_waiting() {
            Some(find_linked_waiting_item(&mut transaction, &input, &turn, &current_run).await?)
        } else {
            None
        };
        let result = sqlx::query(
            "UPDATE ai_agent_turn_input SET status = 'consumed', row_version = row_version + 1, consumed_at = ?1 \
             WHERE input_id = ?2 AND turn_id = ?3 AND status = 'accepted' AND row_version = ?4",
        )
        .bind(input.consumed_at)
        .bind(&input.input_id)
        .bind(&input.turn_id)
        .bind(to_i64(input.expected_row_version, "turn input row version")?)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(ThreadRepositoryError::conflict("agent.input.consume_conflict", "追加输入消费状态已变化"));
        }
        let mut events = Vec::new();
        let mut completed_waiting_item = None;
        if let Some(item) = waiting_item {
            let (completed, event) = complete_waiting_item(&mut transaction, &input, item).await?;
            completed_waiting_item = Some(completed);
            events.push(event);
        }
        events.push(
            append_consume_event(
                &mut transaction,
                &input,
                None,
                "agent.user_input.consumed",
                json!({ "inputId": input.input_id, "turnId": input.turn_id, "runId": input.run_id }),
                format!("input:{}:consume:accepted", input.input_id),
                1,
            )
            .await?,
        );
        if turn.status.is_waiting() {
            resume_waiting_turn(&mut transaction, &input, turn.status.as_str(), turn.row_version).await?;
            events.push(
                append_consume_event(
                    &mut transaction,
                    &input,
                    None,
                    "turn.resumed",
                    json!({ "turnId": input.turn_id, "runId": input.run_id, "status": "running", "inputId": input.input_id }),
                    format!("input:{}:consume:resumed", input.input_id),
                    1,
                )
                .await?,
            );
        }
        let consumed = find_input(&mut transaction, &input.input_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "turn input", key: input.input_id.clone() })?;
        let final_turn = find_turn(&mut transaction, &input.turn_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "turn", key: input.turn_id.clone() })?;
        let run = find_run(&mut transaction, &input.run_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "task run", key: input.run_id.clone() })?;
        transaction.commit().await?;
        Ok(ConsumedTurnInput { input: consumed, turn: final_turn, run, completed_waiting_item, events, replayed: false })
    }
}

async fn consumed_result(
    transaction: &mut Transaction<'_, Sqlite>,
    requested: &ConsumeTurnInput,
    input: AgentTurnInput,
    replayed: bool,
) -> Result<ConsumedTurnInput, ThreadRepositoryError> {
    let prefix = format!("input:{}:consume:", requested.input_id);
    let events = sqlx::query(
        "SELECT id, thread_id, turn_id, item_id, run_id, sequence, thread_sequence, kind, payload_json, dedupe_key, schema_version, created_at \
         FROM ai_agent_task_event WHERE thread_id = ?1 AND substr(dedupe_key, 1, length(?2)) = ?2 ORDER BY thread_sequence ASC",
    )
    .bind(&requested.thread_id)
    .bind(prefix)
    .fetch_all(&mut **transaction)
    .await?
    .into_iter()
    .map(event_from_row)
    .collect::<Result<Vec<_>, _>>()?;
    if !events.iter().any(|event| event.kind == "agent.user_input.consumed") {
        return Err(ThreadRepositoryError::InvalidRecord("consumed Inbox entry is missing its consumed Event".to_string()));
    }
    let completed_waiting_item = match events
        .iter()
        .find(|event| event.kind == "item.completed")
        .and_then(|event| event.item_id.as_deref())
    {
        Some(item_id) => find_item(transaction, &requested.turn_id, item_id).await?,
        None => None,
    };
    let turn = find_turn(transaction, &requested.turn_id)
        .await?
        .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "turn", key: requested.turn_id.clone() })?;
    let run = find_run(transaction, &requested.run_id)
        .await?
        .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "task run", key: requested.run_id.clone() })?;
    Ok(ConsumedTurnInput { input, turn, run, completed_waiting_item, events, replayed })
}

async fn find_linked_waiting_item(
    transaction: &mut Transaction<'_, Sqlite>,
    input: &ConsumeTurnInput,
    turn: &AgentTurn,
    run: &AgentTaskRunRef,
) -> Result<AgentItem, ThreadRepositoryError> {
    if run.status != turn.status.as_str() {
        return Err(ThreadRepositoryError::conflict("agent.input.waiting_scope_conflict", "Turn 与 TaskRun 的等待状态不一致"));
    }
    let pending = run
        .pending
        .as_ref()
        .ok_or_else(|| ThreadRepositoryError::InvalidRecord("waiting TaskRun is missing pending_json".to_string()))?;
    let waiting_item_id = pending
        .get("waitingItemId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ThreadRepositoryError::InvalidRecord("waiting TaskRun pending_json is missing waitingItemId".to_string()))?;
    let (expected_kind, expected_item_type) = waiting_contract(turn.status)?;
    if pending.get("kind").and_then(Value::as_str) != Some(expected_kind) {
        return Err(ThreadRepositoryError::conflict(
            "agent.input.waiting_scope_conflict",
            "TaskRun pending.kind 与 Turn 等待状态不一致",
        ));
    }
    let item = find_item(transaction, &input.turn_id, waiting_item_id)
        .await?
        .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "waiting item", key: waiting_item_id.to_string() })?;
    if item.source_run_id.as_deref() != Some(input.run_id.as_str()) || item.item_type != expected_item_type || item.status.as_str() != "waiting" {
        return Err(ThreadRepositoryError::conflict(
            "agent.input.waiting_item_conflict",
            "pending.waitingItemId 未指向当前 Run 对应类型的 waiting Item",
        ));
    }
    Ok(item)
}

async fn complete_waiting_item(
    transaction: &mut Transaction<'_, Sqlite>,
    input: &ConsumeTurnInput,
    item: AgentItem,
) -> Result<(AgentItem, AgentTimelineEvent), ThreadRepositoryError> {
    let payload = payload_with_consumed_input(item.payload, &input.input_id);
    let (sequence, thread_sequence) = allocate_event_sequences(transaction, &input.thread_id, &input.run_id, input.consumed_at).await?;
    let updated = sqlx::query(
        "UPDATE ai_agent_item SET status = 'completed', payload_json = ?1, last_event_sequence = ?2, row_version = row_version + 1, \
             updated_at = ?3, completed_at = ?3 WHERE turn_id = ?4 AND item_id = ?5 AND status = 'waiting' AND row_version = ?6",
    )
    .bind(serde_json::to_string(&payload)?)
    .bind(to_i64(thread_sequence, "thread event sequence")?)
    .bind(input.consumed_at)
    .bind(&input.turn_id)
    .bind(&item.item_id)
    .bind(to_i64(item.row_version, "item row version")?)
    .execute(&mut **transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(ThreadRepositoryError::conflict("agent.input.waiting_item_conflict", "等待交互 Item 状态已变化"));
    }
    let event = insert_event(
        transaction,
        &new_event_id(),
        &input.thread_id,
        &input.turn_id,
        Some(&item.item_id),
        &input.run_id,
        sequence,
        thread_sequence,
        "item.completed",
        &json!({
            "itemId": item.item_id,
            "itemType": item.item_type,
            "status": "completed",
            "payload": payload,
            "inputId": input.input_id,
            "item": {
                "itemId": item.item_id,
                "itemType": item.item_type,
                "status": "completed",
                "payload": payload,
                "inputId": input.input_id,
                "rowVersion": item.row_version + 1
            }
        }),
        &format!("input:{}:consume:item:{}:completed", input.input_id, item.item_id),
        item.schema_version,
        input.consumed_at,
    )
    .await?;
    sqlx::query(
        "UPDATE ai_agent_thread SET updated_at = MAX(updated_at, ?1), last_activity_at = MAX(last_activity_at, ?1) \
         WHERE project_id = ?2 AND thread_id = ?3",
    )
    .bind(input.consumed_at)
    .bind(input.project_id)
    .bind(&input.thread_id)
    .execute(&mut **transaction)
    .await?;
    let completed = find_item(transaction, &input.turn_id, &item.item_id)
        .await?
        .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "item", key: item.item_id })?;
    Ok((completed, event))
}

async fn append_consume_event(
    transaction: &mut Transaction<'_, Sqlite>,
    input: &ConsumeTurnInput,
    item_id: Option<&str>,
    kind: &str,
    payload: Value,
    dedupe_key: String,
    schema_version: u32,
) -> Result<AgentTimelineEvent, ThreadRepositoryError> {
    let (sequence, thread_sequence) = allocate_event_sequences(transaction, &input.thread_id, &input.run_id, input.consumed_at).await?;
    insert_event(
        transaction,
        &new_event_id(),
        &input.thread_id,
        &input.turn_id,
        item_id,
        &input.run_id,
        sequence,
        thread_sequence,
        kind,
        &payload,
        &dedupe_key,
        schema_version,
        input.consumed_at,
    )
    .await
}

async fn resume_waiting_turn(
    transaction: &mut Transaction<'_, Sqlite>,
    input: &ConsumeTurnInput,
    waiting_status: &str,
    turn_row_version: u64,
) -> Result<(), ThreadRepositoryError> {
    let turn = sqlx::query(
        "UPDATE ai_agent_turn SET status = 'running', row_version = row_version + 1, updated_at = MAX(updated_at, ?1) \
         WHERE turn_id = ?2 AND status = ?3 AND row_version = ?4",
    )
    .bind(input.consumed_at)
    .bind(&input.turn_id)
    .bind(waiting_status)
    .bind(to_i64(turn_row_version, "turn row version")?)
    .execute(&mut **transaction)
    .await?;
    let run = sqlx::query(
        "UPDATE ai_agent_task_run SET status = 'running', pending_json = NULL, updated_at = MAX(updated_at, ?1) \
         WHERE turn_id = ?2 AND run_id = ?3 AND status = ?4",
    )
    .bind(input.consumed_at)
    .bind(&input.turn_id)
    .bind(&input.run_id)
    .bind(waiting_status)
    .execute(&mut **transaction)
    .await?;
    if turn.rows_affected() == 1 && run.rows_affected() == 1 {
        Ok(())
    } else {
        Err(ThreadRepositoryError::conflict("agent.input.resume_conflict", "Turn 或 TaskRun 等待状态已变化"))
    }
}

fn payload_with_consumed_input(mut payload: Value, input_id: &str) -> Value {
    if let Some(object) = payload.as_object_mut() {
        object.insert("consumedInputId".to_string(), Value::String(input_id.to_string()));
        payload
    } else {
        json!({ "value": payload, "consumedInputId": input_id })
    }
}

fn waiting_contract(status: crate::backend::ai_driven::thread::state::TurnStatus) -> Result<(&'static str, &'static str), ThreadRepositoryError> {
    use crate::backend::ai_driven::thread::state::TurnStatus;

    match status {
        TurnStatus::WaitingFrontendTool => Ok(("frontendTool", "frontendTool")),
        TurnStatus::WaitingUserInput => Ok(("userInput", "userInput")),
        TurnStatus::WaitingUserConfirmation => Ok(("userConfirmation", "approval")),
        _ => Err(ThreadRepositoryError::InvalidRecord("waiting Item association requires a waiting Turn status".to_string())),
    }
}

async fn replay_existing(
    transaction: &mut Transaction<'_, Sqlite>,
    requested: &AcceptTurnInput,
    existing: AgentTurnInput,
) -> Result<AcceptedTurnInput, ThreadRepositoryError> {
    if existing.turn_id != requested.turn_id {
        return Err(ThreadRepositoryError::ScopeMismatch { entity: "turn input", key: requested.input_id.clone() });
    }
    ensure_scope(
        transaction,
        requested.project_id,
        &requested.thread_id,
        Some(&requested.turn_id),
        Some(&requested.run_id),
        Some(&existing.user_item_id),
    )
    .await?;
    if existing.content_digest != requested.content_digest {
        return Err(ThreadRepositoryError::conflict("agent.input.idempotency_conflict", "inputId 已绑定到不同内容"));
    }
    let item = find_item(transaction, &requested.turn_id, &existing.user_item_id)
        .await?
        .ok_or_else(|| ThreadRepositoryError::InvalidRecord("Inbox references a missing user Item".to_string()))?;
    let events = sqlx::query(
        "SELECT id, thread_id, turn_id, item_id, run_id, sequence, thread_sequence, kind, payload_json, dedupe_key, schema_version, created_at \
         FROM ai_agent_task_event WHERE item_id = ?1 AND kind IN ('item.started','item.completed') ORDER BY thread_sequence ASC",
    )
    .bind(&existing.user_item_id)
    .fetch_all(&mut **transaction)
    .await?
    .into_iter()
    .map(event_from_row)
    .collect::<Result<Vec<_>, _>>()?;
    if events.len() != 2 {
        return Err(ThreadRepositoryError::InvalidRecord("Inbox user Item is missing lifecycle Events".to_string()));
    }
    Ok(AcceptedTurnInput { input: existing, user_item: item, events, replayed: true })
}

async fn ensure_turn_accepts_input(transaction: &mut Transaction<'_, Sqlite>, turn_id: &str) -> Result<(), ThreadRepositoryError> {
    let active = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM ai_agent_turn WHERE turn_id = ?1 \
         AND status IN ('created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation')",
    )
    .bind(turn_id)
    .fetch_one(&mut **transaction)
    .await?;
    if active == 1 {
        Ok(())
    } else {
        Err(ThreadRepositoryError::conflict("agent.input.turn_terminal", "终态 Turn 不接收追加输入"))
    }
}

async fn allocate_item_sequence(transaction: &mut Transaction<'_, Sqlite>, turn_id: &str, updated_at: i64) -> Result<i64, ThreadRepositoryError> {
    sqlx::query_scalar(
        "UPDATE ai_agent_turn SET next_item_sequence = next_item_sequence + 1, row_version = row_version + 1, \
             updated_at = MAX(updated_at, ?1) WHERE turn_id = ?2 \
           AND status IN ('created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation') \
         RETURNING next_item_sequence - 1",
    )
    .bind(updated_at)
    .bind(turn_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| ThreadRepositoryError::conflict("agent.input.turn_terminal", "终态 Turn 不接收追加输入"))
}

async fn append_item_event(
    transaction: &mut Transaction<'_, Sqlite>,
    input: &AcceptTurnInput,
    item_id: &str,
    kind: &str,
    dedupe_key: String,
) -> Result<AgentTimelineEvent, ThreadRepositoryError> {
    let (sequence, thread_sequence) = allocate_event_sequences(transaction, &input.thread_id, &input.run_id, input.accepted_at).await?;
    let payload = item_payload(item_id, &input.user_item_payload);
    insert_event(
        transaction,
        &new_event_id(),
        &input.thread_id,
        &input.turn_id,
        Some(item_id),
        &input.run_id,
        sequence,
        thread_sequence,
        kind,
        &payload,
        &dedupe_key,
        input.schema_version,
        input.accepted_at,
    )
    .await
}

async fn find_input(transaction: &mut Transaction<'_, Sqlite>, input_id: &str) -> Result<Option<AgentTurnInput>, ThreadRepositoryError> {
    let query = format!("{INPUT_SELECT} WHERE input_id = ?1");
    sqlx::query(&query)
        .bind(input_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(turn_input_from_row)
        .transpose()
}

async fn find_item(transaction: &mut Transaction<'_, Sqlite>, turn_id: &str, item_id: &str) -> Result<Option<AgentItem>, ThreadRepositoryError> {
    let query = format!("{ITEM_SELECT} WHERE turn_id = ?1 AND item_id = ?2");
    sqlx::query(&query)
        .bind(turn_id)
        .bind(item_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(item_from_row)
        .transpose()
}

async fn find_turn(transaction: &mut Transaction<'_, Sqlite>, turn_id: &str) -> Result<Option<AgentTurn>, ThreadRepositoryError> {
    let query = format!("{TURN_SELECT} WHERE turn_id = ?1");
    sqlx::query(&query)
        .bind(turn_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(turn_from_row)
        .transpose()
}

async fn find_run(transaction: &mut Transaction<'_, Sqlite>, run_id: &str) -> Result<Option<AgentTaskRunRef>, ThreadRepositoryError> {
    let query = format!("{RUN_SELECT} WHERE run.run_id = ?1");
    sqlx::query(&query)
        .bind(run_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(run_from_row)
        .transpose()
}

fn item_payload(item_id: &str, payload: &Value) -> Value {
    json!({
        "item": {
            "itemId": item_id,
            "itemType": "userMessage",
            "status": "completed",
            "payload": payload
        }
    })
}

fn to_i64(value: u64, field: &str) -> Result<i64, ThreadRepositoryError> {
    i64::try_from(value).map_err(|_| ThreadRepositoryError::InvalidRecord(format!("{field} exceeds SQLite INTEGER")))
}

fn validate_accept(input: &AcceptTurnInput) -> Result<(), ThreadRepositoryError> {
    if input.project_id <= 0
        || [
            input.thread_id.as_str(),
            input.turn_id.as_str(),
            input.run_id.as_str(),
            input.input_id.as_str(),
            input.content_digest.as_str(),
        ]
        .iter()
        .any(|value| value.trim().is_empty())
        || input.schema_version == 0
    {
        Err(ThreadRepositoryError::InvalidRecord("追加输入作用域、摘要和 schema_version 必须有效".to_string()))
    } else {
        Ok(())
    }
}
