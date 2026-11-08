/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use super::*;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

async fn database() -> DbState {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    datavisool_db::run_base_migrations(&pool).await.unwrap();
    datavisool_db::run_custom_migrations(&pool, sqlx::migrate!("./migrations_desktop"))
        .await
        .unwrap();
    DbState { pool }
}

async fn seed(db: &DbState, database_type: &str, port: &str, allow_script: bool) -> i64 {
    sqlx::query(
        "INSERT INTO common_database (name, type, ip, port, database_name, username, password, url, allow_sql_script, deleted) \
         VALUES ('driver test', ?1, '127.0.0.1', ?2, 'demo', 'tester', 'private-password', '', ?3, 0)",
    )
    .bind(database_type)
    .bind(port)
    .bind(if allow_script { 1_i64 } else { 0_i64 })
    .execute(&db.pool)
    .await
    .unwrap()
    .last_insert_rowid()
}

#[tokio::test]
async fn oracle_and_sql_server_test_connection_reach_the_driver() {
    let db = database().await;
    for database_type in ["Oracle", "ORACLE", "SQLServer"] {
        let id = seed(&db, database_type, "1", false).await;
        let result = test(&db, id).await.unwrap();
        assert_eq!(result.code, 500, "{database_type}: {}", result.msg);
        assert!(result.msg.contains("连接数据库失败"));
        assert!(!result.msg.contains("不支持的数据源类型"));
        assert!(!result.msg.contains("private-password"));
    }
}

#[tokio::test]
async fn oracle_and_sql_server_query_reach_the_driver() {
    let db = database().await;
    for database_type in ["Oracle", "SQLServer"] {
        let id = seed(&db, database_type, "1", false).await;
        let statement = base64::engine::general_purpose::STANDARD.encode("SELECT 1");
        let result = executor_execute(&db, json!({"id": id.to_string(), "statement": statement}))
            .await
            .unwrap();
        assert_eq!(result.code, 500, "{database_type}: {}", result.msg);
        assert!(result.msg.contains("连接数据库失败"));
        assert!(!result.msg.contains("private-password"));
    }
}

#[tokio::test]
async fn new_driver_configuration_errors_are_client_errors() {
    let db = database().await;
    for database_type in ["Oracle", "SQLServer"] {
        let id = seed(&db, database_type, "invalid-port", false).await;
        let result = test(&db, id).await.unwrap();
        assert_eq!(result.code, 400, "{database_type}: {}", result.msg);
        assert!(result.msg.contains("配置无效"));
    }
}

#[tokio::test]
async fn new_drivers_preserve_the_existing_statement_permission_gate() {
    let db = database().await;
    let overly_complex_query = format!("SELECT {}", vec!["1"; 1000].join(" + "));
    for database_type in ["Oracle", "SQLServer"] {
        let id = seed(&db, database_type, "invalid-port", false).await;
        let additional_sql: &[&str] = if database_type == "Oracle" {
            &["SELECT users_seq.NEXTVAL@remote_db FROM dual"]
        } else {
            &["SELECT id FROM users (UPDLOCK)", "SELECT id FROM users (TABLOCKX)", "SELECT id FROM users (XLOCK)"]
        };
        for sql in [
            "DELETE FROM demo",
            "SELECT 1; DELETE FROM demo",
            "WITH items AS (SELECT * FROM demo) DELETE FROM items",
            "SELECT * INTO copy_of_demo FROM demo",
            overly_complex_query.as_str(),
        ]
        .into_iter()
        .chain(additional_sql.iter().copied())
        {
            let statement = base64::engine::general_purpose::STANDARD.encode(sql);
            let result = executor_execute(&db, json!({"id": id.to_string(), "statement": statement}))
                .await
                .unwrap();
            assert_eq!(result.code, 400);
            assert_eq!(result.msg, "仅允许执行 SELECT/WITH 查询语句");
        }

        let permitted_sql: &[&str] = if database_type == "Oracle" {
            &[
                "SELECT * FROM demo AS OF SCN 123",
                "SELECT * FROM demo AS OF TIMESTAMP SYSTIMESTAMP",
                "SELECT changes.id FROM employees changes",
                "SELECT at.id FROM employees at",
                "SELECT before.id FROM employees before",
                "SELECT changes.id, at.id, before.id FROM employees AS OF SCN 123 changes JOIN departments AS OF TIMESTAMP SYSTIMESTAMP at ON changes.department_id = at.id JOIN locations before ON at.location_id = before.id",
            ]
        } else {
            &["SELECT 1 OPTION (MAXDOP 1)"]
        };
        for sql in permitted_sql {
            let statement = base64::engine::general_purpose::STANDARD.encode(sql);
            let result = executor_execute(&db, json!({"id": id.to_string(), "statement": statement}))
                .await
                .unwrap();
            assert_eq!(result.code, 400, "{database_type} {sql}: {}", result.msg);
            assert!(
                result.msg.contains("数据库连接配置无效"),
                "{database_type} {sql} should pass the permission gate and reach invalid-port validation: {}",
                result.msg
            );
        }
    }
}

#[tokio::test]
async fn unknown_database_types_are_still_rejected() {
    let db = database().await;
    let id = seed(&db, "UnknownDatabase", "1", false).await;
    let result = test(&db, id).await.unwrap();
    assert_eq!(result.code, 400);
    assert!(result.msg.contains("不支持的数据源类型"));
}
