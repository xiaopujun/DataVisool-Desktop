/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Tauri 桌面端后端的本地数据库状态管理模块。
//!
//! 这个文件负责：
//! - 初始化应用本地的 SQLite 数据库连接池
//! - 执行基础迁移和桌面端专属迁移，保证 Schema 自动升级
//! - 读取和写回存储在 SQLite 中的应用配置

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;
use sqlx::SqlitePool;

use crate::backend::ThreadService;

/// `kv_config` 表中保存资源存储目录的键名。
const KV_CONFIG_RESOURCE_STORAGE_DIR: &str = "resource_storage_dir";

/// 应用级持久化配置。
///
/// 当前只保存资源存储目录，后续如果增加其他桌面端设置，也应优先放到这里统一管理。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    /// 用户配置的资源存储目录；为空时回退到应用数据目录。
    #[serde(default)]
    pub resource_storage_dir: Option<String>,
}

/// 桌面端后端共享的数据库与路径状态。
///
/// 该状态会通过 Tauri `manage` 注入到全局容器中，供各个 Command 和路由处理器读取。
pub struct DbState {
    /// SQLite 连接池，供后端所有数据读写使用。
    pub pool: SqlitePool,
}

impl DbState {
    /// 初始化本地 SQLite 数据库并执行迁移。
    ///
    /// 初始化流程包括：
    /// 1. 确定数据库文件路径
    /// 2. 创建 SQLite 连接池
    /// 3. 执行基础迁移与桌面端迁移
    /// 4. 终止上次进程遗留的非终态 Agent 任务
    pub async fn init(db_file: &Path) -> Result<Self, String> {
        if let Some(parent) = db_file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // 使用本地 SQLite 文件作为桌面端的默认持久化存储。
        let options = SqliteConnectOptions::new().filename(db_file).create_if_missing(true);

        // 连接池限制为 8 个连接，满足桌面端并发请求同时避免无节制占用资源。
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await
            .map_err(|e| e.to_string())?;

        // 基础迁移来自 datavisool-db，桌面端专属迁移来自 migrations_desktop。
        datavisool_db::run_base_migrations(&pool).await.map_err(|e| e.to_string())?;
        datavisool_db::run_custom_migrations(&pool, sqlx::migrate!("./migrations_desktop"))
            .await
            .map_err(|e| e.to_string())?;
        ThreadService::new(pool.clone())
            .interrupt_active_turns(current_time_ms()?)
            .await
            .map_err(|error| error.to_string())?;

        Ok(Self { pool })
    }

    /// 从数据库加载应用配置。
    pub async fn load_app_config(&self) -> Result<AppConfig, String> {
        load_app_config_from_db(&self.pool).await
    }

    /// 将应用配置写回数据库。
    pub async fn save_app_config(&self, cfg: &AppConfig) -> Result<(), String> {
        persist_app_config_to_db(&self.pool, cfg).await
    }
}

fn current_time_ms() -> Result<i64, String> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?;
    i64::try_from(elapsed.as_millis()).map_err(|_| "current timestamp exceeds SQLite INTEGER".to_string())
}

/// 从数据库加载应用配置。
///
/// 当前只读取资源存储目录配置；若 kv 表中没有对应记录，则返回默认空配置。
async fn load_app_config_from_db(pool: &SqlitePool) -> Result<AppConfig, String> {
    let row = sqlx::query("SELECT v FROM kv_config WHERE k = ?1")
        .bind(KV_CONFIG_RESOURCE_STORAGE_DIR)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

    let dir = row
        .and_then(|r| r.try_get::<String, _>("v").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Ok(AppConfig { resource_storage_dir: dir })
}

/// 将应用配置持久化到数据库。
///
/// 空资源目录会删除对应 kv 记录，使运行时回退到默认 AppData 目录。
async fn persist_app_config_to_db(pool: &SqlitePool, cfg: &AppConfig) -> Result<(), String> {
    let dir = cfg
        .resource_storage_dir
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(dir) = dir {
        sqlx::query(
            "INSERT INTO kv_config (k, v) VALUES (?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        )
        .bind(KV_CONFIG_RESOURCE_STORAGE_DIR)
        .bind(dir)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    } else {
        sqlx::query("DELETE FROM kv_config WHERE k = ?1")
            .bind(KV_CONFIG_RESOURCE_STORAGE_DIR)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[cfg(test)]
#[path = "db_restart_tests.rs"]
mod tests;
