/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 应用配置接口。
//!
//! 目前主要管理资源存储目录。配置会同时写入 SQLite 和运行时 `AppState`，
//! 让后续文件上传、资源 URL 解析立即使用新的目录。

use serde::Deserialize;
use serde_json::json;

use std::io::Write;

use crate::app_state::AppState;
use crate::backend::types::{err, err_with_detail, ok, HttpResponse};
use crate::db::{AppConfig, DbState};

/// 读取当前运行时生效的应用配置。
pub(crate) async fn get(app_state: &AppState) -> Result<HttpResponse, String> {
    let cfg = app_state.app_config()?;
    Ok(ok(json!({ "resourceStorageDir": cfg.resource_storage_dir })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 保存应用配置的请求体。
struct SaveReq {
    resource_storage_dir: Option<String>,
}

/// 保存应用配置。
///
/// 资源目录必须是绝对路径，并且在写入配置前会做可写性探测。
pub(crate) async fn save(db: &DbState, app_state: &AppState, body: serde_json::Value) -> Result<HttpResponse, String> {
    let req: SaveReq = serde_json::from_value(body).map_err(|e| e.to_string())?;
    let normalized = req.resource_storage_dir.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

    if let Some(dir) = normalized.as_ref() {
        // 资源目录会被静态资源读写路径复用，因此这里必须先收紧路径合法性。
        let path = std::path::PathBuf::from(dir);
        if !path.is_absolute() {
            return Ok(err(400, "资源存储位置必须是绝对路径"));
        }

        if is_windows_device_path(&path) {
            return Ok(err(400, "资源存储位置不支持设备路径"));
        }

        if let Err(resp) = ensure_dir_writable(&path) {
            return Ok(resp);
        }
    }

    let new_cfg = AppConfig { resource_storage_dir: normalized };

    // 先持久化，再更新运行时状态，保证重启后和当前进程看到的配置一致。
    db.save_app_config(&new_cfg).await?;
    app_state.apply_app_config(&new_cfg)?;

    Ok(ok(json!({})))
}

/// 确认目标目录存在且当前进程可写。
fn ensure_dir_writable(dir: &std::path::Path) -> Result<(), HttpResponse> {
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::error!("app_config.save: create_dir_all failed: dir={dir:?}, err={e}");
        return Err(err_with_detail(500, "创建资源目录失败", &e.to_string()));
    }

    match std::fs::metadata(dir) {
        Ok(m) => {
            if !m.is_dir() {
                return Err(err(400, "资源存储位置必须是目录"));
            }
        }
        Err(e) => {
            tracing::error!("app_config.save: metadata failed: dir={dir:?}, err={e}");
            return Err(err_with_detail(500, "读取目录信息失败", &e.to_string()));
        }
    }

    // 通过真实写入探针文件验证权限，比只看 metadata 更可靠。
    let probe_path = dir.join(format!(".datavisool-write-probe-{:016x}", rand::random::<u64>()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_path)
    {
        Ok(mut f) => {
            if let Err(e) = f.write_all(b"ok") {
                drop(f);
                let _ = std::fs::remove_file(&probe_path);
                tracing::error!("app_config.save: probe write failed: path={probe_path:?}, err={e}");
                let code = if e.kind() == std::io::ErrorKind::PermissionDenied { 403 } else { 500 };
                return Err(err_with_detail(code, "资源存储位置不可写", &e.to_string()));
            }
            if let Err(e) = f.sync_all() {
                drop(f);
                let _ = std::fs::remove_file(&probe_path);
                tracing::error!("app_config.save: probe sync failed: path={probe_path:?}, err={e}");
                let code = if e.kind() == std::io::ErrorKind::PermissionDenied { 403 } else { 500 };
                return Err(err_with_detail(code, "资源存储位置不可写", &e.to_string()));
            }
        }
        Err(e) => {
            tracing::error!("app_config.save: probe open failed: path={probe_path:?}, err={e}");
            let code = if e.kind() == std::io::ErrorKind::PermissionDenied { 403 } else { 500 };
            return Err(err_with_detail(code, "资源存储位置不可写", &e.to_string()));
        }
    }

    if let Err(e) = std::fs::remove_file(&probe_path) {
        tracing::warn!("app_config.save: remove probe failed: path={probe_path:?}, err={e}");
    }

    Ok(())
}

/// Windows 设备路径不能作为资源根目录。
fn is_windows_device_path(path: &std::path::Path) -> bool {
    if !cfg!(windows) {
        return false;
    }
    let s = path.to_string_lossy();
    s.starts_with("\\\\.\\")
}

#[cfg(test)]
mod tests {
    use super::ensure_dir_writable;

    #[test]
    fn writable_probe_preserves_existing_files_and_removes_its_own_file() {
        let root = std::env::temp_dir().join(format!("datavisool-write-probe-test-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        let sentinel = root.join(".datavisool_write_probe");
        let old_sentinel = root.join(".lcp_write_probe");
        std::fs::write(&sentinel, b"keep").unwrap();
        std::fs::write(&old_sentinel, b"old").unwrap();
        assert!(ensure_dir_writable(&root).is_ok());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep");
        assert_eq!(std::fs::read(&old_sentinel).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }
}
