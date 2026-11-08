/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

use super::*;

#[tokio::test]
async fn init_interrupts_the_whole_active_turn_but_preserves_terminal_turns() {
    let db_file = std::env::temp_dir().join(format!(
        "datavisool-agent-restart-{}-{}.sqlite",
        std::process::id(),
        current_time_ms().expect("read current timestamp")
    ));
    let setup_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(&db_file).create_if_missing(true))
        .await
        .expect("connect setup database");
    datavisool_db::run_base_migrations(&setup_pool).await.expect("run base migrations");
    datavisool_db::run_custom_migrations(&setup_pool, sqlx::migrate!("./migrations_desktop"))
        .await
        .expect("run desktop migrations");

    insert_scope(&setup_pool, "active", "running", None).await;
    insert_scope(&setup_pool, "terminal", "completed", Some(20)).await;
    setup_pool.close().await;

    let db = DbState::init(&db_file).await.expect("initialize database");
    let active = sqlx::query(
        "SELECT turn.status AS turn_status, turn.error_code AS turn_error, run.status AS run_status, \
                run.error_code AS run_error, turn.ended_at AS turn_ended_at, run.ended_at AS run_ended_at \
         FROM ai_agent_turn turn JOIN ai_agent_task_run run ON run.turn_id = turn.turn_id \
         WHERE turn.turn_id = 'turn-active'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read interrupted scope");
    assert_eq!(active.try_get::<String, _>("turn_status").unwrap(), "interrupted");
    assert_eq!(active.try_get::<String, _>("run_status").unwrap(), "failed");
    assert_eq!(active.try_get::<Option<String>, _>("turn_error").unwrap().as_deref(), Some("agent.restart_interrupted"));
    assert_eq!(active.try_get::<Option<String>, _>("run_error").unwrap().as_deref(), Some("agent.restart_interrupted"));
    assert!(active.try_get::<Option<i64>, _>("turn_ended_at").unwrap().is_some());
    assert!(active.try_get::<Option<i64>, _>("run_ended_at").unwrap().is_some());

    let terminal: (String, String, Option<String>) = sqlx::query_as(
        "SELECT turn.status, run.status, run.error_code FROM ai_agent_turn turn \
         JOIN ai_agent_task_run run ON run.turn_id = turn.turn_id WHERE turn.turn_id = 'turn-terminal'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read terminal scope");
    assert_eq!(terminal, ("completed".to_string(), "completed".to_string(), None));

    let interrupted_events: i64 =
        sqlx::query_scalar("SELECT COUNT(1) FROM ai_agent_task_event WHERE turn_id = 'turn-active' AND kind = 'turn.interrupted'")
            .fetch_one(&db.pool)
            .await
            .expect("count interrupted events");
    assert_eq!(interrupted_events, 1);
    assert_eq!(
        ThreadService::new(db.pool.clone())
            .interrupt_active_turns(current_time_ms().unwrap())
            .await
            .unwrap(),
        0
    );

    db.pool.close().await;
    let _ = std::fs::remove_file(db_file);
}

async fn insert_scope(pool: &SqlitePool, suffix: &str, status: &str, ended_at: Option<i64>) {
    let project_id = sqlx::query("INSERT INTO project (name, des, data_json, deleted) VALUES (?1, '', '{}', 0)")
        .bind(format!("project-{suffix}"))
        .execute(pool)
        .await
        .expect("insert project")
        .last_insert_rowid();
    let thread_id = format!("thread-{suffix}");
    let turn_id = format!("turn-{suffix}");
    let run_id = format!("run-{suffix}");
    let request_id = format!("request-{suffix}");
    sqlx::query(
        "INSERT INTO ai_agent_thread (thread_id, project_id, title, title_source, preview, status, create_request_id, \
            next_turn_sequence, next_event_sequence, row_version, created_at, updated_at, last_activity_at, archived_at) \
         VALUES (?1, ?2, NULL, 'auto', NULL, 'active', ?3, 2, 1, 1, 1, 10, 10, NULL)",
    )
    .bind(&thread_id)
    .bind(project_id)
    .bind(format!("create-{suffix}"))
    .execute(pool)
    .await
    .expect("insert thread");
    sqlx::query(
        "INSERT INTO ai_agent_turn (turn_id, project_id, thread_id, turn_sequence, request_id, status, request_json, input_summary, \
            model_config_id, runtime_model_id, provider, entry_digest, next_item_sequence, row_version, created_at, started_at, updated_at, ended_at) \
         VALUES (?1, ?2, ?3, 1, ?4, ?5, '{}', 'test', 'model-config', 'model', NULL, ?6, 1, 1, 1, 2, 10, ?7)",
    )
    .bind(&turn_id)
    .bind(project_id)
    .bind(&thread_id)
    .bind(&request_id)
    .bind(status)
    .bind(format!("sha256:{suffix}"))
    .bind(ended_at)
    .execute(pool)
    .await
    .expect("insert turn");
    sqlx::query(
        "INSERT INTO ai_agent_task_run (task_id, run_id, request_id, turn_id, attempt_no, client_session_id, status, task_type, \
            limits_json, next_event_sequence, created_at, started_at, updated_at, ended_at) \
         VALUES (?1, ?2, ?3, ?4, 1, 'client-session', ?5, 'general', '{}', 1, 1, 2, 10, ?6)",
    )
    .bind(format!("task-{suffix}"))
    .bind(run_id)
    .bind(request_id)
    .bind(turn_id)
    .bind(status)
    .bind(ended_at)
    .execute(pool)
    .await
    .expect("insert task run");
}
