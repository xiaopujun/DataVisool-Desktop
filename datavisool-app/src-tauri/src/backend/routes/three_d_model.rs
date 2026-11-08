/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 3D 模型资源接口。
//!
//! 负责模型资源的分页、详情、上传、更新和删除。
//! 支持直接上传模型文件，也支持上传 zip 包并解压出主模型入口。

use std::io::{Cursor, Read};
use std::path::Path;
use std::path::PathBuf;

use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use sqlx::Row;
use zip::ZipArchive;

use crate::app_state::AppState;
use crate::backend::types::{err, err_with_detail, ok, parse_i64_list_param, parse_required_i64_param, HttpResponse};
use crate::backend::utils::{sanitize_upload_file_name, to_asset_url};
use crate::db::DbState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 3D 模型分页查询参数。
struct PageParam {
    current: i64,
    size: i64,
    search_value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 3D 模型更新请求体。
struct ModelUpdateReq {
    id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 单个 3D 模型上传项。
struct ModelUploadItem {
    file_name: String,
    data_url: String,
    mime: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 批量 3D 模型上传请求体。
struct ModelUploadReq {
    files: Vec<ModelUploadItem>,
    id: Option<String>,
}

/// 分页查询 3D 模型资源。
pub(crate) async fn page(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let page: PageParam = serde_json::from_value(body).map_err(|e| format!("three_d_model.page: invalid body: {e}"))?;
    let current = if page.current <= 0 { 1 } else { page.current };
    let size = if page.size <= 0 { 15 } else { page.size };
    let offset = (current - 1) * size;

    let total_row = sqlx::query("SELECT COUNT(*) AS cnt FROM three_d_model WHERE deleted = 0 AND (?1 IS NULL OR name LIKE '%' || ?1 || '%')")
        .bind(page.search_value.clone())
        .fetch_one(&db.pool)
        .await
        .map_err(|e| format!("three_d_model.page: total query failed: {e}"))?;
    let total: i64 = total_row
        .try_get("cnt")
        .map_err(|e| format!("three_d_model.page: read total failed: {e}"))?;

    let rows = sqlx::query(
        "SELECT id, name, url, size, type, user_id, is_compressed, create_time, update_time \
         FROM three_d_model \
         WHERE deleted = 0 AND (?1 IS NULL OR name LIKE '%' || ?1 || '%') \
         ORDER BY update_time DESC \
         LIMIT ?2 OFFSET ?3",
    )
    .bind(page.search_value)
    .bind(size)
    .bind(offset)
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("three_d_model.page: list query failed: {e}"))?;

    let mut records: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("three_d_model.page: read id failed: {e}"))?;
        let name: String = row
            .try_get("name")
            .map_err(|e| format!("three_d_model.page: read name failed: {e}"))?;
        let url: String = row
            .try_get("url")
            .map_err(|e| format!("three_d_model.page: read url failed: {e}"))?;
        let size_v: Option<i64> = row
            .try_get("size")
            .map_err(|e| format!("three_d_model.page: read size failed: {e}"))?;
        let ty: Option<String> = row
            .try_get("type")
            .map_err(|e| format!("three_d_model.page: read type failed: {e}"))?;
        let user_id: Option<String> = row
            .try_get("user_id")
            .map_err(|e| format!("three_d_model.page: read user_id failed: {e}"))?;
        let is_compressed: i64 = row
            .try_get("is_compressed")
            .map_err(|e| format!("three_d_model.page: read is_compressed failed: {e}"))?;
        let create_time: String = row
            .try_get("create_time")
            .map_err(|e| format!("three_d_model.page: read create_time failed: {e}"))?;
        let update_time: String = row
            .try_get("update_time")
            .map_err(|e| format!("three_d_model.page: read update_time failed: {e}"))?;

        records.push(serde_json::json!({
            "id": id.to_string(),
            "name": name,
            "url": resolve_model_url(app_state, &url),
            "size": size_v,
            "type": ty,
            "userId": user_id,
            "isCompressed": is_compressed,
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

/// 批量逻辑删除 3D 模型资源。
pub(crate) async fn batch_delete(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let ids: Vec<String> = serde_json::from_value(body).map_err(|e| format!("three_d_model.batchDelete: invalid body: {e}"))?;
    if ids.is_empty() {
        return Ok(ok(Value::Bool(true)));
    }
    let parsed = match parse_i64_list_param(&ids, "id") {
        Ok(parsed) => parsed,
        Err(response) => return Ok(response),
    };

    let mut query = String::from("UPDATE three_d_model SET deleted = 1 WHERE deleted = 0 AND id IN (");
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
        Err(e) => Ok(err_with_detail(500, "删除 3D 模型失败，请稍后重试", &e.to_string())),
    }
}

/// 更新 3D 模型展示名。
pub(crate) async fn update(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let req: ModelUpdateReq = serde_json::from_value(body).map_err(|e| format!("three_d_model.update: invalid body: {e}"))?;
    let id = match parse_required_i64_param(req.id.as_deref(), "id") {
        Ok(id) => id,
        Err(response) => return Ok(response),
    };
    let name = req.name.unwrap_or_default().trim().to_string();
    if name.is_empty() {
        return Ok(err(400, "请求参数错误：名称不能为空"));
    }

    match sqlx::query("UPDATE three_d_model SET name = ?2 WHERE id = ?1 AND deleted = 0")
        .bind(id)
        .bind(name)
        .execute(&db.pool)
        .await
    {
        Ok(r) => Ok(ok(Value::Bool(r.rows_affected() > 0))),
        Err(e) => Ok(err_with_detail(500, "重命名失败，请稍后重试", &e.to_string())),
    }
}

/// 获取单个 3D 模型详情。
pub(crate) async fn get(db: &DbState, app_state: &AppState, id: i64) -> Result<HttpResponse, String> {
    let row = match sqlx::query(
        "SELECT id, name, url, size, type, user_id, is_compressed, create_time, update_time \
         FROM three_d_model WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .fetch_optional(&db.pool)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            return Ok(err_with_detail(500, "查询 3D 模型失败，请稍后重试", &e.to_string()));
        }
    };

    let Some(row) = row else {
        return Ok(err(404, "3D 模型不存在或已删除"));
    };

    let id: i64 = row
        .try_get("id")
        .map_err(|e| format!("three_d_model.get: read id failed: {e}"))?;
    let name: String = row
        .try_get("name")
        .map_err(|e| format!("three_d_model.get: read name failed: {e}"))?;
    let url: String = row
        .try_get("url")
        .map_err(|e| format!("three_d_model.get: read url failed: {e}"))?;
    let size_v: Option<i64> = row
        .try_get("size")
        .map_err(|e| format!("three_d_model.get: read size failed: {e}"))?;
    let ty: Option<String> = row
        .try_get("type")
        .map_err(|e| format!("three_d_model.get: read type failed: {e}"))?;
    let user_id: Option<String> = row
        .try_get("user_id")
        .map_err(|e| format!("three_d_model.get: read user_id failed: {e}"))?;
    let is_compressed: i64 = row
        .try_get("is_compressed")
        .map_err(|e| format!("three_d_model.get: read is_compressed failed: {e}"))?;
    let create_time: String = row
        .try_get("create_time")
        .map_err(|e| format!("three_d_model.get: read create_time failed: {e}"))?;
    let update_time: String = row
        .try_get("update_time")
        .map_err(|e| format!("three_d_model.get: read update_time failed: {e}"))?;

    Ok(ok(serde_json::json!({
        "id": id.to_string(),
        "name": name,
        "url": resolve_model_url(app_state, &url),
        "size": size_v,
        "type": ty,
        "userId": user_id,
        "isCompressed": is_compressed,
        "createTime": create_time,
        "updateTime": update_time
    })))
}

/// 批量上传 3D 模型文件。
///
/// zip 包会被解压到独立目录，并记录主模型文件的相对路径。
pub(crate) async fn upload(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: ModelUploadReq = serde_json::from_value(body).map_err(|e| format!("three_d_model.upload: invalid body: {e}"))?;
    if req.files.is_empty() {
        return Ok(err(400, "请求参数错误：请选择要上传的模型文件"));
    }

    let mut replacement_name = None;
    let replacement_id = if let Some(id) = req.id.as_deref().filter(|id| !id.trim().is_empty()) {
        if req.files.len() != 1 { return Ok(err(400, "替换模型时请选择一个文件")); }
        let id = match parse_required_i64_param(Some(id), "id") {
            Ok(id) => id,
            Err(response) => return Ok(response),
        };
        let exists = sqlx::query("SELECT name FROM three_d_model WHERE id = ?1 AND deleted = 0")
            .bind(id).fetch_optional(&db.pool).await
            .map_err(|e| format!("three_d_model.upload: query failed: {e}"))?;
        let Some(row) = exists else { return Ok(err(404, "模型不存在")); };
        replacement_name = Some(row.try_get::<String, _>("name").map_err(|e| format!("three_d_model.upload: read name failed: {e}"))?);
        Some(id)
    } else { None };

    let models_dir = app_state.static_storage_dir().join("models");
    std::fs::create_dir_all(&models_dir).map_err(|e| format!("three_d_model.upload: create dir failed: {e}"))?;

    let mut inserted: Vec<Value> = Vec::with_capacity(req.files.len());
    for item in req.files {
        let (mime, bytes) = match parse_data_url(&item.data_url, item.mime.as_deref()) {
            Ok(v) => v,
            Err(e) => {
                return Ok(err_with_detail(400, "请求参数错误：文件数据不合法", &e));
            }
        };
        let bytes_len = bytes.len() as i64;
        let file_name = sanitize_upload_file_name(&item.file_name, "model");
        let ext = file_extension(&file_name).unwrap_or_else(|| mime_to_ext(&mime).unwrap_or_else(|| "bin".to_string()));
        let is_zip = ext.eq_ignore_ascii_case("zip");
        let is_compressed = if is_zip { 1 } else { 0 };

        tracing::debug!("three_d_model.save: detected compressed model archive = {}", is_zip);
        let (stored_url, resolved_mime, resolved_size) = if is_zip {
            // 压缩包先扫描出最适合作为入口的模型文件，再用入口文件 hash 作为目录键。
            let (hash, main_rel_path) = match scan_zip_for_main_hash(&bytes) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(err_with_detail(400, "压缩包中未找到支持的3D模型主文件（.gltf/.glb/.obj/.fbx）", &e));
                }
            };

            let dir_key = hash;
            let target_dir = models_dir.join(&dir_key);
            if !target_dir.exists() {
                if let Err(e) = std::fs::create_dir_all(&target_dir) {
                    return Ok(err_with_detail(500, "创建模型解压目录失败，请检查磁盘权限", &e.to_string()));
                }

                if let Err(e) = extract_zip(&bytes, &target_dir) {
                    return Ok(err_with_detail(500, "解压模型失败", &e));
                }
            }

            let main_path = target_dir.join(&main_rel_path);
            let meta = std::fs::metadata(&main_path).ok();
            let size = meta.map(|m| m.len() as i64).unwrap_or(bytes_len);
            let ext = file_extension(&main_rel_path).unwrap_or_else(|| "bin".to_string());
            let mime = infer_mime_by_ext(&ext).unwrap_or_else(|| "application/octet-stream".to_string());

            (format!("{dir_key}/{main_rel_path}"), mime, size)
        } else {
            let hash = crate::backend::utils::sha256_hex(&bytes);
            let stored_file = format!("{}.{}", hash, ext);
            let file_path = models_dir.join(&stored_file);

            if !file_path.exists() {
                if let Err(e) = std::fs::write(&file_path, bytes) {
                    return Ok(err_with_detail(500, "写入本地模型文件失败，请检查磁盘空间或权限", &e.to_string()));
                }
            }
            (stored_file, mime.clone(), bytes_len)
        };

        let inserted_id = if let Some(id) = replacement_id {
            let result = sqlx::query("UPDATE three_d_model SET url = ?2, size = ?3, type = ?4, is_compressed = ?5 WHERE id = ?1 AND deleted = 0")
                .bind(id).bind(&stored_url).bind(resolved_size).bind(&resolved_mime).bind(is_compressed)
                .execute(&db.pool).await
                .map_err(|e| format!("three_d_model.upload: update failed: {e}"))?;
            if result.rows_affected() == 0 { return Ok(err(404, "模型不存在")); }
            id
        } else {
            sqlx::query("INSERT INTO three_d_model (name, url, size, type, is_compressed, deleted) VALUES (?1, ?2, ?3, ?4, ?5, 0)")
                .bind(&file_name).bind(&stored_url).bind(resolved_size).bind(&resolved_mime).bind(is_compressed)
                .execute(&db.pool).await
                .map_err(|e| format!("three_d_model.upload: insert failed: {e}"))?.last_insert_rowid()
        };

        inserted.push(serde_json::json!({
            "id": inserted_id.to_string(),
            "name": replacement_name.as_deref().unwrap_or(&file_name),
            "url": resolve_model_url(app_state, &stored_url),
            "size": resolved_size,
            "type": resolved_mime,
            "isCompressed": is_compressed
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

/// 将常见模型 MIME 转换为扩展名。
fn mime_to_ext(mime: &str) -> Option<String> {
    let m = mime.to_lowercase();
    if m == "model/gltf-binary" {
        return Some("glb".to_string());
    }
    if m == "model/gltf+json" {
        return Some("gltf".to_string());
    }
    if m == "application/zip" {
        return Some("zip".to_string());
    }
    None
}

/// 将数据库中的模型路径解析成前端可访问的 URL。
fn resolve_model_url(app_state: &AppState, raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return "".to_string();
    }
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("data:") || s.starts_with("blob:") {
        return s.to_string();
    }
    if s.contains(':') || s.starts_with('/') || s.starts_with('\\') {
        return to_asset_url(&PathBuf::from(s));
    }
    let p1 = app_state.static_storage_dir().join("models").join(s);
    if p1.exists() {
        return to_asset_url(&p1);
    }
    to_asset_url(&app_state.files_dir.join("static").join("models").join(s))
}

/// 扫描 zip 包，选出主模型文件并计算其内容 hash。
fn scan_zip_for_main_hash(zip_bytes: &[u8]) -> Result<(String, String), String> {
    let cursor = Cursor::new(zip_bytes);
    let mut archive = ZipArchive::new(cursor).map_err(|e| format!("zip open failed: {e}"))?;

    let mut best_main: Option<(i32, usize)> = None;
    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(|e| format!("zip read entry failed: {e}"))?;
        if file.is_dir() {
            continue;
        }
        let name = file.name().to_string();
        if safe_zip_rel_path(&name).is_none() {
            continue;
        }
        let entry_lower = name.to_lowercase();
        let prio = model_main_priority(&entry_lower);
        if prio > 0 {
            match best_main {
                Some((best_prio, _)) if best_prio >= prio => {}
                _ => best_main = Some((prio, i)),
            }
        }
    }

    let idx = match best_main {
        Some((_, idx)) => idx,
        None => return Err("zip has no supported model file".to_string()),
    };

    let mut file = archive.by_index(idx).unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let hash = crate::backend::utils::sha256_hex(&buf);
    let rel = safe_zip_rel_path(file.name()).unwrap();

    Ok((hash, rel))
}

/// 安全解压模型 zip 包。
fn extract_zip(zip_bytes: &[u8], target_dir: &Path) -> Result<(), String> {
    let cursor = Cursor::new(zip_bytes);
    let mut archive = ZipArchive::new(cursor).map_err(|e| format!("zip open failed: {e}"))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| format!("zip read entry failed: {e}"))?;
        if file.is_dir() {
            continue;
        }
        let Some(rel) = safe_zip_rel_path(file.name()) else {
            continue;
        };

        let out_path = target_dir.join(&rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("zip create dir failed: {e}"))?;
        }
        {
            let mut out_file = std::fs::File::create(&out_path).map_err(|e| format!("zip create file failed: {e}"))?;
            std::io::copy(&mut file, &mut out_file).map_err(|e| format!("zip write file failed: {e}"))?;
        }
    }
    Ok(())
}

/// 过滤 zip 条目路径，阻止绝对路径和目录穿越。
fn safe_zip_rel_path(name: &str) -> Option<String> {
    let normalized = name.replace('\\', "/");
    let p = Path::new(&normalized);
    if p.is_absolute() {
        return None;
    }
    let mut out = Vec::<String>::new();
    for c in p.components() {
        use std::path::Component;
        match c {
            Component::Normal(s) => out.push(s.to_string_lossy().to_string()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if out.is_empty() {
        return None;
    }
    Some(out.join("/"))
}

/// 计算模型入口文件优先级。
fn model_main_priority(name: &str) -> i32 {
    if name.ends_with(".gltf") {
        return 4;
    }
    if name.ends_with(".glb") {
        return 3;
    }
    if name.ends_with(".obj") {
        return 2;
    }
    if name.ends_with(".fbx") {
        return 1;
    }
    0
}

/// 根据扩展名推断模型 MIME。
fn infer_mime_by_ext(ext: &str) -> Option<String> {
    let e = ext.to_lowercase();
    if e == "glb" {
        return Some("model/gltf-binary".to_string());
    }
    if e == "gltf" {
        return Some("model/gltf+json".to_string());
    }
    if e == "fbx" {
        return Some("application/octet-stream".to_string());
    }
    if e == "obj" {
        return Some("text/plain".to_string());
    }
    None
}
