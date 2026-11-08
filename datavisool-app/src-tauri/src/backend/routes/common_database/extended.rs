/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Adapt the shared network drivers to the existing desktop API response.

use datavisool_datasource::{ConnectionConfig, DriverError, ErrorKind};
use serde_json::Value;

use crate::backend::types::{err_with_detail, ok, HttpResponse};

pub(super) async fn test(database_type: &str, config: ConnectionConfig) -> HttpResponse {
    response(
        datavisool_datasource::test_connection(database_type, config)
            .await
            .map(|_| Value::Bool(true)),
    )
}

pub(super) async fn query(database_type: &str, config: ConnectionConfig, statement: String) -> HttpResponse {
    response(datavisool_datasource::query(database_type, config, statement).await)
}

fn response(result: Result<Value, DriverError>) -> HttpResponse {
    match result {
        Ok(value) => ok(value),
        Err(error) => {
            let (code, message) = match error.kind {
                ErrorKind::Configuration => (400, "数据库连接配置无效"),
                ErrorKind::Connection => (500, "连接数据库失败，请检查 IP/端口/账号密码/白名单"),
                ErrorKind::Query => (500, "执行 SQL 失败，请检查 SQL 语法与权限"),
            };
            err_with_detail(code, message, &error.message)
        }
    }
}
