/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 应用重启时对活动 Turn、TaskRun、Item 与 Inbox 做一次性联合收口。

use serde_json::json;
use sqlx::{Row, Sqlite, Transaction};

use crate::backend::ai_driven::thread::error::ThreadRepositoryError;
use crate::backend::ai_driven::thread::id::new_event_id;

use super::{allocate_event_sequences, insert_event, SqliteThreadRepository};

const ACTIVE_TURN_STATUSES: &str = "'created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation'";

#[derive(Debug)]
struct InterruptedScope {
    project_id: i64,
    thread_id: String,
    turn_id: String,
    run_id: String,
}

impl SqliteThreadRepository {
    pub(crate) async fn interrupt_active_turns(&self, interrupted_at: i64) -> Result<u64, ThreadRepositoryError> {
        if interrupted_at < 0 {
            return Err(ThreadRepositoryError::InvalidRecord("interrupted_at must not be negative".to_string()));
        }
        let mut transaction = self.pool.begin().await?;
        let scopes = active_scopes(&mut transaction).await?;
        for scope in &scopes {
            close_open_items(&mut transaction, scope, interrupted_at).await?;
            sqlx::query(
                "UPDATE ai_agent_turn_input SET status = 'abandoned', row_version = row_version + 1, abandoned_at = ?1 \
                 WHERE turn_id = ?2 AND status = 'accepted'",
            )
            .bind(interrupted_at)
            .bind(&scope.turn_id)
            .execute(&mut *transaction)
            .await?;

            let run_updated = sqlx::query(
                "UPDATE ai_agent_task_run SET status = 'failed', pending_json = NULL, error_code = 'agent.restart_interrupted', \
                     error_message = '应用重启，中断未完成的执行', updated_at = ?1, ended_at = ?1 \
                 WHERE run_id = ?2 AND turn_id = ?3 \
                   AND status IN ('created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation')",
            )
            .bind(interrupted_at)
            .bind(&scope.run_id)
            .bind(&scope.turn_id)
            .execute(&mut *transaction)
            .await?;
            let turn_updated = sqlx::query(&format!(
                "UPDATE ai_agent_turn SET status = 'interrupted', error_code = 'agent.restart_interrupted', \
                     error_message = '应用重启，中断未完成的执行', row_version = row_version + 1, updated_at = ?1, ended_at = ?1 \
                 WHERE turn_id = ?2 AND project_id = ?3 AND thread_id = ?4 AND status IN ({ACTIVE_TURN_STATUSES})"
            ))
            .bind(interrupted_at)
            .bind(&scope.turn_id)
            .bind(scope.project_id)
            .bind(&scope.thread_id)
            .execute(&mut *transaction)
            .await?;
            if run_updated.rows_affected() != 1 || turn_updated.rows_affected() != 1 {
                return Err(ThreadRepositoryError::conflict(
                    "agent.recovery.concurrent_update",
                    format!("Turn {} 的恢复收口发生并发冲突", scope.turn_id),
                ));
            }
            append_interrupted_event(&mut transaction, scope, interrupted_at).await?;
            sqlx::query(
                "UPDATE ai_agent_thread SET updated_at = MAX(updated_at, ?1), last_activity_at = MAX(last_activity_at, ?1) \
                 WHERE project_id = ?2 AND thread_id = ?3",
            )
            .bind(interrupted_at)
            .bind(scope.project_id)
            .bind(&scope.thread_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        u64::try_from(scopes.len()).map_err(|_| ThreadRepositoryError::InvalidRecord("interrupted Turn count is out of range".to_string()))
    }
}

async fn active_scopes(transaction: &mut Transaction<'_, Sqlite>) -> Result<Vec<InterruptedScope>, ThreadRepositoryError> {
    let rows = sqlx::query(&format!(
        "SELECT turn.project_id, turn.thread_id, turn.turn_id, run.run_id \
         FROM ai_agent_turn turn \
         LEFT JOIN ai_agent_task_run run ON run.task_id = (\
             SELECT candidate.task_id FROM ai_agent_task_run candidate \
             WHERE candidate.turn_id = turn.turn_id ORDER BY candidate.attempt_no DESC LIMIT 1\
         ) WHERE turn.status IN ({ACTIVE_TURN_STATUSES}) ORDER BY turn.project_id, turn.thread_id"
    ))
    .fetch_all(&mut **transaction)
    .await?;
    rows.into_iter()
        .map(|row| {
            let turn_id: String = row.try_get("turn_id")?;
            let run_id: Option<String> = row.try_get("run_id")?;
            Ok(InterruptedScope {
                project_id: row.try_get("project_id")?,
                thread_id: row.try_get("thread_id")?,
                turn_id: turn_id.clone(),
                run_id: run_id.ok_or_else(|| ThreadRepositoryError::InvalidRecord(format!("active Turn {turn_id} is missing its TaskRun")))?,
            })
        })
        .collect()
}

async fn close_open_items(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: &InterruptedScope,
    interrupted_at: i64,
) -> Result<(), ThreadRepositoryError> {
    let rows = sqlx::query(
        "SELECT item_id, item_type, schema_version, payload_json FROM ai_agent_item \
         WHERE turn_id = ?1 AND status IN ('created','inProgress','waiting') ORDER BY item_sequence ASC",
    )
    .bind(&scope.turn_id)
    .fetch_all(&mut **transaction)
    .await?;
    for row in rows {
        let item_id: String = row.try_get("item_id")?;
        let item_type: String = row.try_get("item_type")?;
        let schema_version = u32::try_from(row.try_get::<i64, _>("schema_version")?)
            .map_err(|_| ThreadRepositoryError::InvalidRecord("item schema version is out of range".to_string()))?;
        let payload: serde_json::Value = serde_json::from_str(&row.try_get::<String, _>("payload_json")?)?;
        let (sequence, thread_sequence) = allocate_event_sequences(transaction, &scope.thread_id, &scope.run_id, interrupted_at).await?;
        let updated = sqlx::query(
            "UPDATE ai_agent_item SET status = 'cancelled', last_event_sequence = ?1, row_version = row_version + 1, \
                 updated_at = ?2, completed_at = ?2 WHERE turn_id = ?3 AND item_id = ?4 \
                 AND status IN ('created','inProgress','waiting')",
        )
        .bind(
            i64::try_from(thread_sequence)
                .map_err(|_| ThreadRepositoryError::InvalidRecord("thread event sequence exceeds SQLite INTEGER".to_string()))?,
        )
        .bind(interrupted_at)
        .bind(&scope.turn_id)
        .bind(&item_id)
        .execute(&mut **transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(ThreadRepositoryError::conflict("agent.recovery.item_conflict", "恢复时 Item 状态已变化"));
        }
        insert_event(
            transaction,
            &new_event_id(),
            &scope.thread_id,
            &scope.turn_id,
            Some(&item_id),
            &scope.run_id,
            sequence,
            thread_sequence,
            "item.cancelled",
            &json!({
                "itemId": item_id,
                "itemType": item_type,
                "status": "cancelled",
                "payload": payload,
                "reason": "restartInterrupted",
                "item": {
                    "itemId": item_id,
                    "itemType": item_type,
                    "status": "cancelled",
                    "payload": payload,
                    "reason": "restartInterrupted"
                }
            }),
            &format!("restart:{}:item:{}:cancelled", scope.turn_id, item_id),
            schema_version,
            interrupted_at,
        )
        .await?;
    }
    Ok(())
}

async fn append_interrupted_event(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: &InterruptedScope,
    interrupted_at: i64,
) -> Result<(), ThreadRepositoryError> {
    let (sequence, thread_sequence) = allocate_event_sequences(transaction, &scope.thread_id, &scope.run_id, interrupted_at).await?;
    insert_event(
        transaction,
        &new_event_id(),
        &scope.thread_id,
        &scope.turn_id,
        None,
        &scope.run_id,
        sequence,
        thread_sequence,
        "turn.interrupted",
        &json!({
            "turnId": scope.turn_id,
            "runId": scope.run_id,
            "status": "interrupted",
            "errorCode": "agent.restart_interrupted"
        }),
        &format!("restart:{}:interrupted", scope.turn_id),
        1,
        interrupted_at,
    )
    .await?;
    Ok(())
}
