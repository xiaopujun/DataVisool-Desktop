/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! TaskRun 检查点重试、全作用域状态读取与孤立执行收口。

use serde_json::Value;

use super::{
    AgentArtifactRef, AgentPendingSnapshot, AgentTaskCheckpointResult, AgentTaskError, AgentTaskEvent, AgentTaskRepository, AgentTaskRun,
    AgentTaskStatus, AiDrivenTaskRegistry, AiDrivenTaskRunSnapshot, NewAgentTaskEvent, SaveAgentTaskCheckpoint,
};

const PERSISTENCE_ATTEMPTS: usize = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ScopedTaskStatusError {
    NotFound,
    ScopeMismatch,
    Persistence(String),
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn save_status(
    repository: &dyn AgentTaskRepository,
    task: &AgentTaskRun,
    status: AgentTaskStatus,
    pending: Option<AgentPendingSnapshot>,
    usage: Option<Value>,
    last_artifact: Option<AgentArtifactRef>,
    error: Option<AgentTaskError>,
    thread_preview: Option<String>,
    ended_at: Option<i64>,
) -> Result<AgentTaskCheckpointResult, String> {
    let mut last_error = None;
    let mut attempts = 0;
    for attempt in 0..PERSISTENCE_ATTEMPTS {
        attempts += 1;
        let checkpoint = SaveAgentTaskCheckpoint {
            project_id: task.project_id,
            thread_id: task.thread_id.clone(),
            turn_id: task.turn_id.clone(),
            run_id: task.run_id.clone(),
            status,
            pending: pending.clone(),
            usage: usage.clone(),
            last_artifact: last_artifact.clone(),
            error: error.clone(),
            thread_preview: thread_preview.clone(),
            project_snapshot_ref: task.project_snapshot_ref.clone(),
            rig_version: task.rig_version.clone(),
            rig_snapshot_ref: task.rig_snapshot_ref.clone(),
            updated_at: now_ms(),
            ended_at,
        };
        match repository.save_checkpoint(checkpoint).await {
            Ok(saved) => return Ok(saved),
            Err(error) => {
                let retryable = error.is_transient_write_conflict();
                last_error = Some(error.to_string());
                if !retryable {
                    break;
                }
            }
        }
        retry_delay(attempt).await;
    }
    Err(format!(
        "checkpoint failed after {attempts} attempt(s): {}",
        last_error.unwrap_or_else(|| "unknown persistence error".to_string())
    ))
}

pub(super) async fn append_event_record(
    repository: &dyn AgentTaskRepository,
    task: &AgentTaskRun,
    item_id: Option<String>,
    kind: &str,
    payload: Value,
    dedupe_key: String,
) -> Result<AgentTaskEvent, String> {
    let mut last_error = None;
    let mut attempts = 0;
    for attempt in 0..PERSISTENCE_ATTEMPTS {
        attempts += 1;
        let event = NewAgentTaskEvent {
            project_id: task.project_id,
            thread_id: task.thread_id.clone(),
            turn_id: task.turn_id.clone(),
            run_id: task.run_id.clone(),
            item_id: item_id.clone(),
            kind: kind.to_string(),
            payload: payload.clone(),
            dedupe_key: dedupe_key.clone(),
            schema_version: 1,
            created_at: now_ms(),
        };
        match repository.append_event(event).await {
            Ok(event) => return Ok(event),
            Err(error) => {
                let retryable = error.is_transient_write_conflict();
                last_error = Some(error.to_string());
                if !retryable {
                    break;
                }
            }
        }
        retry_delay(attempt).await;
    }
    Err(format!(
        "event failed after {attempts} attempt(s): {}",
        last_error.unwrap_or_else(|| "unknown persistence error".to_string())
    ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn scoped_task_status(
    repository: &dyn AgentTaskRepository,
    registry: &AiDrivenTaskRegistry,
    project_id: i64,
    thread_id: &str,
    turn_id: &str,
    run_id: &str,
) -> Result<AiDrivenTaskRunSnapshot, ScopedTaskStatusError> {
    let task = repository
        .find_by_run_id(run_id.to_string())
        .await
        .map_err(|error| ScopedTaskStatusError::Persistence(error.to_string()))?
        .ok_or(ScopedTaskStatusError::NotFound)?;
    if task.project_id != project_id || task.thread_id != thread_id || task.turn_id != turn_id || task.run_id != run_id {
        return Err(ScopedTaskStatusError::ScopeMismatch);
    }
    if task.status.is_terminal() || registry.is_active(project_id, thread_id, turn_id, run_id) {
        return Ok(AiDrivenTaskRunSnapshot::from_persisted(&task));
    }

    let error = AgentTaskError {
        code: "agent.task.orphaned_non_terminal".to_string(),
        message: "任务处于非终态，但进程内没有对应执行器，已安全收口为失败".to_string(),
    };
    let ended_at = now_ms();
    let reconciled = save_status(
        repository,
        &task,
        AgentTaskStatus::Failed,
        None,
        task.usage.clone(),
        task.last_artifact.clone(),
        Some(error),
        None,
        Some(ended_at),
    )
    .await
    .map_err(ScopedTaskStatusError::Persistence)?;
    Ok(AiDrivenTaskRunSnapshot::from_persisted(&reconciled.task))
}

async fn retry_delay(attempt: usize) {
    if attempt + 1 < PERSISTENCE_ATTEMPTS {
        tokio::time::sleep(std::time::Duration::from_millis(10 * (attempt as u64 + 1))).await;
    }
}

fn now_ms() -> i64 {
    crate::backend::ai_driven::time::now_ms()
}
