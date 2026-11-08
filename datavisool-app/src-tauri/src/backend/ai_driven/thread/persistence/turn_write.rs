/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Turn 与 TaskRun 的联合领取、等待、恢复和终态收口。

use serde_json::{json, Value};
use sqlx::{Sqlite, Transaction};

use crate::backend::ai_driven::thread::error::ThreadRepositoryError;
use crate::backend::ai_driven::thread::id::new_event_id;
use crate::backend::ai_driven::thread::lifecycle::ensure_turn_transition;
use crate::backend::ai_driven::thread::state::{
    AgentItem, AgentTaskRunRef, AgentTimelineEvent, AgentTurn, ClaimTurnRunningInput, CompleteTurnInput, CompleteTurnResult, ItemStatus,
    MarkTurnWaitingInput, ResumeTurnInput, TransitionTurnExecutionStatusInput, TurnStatus,
};

use super::{
    allocate_event_sequences, ensure_scope, event_from_row, find_deduplicated_event, insert_event, item_from_row, run_from_row, turn_from_row,
    SqliteThreadRepository,
};

const TURN_SELECT: &str = concat!(
    "SELECT turn_id, project_id, thread_id, turn_sequence, request_id, status, request_json, COALESCE(input_summary, '') AS input_summary, ",
    "model_config_id, runtime_model_id, provider, entry_digest, usage_json, last_artifact_id, last_artifact_revision, error_code, error_message, ",
    "row_version, created_at, started_at, updated_at, ended_at FROM ai_agent_turn "
);
const ITEM_SELECT: &str = concat!(
    "SELECT item_id, turn_id, item_sequence, source_run_id, logical_key, item_type, status, schema_version, payload_json, ",
    "last_event_sequence, row_version, created_at, updated_at, completed_at FROM ai_agent_item "
);
const RUN_SELECT: &str = concat!(
    "SELECT r.task_id, r.run_id, r.request_id, turn.entry_digest, turn.project_id, turn.thread_id, r.turn_id, r.attempt_no, r.client_session_id, ",
    "r.status, r.task_type, r.limits_json, r.pending_json, r.usage_json, r.last_artifact_id, r.last_artifact_revision, ",
    "r.error_code, r.error_message, r.project_snapshot_ref, r.rig_version, r.rig_snapshot_ref, r.created_at, r.started_at, r.updated_at, r.ended_at ",
    "FROM ai_agent_task_run r JOIN ai_agent_turn turn ON turn.turn_id = r.turn_id "
);

impl SqliteThreadRepository {
    pub(crate) async fn claim_turn_running(
        &self,
        input: ClaimTurnRunningInput,
    ) -> Result<(AgentTurn, AgentTaskRunRef, AgentTimelineEvent), ThreadRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        ensure_scope(&mut transaction, input.project_id, &input.thread_id, Some(&input.turn_id), Some(&input.run_id), None).await?;
        let dedupe_key = format!("turn:{}:started", input.turn_id);
        if let Some(event) = find_deduplicated_event(&mut transaction, &input.thread_id, &dedupe_key).await? {
            let result = lifecycle_result(&mut transaction, &input.turn_id, &input.run_id, event).await?;
            transaction.commit().await?;
            return Ok(result);
        }
        let current = find_turn(&mut transaction, &input.turn_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "turn", key: input.turn_id.clone() })?;
        ensure_turn_transition(current.status, TurnStatus::Running)?;
        let turn_updated = sqlx::query(
            "UPDATE ai_agent_turn SET status = 'running', started_at = COALESCE(started_at, ?1), updated_at = MAX(updated_at, ?1), \
                 row_version = row_version + 1 WHERE turn_id = ?2 AND status = 'created' AND row_version = ?3",
        )
        .bind(input.started_at)
        .bind(&input.turn_id)
        .bind(i64::try_from(current.row_version).unwrap_or(i64::MAX))
        .execute(&mut *transaction)
        .await?;
        let run_updated = sqlx::query(
            "UPDATE ai_agent_task_run SET status = 'running', started_at = COALESCE(started_at, ?1), updated_at = MAX(updated_at, ?1) \
             WHERE turn_id = ?2 AND run_id = ?3 AND status = 'created'",
        )
        .bind(input.started_at)
        .bind(&input.turn_id)
        .bind(&input.run_id)
        .execute(&mut *transaction)
        .await?;
        if turn_updated.rows_affected() != 1 || run_updated.rows_affected() != 1 {
            return Err(ThreadRepositoryError::conflict("agent.turn.claim_conflict", "Turn 或 TaskRun 已被其他执行器领取"));
        }
        let event = append_turn_event(
            &mut transaction,
            &input.thread_id,
            &input.turn_id,
            &input.run_id,
            "turn.started",
            json!({ "turnId": input.turn_id, "runId": input.run_id, "status": "running" }),
            dedupe_key,
            input.started_at,
        )
        .await?;
        let result = lifecycle_result(&mut transaction, &input.turn_id, &input.run_id, event).await?;
        transaction.commit().await?;
        Ok(result)
    }

    pub(crate) async fn transition_turn_execution_status(
        &self,
        input: TransitionTurnExecutionStatusInput,
    ) -> Result<(AgentTurn, AgentTaskRunRef, AgentTimelineEvent), ThreadRepositoryError> {
        if !matches!(input.status, TurnStatus::Running | TurnStatus::Verifying) {
            return Err(ThreadRepositoryError::InvalidRecord(
                "execution status transition only accepts running or verifying".to_string(),
            ));
        }
        let mut transaction = self.pool.begin().await?;
        ensure_scope(&mut transaction, input.project_id, &input.thread_id, Some(&input.turn_id), Some(&input.run_id), None).await?;
        let current = find_turn(&mut transaction, &input.turn_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "turn", key: input.turn_id.clone() })?;
        ensure_turn_transition(current.status, input.status)?;
        let expected = match input.status {
            TurnStatus::Verifying => TurnStatus::Running,
            TurnStatus::Running => TurnStatus::Verifying,
            _ => unreachable!(),
        };
        if current.status != expected {
            return Err(ThreadRepositoryError::InvalidTransition(format!(
                "turn cannot transition from {} to {}",
                current.status.as_str(),
                input.status.as_str()
            )));
        }
        let turn_updated = sqlx::query(
            "UPDATE ai_agent_turn SET status = ?1, updated_at = MAX(updated_at, ?2), row_version = row_version + 1 \
             WHERE turn_id = ?3 AND status = ?4 AND row_version = ?5",
        )
        .bind(input.status.as_str())
        .bind(input.updated_at)
        .bind(&input.turn_id)
        .bind(expected.as_str())
        .bind(i64::try_from(current.row_version).unwrap_or(i64::MAX))
        .execute(&mut *transaction)
        .await?;
        let run_updated = sqlx::query(
            "UPDATE ai_agent_task_run SET status = ?1, updated_at = MAX(updated_at, ?2) \
             WHERE turn_id = ?3 AND run_id = ?4 AND status = ?5",
        )
        .bind(input.status.as_str())
        .bind(input.updated_at)
        .bind(&input.turn_id)
        .bind(&input.run_id)
        .bind(expected.as_str())
        .execute(&mut *transaction)
        .await?;
        if turn_updated.rows_affected() != 1 || run_updated.rows_affected() != 1 {
            return Err(ThreadRepositoryError::conflict("agent.turn.execution_status_conflict", "Turn 或 TaskRun 执行状态已变化"));
        }
        let event_kind = if input.status == TurnStatus::Verifying {
            "turn.verifying"
        } else {
            "turn.resumed"
        };
        let event = append_turn_event(
            &mut transaction,
            &input.thread_id,
            &input.turn_id,
            &input.run_id,
            event_kind,
            json!({ "turnId": input.turn_id, "runId": input.run_id, "status": input.status.as_str() }),
            format!("turn:{}:execution:{}:{}", input.turn_id, input.status.as_str(), input.updated_at),
            input.updated_at,
        )
        .await?;
        let result = lifecycle_result(&mut transaction, &input.turn_id, &input.run_id, event).await?;
        transaction.commit().await?;
        Ok(result)
    }

    pub(crate) async fn mark_turn_waiting(
        &self,
        input: MarkTurnWaitingInput,
    ) -> Result<(AgentTurn, AgentTaskRunRef, AgentTimelineEvent), ThreadRepositoryError> {
        if !input.status.is_waiting() {
            return Err(ThreadRepositoryError::InvalidRecord("waiting transition requires a waiting Turn status".to_string()));
        }
        validate_waiting_identity(&input)?;
        let mut transaction = self.pool.begin().await?;
        ensure_scope(
            &mut transaction,
            input.project_id,
            &input.thread_id,
            Some(&input.turn_id),
            Some(&input.run_id),
            Some(&input.waiting_item_id),
        )
        .await?;
        let waiting_item = find_item(&mut transaction, &input.turn_id, &input.waiting_item_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "waiting item", key: input.waiting_item_id.clone() })?;
        validate_waiting_item(&waiting_item, &input)?;
        let dedupe_key = format!("turn:{}:waiting:item:{}", input.turn_id, input.waiting_item_id);
        let event_payload = json!({
            "turnId": input.turn_id,
            "runId": input.run_id,
            "status": input.status.as_str(),
            "pending": input.pending
        });
        if let Some(event) = find_deduplicated_event(&mut transaction, &input.thread_id, &dedupe_key).await? {
            validate_waiting_event(&event, &input, &event_payload)?;
            let result = lifecycle_result(&mut transaction, &input.turn_id, &input.run_id, event).await?;
            if result.0.status != input.status || result.1.status != input.status.as_str() || result.1.pending.as_ref() != Some(&input.pending) {
                return Err(ThreadRepositoryError::conflict(
                    "agent.turn.waiting_idempotency_conflict",
                    "waitingItemId 已绑定，但 Turn 或 TaskRun 当前等待状态不一致",
                ));
            }
            transaction.commit().await?;
            return Ok(result);
        }
        let current = find_turn(&mut transaction, &input.turn_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "turn", key: input.turn_id.clone() })?;
        ensure_turn_transition(current.status, input.status)?;
        let turn_updated = sqlx::query(
            "UPDATE ai_agent_turn SET status = ?1, updated_at = MAX(updated_at, ?2), row_version = row_version + 1 \
             WHERE turn_id = ?3 AND status = 'running' AND row_version = ?4",
        )
        .bind(input.status.as_str())
        .bind(input.updated_at)
        .bind(&input.turn_id)
        .bind(i64::try_from(current.row_version).unwrap_or(i64::MAX))
        .execute(&mut *transaction)
        .await?;
        let run_updated = sqlx::query(
            "UPDATE ai_agent_task_run SET status = ?1, pending_json = ?2, updated_at = MAX(updated_at, ?3) \
             WHERE turn_id = ?4 AND run_id = ?5 AND status = 'running'",
        )
        .bind(input.status.as_str())
        .bind(serde_json::to_string(&input.pending)?)
        .bind(input.updated_at)
        .bind(&input.turn_id)
        .bind(&input.run_id)
        .execute(&mut *transaction)
        .await?;
        if turn_updated.rows_affected() != 1 || run_updated.rows_affected() != 1 {
            return Err(ThreadRepositoryError::conflict("agent.turn.waiting_conflict", "Turn 等待状态已变化"));
        }
        let event = append_turn_event(
            &mut transaction,
            &input.thread_id,
            &input.turn_id,
            &input.run_id,
            "turn.waiting",
            event_payload,
            dedupe_key,
            input.updated_at,
        )
        .await?;
        let result = lifecycle_result(&mut transaction, &input.turn_id, &input.run_id, event).await?;
        transaction.commit().await?;
        Ok(result)
    }

    pub(crate) async fn resume_turn(
        &self,
        input: ResumeTurnInput,
    ) -> Result<(AgentTurn, AgentTaskRunRef, AgentTimelineEvent), ThreadRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        ensure_scope(&mut transaction, input.project_id, &input.thread_id, Some(&input.turn_id), Some(&input.run_id), None).await?;
        let dedupe_key = format!("turn:{}:resumed:{}", input.turn_id, input.updated_at);
        if let Some(event) = find_deduplicated_event(&mut transaction, &input.thread_id, &dedupe_key).await? {
            let result = lifecycle_result(&mut transaction, &input.turn_id, &input.run_id, event).await?;
            transaction.commit().await?;
            return Ok(result);
        }
        let current = find_turn(&mut transaction, &input.turn_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "turn", key: input.turn_id.clone() })?;
        ensure_turn_transition(current.status, TurnStatus::Running)?;
        if !current.status.is_waiting() {
            return Err(ThreadRepositoryError::InvalidTransition("only a waiting Turn can resume".to_string()));
        }
        let turn_updated = sqlx::query(
            "UPDATE ai_agent_turn SET status = 'running', updated_at = MAX(updated_at, ?1), row_version = row_version + 1 \
             WHERE turn_id = ?2 AND status = ?3 AND row_version = ?4",
        )
        .bind(input.updated_at)
        .bind(&input.turn_id)
        .bind(current.status.as_str())
        .bind(i64::try_from(current.row_version).unwrap_or(i64::MAX))
        .execute(&mut *transaction)
        .await?;
        let run_updated = sqlx::query(
            "UPDATE ai_agent_task_run SET status = 'running', pending_json = NULL, updated_at = MAX(updated_at, ?1) \
             WHERE turn_id = ?2 AND run_id = ?3 AND status = ?4",
        )
        .bind(input.updated_at)
        .bind(&input.turn_id)
        .bind(&input.run_id)
        .bind(current.status.as_str())
        .execute(&mut *transaction)
        .await?;
        if turn_updated.rows_affected() != 1 || run_updated.rows_affected() != 1 {
            return Err(ThreadRepositoryError::conflict("agent.turn.resume_conflict", "Turn 恢复状态已变化"));
        }
        let event = append_turn_event(
            &mut transaction,
            &input.thread_id,
            &input.turn_id,
            &input.run_id,
            "turn.resumed",
            json!({ "turnId": input.turn_id, "runId": input.run_id, "status": "running" }),
            dedupe_key,
            input.updated_at,
        )
        .await?;
        let result = lifecycle_result(&mut transaction, &input.turn_id, &input.run_id, event).await?;
        transaction.commit().await?;
        Ok(result)
    }

    pub(crate) async fn complete_turn(&self, input: CompleteTurnInput) -> Result<CompleteTurnResult, ThreadRepositoryError> {
        validate_complete(&input)?;
        let mut transaction = self.pool.begin().await?;
        ensure_scope(&mut transaction, input.project_id, &input.thread_id, Some(&input.turn_id), Some(&input.run_id), None).await?;
        let current = find_turn(&mut transaction, &input.turn_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "turn", key: input.turn_id.clone() })?;
        if current.status.is_terminal() {
            if current.status != input.status {
                return Err(ThreadRepositoryError::conflict("agent.turn.terminal_conflict", "Turn 已进入不同终态"));
            }
            let run = find_run(&mut transaction, &input.run_id)
                .await?
                .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "task run", key: input.run_id })?;
            let events = terminal_events(&mut transaction, &current.turn_id).await?;
            transaction.commit().await?;
            return Ok(CompleteTurnResult { turn: current, run, closed_items: Vec::new(), events });
        }
        let accepted_inputs = accepted_input_count(&mut transaction, &input.turn_id).await?;
        if input.status == TurnStatus::Completed && accepted_inputs > 0 {
            return Err(ThreadRepositoryError::conflict("agent.turn.pending_input", "仍有已接纳但未消费的追加输入，不能完成 Turn"));
        }
        ensure_turn_transition(current.status, input.status)?;
        if input.status != TurnStatus::Completed {
            abandon_inputs(&mut transaction, &input.turn_id, input.ended_at).await?;
        }
        let (closed_items, mut events) = close_open_items(&mut transaction, &input).await?;
        let run_status = run_terminal_status(input.status);
        let usage_json = input.usage.as_ref().map(serde_json::to_string).transpose()?;
        let run_updated = sqlx::query(
            "UPDATE ai_agent_task_run SET status = ?1, pending_json = NULL, usage_json = ?2, last_artifact_id = ?3, \
                 last_artifact_revision = ?4, error_code = ?5, error_message = ?6, updated_at = ?7, ended_at = ?7 \
             WHERE turn_id = ?8 AND run_id = ?9 AND status NOT IN ('completed','cancelled','failed','budgetExhausted')",
        )
        .bind(run_status)
        .bind(usage_json.as_deref())
        .bind(input.last_artifact_id.as_deref())
        .bind(
            input
                .last_artifact_revision
                .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
        )
        .bind(input.error_code.as_deref())
        .bind(input.error_message.as_deref())
        .bind(input.ended_at)
        .bind(&input.turn_id)
        .bind(&input.run_id)
        .execute(&mut *transaction)
        .await?;
        let turn_updated = sqlx::query(
            "UPDATE ai_agent_turn SET status = ?1, usage_json = ?2, last_artifact_id = ?3, last_artifact_revision = ?4, \
                 error_code = ?5, error_message = ?6, row_version = row_version + 1, updated_at = ?7, ended_at = ?7 \
             WHERE turn_id = ?8 AND row_version = ?9 \
               AND status IN ('created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation')",
        )
        .bind(input.status.as_str())
        .bind(usage_json.as_deref())
        .bind(input.last_artifact_id.as_deref())
        .bind(
            input
                .last_artifact_revision
                .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
        )
        .bind(input.error_code.as_deref())
        .bind(input.error_message.as_deref())
        .bind(input.ended_at)
        .bind(&input.turn_id)
        .bind(i64::try_from(current.row_version).unwrap_or(i64::MAX))
        .execute(&mut *transaction)
        .await?;
        if run_updated.rows_affected() != 1 || turn_updated.rows_affected() != 1 {
            return Err(ThreadRepositoryError::conflict("agent.turn.terminal_update_conflict", "Turn 或 TaskRun 终态更新冲突"));
        }
        let terminal_kind = format!("turn.{}", input.status.as_str());
        let terminal_event = append_turn_event(
            &mut transaction,
            &input.thread_id,
            &input.turn_id,
            &input.run_id,
            &terminal_kind,
            json!({
                "turnId": input.turn_id,
                "runId": input.run_id,
                "status": input.status.as_str(),
                "usage": input.usage,
                "artifactId": input.last_artifact_id,
                "artifactRevision": input.last_artifact_revision,
                "errorCode": input.error_code,
                "errorMessage": input.error_message,
            }),
            format!("turn:{}:terminal:{}", input.turn_id, input.status.as_str()),
            input.ended_at,
        )
        .await?;
        events.push(terminal_event);
        sqlx::query(
            "UPDATE ai_agent_thread SET preview = COALESCE(?1, preview), updated_at = MAX(updated_at, ?2), \
                 last_activity_at = MAX(last_activity_at, ?2) WHERE project_id = ?3 AND thread_id = ?4",
        )
        .bind(input.thread_preview.as_deref())
        .bind(input.ended_at)
        .bind(input.project_id)
        .bind(&input.thread_id)
        .execute(&mut *transaction)
        .await?;
        let turn = find_turn(&mut transaction, &input.turn_id)
            .await?
            .expect("updated Turn must exist");
        let run = find_run(&mut transaction, &input.run_id)
            .await?
            .expect("updated TaskRun must exist");
        transaction.commit().await?;
        Ok(CompleteTurnResult { turn, run, closed_items, events })
    }
}

async fn close_open_items(
    transaction: &mut Transaction<'_, Sqlite>,
    input: &CompleteTurnInput,
) -> Result<(Vec<AgentItem>, Vec<AgentTimelineEvent>), ThreadRepositoryError> {
    let query = format!("{ITEM_SELECT} WHERE turn_id = ?1 AND status IN ('created','inProgress','waiting') ORDER BY item_sequence ASC");
    let open_items = sqlx::query(&query)
        .bind(&input.turn_id)
        .fetch_all(&mut **transaction)
        .await?
        .into_iter()
        .map(item_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let target = item_terminal_status(input.status);
    let event_kind = format!("item.{}", target.as_str());
    let mut closed = Vec::with_capacity(open_items.len());
    let mut events = Vec::with_capacity(open_items.len());
    for item in open_items {
        let (run_sequence, thread_sequence) = allocate_event_sequences(transaction, &input.thread_id, &input.run_id, input.ended_at).await?;
        sqlx::query(
            "UPDATE ai_agent_item SET status = ?1, last_event_sequence = ?2, row_version = row_version + 1, updated_at = ?3, completed_at = ?3 \
             WHERE item_id = ?4 AND turn_id = ?5 AND status IN ('created','inProgress','waiting')",
        )
        .bind(target.as_str())
        .bind(i64::try_from(thread_sequence).unwrap_or(i64::MAX))
        .bind(input.ended_at)
        .bind(&item.item_id)
        .bind(&input.turn_id)
        .execute(&mut **transaction)
        .await?;
        let event = insert_event(
            transaction,
            &new_event_id(),
            &input.thread_id,
            &input.turn_id,
            Some(&item.item_id),
            &input.run_id,
            run_sequence,
            thread_sequence,
            &event_kind,
            &json!({
                "itemId": item.item_id,
                "itemType": item.item_type,
                "status": target.as_str(),
                "payload": item.payload,
                "item": {
                    "itemId": item.item_id,
                    "itemType": item.item_type,
                    "status": target.as_str(),
                    "payload": item.payload,
                    "rowVersion": item.row_version + 1
                }
            }),
            &format!("turn:{}:terminal:{}:item:{}", input.turn_id, input.status.as_str(), item.item_id),
            item.schema_version,
            input.ended_at,
        )
        .await?;
        events.push(event);
        closed.push(
            find_item(transaction, &input.turn_id, &item.item_id)
                .await?
                .expect("closed Item must exist"),
        );
    }
    Ok((closed, events))
}

async fn append_turn_event(
    transaction: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
    turn_id: &str,
    run_id: &str,
    kind: &str,
    payload: Value,
    dedupe_key: String,
    created_at: i64,
) -> Result<AgentTimelineEvent, ThreadRepositoryError> {
    let (run_sequence, thread_sequence) = allocate_event_sequences(transaction, thread_id, run_id, created_at).await?;
    insert_event(
        transaction,
        &new_event_id(),
        thread_id,
        turn_id,
        None,
        run_id,
        run_sequence,
        thread_sequence,
        kind,
        &payload,
        &dedupe_key,
        1,
        created_at,
    )
    .await
}

async fn lifecycle_result(
    transaction: &mut Transaction<'_, Sqlite>,
    turn_id: &str,
    run_id: &str,
    event: AgentTimelineEvent,
) -> Result<(AgentTurn, AgentTaskRunRef, AgentTimelineEvent), ThreadRepositoryError> {
    let turn = find_turn(transaction, turn_id)
        .await?
        .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "turn", key: turn_id.to_string() })?;
    let run = find_run(transaction, run_id)
        .await?
        .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "task run", key: run_id.to_string() })?;
    Ok((turn, run, event))
}

fn validate_waiting_identity(input: &MarkTurnWaitingInput) -> Result<(), ThreadRepositoryError> {
    if input.waiting_item_id.trim().is_empty() {
        return Err(ThreadRepositoryError::InvalidRecord("waiting transition requires waiting_item_id".to_string()));
    }
    let pending_item_id = input.pending.get("waitingItemId").and_then(Value::as_str).map(str::trim);
    if pending_item_id != Some(input.waiting_item_id.as_str()) {
        return Err(ThreadRepositoryError::conflict(
            "agent.turn.waiting_item_mismatch",
            "pending.waitingItemId 与等待 Item 不一致",
        ));
    }
    let (expected_kind, _) = waiting_contract(input.status)?;
    if input.pending.get("kind").and_then(Value::as_str) != Some(expected_kind) {
        return Err(ThreadRepositoryError::conflict("agent.turn.waiting_kind_mismatch", "pending.kind 与 Turn 等待状态不一致"));
    }
    Ok(())
}

fn validate_waiting_item(item: &AgentItem, input: &MarkTurnWaitingInput) -> Result<(), ThreadRepositoryError> {
    let (_, expected_item_type) = waiting_contract(input.status)?;
    if item.turn_id == input.turn_id
        && item.source_run_id.as_deref() == Some(input.run_id.as_str())
        && item.item_type == expected_item_type
        && item.status == ItemStatus::Waiting
    {
        Ok(())
    } else {
        Err(ThreadRepositoryError::conflict(
            "agent.turn.waiting_item_mismatch",
            "waitingItemId 未指向当前 Run 对应类型的 waiting Item",
        ))
    }
}

fn validate_waiting_event(event: &AgentTimelineEvent, input: &MarkTurnWaitingInput, expected_payload: &Value) -> Result<(), ThreadRepositoryError> {
    if event.turn_id == input.turn_id
        && event.run_id == input.run_id
        && event.item_id.is_none()
        && event.kind == "turn.waiting"
        && event.payload == *expected_payload
        && event.schema_version == 1
    {
        Ok(())
    } else {
        Err(ThreadRepositoryError::conflict(
            "agent.turn.waiting_idempotency_conflict",
            "waitingItemId 已绑定到不同等待 checkpoint",
        ))
    }
}

fn waiting_contract(status: TurnStatus) -> Result<(&'static str, &'static str), ThreadRepositoryError> {
    match status {
        TurnStatus::WaitingFrontendTool => Ok(("frontendTool", "frontendTool")),
        TurnStatus::WaitingUserInput => Ok(("userInput", "userInput")),
        TurnStatus::WaitingUserConfirmation => Ok(("userConfirmation", "approval")),
        _ => Err(ThreadRepositoryError::InvalidRecord("waiting transition requires a waiting Turn status".to_string())),
    }
}

async fn accepted_input_count(transaction: &mut Transaction<'_, Sqlite>, turn_id: &str) -> Result<i64, ThreadRepositoryError> {
    Ok(sqlx::query_scalar("SELECT COUNT(1) FROM ai_agent_turn_input WHERE turn_id = ?1 AND status = 'accepted'")
        .bind(turn_id)
        .fetch_one(&mut **transaction)
        .await?)
}

async fn abandon_inputs(transaction: &mut Transaction<'_, Sqlite>, turn_id: &str, ended_at: i64) -> Result<(), ThreadRepositoryError> {
    sqlx::query(
        "UPDATE ai_agent_turn_input SET status = 'abandoned', row_version = row_version + 1, abandoned_at = ?1 \
         WHERE turn_id = ?2 AND status = 'accepted'",
    )
    .bind(ended_at)
    .bind(turn_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn terminal_events(transaction: &mut Transaction<'_, Sqlite>, turn_id: &str) -> Result<Vec<AgentTimelineEvent>, ThreadRepositoryError> {
    sqlx::query(
        "SELECT id, thread_id, turn_id, item_id, run_id, sequence, thread_sequence, kind, payload_json, dedupe_key, schema_version, created_at \
         FROM ai_agent_task_event WHERE turn_id = ?1 AND kind IN ('turn.completed','turn.cancelled','turn.failed','turn.interrupted') \
         ORDER BY thread_sequence ASC",
    )
    .bind(turn_id)
    .fetch_all(&mut **transaction)
    .await?
    .into_iter()
    .map(event_from_row)
    .collect()
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
    let query = format!("{RUN_SELECT} WHERE r.run_id = ?1");
    sqlx::query(&query)
        .bind(run_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(run_from_row)
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

fn item_terminal_status(turn_status: TurnStatus) -> ItemStatus {
    match turn_status {
        TurnStatus::Completed => ItemStatus::Completed,
        TurnStatus::Failed => ItemStatus::Failed,
        TurnStatus::Cancelled | TurnStatus::Interrupted | TurnStatus::BudgetExhausted => ItemStatus::Cancelled,
        _ => unreachable!("validated terminal Turn status"),
    }
}

fn run_terminal_status(turn_status: TurnStatus) -> &'static str {
    match turn_status {
        TurnStatus::Completed => "completed",
        TurnStatus::BudgetExhausted => "budgetExhausted",
        TurnStatus::Cancelled => "cancelled",
        TurnStatus::Failed | TurnStatus::Interrupted => "failed",
        _ => unreachable!("validated terminal Turn status"),
    }
}

fn validate_complete(input: &CompleteTurnInput) -> Result<(), ThreadRepositoryError> {
    if !input.status.is_terminal() {
        return Err(ThreadRepositoryError::InvalidRecord("complete_turn requires a terminal status".to_string()));
    }
    if input.last_artifact_id.is_some() != input.last_artifact_revision.is_some() {
        return Err(ThreadRepositoryError::InvalidRecord("artifact id and revision must be present together".to_string()));
    }
    if input.error_code.is_some() != input.error_message.is_some() {
        return Err(ThreadRepositoryError::InvalidRecord("error code and message must be present together".to_string()));
    }
    Ok(())
}
