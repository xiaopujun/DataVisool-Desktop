/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Shared Oracle and SQL Server connectivity for the designer and release runtime.

mod config;
mod error;
mod oracle;
mod readonly;
mod sql_server;

pub use config::ConnectionConfig;
pub use error::{DriverError, ErrorKind};
pub use readonly::is_readonly_query;

pub fn supports(database_type: &str) -> bool {
    matches!(
        database_type.trim().to_ascii_lowercase().as_str(),
        "oracle" | "sqlserver" | "mssql"
    )
}

pub async fn test_connection(
    database_type: &str,
    config: ConnectionConfig,
) -> Result<(), DriverError> {
    initialize_tls();
    let result = match database_type.trim().to_ascii_lowercase().as_str() {
        "oracle" => oracle::test_connection(config.clone()).await,
        "sqlserver" | "mssql" => sql_server::test_connection(config.clone()).await,
        _ => Err(DriverError::new(
            ErrorKind::Configuration,
            "不支持的数据源类型",
        )),
    };
    result.map_err(|error| error.sanitize(&config))
}

pub async fn query(
    database_type: &str,
    config: ConnectionConfig,
    statement: String,
) -> Result<serde_json::Value, DriverError> {
    initialize_tls();
    let result = match database_type.trim().to_ascii_lowercase().as_str() {
        "oracle" => oracle::query(config.clone(), statement).await,
        "sqlserver" | "mssql" => sql_server::query(config.clone(), statement).await,
        _ => Err(DriverError::new(
            ErrorKind::Configuration,
            "不支持的数据源类型",
        )),
    };
    result.map_err(|error| error.sanitize(&config))
}

fn initialize_tls() {
    // Both ring and aws-lc can be enabled by the host application. Oracle uses
    // rustls::ClientConfig::builder(), which otherwise panics with two providers.
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        }
    });
}
