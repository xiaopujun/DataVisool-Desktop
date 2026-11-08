/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 桌面端 SQLite 迁移验证。
//!
//! 这些测试使用内存数据库执行基础迁移和桌面端迁移，确保关键表、字段变更和 ref_key 回填逻辑可重复执行。

const CLEANUP_MARKER_KEY: &str = "1014_desktop_ai_thread_turn_item";
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::collections::HashSet;

/// 验证基础迁移和桌面端迁移能创建应用运行所需的核心表。
#[tokio::test]
async fn baseline_migration_creates_expected_tables() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite memory failed");

    datavisool_db::run_base_migrations(&pool).await.expect("run migrations failed");
    datavisool_db::run_custom_migrations(&pool, sqlx::migrate!("./migrations_desktop"))
        .await
        .expect("run desktop migrations failed");

    let rows: Vec<(String,)> = sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .fetch_all(&pool)
        .await
        .expect("query sqlite_master failed");

    // 只关心业务表是否存在，不依赖 SQLite 内部表。
    let names: HashSet<String> = rows.into_iter().map(|(s,)| s).collect();
    let expected = [
        "project",
        "font",
        "common_database",
        "common_api",
        "dynamic_script",
        "video",
        "three_d_model",
        "remote_component",
        "project_release",
        "image",
        "kv_config",
    ];
    for t in expected {
        assert!(names.contains(t), "missing table: {t}");
    }
}

#[tokio::test]
async fn verifying_status_migration_preserves_active_turn_and_run() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite memory failed");
    sqlx::raw_sql(include_str!("../migrations_desktop/1014_desktop_ai_thread_turn_item.sql"))
        .execute(&pool)
        .await
        .expect("apply thread schema failed");
    sqlx::raw_sql(include_str!("../migrations_desktop/1015_desktop_ai_canvas_operation.sql"))
        .execute(&pool)
        .await
        .expect("apply operation schema failed");
    sqlx::raw_sql(
        "INSERT INTO ai_agent_thread (thread_id, project_id, title_source, status, create_request_id, created_at, updated_at, last_activity_at) \
         VALUES ('thread-1', 1, 'auto', 'active', 'create-1', 1, 1, 1); \
         INSERT INTO ai_agent_turn (turn_id, project_id, thread_id, turn_sequence, request_id, status, request_json, input_summary, \
           model_config_id, runtime_model_id, entry_digest, created_at, started_at, updated_at) \
         VALUES ('turn-1', 1, 'thread-1', 1, 'request-1', 'running', '{}', 'test', 'config', 'model', 'sha256:test', 2, 3, 3); \
         INSERT INTO ai_agent_task_run (task_id, run_id, request_id, turn_id, attempt_no, client_session_id, status, task_type, \
           limits_json, created_at, started_at, updated_at) \
         VALUES ('task-1', 'run-1', 'request-1', 'turn-1', 1, 'client-1', 'running', 'general', '{}', 2, 3, 3);",
    )
    .execute(&pool)
    .await
    .expect("insert pre-migration graph failed");

    sqlx::raw_sql(include_str!("../migrations_desktop/1016_desktop_ai_turn_verifying.sql"))
        .execute(&pool)
        .await
        .expect("apply verifying migration failed");
    sqlx::raw_sql(
        "UPDATE ai_agent_turn SET status = 'verifying' WHERE turn_id = 'turn-1'; \
         UPDATE ai_agent_task_run SET status = 'verifying' WHERE run_id = 'run-1';",
    )
    .execute(&pool)
    .await
    .expect("write verifying status failed");

    let turn_status: String = sqlx::query_scalar("SELECT status FROM ai_agent_turn WHERE turn_id = 'turn-1'")
        .fetch_one(&pool)
        .await
        .expect("read migrated turn failed");
    let run_status: String = sqlx::query_scalar("SELECT status FROM ai_agent_task_run WHERE run_id = 'run-1'")
        .fetch_one(&pool)
        .await
        .expect("read migrated run failed");
    assert_eq!(turn_status, "verifying");
    assert_eq!(run_status, "verifying");
}

/// 验证移除 ai_model.model_name 字段的迁移不会丢失其他列数据。
#[tokio::test]
async fn desktop_ai_model_model_name_drop_migration_preserves_other_columns() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite memory failed");

    // 先手动构造旧版表结构和旧数据，再单独执行字段删除迁移。
    sqlx::query(include_str!("../migrations_desktop/1004_desktop_ai_model.sql"))
        .execute(&pool)
        .await
        .expect("apply ai_model create migration failed");

    let inserted = sqlx::query(
        "INSERT INTO ai_model (name, provider, model_name, base_url, api_key, description, temperature, max_tokens, enabled, is_default, deleted) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0)",
    )
    .bind("测试模型")
    .bind("openai")
    .bind("gpt-4o-mini")
    .bind("https://api.openai.com/v1")
    .bind("enc:v1:test")
    .bind("legacy")
    .bind(0.9f64)
    .bind(4096i64)
    .bind(1i64)
    .bind(1i64)
    .execute(&pool)
    .await
    .expect("insert legacy ai_model row failed");

    sqlx::query(include_str!("../migrations_desktop/1011_desktop_ai_model_drop_model_name.sql"))
        .execute(&pool)
        .await
        .expect("apply ai_model drop migration failed");

    let columns = sqlx::query("PRAGMA table_info(ai_model)")
        .fetch_all(&pool)
        .await
        .expect("query ai_model columns failed");
    let column_names: Vec<String> = columns
        .into_iter()
        .map(|row| row.try_get("name").expect("read column name failed"))
        .collect();
    assert!(!column_names.iter().any(|name| name == "model_name"), "model_name column should be removed");

    let row = sqlx::query(
        "SELECT name, provider, base_url, api_key, description, temperature, max_tokens, enabled, is_default, deleted \
         FROM ai_model WHERE id = ?1",
    )
    .bind(inserted.last_insert_rowid())
    .fetch_one(&pool)
    .await
    .expect("query migrated ai_model row failed");

    assert_eq!(row.try_get::<String, _>("name").expect("read name failed"), "测试模型");
    assert_eq!(row.try_get::<String, _>("provider").expect("read provider failed"), "openai");
    assert_eq!(row.try_get::<String, _>("base_url").expect("read base_url failed"), "https://api.openai.com/v1");
    assert_eq!(row.try_get::<String, _>("api_key").expect("read api_key failed"), "enc:v1:test");
    assert_eq!(row.try_get::<String, _>("description").expect("read description failed"), "legacy");
    assert_eq!(row.try_get::<f64, _>("temperature").expect("read temperature failed"), 0.9);
    assert_eq!(row.try_get::<i64, _>("max_tokens").expect("read max_tokens failed"), 4096);
    assert_eq!(row.try_get::<i64, _>("enabled").expect("read enabled failed"), 1);
    assert_eq!(row.try_get::<i64, _>("is_default").expect("read is_default failed"), 1);
    assert_eq!(row.try_get::<i64, _>("deleted").expect("read deleted failed"), 0);
}

/// 验证 Agent 迁移建立完整目标结构、关键索引和逻辑外键策略。
#[tokio::test]
async fn desktop_agent_runtime_migration_creates_target_schema_without_physical_foreign_keys() {
    let pool = migrated_pool().await;
    let target_tables = [
        "ai_agent_thread",
        "ai_agent_turn",
        "ai_agent_item",
        "ai_agent_task_run",
        "ai_agent_task_event",
        "ai_agent_turn_input",
        "ai_agent_conversation",
        "ai_agent_conversation_message",
        "ai_agent_attachment",
        "ai_agent_runtime_cleanup_marker",
        "ai_agent_canvas_operation",
    ];
    let table_names: HashSet<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table'")
        .fetch_all(&pool)
        .await
        .expect("query target tables failed")
        .into_iter()
        .collect();
    for table in target_tables {
        assert!(table_names.contains(table), "missing target table: {table}");
        let foreign_keys = sqlx::query(&format!("PRAGMA foreign_key_list({table})"))
            .fetch_all(&pool)
            .await
            .expect("query physical foreign keys failed");
        assert!(foreign_keys.is_empty(), "target table must not have physical foreign keys: {table}");
    }
    assert!(!table_names.contains("ai_fusion_attachment"));

    for table in ["ai_agent_thread", "ai_agent_turn", "ai_agent_conversation", "ai_agent_attachment"] {
        let project_id_type: String = sqlx::query_scalar(&format!("SELECT type FROM pragma_table_info('{table}') WHERE name = 'project_id'"))
            .fetch_one(&pool)
            .await
            .expect("read project_id type failed");
        assert_eq!(project_id_type, "INTEGER", "project_id type mismatch for {table}");
    }

    let expected_indexes = [
        "idx_ai_agent_thread_project_activity",
        "uq_ai_agent_thread_scope",
        "uq_ai_agent_thread_create_request",
        "uq_ai_agent_turn_order",
        "uq_ai_agent_turn_request",
        "uq_ai_agent_turn_scope",
        "uq_ai_agent_turn_single_active",
        "uq_ai_agent_turn_single_project_active",
        "uq_ai_agent_item_order",
        "uq_ai_agent_item_scope",
        "uq_ai_agent_item_logical_key",
        "uq_ai_agent_task_run_run_id",
        "uq_ai_agent_task_run_attempt",
        "uq_ai_agent_task_run_scope",
        "uq_ai_agent_event_run_sequence",
        "uq_ai_agent_event_thread_sequence",
        "uq_ai_agent_event_dedupe",
        "uq_ai_agent_turn_input_item",
        "uq_ai_agent_conversation_thread",
        "uq_ai_agent_conversation_message_order",
        "uq_ai_agent_conversation_message_source_item",
        "idx_ai_agent_attachment_upload_request",
        "uq_ai_agent_canvas_operation_tool_call",
        "idx_ai_agent_canvas_operation_run",
        "idx_ai_agent_canvas_operation_status",
    ];
    let index_names: HashSet<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'index'")
        .fetch_all(&pool)
        .await
        .expect("query target indexes failed")
        .into_iter()
        .collect();
    for index in expected_indexes {
        assert!(index_names.contains(index), "missing target index: {index}");
    }

    for index in [
        "uq_ai_agent_turn_single_active",
        "uq_ai_agent_turn_single_project_active",
        "uq_ai_agent_item_logical_key",
        "idx_ai_agent_event_item_sequence",
        "uq_ai_agent_conversation_message_source_item",
        "idx_ai_agent_attachment_claim",
    ] {
        let definition: String = sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1")
            .bind(index)
            .fetch_one(&pool)
            .await
            .expect("query partial index definition failed");
        assert!(definition.contains("WHERE"), "index should be partial: {index}");
    }

    let event_columns = table_columns(&pool, "ai_agent_task_event").await;
    for column in ["sequence", "thread_sequence", "dedupe_key", "schema_version"] {
        assert!(event_columns.contains(column), "missing Event cursor/dedupe column: {column}");
    }
    assert!(table_columns(&pool, "ai_agent_thread").await.contains("next_event_sequence"));
    assert!(table_columns(&pool, "ai_agent_task_run").await.contains("next_event_sequence"));
    let marker_columns = table_columns(&pool, "ai_agent_runtime_cleanup_marker").await;
    for column in ["owner_token", "lease_expires_at"] {
        assert!(marker_columns.contains(column), "missing cleanup lease column: {column}");
    }

    let marker: (String, i64, Option<String>, Option<i64>) = sqlx::query_as(
        "SELECT status, attempt_count, owner_token, lease_expires_at \
         FROM ai_agent_runtime_cleanup_marker WHERE migration_key = ?1",
    )
    .bind(CLEANUP_MARKER_KEY)
    .fetch_one(&pool)
    .await
    .expect("query cleanup marker failed");
    assert_eq!(marker, ("pending".to_string(), 0, None, None));
}

/// 验证 common_api 的 ref_key 迁移会回填旧数据，并让新数据自动获得 ref_key。
#[tokio::test]
async fn desktop_common_api_ref_key_migration_backfills_existing_and_new_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite memory failed");

    datavisool_db::run_base_migrations(&pool).await.expect("run base migrations failed");

    sqlx::query("INSERT INTO common_api (name, url, method, params, headers, deleted) VALUES (?1, ?2, ?3, ?4, ?5, 0)")
        .bind("历史公共API")
        .bind("https://example.com/old")
        .bind("GET")
        .bind("{}")
        .bind("{}")
        .execute(&pool)
        .await
        .expect("insert legacy common_api failed");

    datavisool_db::run_custom_migrations(&pool, sqlx::migrate!("./migrations_desktop"))
        .await
        .expect("run desktop migrations failed");

    let legacy_ref_key: Option<String> = sqlx::query_scalar("SELECT ref_key FROM common_api WHERE name = '历史公共API'")
        .fetch_one(&pool)
        .await
        .expect("query legacy ref_key failed");
    assert!(legacy_ref_key.as_ref().is_some_and(|value| !value.trim().is_empty()));

    let inserted = sqlx::query("INSERT INTO common_api (name, url, method, params, headers, deleted) VALUES (?1, ?2, ?3, ?4, ?5, 0)")
        .bind("新公共API")
        .bind("https://example.com/new")
        .bind("POST")
        .bind("{}")
        .bind("{}")
        .execute(&pool)
        .await
        .expect("insert new common_api failed");

    let new_ref_key: Option<String> = sqlx::query_scalar("SELECT ref_key FROM common_api WHERE id = ?1")
        .bind(inserted.last_insert_rowid())
        .fetch_one(&pool)
        .await
        .expect("query new ref_key failed");
    assert!(new_ref_key.as_ref().is_some_and(|value| !value.trim().is_empty()));
    assert_ne!(legacy_ref_key, new_ref_key);
}

/// 验证 common_database 的 ref_key 迁移会回填旧数据，并让新数据自动获得 ref_key。
#[tokio::test]
async fn desktop_common_database_ref_key_migration_backfills_existing_and_new_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite memory failed");

    datavisool_db::run_base_migrations(&pool).await.expect("run base migrations failed");

    sqlx::query(
        "INSERT INTO common_database (name, type, ip, port, database_name, username, password, url, des, deleted) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
    )
    .bind("历史数据库")
    .bind("MySQL")
    .bind("127.0.0.1")
    .bind("3306")
    .bind("legacy_db")
    .bind("root")
    .bind("secret")
    .bind("mysql://root:secret@127.0.0.1:3306/legacy_db")
    .bind("legacy")
    .execute(&pool)
    .await
    .expect("insert legacy common_database failed");

    datavisool_db::run_custom_migrations(&pool, sqlx::migrate!("./migrations_desktop"))
        .await
        .expect("run desktop migrations failed");

    let legacy_ref_key: Option<String> = sqlx::query_scalar("SELECT ref_key FROM common_database WHERE name = '历史数据库'")
        .fetch_one(&pool)
        .await
        .expect("query legacy common_database ref_key failed");
    assert!(legacy_ref_key.as_ref().is_some_and(|value| !value.trim().is_empty()));

    let inserted = sqlx::query(
        "INSERT INTO common_database (name, type, ip, port, database_name, username, password, url, des, deleted) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
    )
    .bind("新数据库")
    .bind("PostgreSQL")
    .bind("127.0.0.1")
    .bind("5432")
    .bind("new_db")
    .bind("postgres")
    .bind("secret")
    .bind("postgres://postgres:secret@127.0.0.1:5432/new_db")
    .bind("new")
    .execute(&pool)
    .await
    .expect("insert new common_database failed");

    let new_ref_key: Option<String> = sqlx::query_scalar("SELECT ref_key FROM common_database WHERE id = ?1")
        .bind(inserted.last_insert_rowid())
        .fetch_one(&pool)
        .await
        .expect("query new common_database ref_key failed");
    assert!(new_ref_key.as_ref().is_some_and(|value| !value.trim().is_empty()));
    assert_ne!(legacy_ref_key, new_ref_key);
}

/// 验证公共 API 和公共数据库的 ref_key 回填覆盖所有公共数据源类型。
#[tokio::test]
async fn desktop_ref_key_backfill_migration_covers_all_common_data_sources() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite memory failed");

    datavisool_db::run_base_migrations(&pool).await.expect("run base migrations failed");

    sqlx::query("INSERT INTO common_api (name, url, method, params, headers, deleted) VALUES (?1, ?2, ?3, ?4, ?5, 0)")
        .bind("历史公共API")
        .bind("https://example.com/legacy")
        .bind("GET")
        .bind("{}")
        .bind("{}")
        .execute(&pool)
        .await
        .expect("insert legacy common_api failed");

    sqlx::query(
        "INSERT INTO common_database (name, type, ip, port, database_name, username, password, url, des, deleted) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
    )
    .bind("历史数据库")
    .bind("MySQL")
    .bind("127.0.0.1")
    .bind("3306")
    .bind("legacy_db")
    .bind("root")
    .bind("secret")
    .bind("mysql://root:secret@127.0.0.1:3306/legacy_db")
    .bind("legacy")
    .execute(&pool)
    .await
    .expect("insert legacy common_database failed");

    datavisool_db::run_custom_migrations(&pool, sqlx::migrate!("./migrations_desktop"))
        .await
        .expect("run desktop migrations failed");

    let common_api_ref_key: Option<String> = sqlx::query_scalar("SELECT ref_key FROM common_api WHERE name = '历史公共API'")
        .fetch_one(&pool)
        .await
        .expect("query common_api ref_key failed");
    let common_database_ref_key: Option<String> = sqlx::query_scalar("SELECT ref_key FROM common_database WHERE name = '历史数据库'")
        .fetch_one(&pool)
        .await
        .expect("query common_database ref_key failed");

    assert!(common_api_ref_key.as_ref().is_some_and(|value| !value.trim().is_empty()));
    assert!(common_database_ref_key.as_ref().is_some_and(|value| !value.trim().is_empty()));
}

/// 模拟已执行到 1013 的真实升级，验证只清空旧 Agent 数据，不触碰业务数据。
#[tokio::test]
async fn desktop_agent_runtime_upgrade_clears_legacy_rows_and_preserves_business_rows() {
    let pool = migrated_pool().await;
    let project_id = sqlx::query("INSERT INTO project (name, des, data_json, deleted) VALUES ('保留项目', '业务描述', '{\"page\":1}', 0)")
        .execute(&pool)
        .await
        .expect("insert business project failed")
        .last_insert_rowid();
    let api_id = sqlx::query(
        "INSERT INTO common_api (name, url, method, params, headers, deleted) \
         VALUES ('保留接口', 'https://example.com/keep', 'GET', '{}', '{}', 0)",
    )
    .execute(&pool)
    .await
    .expect("insert business API failed")
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO ai_fusion_history (id, project_id, request_id, scene, status, title, summary, provider, model_name, created_at, updated_at) \
         VALUES ('fusion-keep', ?1, 'fusion-request', 'page', 'completed', '保留历史', '摘要', 'openai', 'model', 10, 10)",
    )
    .bind(project_id)
    .execute(&pool)
    .await
    .expect("insert non-Agent history failed");

    recreate_legacy_1013_schema(&pool).await;
    seed_legacy_agent_rows(&pool, project_id).await;

    let project_before: (String, Option<String>, Option<String>, i64) =
        sqlx::query_as("SELECT name, des, data_json, deleted FROM project WHERE id = ?1")
            .bind(project_id)
            .fetch_one(&pool)
            .await
            .expect("read project snapshot failed");
    let api_before: (String, String, String, Option<String>) = sqlx::query_as("SELECT name, url, method, ref_key FROM common_api WHERE id = ?1")
        .bind(api_id)
        .fetch_one(&pool)
        .await
        .expect("read API snapshot failed");
    let fusion_before: (String, String, String, i64) =
        sqlx::query_as("SELECT title, summary, status, deleted FROM ai_fusion_history WHERE id = 'fusion-keep'")
            .fetch_one(&pool)
            .await
            .expect("read fusion history snapshot failed");

    datavisool_db::run_custom_migrations(&pool, sqlx::migrate!("./migrations_desktop"))
        .await
        .expect("upgrade 1013 schema failed");

    for table in [
        "ai_agent_thread",
        "ai_agent_turn",
        "ai_agent_item",
        "ai_agent_task_run",
        "ai_agent_task_event",
        "ai_agent_turn_input",
        "ai_agent_conversation",
        "ai_agent_conversation_message",
        "ai_agent_attachment",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .expect("count rebuilt Agent table failed");
        assert_eq!(count, 0, "rebuilt Agent table should start empty: {table}");
    }
    let old_attachment_exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'ai_fusion_attachment'")
        .fetch_one(&pool)
        .await
        .expect("query old attachment table failed");
    assert_eq!(old_attachment_exists, 0);

    let project_after = sqlx::query_as("SELECT name, des, data_json, deleted FROM project WHERE id = ?1")
        .bind(project_id)
        .fetch_one(&pool)
        .await
        .expect("read migrated project failed");
    let api_after = sqlx::query_as("SELECT name, url, method, ref_key FROM common_api WHERE id = ?1")
        .bind(api_id)
        .fetch_one(&pool)
        .await
        .expect("read migrated API failed");
    let fusion_after = sqlx::query_as("SELECT title, summary, status, deleted FROM ai_fusion_history WHERE id = 'fusion-keep'")
        .fetch_one(&pool)
        .await
        .expect("read migrated fusion history failed");
    assert_eq!(project_before, project_after);
    assert_eq!(api_before, api_after);
    assert_eq!(fusion_before, fusion_after);
}

/// SQLx 迁移记录必须保证 1014 在后续启动不再执行，从而保留新 Agent 数据。
#[tokio::test]
async fn desktop_agent_runtime_migration_second_run_preserves_new_graph() {
    let pool = migrated_pool().await;
    insert_target_agent_graph(&pool).await;
    sqlx::query(
        "UPDATE ai_agent_runtime_cleanup_marker SET \
            status = 'succeeded', started_at = 1, finished_at = 2, updated_at = 2 \
         WHERE migration_key = ?1",
    )
    .bind(CLEANUP_MARKER_KEY)
    .execute(&pool)
    .await
    .expect("mark cleanup succeeded failed");

    datavisool_db::run_custom_migrations(&pool, sqlx::migrate!("./migrations_desktop"))
        .await
        .expect("rerun desktop migrations failed");

    for table in [
        "ai_agent_thread",
        "ai_agent_turn",
        "ai_agent_item",
        "ai_agent_task_run",
        "ai_agent_task_event",
        "ai_agent_turn_input",
        "ai_agent_conversation",
        "ai_agent_conversation_message",
        "ai_agent_attachment",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .expect("count preserved Agent table failed");
        assert_eq!(count, 1, "second migration run cleared new data: {table}");
    }
    let marker_status: String = sqlx::query_scalar("SELECT status FROM ai_agent_runtime_cleanup_marker WHERE migration_key = ?1")
        .bind(CLEANUP_MARKER_KEY)
        .fetch_one(&pool)
        .await
        .expect("read preserved cleanup marker failed");
    assert_eq!(marker_status, "succeeded");
}

async fn migrated_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite memory failed");
    datavisool_db::run_base_migrations(&pool).await.expect("run base migrations failed");
    datavisool_db::run_custom_migrations(&pool, sqlx::migrate!("./migrations_desktop"))
        .await
        .expect("run desktop migrations failed");
    pool
}

async fn table_columns(pool: &SqlitePool, table: &str) -> HashSet<String> {
    sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await
        .expect("query table columns failed")
        .into_iter()
        .map(|row| row.try_get("name").expect("read column name failed"))
        .collect()
}

async fn recreate_legacy_1013_schema(pool: &SqlitePool) {
    sqlx::raw_sql(
        "DROP TABLE IF EXISTS ai_agent_task_event;
         DROP TABLE IF EXISTS ai_agent_turn_input;
         DROP TABLE IF EXISTS ai_agent_conversation_message;
         DROP TABLE IF EXISTS ai_agent_item;
         DROP TABLE IF EXISTS ai_agent_task_run;
         DROP TABLE IF EXISTS ai_agent_conversation;
         DROP TABLE IF EXISTS ai_agent_turn;
         DROP TABLE IF EXISTS ai_agent_attachment;
         DROP TABLE IF EXISTS ai_agent_thread;
         DROP TABLE IF EXISTS ai_agent_runtime_cleanup_marker;",
    )
    .execute(pool)
    .await
    .expect("drop 1014 schema failed");
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 1014")
        .execute(pool)
        .await
        .expect("remove 1014 migration record failed");
    sqlx::raw_sql(include_str!("../migrations_desktop/1012_desktop_ai_fusion_attachment.sql"))
        .execute(pool)
        .await
        .expect("recreate legacy attachment schema failed");
    sqlx::raw_sql(include_str!("../migrations_desktop/1013_desktop_ai_agent_runtime.sql"))
        .execute(pool)
        .await
        .expect("recreate legacy Agent schema failed");
}

async fn seed_legacy_agent_rows(pool: &SqlitePool, project_id: i64) {
    sqlx::query(
        "INSERT INTO ai_agent_conversation (conversation_id, project_id, session_id, created_at, updated_at) \
         VALUES ('old-session', ?1, 'old-session', 1, 1)",
    )
    .bind(project_id.to_string())
    .execute(pool)
    .await
    .expect("insert legacy conversation failed");
    sqlx::query(
        "INSERT INTO ai_agent_conversation_message \
            (id, conversation_id, sequence, role, content_json, estimated_tokens, artifact_refs_json, created_at) \
         VALUES ('old-message', 'old-session', 1, 'user', '{}', 1, '[]', 1)",
    )
    .execute(pool)
    .await
    .expect("insert legacy conversation message failed");
    sqlx::query(
        "INSERT INTO ai_agent_task_run \
            (task_id, run_id, request_id, project_id, session_id, conversation_id, task_type, status, limits_json, created_at, updated_at) \
         VALUES ('old-task', 'old-run', 'old-request', ?1, 'old-session', 'old-session', 'design', 'completed', '{}', 1, 1)",
    )
    .bind(project_id.to_string())
    .execute(pool)
    .await
    .expect("insert legacy task run failed");
    sqlx::query(
        "INSERT INTO ai_agent_task_event (id, run_id, sequence, kind, payload_json, created_at) \
         VALUES ('old-event', 'old-run', 1, 'task.completed', '{}', 1)",
    )
    .execute(pool)
    .await
    .expect("insert legacy task event failed");
    sqlx::query(
        "INSERT INTO ai_fusion_attachment \
            (id, project_id, session_id, upload_request_id, mime_type, original_name, sha256, relative_path, byte_size, status, expire_at, created_at, updated_at) \
         VALUES ('old-attachment', ?1, 'old-session', 'old-upload', 'image/png', 'old.png', 'hash', 'ai-driven/attachments/old.png', 1, 'ready', 10, 1, 1)",
    )
    .bind(project_id)
    .execute(pool)
    .await
    .expect("insert legacy attachment failed");
}

async fn insert_target_agent_graph(pool: &SqlitePool) {
    sqlx::raw_sql(
        "INSERT INTO ai_agent_thread \
            (thread_id, project_id, title, title_source, status, create_request_id, next_turn_sequence, next_event_sequence, created_at, updated_at, last_activity_at) \
         VALUES ('thread-new', 8, '新对话', 'auto', 'active', 'create-new', 2, 2, 1, 1, 1);
         INSERT INTO ai_agent_turn \
            (turn_id, project_id, thread_id, turn_sequence, request_id, status, request_json, input_summary, model_config_id, runtime_model_id, entry_digest, next_item_sequence, created_at, started_at, updated_at, ended_at) \
         VALUES ('turn-new', 8, 'thread-new', 1, 'request-new', 'completed', '{}', '输入', 'model-config', 'model', 'digest', 2, 1, 1, 2, 2);
         INSERT INTO ai_agent_item \
            (item_id, turn_id, item_sequence, source_run_id, logical_key, item_type, status, schema_version, payload_json, last_event_sequence, created_at, updated_at, completed_at) \
         VALUES ('item-new', 'turn-new', 1, 'run-new', 'user:request-new', 'userMessage', 'completed', 1, '{}', 1, 1, 1, 1);
         INSERT INTO ai_agent_task_run \
            (task_id, run_id, request_id, turn_id, attempt_no, client_session_id, status, task_type, limits_json, next_event_sequence, created_at, started_at, updated_at, ended_at) \
         VALUES ('task-new', 'run-new', 'request-new', 'turn-new', 1, 'session-new', 'completed', 'design', '{}', 2, 1, 1, 2, 2);
         INSERT INTO ai_agent_task_event \
            (id, thread_id, turn_id, item_id, run_id, sequence, thread_sequence, kind, payload_json, dedupe_key, schema_version, created_at) \
         VALUES ('event-new', 'thread-new', 'turn-new', 'item-new', 'run-new', 1, 1, 'item.completed', '{}', 'item-new:completed', 1, 1);
         INSERT INTO ai_agent_turn_input \
            (input_id, turn_id, user_item_id, content_digest, status, created_at, consumed_at) \
         VALUES ('input-new', 'turn-new', 'item-new', 'digest', 'consumed', 1, 2);
         INSERT INTO ai_agent_conversation \
            (conversation_id, thread_id, project_id, next_message_sequence, created_at, updated_at) \
         VALUES ('thread-new', 'thread-new', 8, 2, 1, 1);
         INSERT INTO ai_agent_conversation_message \
            (id, conversation_id, sequence, turn_id, source_item_id, role, content_json, estimated_tokens, artifact_refs_json, created_at) \
         VALUES ('message-new', 'thread-new', 1, 'turn-new', 'item-new', 'user', '{}', 1, '[]', 1);
         INSERT INTO ai_agent_attachment \
            (attachment_id, project_id, upload_session_id, upload_request_id, claimed_thread_id, claimed_turn_id, claimed_at, status, mime_type, original_name, sha256, relative_path, byte_size, expire_at, created_at, updated_at) \
         VALUES ('attachment-new', 8, 'session-new', 'upload-new', 'thread-new', 'turn-new', 1, 'claimed', 'image/png', 'new.png', 'hash', 'ai-driven/attachments/new.png', 1, 100, 1, 1);",
    )
    .execute(pool)
    .await
    .expect("insert target Agent graph failed");
}
