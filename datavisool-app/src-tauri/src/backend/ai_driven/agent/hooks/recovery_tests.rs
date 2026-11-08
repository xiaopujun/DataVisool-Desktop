/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::sync::Arc;

use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

use super::AgentRuntimeHook;
use crate::backend::ai_driven::agent::{AgentExecutionLimits, ExecutionBudget};
use crate::backend::ai_driven::agent_context::input::{AgentRunInput, AgentShortcutCommand, AgentShortcutContext};
use crate::backend::ai_driven::task::{
    AgentExecutionLimitsSnapshot, AgentTaskRepository, AgentTaskRun, AgentWorkingCheckpointState, CompletionRequirementsState, NewAgentTaskRun,
    SaveAgentWorkingCheckpoint, SqliteAgentTaskRepository,
};
use crate::backend::ai_driven::thread::{CreateThreadInput, ThreadService};

const THREAD_MIGRATION: &str = include_str!("../../../../../migrations_desktop/1014_desktop_ai_thread_turn_item.sql");
const OPERATION_MIGRATION: &str = include_str!("../../../../../migrations_desktop/1015_desktop_ai_canvas_operation.sql");
const WORKING_CHECKPOINT_MIGRATION: &str = include_str!("../../../../../migrations_desktop/1017_desktop_ai_working_checkpoint.sql");

async fn persisted_runtime() -> (Arc<dyn AgentTaskRepository>, AgentTaskRun) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE project (id INTEGER PRIMARY KEY, name TEXT NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(THREAD_MIGRATION).execute(&pool).await.unwrap();
    sqlx::raw_sql(OPERATION_MIGRATION).execute(&pool).await.unwrap();
    sqlx::raw_sql(WORKING_CHECKPOINT_MIGRATION).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO project (id, name, deleted) VALUES (1, 'project', 0)")
        .execute(&pool)
        .await
        .unwrap();
    let thread = ThreadService::new(pool.clone())
        .create_thread(CreateThreadInput { project_id: 1, create_request_id: "recovery-test-thread".to_string(), title: None, created_at: 1 })
        .await
        .unwrap();
    let repository: Arc<dyn AgentTaskRepository> = Arc::new(SqliteAgentTaskRepository::new(pool.clone()));
    let task = repository
        .create(NewAgentTaskRun {
            project_id: 1,
            thread_id: thread.thread_id,
            request_id: "recovery-test-request".to_string(),
            request: json!({ "prompt": "创建组件" }),
            input_summary: "创建组件".to_string(),
            model_config_id: "config".to_string(),
            runtime_model_id: "model".to_string(),
            provider: None,
            entry_digest: "sha256:recovery-test".to_string(),
            client_session_id: "recovery-test-session".to_string(),
            task_type: "test".to_string(),
            limits: AgentExecutionLimitsSnapshot::default(),
            project_snapshot_ref: None,
            rig_version: None,
            rig_snapshot_ref: None,
            user_item_payload: json!({ "text": "创建组件" }),
            attachments: Vec::new(),
            created_at: 2,
        })
        .await
        .unwrap();
    repository
        .save_working_checkpoint(SaveAgentWorkingCheckpoint {
            project_id: task.project_id,
            thread_id: task.thread_id.clone(),
            task_id: task.task_id.clone(),
            run_id: task.run_id.clone(),
            turn_id: task.turn_id.clone(),
            expected_version: 0,
            state: AgentWorkingCheckpointState::initial("创建组件", 2),
        })
        .await
        .unwrap();
    (repository, task)
}

fn missing_canvas_input() -> AgentRunInput {
    AgentRunInput {
        shortcut: Some(AgentShortcutContext { command: AgentShortcutCommand::Style, raw_text: "调整样式".to_string() }),
        ..AgentRunInput::new("创建组件")
    }
}

#[tokio::test]
async fn completion_stop_budgets_are_persisted_and_restored_before_the_next_check() {
    let (repository, task) = persisted_runtime().await;
    let input = missing_canvas_input();
    let requirements = CompletionRequirementsState::for_input(&input);
    let hook = AgentRuntimeHook::new(ExecutionBudget::new(AgentExecutionLimits::default()), repository.clone(), task.clone())
        .with_completion_requirements(requirements);
    for expected in 1..=2 {
        assert_eq!(hook.completion_requirements.next_completion_attempt(), expected);
        assert_eq!(
            hook.completion_requirements
                .missing_requirement_attempt(Some("missing:createComponents")),
            expected
        );
        assert_eq!(hook.completion_requirements.continuation_state(true, 12), (expected, false));
        hook.persist_run_recovery().await.unwrap();
    }

    let checkpoint = repository.find_working_checkpoint(task.run_id.clone()).await.unwrap().unwrap();
    let recovery = &checkpoint.state.run_recovery_state["completion"];
    assert_eq!(recovery["attempt"], 2);
    assert_eq!(recovery["missingRequirementAttempt"], 2);
    assert_eq!(recovery["continuationCount"], 2);

    let resumed_requirements = CompletionRequirementsState::for_input(&input);
    let resumed_hook = AgentRuntimeHook::new(ExecutionBudget::new(AgentExecutionLimits::default()), repository.clone(), task.clone())
        .with_completion_requirements(resumed_requirements.clone())
        .with_recovery_state(Some(&checkpoint.state.run_recovery_state))
        .unwrap();
    let restored = resumed_hook.completion_requirements.completion_recovery_snapshot();
    assert_eq!(restored["attempt"], 2);
    assert_eq!(restored["missingRequirementAttempt"], 2);
    assert_eq!(restored["continuationCount"], 2);

    assert_eq!(resumed_hook.completion_requirements.next_completion_attempt(), 3);
    assert_eq!(
        resumed_hook
            .completion_requirements
            .missing_requirement_attempt(Some("missing:createComponents")),
        3
    );
    assert_eq!(resumed_hook.completion_requirements.continuation_state(true, 12), (3, false));
    resumed_hook.persist_run_recovery().await.unwrap();
    let checkpoint = repository.find_working_checkpoint(task.run_id).await.unwrap().unwrap();
    assert_eq!(checkpoint.state.run_recovery_state["completion"]["attempt"], 3);
    assert_eq!(checkpoint.state.run_recovery_state["completion"]["missingRequirementAttempt"], 3);
    assert_eq!(checkpoint.state.run_recovery_state["completion"]["continuationCount"], 3);
}
