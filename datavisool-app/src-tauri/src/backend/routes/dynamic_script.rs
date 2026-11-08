/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 动态脚本资源接口。
//!
//! 负责项目级和全局动态脚本的分页、更新、保存、删除与选择列表。
//! 脚本可以是外链，也可以是上传到本地静态目录的文件。

use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use sqlx::Row;

use crate::app_state::AppState;
use crate::backend::types::{err, err_with_detail, ok, parse_i64_list_param, parse_i64_param, parse_required_i64_param, HttpResponse};
use crate::backend::utils::to_asset_url;
use crate::db::DbState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 动态脚本分页查询参数。
struct PageParam {
    current: i64,
    size: i64,
    search_value: Option<String>,
    project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 动态脚本基础更新请求体。
struct DynamicScriptUpdateReq {
    id: Option<String>,
    name: Option<String>,
    url: Option<String>,
    r#type: Option<String>,
    project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 动态脚本保存/更新请求体。
struct SaveOrUpdateReq {
    id: Option<String>,
    name: Option<String>,
    r#type: Option<String>,
    url: Option<String>,
    project_id: Option<String>,
    script_file_name: Option<String>,
    script_data_url: Option<String>,
}

/// 分页查询动态脚本。
///
/// 传入 projectId 时只查项目内脚本；未传入时查询全局脚本。
pub(crate) async fn page_list(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let page: PageParam = serde_json::from_value(body).map_err(|e| format!("dynamicScript.pageList: invalid body: {e}"))?;
    let current = if page.current <= 0 { 1 } else { page.current };
    let size = if page.size <= 0 { 15 } else { page.size };
    let offset = (current - 1) * size;

    // 项目内脚本和全局脚本共用一张表，通过 project_id 是否为空区分。
    let (total, rows) = if let Some(pid) = page.project_id.clone().filter(|s| !s.trim().is_empty()) {
        let total_row = sqlx::query(
            "SELECT COUNT(*) AS cnt \
             FROM dynamic_script \
             WHERE deleted = 0 \
               AND project_id = ?1 \
               AND (?2 IS NULL OR name LIKE '%' || ?2 || '%')",
        )
        .bind(pid.clone())
        .bind(page.search_value.clone())
        .fetch_one(&db.pool)
        .await
        .map_err(|e| format!("dynamicScript.pageList: total query failed: {e}"))?;
        let total: i64 = total_row
            .try_get("cnt")
            .map_err(|e| format!("dynamicScript.pageList: read total failed: {e}"))?;

        let rows = sqlx::query(
            "SELECT id, name, url, type, project_id, create_time, update_time \
             FROM dynamic_script \
             WHERE deleted = 0 \
               AND project_id = ?1 \
               AND (?2 IS NULL OR name LIKE '%' || ?2 || '%') \
             ORDER BY update_time DESC \
             LIMIT ?3 OFFSET ?4",
        )
        .bind(pid)
        .bind(page.search_value)
        .bind(size)
        .bind(offset)
        .fetch_all(&db.pool)
        .await
        .map_err(|e| format!("dynamicScript.pageList: list query failed: {e}"))?;
        (total, rows)
    } else {
        let total_row = sqlx::query(
            "SELECT COUNT(*) AS cnt \
             FROM dynamic_script \
             WHERE deleted = 0 \
               AND project_id IS NULL \
               AND (?1 IS NULL OR name LIKE '%' || ?1 || '%')",
        )
        .bind(page.search_value.clone())
        .fetch_one(&db.pool)
        .await
        .map_err(|e| format!("dynamicScript.pageList: total query failed: {e}"))?;
        let total: i64 = total_row
            .try_get("cnt")
            .map_err(|e| format!("dynamicScript.pageList: read total failed: {e}"))?;

        let rows = sqlx::query(
            "SELECT id, name, url, type, project_id, create_time, update_time \
             FROM dynamic_script \
             WHERE deleted = 0 \
               AND project_id IS NULL \
               AND (?1 IS NULL OR name LIKE '%' || ?1 || '%') \
             ORDER BY update_time DESC \
             LIMIT ?2 OFFSET ?3",
        )
        .bind(page.search_value)
        .bind(size)
        .bind(offset)
        .fetch_all(&db.pool)
        .await
        .map_err(|e| format!("dynamicScript.pageList: list query failed: {e}"))?;
        (total, rows)
    };

    let mut records: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("dynamicScript.pageList: read id failed: {e}"))?;
        let name: Option<String> = row
            .try_get("name")
            .map_err(|e| format!("dynamicScript.pageList: read name failed: {e}"))?;
        let url: String = row
            .try_get("url")
            .map_err(|e| format!("dynamicScript.pageList: read url failed: {e}"))?;
        let ty: String = row
            .try_get("type")
            .map_err(|e| format!("dynamicScript.pageList: read type failed: {e}"))?;
        let project_id: Option<String> = row
            .try_get("project_id")
            .map_err(|e| format!("dynamicScript.pageList: read project_id failed: {e}"))?;
        let create_time: String = row
            .try_get("create_time")
            .map_err(|e| format!("dynamicScript.pageList: read create_time failed: {e}"))?;
        let update_time: String = row
            .try_get("update_time")
            .map_err(|e| format!("dynamicScript.pageList: read update_time failed: {e}"))?;

        records.push(serde_json::json!({
            "id": id.to_string(),
            "name": name.unwrap_or_default(),
            "url": resolve_script_url(app_state, &ty, &url),
            "type": ty,
            "projectId": project_id,
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

/// 批量逻辑删除动态脚本记录。
pub(crate) async fn batch_del(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let ids: Vec<String> = serde_json::from_value(body).map_err(|e| format!("dynamicScript.batchDel: invalid body: {e}"))?;
    if ids.is_empty() {
        return Ok(ok(Value::Bool(true)));
    }

    let parsed = match parse_i64_list_param(&ids, "id") {
        Ok(parsed) => parsed,
        Err(response) => return Ok(response),
    };

    let mut query = String::from("UPDATE dynamic_script SET deleted = 1 WHERE deleted = 0 AND id IN (");
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
        Err(e) => Ok(err_with_detail(500, "删除动态脚本失败，请稍后重试", &e.to_string())),
    }
}

fn is_valid_remote_url(value: &str) -> bool {
    !value.trim().is_empty()
        && url::Url::parse("https://localhost/").ok()
            .and_then(|base| base.join(value.trim()).ok())
            .is_some_and(|parsed| matches!(parsed.scheme(), "http" | "https"))
}

/// 更新动态脚本基础信息。
pub(crate) async fn update(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let req: DynamicScriptUpdateReq = serde_json::from_value(body).map_err(|e| format!("dynamicScript.update: invalid body: {e}"))?;
    let id = match parse_required_i64_param(req.id.as_deref(), "id") {
        Ok(id) => id,
        Err(response) => return Ok(response),
    };

    if req.name.as_ref().is_some_and(|name| name.trim().is_empty()) {
        return Ok(err(400, "脚本名称不能为空"));
    }

    if let Some(url) = req.url.as_deref() {
        let script_type = if let Some(script_type) = req.r#type.as_deref() {
            script_type.to_string()
        } else {
            let row = sqlx::query("SELECT type FROM dynamic_script WHERE id = ?1 AND deleted = 0")
                .bind(id).fetch_optional(&db.pool).await
                .map_err(|e| format!("dynamicScript.update: query failed: {e}"))?;
            match row {
                Some(row) => row.try_get::<String, _>("type").unwrap_or_default(),
                None => return Ok(err(404, "脚本不存在")),
            }
        };
        if script_type.eq_ignore_ascii_case("REMOTE") && !is_valid_remote_url(url) {
            return Ok(err(400, "请输入有效的脚本地址"));
        }
    }

    let changed = sqlx::query(
        "UPDATE dynamic_script SET \
           name = COALESCE(?2, name), \
           url = COALESCE(?3, url), \
           type = COALESCE(?4, type), \
           project_id = COALESCE(?5, project_id) \
         WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .bind(req.name)
    .bind(req.url)
    .bind(req.r#type)
    .bind(req.project_id)
    .execute(&db.pool)
    .await;

    match changed {
        Ok(r) => Ok(ok(Value::Bool(r.rows_affected() > 0))),
        Err(e) => Ok(err_with_detail(500, "更新动态脚本失败，请稍后重试", &e.to_string())),
    }
}

/// 保存或更新动态脚本。
///
/// 上传脚本文件时会先保存文件，再把静态资源 URL 写入数据库。
pub(crate) async fn save_or_update(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: SaveOrUpdateReq = serde_json::from_value(body).map_err(|e| format!("dynamicScript.saveOrUpdate: invalid body: {e}"))?;

    let ty = req.r#type.unwrap_or_else(|| "REMOTE".to_string());
    let ty = ty.trim().to_string();
    if ty != "REMOTE" && ty != "LOCAL" {
        return Ok(err(400, "请求参数错误：type 必须为 REMOTE 或 LOCAL"));
    }

    let mut name = req.name.unwrap_or_default().trim().to_string();
    let project_id = req.project_id.clone();

    let url = if ty == "REMOTE" {
        let url = req.url.unwrap_or_default().trim().to_string();
        if url.is_empty() {
            return Ok(err(400, "请求参数错误：远程脚本地址不能为空"));
        }
        if !is_valid_remote_url(&url) {
            return Ok(err(400, "请输入有效的脚本地址"));
        }
        if name.is_empty() {
            name = derive_name_from_url(&url);
        }
        url
    } else {
        // 本地脚本通过 data URL 上传，只接受 .js 文件。
        let data_url = req.script_data_url.unwrap_or_default().trim().to_string();
        if data_url.is_empty() {
            return Ok(err(400, "请求参数错误：请上传脚本文件"));
        }
        let file_name = req.script_file_name.unwrap_or_else(|| "script.js".to_string());
        if !file_name.to_ascii_lowercase().ends_with(".js") {
            return Ok(err(400, "请求参数错误：仅支持上传 .js 文件"));
        }
        let (_mime, bytes) = match parse_data_url(&data_url) {
            Ok(v) => v,
            Err(e) => return Ok(err_with_detail(400, "请求参数错误：文件数据不合法", &e)),
        };
        let stored_file = match save_script_file(app_state, bytes) {
            Ok(v) => v,
            Err(e) => return Ok(err_with_detail(500, "写入本地脚本文件失败，请检查磁盘空间或权限", &e)),
        };
        if name.is_empty() {
            name = file_name;
        }
        stored_file
    };

    if name.is_empty() {
        return Ok(err(400, "请求参数错误：脚本名称不能为空"));
    }

    if let Some(id_str) = req.id {
        let id = match parse_i64_param(&id_str, "id") {
            Ok(id) => id,
            Err(response) => return Ok(response),
        };
        let old = sqlx::query("SELECT url, type FROM dynamic_script WHERE id = ?1 AND deleted = 0")
            .bind(id)
            .fetch_optional(&db.pool)
            .await
            .map_err(|e| format!("dynamicScript.saveOrUpdate: query old failed: {e}"))?;
        let Some(old) = old else {
            return Ok(err(404, "动态脚本不存在，无法替换"));
        };
        let old_url: String = old.try_get("url").unwrap_or_default();
        let old_ty: String = old.try_get("type").unwrap_or_default();

        let changed = sqlx::query(
            "UPDATE dynamic_script SET name = ?2, url = ?3, type = ?4, project_id = ?5 \
             WHERE id = ?1 AND deleted = 0",
        )
        .bind(id)
        .bind(name)
        .bind(url)
        .bind(&ty)
        .bind(project_id)
        .execute(&db.pool)
        .await;

        // 替换本地脚本后清理旧文件，避免静态目录持续堆积无引用资源。
        if old_ty.to_ascii_uppercase() == "LOCAL" && ty == "LOCAL" {
            delete_script_file_if_exists(app_state, &old_url);
        }

        return match changed {
            Ok(r) => Ok(ok(Value::Bool(r.rows_affected() > 0))),
            Err(e) => Ok(err_with_detail(500, "保存动态脚本失败，请稍后重试", &e.to_string())),
        };
    }

    let inserted = sqlx::query("INSERT INTO dynamic_script (name, url, type, project_id, deleted) VALUES (?1, ?2, ?3, ?4, 0)")
        .bind(name)
        .bind(url)
        .bind(ty)
        .bind(project_id)
        .execute(&db.pool)
        .await;

    match inserted {
        Ok(r) => Ok(ok(Value::String(r.last_insert_rowid().to_string()))),
        Err(e) => Ok(err_with_detail(500, "新增动态脚本失败，请稍后重试", &e.to_string())),
    }
}

/// 查询可供项目引用的动态脚本列表。
pub(crate) async fn list(db: &DbState, app_state: &AppState, full_url: &str) -> Result<HttpResponse, String> {
    let project_id = get_query_value(full_url, "projectId").map(|s| s.trim().to_string());
    let project_id = project_id.filter(|s| !s.is_empty());

    let rows = if let Some(pid) = project_id.clone() {
        sqlx::query(
            "SELECT id, name, url, type, project_id \
             FROM dynamic_script \
             WHERE deleted = 0 AND (project_id IS NULL OR project_id = ?1) \
             ORDER BY update_time DESC",
        )
        .bind(pid)
        .fetch_all(&db.pool)
        .await
    } else {
        sqlx::query(
            "SELECT id, name, url, type, project_id \
             FROM dynamic_script \
             WHERE deleted = 0 AND project_id IS NULL \
             ORDER BY update_time DESC",
        )
        .fetch_all(&db.pool)
        .await
    };

    match rows {
        Ok(rows) => {
            let mut items: Vec<Value> = Vec::with_capacity(rows.len());
            for row in rows {
                let id: i64 = row
                    .try_get("id")
                    .map_err(|e| format!("dynamicScript.list: read id failed: {e}"))?;
                let name: Option<String> = row
                    .try_get("name")
                    .map_err(|e| format!("dynamicScript.list: read name failed: {e}"))?;
                let url: String = row
                    .try_get("url")
                    .map_err(|e| format!("dynamicScript.list: read url failed: {e}"))?;
                let ty: String = row
                    .try_get("type")
                    .map_err(|e| format!("dynamicScript.list: read type failed: {e}"))?;
                let project_id: Option<String> = row
                    .try_get("project_id")
                    .map_err(|e| format!("dynamicScript.list: read project_id failed: {e}"))?;
                items.push(serde_json::json!({
                    "id": id.to_string(),
                    "name": name.unwrap_or_default(),
                    "url": resolve_script_url(app_state, &ty, &url),
                    "type": ty,
                    "projectId": project_id
                }));
            }
            Ok(ok(Value::Array(items)))
        }
        Err(e) => Ok(err_with_detail(500, "加载动态脚本失败，请稍后重试", &e.to_string())),
    }
}

/// 从 URL 文件名推导脚本展示名。
fn derive_name_from_url(url: &str) -> String {
    let u = url.trim_end_matches('/');
    let seg = u.rsplit('/').next().unwrap_or("");
    derive_name_from_file_name(seg)
}

/// 从上传文件名推导脚本展示名。
fn derive_name_from_file_name(file_name: &str) -> String {
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

/// 从完整 URL 的 query string 中取指定参数。
fn get_query_value(full_url: &str, key: &str) -> Option<String> {
    let qs = full_url.splitn(2, '?').nth(1)?;
    for pair in qs.split('&') {
        if pair.is_empty() {
            continue;
        }
        let mut it = pair.splitn(2, '=');
        let k = it.next().unwrap_or("");
        let v = it.next().unwrap_or("");
        if k == key {
            return Some(urlencoding::decode(v).ok()?.to_string());
        }
    }
    None
}

/// 将数据库里的脚本路径解析成前端可访问的 asset URL。
fn resolve_script_url(app_state: &AppState, ty: &str, raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return "".to_string();
    }
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("data:") || s.starts_with("blob:") {
        return s.to_string();
    }
    if ty.to_ascii_uppercase() != "LOCAL" {
        return s.to_string();
    }
    let p1 = app_state.static_storage_dir().join("lib").join(s);
    if p1.exists() {
        return to_asset_url(&p1);
    }
    to_asset_url(&app_state.files_dir.join("static").join("lib").join(s))
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

/// 保存上传脚本文件并返回相对静态资源路径。
fn save_script_file(app_state: &AppState, bytes: Vec<u8>) -> Result<String, String> {
    let dir = app_state.static_storage_dir().join("lib");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let hash = crate::backend::utils::sha256_hex(&bytes);
    let file_name = format!("{}.js", hash);
    let path = dir.join(&file_name);

    if !path.exists() {
        std::fs::write(path, &bytes).map_err(|e| e.to_string())?;
    }
    Ok(file_name)
}

/// 删除动态脚本关联的本地文件。
fn delete_script_file_if_exists(app_state: &AppState, url: &str) {
    let u = url.trim();
    if u.is_empty() || u.starts_with("data:") || u.starts_with("http://") || u.starts_with("https://") {
        return;
    }
    let p1 = app_state.static_storage_dir().join("lib").join(u);
    if p1.exists() {
        let _ = std::fs::remove_file(p1);
        return;
    }
    let _ = std::fs::remove_file(app_state.files_dir.join("static").join("lib").join(u));
}
