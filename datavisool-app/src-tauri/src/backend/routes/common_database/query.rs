/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Execute SQLx database requests and convert the returned rows.

use base64::Engine;
use serde_json::{Map, Value};
use sqlx::{Acquire, Column, Row};

pub(super) async fn mysql(pool: &sqlx::MySqlPool, sql: &str, allow_script: bool) -> Result<Value, sqlx::Error> {
    let rows = if allow_script {
        // Stored routine definitions are not supported by MySQL's prepared-statement protocol.
        sqlx::raw_sql(sql).fetch_all(pool).await?
    } else {
        // SET applies to the next transaction on this exact connection.
        let mut connection = pool.acquire().await?;
        sqlx::query("SET TRANSACTION READ ONLY").execute(&mut *connection).await?;
        let mut tx = connection.begin().await?;
        let result = sqlx::query(sql).fetch_all(&mut *tx).await;
        let rollback = tx.rollback().await;
        let rows = result?;
        rollback?;
        rows
    };
    Ok(mysql_rows(rows))
}

pub(super) async fn postgres(pool: &sqlx::PgPool, sql: &str, allow_script: bool) -> Result<Value, sqlx::Error> {
    let rows = if allow_script {
        sqlx::query(sql).fetch_all(pool).await?
    } else {
        // A SELECT can call a function with side effects: enforce the mode in
        // the database, on the same connection and in the same transaction.
        let mut tx = pool.begin().await?;
        let result = async {
            sqlx::query("SET TRANSACTION READ ONLY").execute(&mut *tx).await?;
            sqlx::query(sql).fetch_all(&mut *tx).await
        }
        .await;
        let rollback = tx.rollback().await;
        let rows = result?;
        rollback?;
        rows
    };
    Ok(postgres_rows(rows))
}

fn mysql_rows(rows: Vec<sqlx::mysql::MySqlRow>) -> Value {
    Value::Array(
        rows.into_iter()
            .map(|row| {
                let mut object = Map::new();
                for (index, column) in row.columns().iter().enumerate() {
                    let value = row
                        .try_get::<i64, _>(index)
                        .map(Value::from)
                        .or_else(|_| row.try_get::<f64, _>(index).map(Value::from))
                        .or_else(|_| row.try_get::<bool, _>(index).map(Value::from))
                        .or_else(|_| row.try_get::<String, _>(index).map(Value::from))
                        .or_else(|_| {
                            row.try_get::<Vec<u8>, _>(index)
                                .map(|value| Value::from(base64::engine::general_purpose::STANDARD.encode(value)))
                        })
                        .unwrap_or(Value::Null);
                    object.insert(column.name().to_string(), value);
                }
                Value::Object(object)
            })
            .collect(),
    )
}

fn postgres_rows(rows: Vec<sqlx::postgres::PgRow>) -> Value {
    Value::Array(
        rows.into_iter()
            .map(|row| {
                let mut object = Map::new();
                for (index, column) in row.columns().iter().enumerate() {
                    let value = row
                        .try_get::<i16, _>(index)
                        .map(Value::from)
                        .or_else(|_| row.try_get::<i32, _>(index).map(Value::from))
                        .or_else(|_| row.try_get::<i64, _>(index).map(Value::from))
                        .or_else(|_| row.try_get::<f64, _>(index).map(Value::from))
                        .or_else(|_| row.try_get::<bool, _>(index).map(Value::from))
                        .or_else(|_| row.try_get::<String, _>(index).map(Value::from))
                        .or_else(|_| {
                            row.try_get::<Vec<u8>, _>(index)
                                .map(|value| Value::from(base64::engine::general_purpose::STANDARD.encode(value)))
                        })
                        .unwrap_or(Value::Null);
                    object.insert(column.name().to_string(), value);
                }
                Value::Object(object)
            })
            .collect(),
    )
}
