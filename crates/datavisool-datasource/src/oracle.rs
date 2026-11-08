/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
#[path = "oracle_config.rs"]
mod oracle_config;
#[cfg(test)]
#[path = "oracle_tests.rs"]
mod oracle_tests;
#[path = "oracle_values.rs"]
mod oracle_values;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{ConnectionConfig, DriverError, ErrorKind};

const MAX_BLOCKING_ORACLE_WORKERS: usize = 4;
const CONNECTION_DEADLINE: Duration = Duration::from_secs(10);
const QUERY_DEADLINE: Duration = Duration::from_secs(30);
const CONNECTED_CALL_TIMEOUT: Duration = Duration::from_secs(25);
const TEST_CALL_TIMEOUT: Duration = Duration::from_secs(8);

static ORACLE_WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();

struct CancellationGuard(Arc<AtomicBool>);

impl Drop for CancellationGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn worker_semaphore() -> Arc<Semaphore> {
    ORACLE_WORKERS
        .get_or_init(|| Arc::new(Semaphore::new(MAX_BLOCKING_ORACLE_WORKERS)))
        .clone()
}

fn acquire_worker(kind: ErrorKind) -> Result<OwnedSemaphorePermit, DriverError> {
    worker_semaphore()
        .try_acquire_owned()
        .map_err(|_| DriverError::new(kind, "Oracle连接操作繁忙，请稍后重试"))
}

async fn run_blocking<T, F>(
    kind: ErrorKind,
    deadline: Duration,
    operation: F,
) -> Result<T, DriverError>
where
    T: Send + 'static,
    F: FnOnce(Arc<AtomicBool>) -> Result<T, DriverError> + Send + 'static,
{
    let permit = acquire_worker(kind)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    // Also cancel if the caller drops this future (for example, when the
    // frontend request is aborted); the blocking worker may otherwise outlive it.
    let _cancellation_guard = CancellationGuard(Arc::clone(&cancelled));
    let worker_cancelled = Arc::clone(&cancelled);
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        operation(worker_cancelled)
    });

    match tokio::time::timeout(deadline, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(DriverError::new(kind, "Oracle后台连接任务异常终止")),
        Err(_) => {
            cancelled.store(true, Ordering::Release);
            let seconds = deadline.as_secs();
            let message = match kind {
                ErrorKind::Connection => format!("Oracle连接测试超过{seconds}秒，已取消后续操作"),
                ErrorKind::Query => format!("Oracle SQL执行超过{seconds}秒，已取消后续操作"),
                ErrorKind::Configuration => "Oracle操作超时".to_string(),
            };
            Err(DriverError::new(kind, message))
        }
    }
}

fn is_cancelled(cancelled: &AtomicBool) -> bool {
    cancelled.load(Ordering::Acquire)
}

fn cancelled_error(kind: ErrorKind) -> DriverError {
    DriverError::new(kind, "Oracle操作已超时并取消后续操作")
}

fn connect(
    config: &ConnectionConfig,
    call_timeout: Duration,
    kind: ErrorKind,
) -> Result<oracledb::Connection, DriverError> {
    let driver_config = oracle_config::driver_config(config)?;
    let connection = oracledb::connect(driver_config)
        .map_err(|error| DriverError::new(kind, format!("Oracle数据库连接失败：{error}")))?;
    connection
        .set_call_timeout(Some(call_timeout))
        .map_err(|error| DriverError::new(kind, format!("设置Oracle调用超时失败：{error}")))?;
    Ok(connection)
}

pub(crate) async fn test_connection(config: ConnectionConfig) -> Result<(), DriverError> {
    run_blocking(
        ErrorKind::Connection,
        CONNECTION_DEADLINE,
        move |cancelled| {
            if is_cancelled(&cancelled) {
                return Err(cancelled_error(ErrorKind::Connection));
            }
            let connection = connect(&config, TEST_CALL_TIMEOUT, ErrorKind::Connection)?;
            if is_cancelled(&cancelled) {
                return Err(cancelled_error(ErrorKind::Connection));
            }
            connection.ping().map_err(|error| {
                DriverError::new(
                    ErrorKind::Connection,
                    format!("Oracle连通性校验失败：{error}"),
                )
            })?;
            Ok(())
        },
    )
    .await
}

pub(crate) async fn query(
    config: ConnectionConfig,
    statement: String,
) -> Result<serde_json::Value, DriverError> {
    run_blocking(ErrorKind::Query, QUERY_DEADLINE, move |cancelled| {
        if is_cancelled(&cancelled) {
            return Err(cancelled_error(ErrorKind::Query));
        }
        let connection = connect(&config, CONNECTED_CALL_TIMEOUT, ErrorKind::Connection)?;
        if is_cancelled(&cancelled) {
            return Err(cancelled_error(ErrorKind::Query));
        }

        let statement = connection
            .statement(statement.trim())
            .map_err(|error| {
                DriverError::new(ErrorKind::Query, format!("Oracle SQL解析失败：{error}"))
            })?
            .build()
            .map_err(|error| {
                DriverError::new(ErrorKind::Query, format!("Oracle SQL准备失败：{error}"))
            })?;

        if is_cancelled(&cancelled) {
            return Err(cancelled_error(ErrorKind::Query));
        }

        if statement.is_query() {
            let cursor = statement.query(&[]).map_err(|error| {
                DriverError::new(ErrorKind::Query, format!("Oracle查询执行失败：{error}"))
            })?;
            oracle_values::cursor_to_json(cursor, &cancelled)
        } else {
            let mut statement = statement;
            statement.execute(&[]).map_err(|error| {
                DriverError::new(ErrorKind::Query, format!("Oracle SQL执行失败：{error}"))
            })?;
            if is_cancelled(&cancelled) {
                let _ = connection.rollback();
                return Err(cancelled_error(ErrorKind::Query));
            }
            connection.commit().map_err(|error| {
                DriverError::new(ErrorKind::Query, format!("Oracle事务提交失败：{error}"))
            })?;
            Ok(serde_json::Value::Array(Vec::new()))
        }
    })
    .await
}
