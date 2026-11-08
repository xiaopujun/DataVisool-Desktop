/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Task persistence regression tests, kept separate from the repository implementation.

use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

use super::*;
use crate::backend::ai_driven::task::{AgentExecutionLimitsSnapshot, AgentWorkingCheckpointState};
use crate::backend::ai_driven::thread::{CreateThreadInput, ThreadService};

const MIGRATION: &str = include_str!("../../../../migrations_desktop/1014_desktop_ai_thread_turn_item.sql");
const VERIFYING_MIGRATION: &str = include_str!("../../../../migrations_desktop/1016_desktop_ai_turn_verifying.sql");
const WORKING_CHECKPOINT_MIGRATION: &str = include_str!("../../../../migrations_desktop/1017_desktop_ai_working_checkpoint.sql");

async fn repository() -> (SqliteAgentTaskRepository, String) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE project (id INTEGER PRIMARY KEY, name TEXT NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(MIGRATION).execute(&pool).await.unwrap();
    sqlx::raw_sql(VERIFYING_MIGRATION).execute(&pool).await.unwrap();
    sqlx::raw_sql(WORKING_CHECKPOINT_MIGRATION).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO project (id, name, deleted) VALUES (1, 'project', 0)")
        .execute(&pool)
        .await
        .unwrap();
    let thread = ThreadService::new(pool.clone())
        .create_thread(CreateThreadInput { project_id: 1, create_request_id: "create-thread".to_string(), title: None, created_at: 1 })
        .await
        .unwrap();
    (SqliteAgentTaskRepository::new(pool), thread.thread_id)
}

fn new_task(thread_id: String) -> NewAgentTaskRun {
    NewAgentTaskRun {
        project_id: 1,
        thread_id,
        request_id: "request-1".to_string(),
        request: json!({ "prompt": "hello" }),
        input_summary: "hello".to_string(),
        model_config_id: "config".to_string(),
        runtime_model_id: "model".to_string(),
        provider: None,
        entry_digest: "sha256:entry".to_string(),
        client_session_id: "client".to_string(),
        task_type: "general".to_string(),
        limits: AgentExecutionLimitsSnapshot::default(),
        project_snapshot_ref: Some("workspace:snapshot".to_string()),
        rig_version: None,
        rig_snapshot_ref: None,
        user_item_payload: json!({ "text": "hello" }),
        attachments: Vec::new(),
        created_at: 2,
    }
}

#[tokio::test]
async fn creates_real_identity_and_appends_fully_scoped_event() {
    let (repository, thread_id) = repository().await;
    let task = repository.create(new_task(thread_id.clone())).await.unwrap();
    assert_eq!(task.project_id, 1);
    assert_eq!(task.thread_id, thread_id);
    assert!(task.turn_id.starts_with("ai-agent-turn-"));
    assert!(task.run_id.starts_with("ai-agent-run-"));

    let event = repository
        .append_event(NewAgentTaskEvent {
            project_id: 1,
            thread_id: task.thread_id.clone(),
            turn_id: task.turn_id.clone(),
            run_id: task.run_id.clone(),
            item_id: None,
            kind: "agent.queued".to_string(),
            payload: json!({}),
            dedupe_key: "agent:queued".to_string(),
            schema_version: 1,
            created_at: 3,
        })
        .await
        .unwrap();
    assert_eq!(event.thread_id, task.thread_id);
    assert_eq!(event.turn_id, task.turn_id);
    assert_eq!(event.run_id, task.run_id);
}

#[tokio::test]
async fn working_checkpoint_is_versioned_and_idempotent() {
    let (repository, thread_id) = repository().await;
    let task = repository.create(new_task(thread_id)).await.unwrap();
    let state = AgentWorkingCheckpointState::initial("hello", 3);
    let save = || SaveAgentWorkingCheckpoint {
        project_id: task.project_id,
        thread_id: task.thread_id.clone(),
        task_id: task.task_id.clone(),
        run_id: task.run_id.clone(),
        turn_id: task.turn_id.clone(),
        expected_version: 0,
        state: state.clone(),
    };

    let first = repository.save_working_checkpoint(save()).await.unwrap();
    let replay = repository.save_working_checkpoint(save()).await.unwrap();

    assert_eq!(first.checkpoint_version, 1);
    assert_eq!(replay, first);
    assert_eq!(repository.find_working_checkpoint(task.run_id).await.unwrap(), Some(first));
}

#[tokio::test]
async fn working_checkpoint_read_rejects_legacy_and_unknown_schema() {
    let (repository, thread_id) = repository().await;
    let task = repository.create(new_task(thread_id)).await.unwrap();
    sqlx::query(
        "INSERT INTO ai_agent_working_checkpoint \
             (run_id, task_id, turn_id, checkpoint_version, state_json, updated_at) \
             VALUES (?1, ?2, ?3, 1, ?4, 10)",
    )
    .bind(&task.run_id)
    .bind(&task.task_id)
    .bind(&task.turn_id)
    .bind(
        serde_json::json!({
            "schemaVersion": 3,
            "objective": "创建图表",
            "operationIds": ["operation-1"],
            "requiredCanvasEffects": ["createComponents"],
            "allowWorkspaceWrite": true,
            "updatedAt": 10,
        })
        .to_string(),
    )
    .execute(&repository.pool)
    .await
    .unwrap();

    let error = repository.find_working_checkpoint(task.run_id.clone()).await.unwrap_err();
    assert!(error.to_string().contains("不支持的工作检查点版本"));

    sqlx::query("UPDATE ai_agent_working_checkpoint SET state_json = ?1 WHERE run_id = ?2")
        .bind(serde_json::json!({"schemaVersion": 99, "objective": "创建图表", "updatedAt": 10}).to_string())
        .bind(&task.run_id)
        .execute(&repository.pool)
        .await
        .unwrap();
    let error = repository.find_working_checkpoint(task.run_id).await.unwrap_err();
    assert!(error.to_string().contains("不支持的工作检查点版本"));
}

#[tokio::test]
async fn completion_audit_transitions_running_to_verifying_then_terminal() {
    let (repository, thread_id) = repository().await;
    let task = repository.create(new_task(thread_id)).await.unwrap();
    let running = repository
        .save_checkpoint(checkpoint(&task, AgentTaskStatus::Running, 3, None))
        .await
        .unwrap();
    let verifying = repository
        .save_checkpoint(checkpoint(&running.task, AgentTaskStatus::Verifying, 4, None))
        .await
        .unwrap();
    assert_eq!(verifying.task.status, AgentTaskStatus::Verifying);
    assert_eq!(verifying.events[0].kind, "turn.verifying");

    let completed = repository
        .save_checkpoint(checkpoint(&verifying.task, AgentTaskStatus::Completed, 5, Some(5)))
        .await
        .unwrap();
    assert_eq!(completed.task.status, AgentTaskStatus::Completed);
    assert_eq!(completed.events.last().unwrap().kind, "turn.completed");
}

#[tokio::test]
async fn workspace_history_is_not_truncated_and_finds_older_completed_result() {
    let (repository, thread_id) = repository().await;
    sqlx::query(
        "WITH RECURSIVE sequence(value) AS (SELECT 1 UNION ALL SELECT value + 1 FROM sequence WHERE value < 502) \
             INSERT INTO ai_agent_turn (turn_id, project_id, thread_id, turn_sequence, request_id, status, request_json, \
               model_config_id, runtime_model_id, entry_digest, last_artifact_id, last_artifact_revision, created_at, updated_at, ended_at) \
             SELECT 'turn-' || value, 1, ?1, value, 'request-' || value, \
               CASE WHEN value = 1 THEN 'completed' ELSE 'failed' END, '{}', 'config', 'model', 'sha256:entry', \
               CASE WHEN value = 1 THEN 'result' ELSE NULL END, CASE WHEN value = 1 THEN 7 ELSE NULL END, value, value, value \
             FROM sequence",
    )
    .bind(&thread_id)
    .execute(&repository.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO ai_agent_task_run (task_id, run_id, request_id, turn_id, attempt_no, client_session_id, status, task_type, \
               limits_json, last_artifact_id, last_artifact_revision, created_at, updated_at, ended_at) \
             SELECT 'task-' || turn_sequence, 'run-' || turn_sequence, request_id, turn_id, 1, 'client', status, 'general', '{}', \
               last_artifact_id, last_artifact_revision, created_at, updated_at, ended_at \
             FROM ai_agent_turn WHERE thread_id = ?1",
    )
    .bind(&thread_id)
    .execute(&repository.pool)
    .await
    .unwrap();

    let history = repository.list_workspace_history(1).await.unwrap();
    assert_eq!(history.len(), 502);
    assert_eq!(history.first().map(|entry| entry.turn_id.as_str()), Some("turn-502"));
    assert_eq!(history.last().map(|entry| entry.turn_id.as_str()), Some("turn-1"));
    assert_eq!(history.last().unwrap().result_revision, Some(7));
    assert_eq!(history.last().unwrap().status, AgentTaskStatus::Completed);
}

fn checkpoint(task: &AgentTaskRun, status: AgentTaskStatus, updated_at: i64, ended_at: Option<i64>) -> SaveAgentTaskCheckpoint {
    SaveAgentTaskCheckpoint {
        project_id: task.project_id,
        thread_id: task.thread_id.clone(),
        turn_id: task.turn_id.clone(),
        run_id: task.run_id.clone(),
        status,
        pending: None,
        usage: None,
        last_artifact: task.last_artifact.clone(),
        error: None,
        thread_preview: None,
        project_snapshot_ref: task.project_snapshot_ref.clone(),
        rig_version: task.rig_version.clone(),
        rig_snapshot_ref: task.rig_snapshot_ref.clone(),
        updated_at,
        ended_at,
    }
}
