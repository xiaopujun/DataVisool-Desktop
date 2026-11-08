/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

use super::*;
use crate::backend::crypto::server_public_key_pem;
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

fn public_key() -> String {
    server_public_key_pem()
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect()
}

fn encrypted(value: Value) -> Value {
    serde_json::to_value(encrypt_response(&public_key(), &value.to_string()).unwrap()).unwrap()
}

fn config(name: &str, description: &str) -> Value {
    json!({"name": name, "type": "MySQL", "ip": "127.0.0.1", "port": "3306", "databaseName": "demo", "username": "user", "password": "sensitive-secret", "url": "", "des": description})
}

#[tokio::test]
async fn search_and_list_do_not_expose_sensitive_url() {
    let db = database().await;
    let mut custom = config("primary", "unique description");
    custom["url"] = json!("mysql://user:sensitive-secret@127.0.0.1:3306/demo?ssl-mode=required");
    assert_eq!(add(&db, encrypted(custom)).await.unwrap().code, 200);
    for term in ["primary", "MySQL", "unique description"] {
        let response = page_list(&db, json!({"current": 1, "size": 10, "searchValue": term}))
            .await
            .unwrap();
        assert_eq!(response.data["total"], 1, "{term}");
        let serialized = response.data.to_string();
        assert!(!serialized.contains("sensitive-secret"));
        assert!(response.data["records"][0].get("url").is_none());
        assert!(response.data["records"][0].get("password").is_none());
    }
}

#[tokio::test]
async fn legacy_generated_urls_are_hidden_on_edit_and_custom_urls_survive() {
    let db = database().await;
    let url = policy::generated_url("MySQL", "127.0.0.1", "3306", "demo", "user", "sensitive-secret");
    let id: i64 = sqlx::query("INSERT INTO common_database (name, type, ip, port, database_name, username, password, url, deleted) VALUES ('legacy', 'MySQL', '127.0.0.1', '3306', 'demo', 'user', 'sensitive-secret', ?1, 0)")
        .bind(&url).execute(&db.pool).await.unwrap().last_insert_rowid();
    let response = get(&db, json!({"id": id.to_string(), "key": public_key()})).await.unwrap();
    let decrypted = decrypt_request(serde_json::from_value(response.data).unwrap()).unwrap();
    let data: Value = serde_json::from_str(&decrypted).unwrap();
    assert_eq!(data["url"], "");
    let mut updated = data;
    updated["port"] = json!("3307");
    updated["password"] = json!("new-secret");
    assert_eq!(update(&db, encrypted(updated)).await.unwrap().code, 200);
    let row = sqlx::query("SELECT port, password, url FROM common_database WHERE id = ?1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let stored_url: String = row.get("url");
    assert_eq!(stored_url, "");
    assert_eq!(
        build_url(Some(stored_url), "MySQL", "127.0.0.1", row.get("port"), "demo", "user", row.get("password")),
        "mysql://user:new-secret@127.0.0.1:3307/demo"
    );
    let custom = format!("{url}?ssl-mode=required");
    sqlx::query("UPDATE common_database SET url = ?1 WHERE id = ?2")
        .bind(&custom)
        .bind(id)
        .execute(&db.pool)
        .await
        .unwrap();
    let response = get(&db, json!({"id": id.to_string(), "key": public_key()})).await.unwrap();
    let decrypted = decrypt_request(serde_json::from_value(response.data).unwrap()).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&decrypted).unwrap()["url"], custom);
}

#[tokio::test]
async fn rejects_invalid_writes_without_mutating_data() {
    let db = database().await;
    for port in [json!(0), json!(65536), json!("bad")] {
        let mut value = config("invalid", "ok");
        value["port"] = port;
        assert_eq!(add(&db, encrypted(value)).await.unwrap().code, 400);
    }
    assert_eq!(add(&db, encrypted(config("invalid", &"a".repeat(501)))).await.unwrap().code, 400);
    let created = add(&db, encrypted(config("valid", "ok"))).await.unwrap();
    let mut update_value = config("valid", &"😀".repeat(251));
    update_value["id"] = created.data;
    assert_eq!(update(&db, encrypted(update_value)).await.unwrap().code, 400);
    let page = page_list(&db, json!({"current": 1, "size": 10})).await.unwrap();
    assert_eq!(page.data["total"], 1);
    assert_eq!(page.data["records"][0]["des"], "ok");
}

#[tokio::test]
async fn rejects_missing_or_whitespace_names_on_add_and_update_and_preserves_valid_text() {
    let db = database().await;
    let invalid_names = [None, Some(""), Some("   "), Some("\t"), Some("\u{3000}"), Some("\u{feff}"), Some("\u{0085}")];

    for name in invalid_names {
        let mut value = config("ignored", "ok");
        if let Some(name) = name {
            value["name"] = json!(name);
        } else {
            value.as_object_mut().unwrap().remove("name");
        }
        assert_eq!(add(&db, encrypted(value)).await.unwrap().code, 400);
    }

    let created = add(&db, encrypted(config("  original name  ", "ok"))).await.unwrap();
    let id = created.data.as_str().unwrap().to_owned();
    let stored_name: String = sqlx::query("SELECT name FROM common_database WHERE id = ?1")
        .bind(id.parse::<i64>().unwrap())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get("name");
    assert_eq!(stored_name, "  original name  ");

    for name in invalid_names {
        let mut value = config("ignored", "ok");
        value["id"] = json!(id);
        if let Some(name) = name {
            value["name"] = json!(name);
        } else {
            value.as_object_mut().unwrap().remove("name");
        }
        assert_eq!(update(&db, encrypted(value)).await.unwrap().code, 400);
    }
    let stored_name: String = sqlx::query("SELECT name FROM common_database WHERE id = ?1")
        .bind(id.parse::<i64>().unwrap())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get("name");
    assert_eq!(stored_name, "  original name  ");

    let mut update_value = config("  updated name  ", "ok");
    update_value["id"] = json!(id);
    assert_eq!(update(&db, encrypted(update_value)).await.unwrap().code, 200);
    let response = page_list(&db, json!({"current": 1, "size": 10})).await.unwrap();
    assert_eq!(response.data["records"][0]["name"], "  updated name  ");
}

#[tokio::test]
async fn explicitly_entered_generated_looking_url_is_preserved_on_edit_and_copy() {
    let db = database().await;
    let url = policy::generated_url("MySQL", "127.0.0.1", "3306", "demo", "user", "sensitive-secret");
    let mut source = config("explicit", "ok");
    source["url"] = json!(url);
    let created = add(&db, encrypted(source)).await.unwrap();
    let id = created.data.as_str().unwrap().to_owned();
    let response = get(&db, json!({"id": id, "key": public_key()})).await.unwrap();
    let decoded: Value = serde_json::from_str(&decrypt_request(serde_json::from_value(response.data).unwrap()).unwrap()).unwrap();
    assert_eq!(decoded["url"], url);
    let copied = copy(&db, id.parse().unwrap()).await.unwrap();
    let response = get(&db, json!({"id": copied.data, "key": public_key()})).await.unwrap();
    let decoded: Value = serde_json::from_str(&decrypt_request(serde_json::from_value(response.data).unwrap()).unwrap()).unwrap();
    assert_eq!(decoded["url"], url);
}
