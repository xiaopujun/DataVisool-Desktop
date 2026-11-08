/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Tauri 桌面端后端的应用运行时状态模块。
//!
//! 这个文件负责：
//! - 维护应用运行时使用的基础目录（AppData、Home、数据库文件路径）
//! - 维护资源存储目录的当前生效值
//! - 提供资源、密钥等文件的统一路径入口

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};

use crate::db::AppConfig;

/// AI 任务输入、执行过程和结果产物的统一目录名。
pub(crate) const AI_DRIVEN_STORAGE_DIR_NAME: &str = "ai-driven";
/// AI 模型 API Key 加密密钥文件在 AppData 下的相对路径。
const AI_MODEL_API_KEY_SECRET_FILE: &str = "security/ai_model_api_key.key";
/// 新应用使用独立的数据库文件，不读取旧应用的数据库。
const APP_DATABASE_FILE: &str = "datavisool.sqlite";
/// 桌面端后端共享的应用运行时状态。
///
/// 该状态不持有数据库连接，只负责运行期的路径信息与其派生结果。
pub struct AppState {
    /// 应用数据目录，是默认的文件落盘根目录。
    pub files_dir: PathBuf,
    /// 用户主目录，保留给需要读取用户目录的功能使用。
    #[allow(dead_code)]
    pub home_dir: PathBuf,
    db_file: PathBuf,
    resource_storage_dir: RwLock<Option<PathBuf>>,
}

impl AppState {
    /// 初始化应用运行时状态。
    pub fn init(app: &AppHandle) -> Result<Self, String> {
        // AppData 是默认资源和数据库根目录，所有必需目录在启动时先确保存在。
        let files_dir = app
            .path()
            .resolve(".", BaseDirectory::AppData)
            .map_err(|e: tauri::Error| e.to_string())?;
        std::fs::create_dir_all(&files_dir).map_err(|e| e.to_string())?;

        let db_dir = app
            .path()
            .resolve("db", BaseDirectory::AppData)
            .map_err(|e: tauri::Error| e.to_string())?;
        std::fs::create_dir_all(&db_dir).map_err(|e| e.to_string())?;
        let db_file = db_dir.join(APP_DATABASE_FILE);

        let home_dir = app
            .path()
            .resolve(".", BaseDirectory::Home)
            .map_err(|e: tauri::Error| e.to_string())?;

        Ok(Self { files_dir, home_dir, db_file, resource_storage_dir: RwLock::new(None) })
    }

    /// 返回数据库文件路径。
    pub fn db_file(&self) -> &Path {
        &self.db_file
    }

    /// 将数据库中读取到的持久化配置应用到运行时状态。
    ///
    /// 这里不会重新校验旧值，只做去空白和落地，避免升级时破坏已有用户配置。
    pub fn apply_app_config(&self, cfg: &AppConfig) -> Result<(), String> {
        let dir = cfg
            .resource_storage_dir
            .as_ref()
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .map(PathBuf::from);

        self.set_resource_storage_dir(dir)
    }

    /// 返回当前运行时配置对应的 `AppConfig` 视图。
    pub fn app_config(&self) -> Result<AppConfig, String> {
        let guard = self
            .resource_storage_dir
            .read()
            .map_err(|_| "read app state lock failed".to_string())?;
        Ok(AppConfig {
            resource_storage_dir: guard
                .as_ref()
                .map(|path| path.to_string_lossy().trim().to_string())
                .filter(|item| !item.is_empty()),
        })
    }

    /// 设置资源存储目录的当前生效值。
    pub fn set_resource_storage_dir(&self, dir: Option<PathBuf>) -> Result<(), String> {
        let mut guard = self
            .resource_storage_dir
            .write()
            .map_err(|_| "write app state lock failed".to_string())?;
        *guard = dir;
        Ok(())
    }

    /// 获取资源存储根目录。
    ///
    /// 优先使用用户配置的目录；未配置时回退到默认 `AppData` 目录。
    pub fn resource_storage_root(&self) -> PathBuf {
        self.resource_storage_dir
            .read()
            .ok()
            .and_then(|guard| guard.clone())
            .unwrap_or_else(|| self.files_dir.clone())
    }

    /// 获取 AI 任务产物的统一存储目录。
    pub(crate) fn ai_driven_storage_dir(&self) -> PathBuf {
        self.resource_storage_root().join(AI_DRIVEN_STORAGE_DIR_NAME)
    }

    /// 获取静态资源目录。
    pub fn static_storage_dir(&self) -> PathBuf {
        self.resource_storage_root().join("static")
    }

    /// 获取 AI 模型密钥文件路径。
    pub fn ai_model_api_key_secret_file(&self) -> PathBuf {
        self.files_dir.join(AI_MODEL_API_KEY_SECRET_FILE)
    }

    /// 构造测试专用状态，避免依赖真实 Tauri AppHandle。
    #[cfg(test)]
    pub(crate) fn new_for_test(files_dir: PathBuf, home_dir: PathBuf) -> Self {
        Self {
            db_file: files_dir.join("db").join(APP_DATABASE_FILE),
            files_dir,
            home_dir,
            resource_storage_dir: RwLock::new(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_uses_new_app_data_path() {
        let files_dir = std::env::temp_dir().join("datavisool-test-data");
        let app_state = AppState::new_for_test(files_dir.clone(), files_dir.clone());
        assert_eq!(app_state.db_file(), files_dir.join("db").join("datavisool.sqlite"));
    }

    #[test]
    fn ai_driven_storage_dir_follows_resource_storage_root() {
        let files_dir = std::env::temp_dir().join("datavisool-app-state-files");
        let configured_root = std::env::temp_dir().join("datavisool-app-state-resources");
        let app_state = AppState::new_for_test(files_dir.clone(), files_dir.clone());

        assert_eq!(app_state.ai_driven_storage_dir(), files_dir.join(AI_DRIVEN_STORAGE_DIR_NAME));

        app_state.set_resource_storage_dir(Some(configured_root.clone())).unwrap();
        assert_eq!(app_state.ai_driven_storage_dir(), configured_root.join(AI_DRIVEN_STORAGE_DIR_NAME));
    }
}
