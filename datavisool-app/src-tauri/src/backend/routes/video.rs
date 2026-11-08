/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 视频资源接口。
//!
//! 负责视频资源的分页、上传、更新和删除。
//! 本地视频文件按内容哈希落盘，列表返回前端可直接访问的 asset URL。

use std::path::Path;
use std::path::PathBuf;

use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use sqlx::Row;

use crate::app_state::AppState;
use crate::backend::types::{err, err_with_detail, ok, parse_i64_list_param, parse_required_i64_param, HttpResponse};
use crate::backend::utils::{sanitize_upload_file_name, to_asset_url};
use crate::db::DbState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 视频分页查询参数。
struct PageParam {
    current: i64,
    size: i64,
    search_value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 视频更新请求体。
struct VideoUpdateReq {
    id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 单个视频上传项。
struct VideoUploadItem {
    file_name: String,
    data_url: String,
    mime: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 批量视频上传请求体。
struct VideoUploadReq {
    files: Vec<VideoUploadItem>,
}

/// 分页查询视频资源。
pub(crate) async fn page(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let page: PageParam = serde_json::from_value(body).map_err(|e| format!("video.page: invalid body: {e}"))?;
    let current = if page.current <= 0 { 1 } else { page.current };
    let size = if page.size <= 0 { 15 } else { page.size };
    let offset = (current - 1) * size;

    let total_row = sqlx::query("SELECT COUNT(*) AS cnt FROM video WHERE deleted = 0 AND (?1 IS NULL OR name LIKE '%' || ?1 || '%')")
        .bind(page.search_value.clone())
        .fetch_one(&db.pool)
        .await
        .map_err(|e| format!("video.page: total query failed: {e}"))?;
    let total: i64 = total_row
        .try_get("cnt")
        .map_err(|e| format!("video.page: read total failed: {e}"))?;

    let rows = sqlx::query(
        "SELECT id, name, url, size, type, user_id, create_time, update_time \
         FROM video \
         WHERE deleted = 0 AND (?1 IS NULL OR name LIKE '%' || ?1 || '%') \
         ORDER BY update_time DESC \
         LIMIT ?2 OFFSET ?3",
    )
    .bind(page.search_value)
    .bind(size)
    .bind(offset)
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("video.page: list query failed: {e}"))?;

    let mut records: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.try_get("id").map_err(|e| format!("video.page: read id failed: {e}"))?;
        let name: String = row.try_get("name").map_err(|e| format!("video.page: read name failed: {e}"))?;
        let url: String = row.try_get("url").map_err(|e| format!("video.page: read url failed: {e}"))?;
        let size_v: Option<i64> = row.try_get("size").map_err(|e| format!("video.page: read size failed: {e}"))?;
        let ty: Option<String> = row.try_get("type").map_err(|e| format!("video.page: read type failed: {e}"))?;
        let user_id: Option<String> = row
            .try_get("user_id")
            .map_err(|e| format!("video.page: read user_id failed: {e}"))?;
        let create_time: String = row
            .try_get("create_time")
            .map_err(|e| format!("video.page: read create_time failed: {e}"))?;
        let update_time: String = row
            .try_get("update_time")
            .map_err(|e| format!("video.page: read update_time failed: {e}"))?;

        records.push(serde_json::json!({
            "id": id.to_string(),
            "name": name,
            "url": resolve_video_url(app_state, &url),
            "size": size_v,
            "type": ty,
            "userId": user_id,
            "createTime": create_time,
            "updateTime": update_time
        }));
    }

    Ok(ok(serde_json::json!({
        "records": records,
        "total": total,
        "size": size,
        "current": current
    })))
}

/// 批量逻辑删除视频资源记录。
pub(crate) async fn batch_delete(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let ids: Vec<String> = serde_json::from_value(body).map_err(|e| format!("video.batchDelete: invalid body: {e}"))?;
    if ids.is_empty() {
        return Ok(ok(Value::Bool(true)));
    }
    let parsed = match parse_i64_list_param(&ids, "id") {
        Ok(parsed) => parsed,
        Err(response) => return Ok(response),
    };

    let mut query = String::from("UPDATE video SET deleted = 1 WHERE deleted = 0 AND id IN (");
    for i in 0..parsed.len() {
        if i > 0 {
            query.push(',');
        }
        query.push('?');
        query.push_str(&(i + 1).to_string());
    }
    query.push(')');

    let mut q = sqlx::query(&query);
    for id in parsed {
        q = q.bind(id);
    }

    match q.execute(&db.pool).await {
        Ok(r) => Ok(ok(Value::Bool(r.rows_affected() > 0))),
        Err(e) => Ok(err_with_detail(500, "删除视频失败，请稍后重试", &e.to_string())),
    }
}

/// 删除单个视频资源。
pub(crate) async fn delete(db: &DbState, id: i64) -> Result<HttpResponse, String> {
    match sqlx::query("UPDATE video SET deleted = 1 WHERE id = ?1 AND deleted = 0")
        .bind(id)
        .execute(&db.pool)
        .await
    {
        Ok(r) => Ok(ok(Value::Bool(r.rows_affected() > 0))),
        Err(e) => Ok(err_with_detail(500, "删除视频失败，请稍后重试", &e.to_string())),
    }
}

/// 更新视频展示名。
pub(crate) async fn update(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let req: VideoUpdateReq = serde_json::from_value(body).map_err(|e| format!("video.update: invalid body: {e}"))?;
    let id = match parse_required_i64_param(req.id.as_deref(), "id") {
        Ok(id) => id,
        Err(response) => return Ok(response),
    };
    let name = req.name.unwrap_or_default().trim().to_string();
    if name.is_empty() {
        return Ok(err(400, "请求参数错误：名称不能为空"));
    }

    match sqlx::query("UPDATE video SET name = ?2 WHERE id = ?1 AND deleted = 0")
        .bind(id)
        .bind(name)
        .execute(&db.pool)
        .await
    {
        Ok(r) => Ok(ok(Value::Bool(r.rows_affected() > 0))),
        Err(e) => Ok(err_with_detail(500, "重命名失败，请稍后重试", &e.to_string())),
    }
}

/// 批量上传视频文件。
pub(crate) async fn upload(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: VideoUploadReq = serde_json::from_value(body).map_err(|e| format!("video.upload: invalid body: {e}"))?;
    if req.files.is_empty() {
        return Ok(err(400, "请求参数错误：请选择要上传的视频文件"));
    }

    for item in &req.files {
        let Some((meta, _)) = item.data_url.trim().split_once(',') else {
            return Ok(err(400, "请求参数错误：文件数据不合法"));
        };
        let mime = meta.strip_prefix("data:").and_then(|value| value.split(';').next())
            .filter(|value| !value.is_empty()).or(item.mime.as_deref()).unwrap_or("");
        if !mime.to_ascii_lowercase().starts_with("video/") {
            return Ok(err(400, "请上传视频文件"));
        }
    }

    let videos_dir = app_state.static_storage_dir().join("videos");
    std::fs::create_dir_all(&videos_dir).map_err(|e| format!("video.upload: create dir failed: {e}"))?;

    let mut inserted: Vec<Value> = Vec::with_capacity(req.files.len());
    for item in req.files {
        // data URL 中的 MIME 可缺省，缺省时使用前端单独传入的 MIME 兜底。
        let (mime, bytes) = match parse_data_url(&item.data_url, item.mime.as_deref()) {
            Ok(v) => v,
            Err(e) => {
                return Ok(err_with_detail(400, "请求参数错误：文件数据不合法", &e));
            }
        };
        let bytes_len = bytes.len() as i64;
        let file_name = sanitize_upload_file_name(&item.file_name, "video");
        let ext = file_extension(&file_name).unwrap_or_else(|| mime_to_ext(&mime).unwrap_or_else(|| "bin".to_string()));
        let display_name = file_name.clone();

        // 先插入记录拿到 id；文件写入失败时会把这条记录标记为删除。
        let url_placeholder = "";
        let result = sqlx::query("INSERT INTO video (name, url, size, type, deleted) VALUES (?1, ?2, ?3, ?4, 0)")
            .bind(display_name.clone())
            .bind(url_placeholder)
            .bind(bytes_len)
            .bind(mime.clone())
            .execute(&db.pool)
            .await;

        let inserted_id = match result {
            Ok(r) => r.last_insert_rowid(),
            Err(e) => {
                return Ok(err_with_detail(500, "保存视频信息失败，请稍后重试", &e.to_string()));
            }
        };

        let hash = crate::backend::utils::sha256_hex(&bytes);
        let stored_file = format!("{}.{}", hash, ext);
        let file_path = videos_dir.join(&stored_file);

        if !file_path.exists() {
            if let Err(e) = std::fs::write(&file_path, bytes) {
                let _ = sqlx::query("UPDATE video SET deleted = 1 WHERE id = ?1")
                    .bind(inserted_id)
                    .execute(&db.pool)
                    .await;
                return Ok(err_with_detail(500, "写入本地视频文件失败，请检查磁盘空间或权限", &e.to_string()));
            }
        }

        if let Err(e) = sqlx::query("UPDATE video SET url = ?2 WHERE id = ?1")
            .bind(inserted_id)
            .bind(&stored_file)
            .execute(&db.pool)
            .await
        {
            return Ok(err_with_detail(500, "保存视频路径失败，请稍后重试", &e.to_string()));
        }

        inserted.push(serde_json::json!({
            "id": inserted_id.to_string(),
            "name": display_name,
            "url": resolve_video_url(app_state, &stored_file),
            "size": bytes_len,
            "type": mime
        }));
    }

    Ok(ok(Value::Array(inserted)))
}

/// 解析前端上传的 data URL。
fn parse_data_url(data_url: &str, fallback_mime: Option<&str>) -> Result<(String, Vec<u8>), String> {
    let s = data_url.trim();
    if !s.starts_with("data:") {
        return Err("not data url".to_string());
    }
    let Some((meta, b64)) = s.split_once(",") else {
        return Err("data url missing comma".to_string());
    };
    let mime = meta
        .strip_prefix("data:")
        .and_then(|m| m.split(';').next())
        .filter(|m| !m.is_empty())
        .map(|m| m.to_string())
        .or_else(|| fallback_mime.map(|m| m.to_string()))
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("base64 decode failed: {e}"))?;

    Ok((mime, decoded))
}

/// 从文件名取扩展名。
fn file_extension(file_name: &str) -> Option<String> {
    Path::new(file_name).extension().and_then(|s| s.to_str()).map(|s| s.to_string())
}

/// 将数据库中的视频路径解析成前端可访问的 URL。
fn resolve_video_url(app_state: &AppState, raw: &str) -> String {
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
    let p1 = app_state.static_storage_dir().join("videos").join(s);
    if p1.exists() {
        return to_asset_url(&p1);
    }
    to_asset_url(&app_state.files_dir.join("static").join("videos").join(s))
}

/// 将常见视频 MIME 转换为扩展名。
fn mime_to_ext(mime: &str) -> Option<String> {
    let m = mime.to_lowercase();
    if m == "video/mp4" {
        return Some("mp4".to_string());
    }
    if m == "video/webm" {
        return Some("webm".to_string());
    }
    if m == "video/ogg" {
        return Some("ogv".to_string());
    }
    if m == "video/quicktime" {
        return Some("mov".to_string());
    }
    None
}
