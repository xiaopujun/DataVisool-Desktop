/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 应用重启后为同一 Turn 创建不可变的新执行 attempt。

use serde_json::json;
use sqlx::{Row, Sqlite, Transaction};

use crate::backend::ai_driven::task::{AgentWorkingCheckpointState, AGENT_RESTART_INTERRUPTED};
use crate::backend::ai_driven::thread::error::ThreadRepositoryError;
use crate::backend::ai_driven::thread::id::{new_event_id, new_run_id, new_task_id};
use crate::backend::ai_driven::thread::state::{AgentTaskRunRef, CreateResumeAttemptInput, CreateResumeAttemptResult};

use super::{allocate_event_sequences, ensure_scope, insert_event, run_from_row, SqliteThreadRepository};

const RUN_SELECT: &str = concat!(
    "SELECT r.task_id, r.run_id, r.request_id, turn.entry_digest, turn.project_id, turn.thread_id, r.turn_id, r.attempt_no, r.client_session_id, ",
    "r.status, r.task_type, r.limits_json, r.pending_json, r.usage_json, r.last_artifact_id, r.last_artifact_revision, ",
    "r.error_code, r.error_message, r.project_snapshot_ref, r.rig_version, r.rig_snapshot_ref, r.created_at, r.started_at, r.updated_at, r.ended_at ",
    "FROM ai_agent_task_run r JOIN ai_agent_turn turn ON turn.turn_id = r.turn_id "
);

impl SqliteThreadRepository {
    pub(crate) async fn create_resume_attempt(&self, input: CreateResumeAttemptInput) -> Result<CreateResumeAttemptResult, ThreadRepositoryError> {
        validate_input(&input)?;
        let mut transaction = self.pool.begin().await?;
        ensure_scope(&mut transaction, input.project_id, &input.thread_id, Some(&input.turn_id), Some(&input.previous_run_id), None).await?;
        if let Some(existing_run_id) = sqlx::query_scalar::<_, String>("SELECT run_id FROM ai_agent_run_resume WHERE previous_run_id = ?1")
            .bind(&input.previous_run_id)
            .fetch_optional(&mut *transaction)
            .await?
        {
            let run = find_run(&mut transaction, &existing_run_id)
                .await?
                .ok_or_else(|| ThreadRepositoryError::InvalidRecord("恢复血缘指向不存在的 Run".to_string()))?;
            transaction.commit().await?;
            return Ok(CreateResumeAttemptResult { run, events: Vec::new(), replayed: true });
        }

        let source = find_run(&mut transaction, &input.previous_run_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "task run", key: input.previous_run_id.clone() })?;
        let latest_run_id =
            sqlx::query_scalar::<_, String>("SELECT run_id FROM ai_agent_task_run WHERE turn_id = ?1 ORDER BY attempt_no DESC LIMIT 1")
                .bind(&input.turn_id)
                .fetch_one(&mut *transaction)
                .await?;
        if latest_run_id != input.previous_run_id {
            return Err(ThreadRepositoryError::conflict("agent.resume.stale_run", "只能恢复当前 Turn 的最后一次执行"));
        }
        let budget_stop = source.status == "budgetExhausted";
        if !budget_stop && (source.status != "failed" || source.error_code.as_deref() != Some(AGENT_RESTART_INTERRUPTED)) {
            return Err(ThreadRepositoryError::InvalidTransition("仅重启中断或预算耗尽的任务可以恢复".to_string()));
        }
        if budget_stop != input.budget_extension.is_some() {
            return Err(ThreadRepositoryError::InvalidRecord("预算停止须显式延长预算；重启恢复不得变更预算".to_string()));
        }
        let turn_status = sqlx::query_scalar::<_, String>("SELECT status FROM ai_agent_turn WHERE turn_id = ?1")
            .bind(&input.turn_id)
            .fetch_one(&mut *transaction)
            .await?;
        if turn_status != if budget_stop { "budgetExhausted" } else { "interrupted" } {
            return Err(ThreadRepositoryError::conflict("agent.resume.turn_not_interrupted", "Turn 已不处于可恢复状态"));
        }
        let active_project_turns = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(1) FROM ai_agent_turn WHERE project_id = ?1 AND turn_id != ?2 \
             AND status IN ('created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation')",
        )
        .bind(input.project_id)
        .bind(&input.turn_id)
        .fetch_one(&mut *transaction)
        .await?;
        if active_project_turns != 0 {
            return Err(ThreadRepositoryError::conflict("agent.resume.project_busy", "当前项目已有正在执行的任务"));
        }
        let expected_checkpoint_version = i64::try_from(input.expected_checkpoint_version)
            .map_err(|_| ThreadRepositoryError::InvalidRecord("checkpoint version exceeds SQLite INTEGER".to_string()))?;
        let checkpoint = sqlx::query(
            "SELECT state_json, checkpoint_version FROM ai_agent_working_checkpoint \
             WHERE run_id = ?1 AND task_id = ?2 AND turn_id = ?3",
        )
        .bind(&source.run_id)
        .bind(&source.task_id)
        .bind(&source.turn_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| ThreadRepositoryError::conflict("agent.resume.checkpoint_missing", "任务没有可恢复的工作检查点"))?;
        let checkpoint_version: i64 = checkpoint.try_get("checkpoint_version")?;
        if checkpoint_version != expected_checkpoint_version {
            return Err(ThreadRepositoryError::conflict("agent.resume.checkpoint_conflict", "工作检查点版本已变化"));
        }
        let checkpoint_state_json: String = checkpoint.try_get("state_json")?;
        let mut checkpoint_state = AgentWorkingCheckpointState::from_json(&checkpoint_state_json).map_err(ThreadRepositoryError::InvalidRecord)?;
        let mut limits: crate::backend::ai_driven::task::AgentExecutionLimitsSnapshot = serde_json::from_value(source.limits.clone())?;
        let meter = crate::backend::ai_driven::agent::ExecutionBudget::new(limits.runtime_settings().execution.to_limits());
        if let Some(usage) = source.usage.clone() {
            meter.restore_usage(&serde_json::from_value(usage)?);
        }
        if let Some(usage) = checkpoint_state.run_recovery_state.get("usage") {
            meter.restore_usage(&serde_json::from_value(usage.clone())?);
        }
        let mut usage = meter.usage_snapshot();
        if let Some(extension) = &input.budget_extension {
            limits = limits
                .extend_budget(extension, &usage)
                .map_err(ThreadRepositoryError::InvalidRecord)?;
            usage.budget_warning_issued = false;
        }
        if !checkpoint_state.run_recovery_state.is_object() {
            checkpoint_state.run_recovery_state = json!({});
        }
        checkpoint_state.run_recovery_state["usage"] = serde_json::to_value(&usage)?;
        checkpoint_state.updated_at = input.created_at;
        checkpoint_state.pending = None;
        checkpoint_state.validate().map_err(ThreadRepositoryError::InvalidRecord)?;
        let checkpoint_state = serde_json::to_string(&checkpoint_state)?;

        let run_id = new_run_id();
        let task_id = new_task_id();
        let attempt_no = source.attempt_no.saturating_add(1);
        let last_artifact_revision = source
            .last_artifact_revision
            .map(i64::try_from)
            .transpose()
            .map_err(|_| ThreadRepositoryError::InvalidRecord("artifact revision exceeds SQLite INTEGER".to_string()))?;
        sqlx::query(
            "INSERT INTO ai_agent_task_run (\
                task_id, run_id, request_id, turn_id, attempt_no, client_session_id, status, task_type, limits_json, pending_json, usage_json, \
                last_artifact_id, last_artifact_revision, error_code, error_message, rig_version, rig_snapshot_ref, project_snapshot_ref, \
                next_event_sequence, created_at, started_at, updated_at, ended_at\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'created', ?7, ?8, NULL, ?15, ?9, ?10, NULL, NULL, ?11, ?12, ?13, 1, ?14, NULL, ?14, NULL)",
        )
        .bind(&task_id)
        .bind(&run_id)
        .bind(&source.request_id)
        .bind(&source.turn_id)
        .bind(i64::from(attempt_no))
        .bind(&input.client_session_id)
        .bind(&source.task_type)
        .bind(serde_json::to_string(&limits)?)
        .bind(source.last_artifact_id.as_deref())
        .bind(last_artifact_revision)
        .bind(source.rig_version.as_deref())
        .bind(source.rig_snapshot_ref.as_deref())
        .bind(source.project_snapshot_ref.as_deref())
        .bind(input.created_at)
        .bind(serde_json::to_string(&usage)?)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("INSERT INTO ai_agent_run_resume (run_id, previous_run_id, turn_id, created_at) VALUES (?1, ?2, ?3, ?4)")
            .bind(&run_id)
            .bind(&source.run_id)
            .bind(&source.turn_id)
            .bind(input.created_at)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO ai_agent_working_checkpoint \
             (run_id, task_id, turn_id, checkpoint_version, state_json, updated_at) VALUES (?1, ?2, ?3, 1, ?4, ?5)",
        )
        .bind(&run_id)
        .bind(&task_id)
        .bind(&source.turn_id)
        .bind(checkpoint_state)
        .bind(input.created_at)
        .execute(&mut *transaction)
        .await?;
        let turn_updated = sqlx::query(
            "UPDATE ai_agent_turn SET status = 'created', error_code = NULL, error_message = NULL, \
                 row_version = row_version + 1, updated_at = ?1, ended_at = NULL \
             WHERE turn_id = ?2 AND project_id = ?3 AND thread_id = ?4 AND status = ?5",
        )
        .bind(input.created_at)
        .bind(&input.turn_id)
        .bind(input.project_id)
        .bind(&input.thread_id)
        .bind(&turn_status)
        .execute(&mut *transaction)
        .await?;
        if turn_updated.rows_affected() != 1 {
            return Err(ThreadRepositoryError::conflict("agent.resume.concurrent_update", "恢复时 Turn 状态已变化"));
        }
        let (sequence, thread_sequence) = allocate_event_sequences(&mut transaction, &input.thread_id, &run_id, input.created_at).await?;
        let event = insert_event(
            &mut transaction,
            &new_event_id(),
            &input.thread_id,
            &input.turn_id,
            None,
            &run_id,
            sequence,
            thread_sequence,
            "agent.resume_attempt.created",
            &json!({
                "turnId": input.turn_id,
                "runId": run_id,
                "previousRunId": source.run_id,
                "checkpointVersion": input.expected_checkpoint_version,
                "budgetExtension": input.budget_extension,
                "authorizedLimits": limits,
                "carriedUsage": usage,
            }),
            &format!("resume:{}:{}", source.run_id, run_id),
            1,
            input.created_at,
        )
        .await?;
        let run = find_run(&mut transaction, &run_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::InvalidRecord("恢复 attempt 创建后无法读取".to_string()))?;
        transaction.commit().await?;
        Ok(CreateResumeAttemptResult { run, events: vec![event], replayed: false })
    }
}

fn validate_input(input: &CreateResumeAttemptInput) -> Result<(), ThreadRepositoryError> {
    if input.project_id <= 0 || input.created_at < 0 {
        return Err(ThreadRepositoryError::InvalidRecord("恢复作用域或时间无效".to_string()));
    }
    if [input.thread_id.as_str(), input.turn_id.as_str(), input.previous_run_id.as_str(), input.client_session_id.as_str()]
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err(ThreadRepositoryError::InvalidRecord("恢复标识不能为空".to_string()));
    }
    Ok(())
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
