/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 图片资源接口。
//!
//! 负责用户上传图片的列表、上传和删除。
//! 图片文件按内容哈希落盘，数据库只保存可解析的静态资源路径。

use std::path::{Path, PathBuf};

use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use sqlx::Row;

use crate::app_state::AppState;
use crate::backend::types::{err, err_with_detail, ok, HttpResponse};
use crate::backend::utils::{sanitize_error_message, to_asset_url};
use crate::db::DbState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 图片上传请求体。
struct ImageUploadReq {
    file_name: String,
    data_url: String,
    #[allow(dead_code)]
    hash: Option<String>,
    name: Option<String>,
}

/// 查询用户上传图片列表。
pub(crate) async fn get_user_uploaded_image_list(db: &DbState, app_state: &AppState) -> Result<HttpResponse, String> {
    let rows = sqlx::query("SELECT id, name, url, hash FROM image WHERE deleted = 0 ORDER BY update_time DESC")
        .fetch_all(&db.pool)
        .await
        .map_err(|e| format!("image.getUserUploadedImageList: query failed: {e}"))?;

    let mut out: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("image.getUserUploadedImageList: read id failed: {e}"))?;
        let name: Option<String> = row
            .try_get("name")
            .map_err(|e| format!("image.getUserUploadedImageList: read name failed: {e}"))?;
        let url: String = row
            .try_get("url")
            .map_err(|e| format!("image.getUserUploadedImageList: read url failed: {e}"))?;
        let hash: Option<String> = row
            .try_get("hash")
            .map_err(|e| format!("image.getUserUploadedImageList: read hash failed: {e}"))?;
        out.push(serde_json::json!({
            "id": id.to_string(),
            "name": name.unwrap_or_default(),
            "url": resolve_image_url(app_state, &url),
            "hash": hash
        }));
    }
    Ok(ok(Value::Array(out)))
}

/// 逻辑删除图片记录，并清理对应的本地图片文件。
pub(crate) async fn del(db: &DbState, app_state: &AppState, id: i64) -> Result<HttpResponse, String> {
    let row = sqlx::query("SELECT url FROM image WHERE id = ?1 AND deleted = 0")
        .bind(id)
        .fetch_optional(&db.pool)
        .await
        .map_err(|e| format!("image.del: query failed: {e}"))?;

    let Some(row) = row else {
        return Ok(ok(Value::Bool(true)));
    };
    let url: String = row.try_get("url").unwrap_or_default();

    let changed = sqlx::query("UPDATE image SET deleted = 1 WHERE id = ?1 AND deleted = 0")
        .bind(id)
        .execute(&db.pool)
        .await
        .map_err(|e| format!("image.del: delete failed: {e}"))?
        .rows_affected();

    if changed > 0 {
        delete_image_file_if_exists(app_state, &url);
    }
    Ok(ok(Value::Bool(changed > 0)))
}

/// 上传图片文件。
pub(crate) async fn upload(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: ImageUploadReq = serde_json::from_value(body).map_err(|e| format!("image.upload: invalid body: {e}"))?;
    let (_mime, bytes) = match parse_data_url(&req.data_url) {
        Ok(v) => v,
        Err(e) => return Ok(err_with_detail(400, "请求参数错误：文件数据不合法", &e)),
    };

    // 上传前先按扩展名收窄类型，避免任意文件进入图片静态目录。
    let ext = match validate_image_ext(&req.file_name) {
        Ok(v) => v,
        Err(msg) => return Ok(err(400, msg)),
    };

    let hash = crate::backend::utils::sha256_hex(&bytes);

    let dir = app_state.static_storage_dir().join("images");
    std::fs::create_dir_all(&dir).map_err(|e| format!("image.upload: create dir failed: {e}"))?;

    let file_name = format!("{}.{}", hash, ext);
    let file_path = dir.join(&file_name);

    if !file_path.exists() {
        std::fs::write(&file_path, bytes).map_err(|e| format!("image.upload: write file failed: {e}"))?;
    }

    let display_name = req.name.unwrap_or_else(|| req.file_name.clone());

    let result = sqlx::query("INSERT INTO image (url, name, hash, deleted) VALUES (?1, ?2, ?3, 0)")
        .bind(&file_name)
        .bind(display_name)
        .bind(Some(&hash))
        .execute(&db.pool)
        .await;
    match result {
        Ok(_) => Ok(ok(Value::String(resolve_image_url(app_state, &file_name)))),
        Err(e) => Ok(err_with_detail(500, "保存图片信息失败，请稍后重试", &sanitize_error_message(&e.to_string()))),
    }
}

/// 校验图片扩展名并返回标准化后缀。
fn validate_image_ext(file_name: &str) -> Result<String, String> {
    let ext = Path::new(file_name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "bmp" | "jpg" | "jpeg" | "png" | "gif" | "svg" | "webp" => Ok(ext),
        _ => Err("请求参数错误：仅支持 bmp/jpg/jpeg/png/gif/svg/webp 图片".to_string()),
    }
}

/// 将数据库中的图片路径解析成前端可访问的 URL。
fn resolve_image_url(app_state: &AppState, raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return "".to_string();
    }
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("data:") || s.starts_with("blob:") {
        return s.to_string();
    }
    if s.contains('/') || s.contains('\\') || Path::new(s).is_absolute() {
        let p = PathBuf::from(s);
        if p.is_absolute() {
            return to_asset_url(&p);
        }
        let p1 = app_state.resource_storage_root().join(s);
        if p1.exists() {
            return to_asset_url(&p1);
        }
        return to_asset_url(&app_state.files_dir.join(s));
    }
    let p1 = app_state.static_storage_dir().join("images").join(s);
    if p1.exists() {
        return to_asset_url(&p1);
    }
    to_asset_url(&app_state.files_dir.join("static").join("images").join(s))
}

/// 解析前端上传的 data URL。
fn parse_data_url(data_url: &str) -> Result<(String, Vec<u8>), String> {
    let s = data_url.trim();
    if !s.starts_with("data:") {
        return Err("not data url".to_string());
    }
    let Some((meta, b64)) = s.split_once(',') else {
        return Err("data url missing comma".to_string());
    };
    if !meta.contains(";base64") {
        return Err("data url missing base64".to_string());
    }
    let mime = meta
        .strip_prefix("data:")
        .and_then(|m| m.split(';').next())
        .filter(|m| !m.is_empty())
        .map(|m| m.to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("base64 decode failed: {e}"))?;
    Ok((mime, bytes))
}

/// 删除图片关联的本地文件。
fn delete_image_file_if_exists(app_state: &AppState, url: &str) {
    let s = url.trim();
    if s.is_empty() {
        return;
    }
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("data:") {
        return;
    }
    if s.contains(':') || s.starts_with('/') || s.starts_with('\\') {
        let _ = std::fs::remove_file(PathBuf::from(s));
        return;
    }

    if s.contains('/') || s.contains('\\') {
        let p1 = app_state.resource_storage_root().join(s);
        if p1.exists() {
            let _ = std::fs::remove_file(p1);
            return;
        }
        let _ = std::fs::remove_file(app_state.files_dir.join(s));
        return;
    }

    let p1 = app_state.static_storage_dir().join("images").join(s);
    if p1.exists() {
        let _ = std::fs::remove_file(p1);
        return;
    }
    let _ = std::fs::remove_file(app_state.files_dir.join("static").join("images").join(s));
}
