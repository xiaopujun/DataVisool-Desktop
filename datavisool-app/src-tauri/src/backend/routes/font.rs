/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 字体资源接口。
//!
//! 负责字体资源的列表、分页、上传、更新和删除。
//! 本地字体文件落在静态资源目录下，并通过 asset URL 暴露给前端使用。

use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use sqlx::Row;
use std::path::Path;

use crate::app_state::AppState;
use crate::backend::types::{err, ok, parse_i64_list_param, parse_i64_param, HttpResponse};
use crate::backend::utils::to_asset_url;
use crate::db::DbState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 字体分页查询参数。
struct PageParam {
    current: i64,
    size: i64,
    search_value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 字体更新请求体。
struct FontUpdateReq {
    id: Option<String>,
    name: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 单个字体上传项。
struct FontUploadItem {
    id: Option<String>,
    file_name: String,
    data_url: String,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 批量字体上传请求体。
struct FontUploadReq {
    fonts: Vec<FontUploadItem>,
}

/// 查询所有可用字体。
pub(crate) async fn list(db: &DbState, app_state: &AppState) -> Result<HttpResponse, String> {
    let rows = sqlx::query("SELECT id, name, url FROM font WHERE deleted = 0 ORDER BY update_time DESC")
        .fetch_all(&db.pool)
        .await
        .map_err(|e| format!("font.list: query failed: {e}"))?;

    let mut items: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.try_get("id").map_err(|e| format!("font.list: read id failed: {e}"))?;
        let name: Option<String> = row.try_get("name").map_err(|e| format!("font.list: read name failed: {e}"))?;
        let url: Option<String> = row.try_get("url").map_err(|e| format!("font.list: read url failed: {e}"))?;
        items.push(serde_json::json!({
            "id": id.to_string(),
            "name": name.unwrap_or_default(),
            "url": resolve_font_url(app_state, url.unwrap_or_default().as_str())
        }));
    }

    Ok(ok(Value::Array(items)))
}

/// 分页查询字体资源。
pub(crate) async fn page_list(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let page: PageParam = serde_json::from_value(body).map_err(|e| format!("font.pageList: invalid body: {e}"))?;
    let current = if page.current <= 0 { 1 } else { page.current };
    let size = if page.size <= 0 { 15 } else { page.size };
    let offset = (current - 1) * size;

    let total_row = sqlx::query("SELECT COUNT(*) AS cnt FROM font WHERE deleted = 0 AND (?1 IS NULL OR name LIKE '%' || ?1 || '%')")
        .bind(page.search_value.clone())
        .fetch_one(&db.pool)
        .await
        .map_err(|e| format!("font.pageList: total query failed: {e}"))?;
    let total: i64 = total_row
        .try_get("cnt")
        .map_err(|e| format!("font.pageList: read total failed: {e}"))?;

    let rows = sqlx::query(
        "SELECT id, name, url, create_time, update_time \
         FROM font \
         WHERE deleted = 0 AND (?1 IS NULL OR name LIKE '%' || ?1 || '%') \
         ORDER BY update_time DESC \
         LIMIT ?2 OFFSET ?3",
    )
    .bind(page.search_value)
    .bind(size)
    .bind(offset)
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("font.pageList: list query failed: {e}"))?;

    let mut records: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.try_get("id").map_err(|e| format!("font.pageList: read id failed: {e}"))?;
        let name: Option<String> = row
            .try_get("name")
            .map_err(|e| format!("font.pageList: read name failed: {e}"))?;
        let url: Option<String> = row.try_get("url").map_err(|e| format!("font.pageList: read url failed: {e}"))?;
        let create_time: String = row
            .try_get("create_time")
            .map_err(|e| format!("font.pageList: read create_time failed: {e}"))?;
        let update_time: String = row
            .try_get("update_time")
            .map_err(|e| format!("font.pageList: read update_time failed: {e}"))?;
        records.push(serde_json::json!({
            "id": id.to_string(),
            "name": name.unwrap_or_default(),
            "url": resolve_font_url(app_state, url.unwrap_or_default().as_str()),
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

/// 更新字体展示名或 URL。
pub(crate) async fn update(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let req: FontUpdateReq = serde_json::from_value(body).map_err(|e| format!("font.update: invalid body: {e}"))?;
    let Some(id_str) = req.id else {
        return Ok(err(400, "请求参数错误：缺少 id"));
    };
    let id: i64 = id_str.parse().map_err(|e| format!("font.update: invalid id `{id_str}`: {e}"))?;

    if req.name.as_ref().is_some_and(|name| name.trim().is_empty()) {
        return Ok(err(400, "字体名称不能为空"));
    }

    let changed = sqlx::query(
        "UPDATE font SET \
           name = COALESCE(?2, name), \
           url = COALESCE(?3, url) \
         WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .bind(req.name)
    .bind(req.url)
    .execute(&db.pool)
    .await
    .map_err(|e| format!("font.update: update failed: {e}"))?
    .rows_affected();

    Ok(ok(Value::Bool(changed > 0)))
}

/// 批量逻辑删除字体记录。
pub(crate) async fn batch_del(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let ids: Vec<String> = serde_json::from_value(body).map_err(|e| format!("font.batchDel: invalid body: {e}"))?;
    if ids.is_empty() {
        return Ok(ok(Value::Bool(true)));
    }
    let parsed = match parse_i64_list_param(&ids, "id") {
        Ok(parsed) => parsed,
        Err(response) => return Ok(response),
    };

    let mut query = String::from("UPDATE font SET deleted = 1 WHERE deleted = 0 AND id IN (");
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

    let changed = q
        .execute(&db.pool)
        .await
        .map_err(|e| format!("font.batchDel: delete failed: {e}"))?
        .rows_affected();
    Ok(ok(Value::Bool(changed > 0)))
}

/// 批量上传字体文件。
pub(crate) async fn upload(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: FontUploadReq = serde_json::from_value(body).map_err(|e| format!("font.upload: invalid body: {e}"))?;
    if req.fonts.is_empty() {
        return Ok(err(400, "请求参数错误：缺少字体数据"));
    }

    let mut inserted_ids: Vec<Value> = Vec::with_capacity(req.fonts.len());
    for item in req.fonts {
        let Some((_mime, bytes)) = parse_data_url(&item.data_url) else {
            return Ok(err(400, "请求参数错误：字体数据不合法"));
        };
        // 上传前先按扩展名收窄类型，避免任意文件进入字体目录。
        let ext = match validate_font_ext(&item.file_name) {
            Ok(ext) => ext,
            Err(msg) => return Ok(err(400, msg)),
        };
        let base_name = item
            .name
            .unwrap_or_else(|| derive_base_name(&item.file_name))
            .trim()
            .to_string();
        if base_name.is_empty() {
            return Ok(err(400, "请求参数错误：字体名不能为空"));
        }

        let saved_file_name = save_font_file(app_state, &ext, bytes).map_err(|e| format!("font.upload: save file failed: {e}"))?;

        if let Some(id_str) = item.id.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let id = match parse_i64_param(id_str, "id") {
                Ok(id) => id,
                Err(response) => return Ok(response),
            };
            let old = sqlx::query("SELECT url FROM font WHERE id = ?1 AND deleted = 0")
                .bind(id)
                .fetch_optional(&db.pool)
                .await
                .map_err(|e| format!("font.upload: query old failed: {e}"))?;
            let Some(old) = old else {
                return Ok(err(404, "字体不存在，无法替换"));
            };
            let old_url: Option<String> = old.try_get("url").ok();

            let changed = sqlx::query("UPDATE font SET url = ?2 WHERE id = ?1 AND deleted = 0")
                .bind(id)
                .bind(&saved_file_name)
                .execute(&db.pool)
                .await
                .map_err(|e| format!("font.upload: update failed: {e}"))?
                .rows_affected();
            if changed == 0 {
                return Ok(err(404, "字体不存在，无法替换"));
            }

            if let Some(old_url) = old_url {
                if old_url != saved_file_name {
                    delete_font_file_if_exists(app_state, &old_url);
                }
            }

            inserted_ids.push(Value::String(id.to_string()));
            continue;
        }

        let result = sqlx::query("INSERT INTO font (name, url, deleted) VALUES (?1, ?2, 0)")
            .bind(base_name)
            .bind(&saved_file_name)
            .execute(&db.pool)
            .await
            .map_err(|e| format!("font.upload: insert failed: {e}"))?;
        inserted_ids.push(Value::String(result.last_insert_rowid().to_string()));
    }

    Ok(ok(Value::Array(inserted_ids)))
}

/// 将数据库中的字体路径解析成前端可访问的 URL。
fn resolve_font_url(app_state: &AppState, raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return "".to_string();
    }
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("data:") || s.starts_with("blob:") {
        return s.to_string();
    }

    to_asset_url(&font_file_path(app_state, s))
}

/// 校验字体扩展名并返回标准化后缀。
fn validate_font_ext(file_name: &str) -> Result<String, String> {
    let ext = Path::new(file_name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "ttf" | "otf" | "woff" | "woff2" | "eot" => Ok(ext),
        _ => Err("请求参数错误：仅支持 .ttf/.otf/.woff/.woff2/.eot 字体文件".to_string()),
    }
}

/// 解析前端上传的 data URL。
fn parse_data_url(data_url: &str) -> Option<(String, Vec<u8>)> {
    let s = data_url.trim();
    if !s.starts_with("data:") {
        return None;
    }
    let (meta, b64) = s.split_once(',')?;
    if !meta.contains(";base64") {
        return None;
    }
    let mime = meta.strip_prefix("data:")?.split(';').next()?.to_string();
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    Some((mime, bytes))
}

/// 从文件名推导字体展示名。
fn derive_base_name(file_name: &str) -> String {
    let base = file_name
        .rsplit('\\')
        .next()
        .unwrap_or(file_name)
        .rsplit('/')
        .next()
        .unwrap_or(file_name);
    match base.rfind('.') {
        Some(i) => base[..i].to_string(),
        None => base.to_string(),
    }
}

/// 保存字体文件并返回静态资源相对路径。
fn save_font_file(app_state: &AppState, ext: &str, bytes: Vec<u8>) -> Result<String, String> {
    let dir = static_font_dir(app_state);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let hash = crate::backend::utils::sha256_hex(&bytes);
    let file_name = format!("{}.{}", hash, ext);
    let full = dir.join(&file_name);

    if !full.exists() {
        std::fs::write(&full, &bytes).map_err(|e| e.to_string())?;
    }
    Ok(file_name)
}

/// 删除字体关联的本地文件。
fn delete_font_file_if_exists(app_state: &AppState, url: &str) {
    let u = url.trim();
    if u.is_empty() || u.starts_with("data:") || u.starts_with("http://") || u.starts_with("https://") {
        return;
    }
    let p = font_file_path(app_state, u);
    let _ = std::fs::remove_file(p);
}

/// 返回字体静态资源目录。
fn static_font_dir(app_state: &AppState) -> std::path::PathBuf {
    app_state.static_storage_dir().join("font")
}

/// 返回字体文件完整路径。
fn font_file_path(app_state: &AppState, file_name: &str) -> std::path::PathBuf {
    let p1 = app_state.static_storage_dir().join("font").join(file_name);
    if p1.exists() {
        return p1;
    }
    app_state.files_dir.join("static").join("font").join(file_name)
}
