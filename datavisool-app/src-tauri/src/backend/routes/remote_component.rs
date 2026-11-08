/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 服务器组件资源接口。
//!
//! 负责远程组件包的列表、分页、上传、更新和删除。
//! 支持单个 `.js/.mjs` 文件，也支持包含入口脚本的 zip 包。

use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use sqlx::Row;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

use crate::app_state::AppState;
use crate::backend::types::{err, err_with_detail, ok, parse_i64_list_param, parse_i64_param, parse_required_i64_param, HttpResponse};
use crate::backend::utils::{reserve_component_package, sanitize_upload_file_name, to_asset_url};
use crate::db::DbState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 组件分页查询参数。
struct PageParam {
    current: i64,
    size: i64,
    search_value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 组件名称更新请求体。
struct RemoteComponentUpdateReq {
    id: Option<String>,
    component_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 组件上传请求体。
struct RemoteComponentUploadReq {
    id: Option<String>,
    file_name: String,
    data_url: String,
    mime: Option<String>,
    component_name: Option<String>,
}

/// 查询所有可用组件脚本 URL。
pub(crate) async fn list(db: &DbState, app_state: &AppState) -> Result<HttpResponse, String> {
    let rows = sqlx::query(
        "SELECT id, url FROM remote_component \
         WHERE deleted = 0 AND TRIM(COALESCE(url, '')) <> '' \
         ORDER BY update_time DESC",
    )
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("remote_component.list: query failed: {e}"))?;

    let mut out: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("remote_component.list: read id failed: {e}"))?;
        let url: String = row
            .try_get("url")
            .map_err(|e| format!("remote_component.list: read url failed: {e}"))?;
        let resolved = resolve_script_url(app_state, &url);
        out.push(serde_json::json!({ "id": id.to_string(), "url": resolved }));
    }

    Ok(ok(Value::Array(out)))
}

/// 分页查询组件资源。
pub(crate) async fn page_list(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let page: PageParam = serde_json::from_value(body).map_err(|e| format!("remote_component.pageList: invalid body: {e}"))?;
    let current = if page.current <= 0 { 1 } else { page.current };
    let size = if page.size <= 0 { 10 } else { page.size };
    let offset = (current - 1) * size;

    let total_row = sqlx::query(
        "SELECT COUNT(*) AS cnt FROM remote_component \
         WHERE deleted = 0 AND TRIM(COALESCE(url, '')) <> '' \
         AND (?1 IS NULL OR component_name LIKE '%' || ?1 || '%')",
    )
    .bind(page.search_value.clone())
    .fetch_one(&db.pool)
    .await
    .map_err(|e| format!("remote_component.pageList: total query failed: {e}"))?;
    let total: i64 = total_row
        .try_get("cnt")
        .map_err(|e| format!("remote_component.pageList: read total failed: {e}"))?;

    let rows = sqlx::query(
        "SELECT id, component_name, url, user_id, create_time, update_time \
         FROM remote_component \
         WHERE deleted = 0 AND TRIM(COALESCE(url, '')) <> '' \
         AND (?1 IS NULL OR component_name LIKE '%' || ?1 || '%') \
         ORDER BY update_time DESC \
         LIMIT ?2 OFFSET ?3",
    )
    .bind(page.search_value)
    .bind(size)
    .bind(offset)
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("remote_component.pageList: list query failed: {e}"))?;

    let mut records: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("remote_component.pageList: read id failed: {e}"))?;
        let component_name: Option<String> = row
            .try_get("component_name")
            .map_err(|e| format!("remote_component.pageList: read component_name failed: {e}"))?;
        let url: String = row
            .try_get("url")
            .map_err(|e| format!("remote_component.pageList: read url failed: {e}"))?;
        let user_id: Option<String> = row
            .try_get("user_id")
            .map_err(|e| format!("remote_component.pageList: read user_id failed: {e}"))?;
        let create_time: String = row
            .try_get("create_time")
            .map_err(|e| format!("remote_component.pageList: read create_time failed: {e}"))?;
        let update_time: String = row
            .try_get("update_time")
            .map_err(|e| format!("remote_component.pageList: read update_time failed: {e}"))?;

        records.push(serde_json::json!({
            "id": id.to_string(),
            "componentName": component_name.unwrap_or_default(),
            "url": resolve_script_url(app_state, &url),
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

/// 批量逻辑删除组件记录。
pub(crate) async fn batch_del(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let ids: Vec<String> = serde_json::from_value(body).map_err(|e| format!("remote_component.batchDel: invalid body: {e}"))?;
    if ids.is_empty() {
        return Ok(ok(Value::Bool(true)));
    }

    let parsed = match parse_i64_list_param(&ids, "id") {
        Ok(parsed) => parsed,
        Err(response) => return Ok(response),
    };

    let mut query = String::from("UPDATE remote_component SET deleted = 1 WHERE deleted = 0 AND id IN (");
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
        Err(e) => Ok(err_with_detail(500, "删除服务器组件失败，请稍后重试", &e.to_string())),
    }
}

/// 更新组件展示名。
pub(crate) async fn update(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let req: RemoteComponentUpdateReq = serde_json::from_value(body).map_err(|e| format!("remote_component.update: invalid body: {e}"))?;
    let id = match parse_required_i64_param(req.id.as_deref(), "id") {
        Ok(id) => id,
        Err(response) => return Ok(response),
    };
    let name = req.component_name.unwrap_or_default().trim().to_string();
    if name.is_empty() {
        return Ok(err(400, "请求参数错误：组件名称不能为空"));
    }

    match sqlx::query("UPDATE remote_component SET component_name = ?2 WHERE id = ?1 AND deleted = 0")
        .bind(id)
        .bind(name)
        .execute(&db.pool)
        .await
    {
        Ok(r) => Ok(ok(Value::Bool(r.rows_affected() > 0))),
        Err(e) => Ok(err_with_detail(500, "更新服务器组件失败，请稍后重试", &e.to_string())),
    }
}

/// 上传或替换组件资源。
///
/// zip 包会先解压到临时目录，确认成功后再原子切换到正式目录。
pub(crate) async fn upload(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: RemoteComponentUploadReq = serde_json::from_value(body).map_err(|e| format!("remote_component.upload: invalid body: {e}"))?;

    let (_mime, bytes) = match parse_data_url(&req.data_url, req.mime.as_deref()) {
        Ok(v) => v,
        Err(e) => return Ok(err_with_detail(400, "请求参数错误：文件数据不合法", &e)),
    };

    let file_name = sanitize_upload_file_name(&req.file_name, "component.js");
    let ext = file_extension(&file_name).unwrap_or_else(|| "js".to_string());
    if !ext.eq_ignore_ascii_case("js") && !ext.eq_ignore_ascii_case("mjs") && !ext.eq_ignore_ascii_case("zip") {
        return Ok(err(400, "请求参数错误：仅支持上传 .js、.mjs 或 .zip 文件"));
    }
    let component_name = req
        .component_name
        .unwrap_or_else(|| derive_base_name(&file_name))
        .trim()
        .to_string();

    let existing_component: Option<(i64, String)> = if let Some(id_str) = &req.id {
        let id = match parse_i64_param(id_str, "id") {
            Ok(id) => id,
            Err(response) => return Ok(response),
        };
        let row = sqlx::query("SELECT url FROM remote_component WHERE id = ?1 AND deleted = 0")
            .bind(id)
            .fetch_optional(&db.pool)
            .await
            .map_err(|e| format!("remote_component.upload: query old url failed: {e}"))?;
        let Some(row) = row else {
            return Ok(err(404, "服务器组件不存在或已删除"));
        };
        let old_url = row.try_get::<String, _>("url").unwrap_or_default();
        Some((id, old_url))
    } else {
        None
    };

    let dir = app_state.static_storage_dir().join("components");
    std::fs::create_dir_all(&dir).map_err(|e| format!("remote_component.upload: create dir failed: {e}"))?;

    let hash = crate::backend::utils::sha256_hex(&bytes);
    let (stored_file, stored_new_file) = if ext.eq_ignore_ascii_case("zip") {
        // 压缩包必须包含 .js/.mjs 入口，入口路径会写入数据库供前端加载。
        let entry = match scan_zip_for_entry_script(&bytes) {
            Ok(v) => v,
            Err(e) => return Ok(err_with_detail(400, "请求参数错误：组件压缩包不合法", &e)),
        };
        let mut package = match reserve_component_package(&dir, &hash, ".upload") {
            Ok(package) => package,
            Err(e) => {
                return Ok(err_with_detail(500, "分配服务器组件目录失败，请检查磁盘空间或权限", &e));
            }
        };
        if let Err(e) = extract_zip(&bytes, &package.temp_dir) {
            package.rollback();
            return Ok(err_with_detail(500, "解压服务器组件失败，请检查压缩包内容", &e));
        }
        // commit 成功前目录仍可回滚，避免半解压组件被前端引用。
        if let Err(e) = package.commit() {
            package.rollback();
            return Ok(err_with_detail(500, "替换服务器组件失败，请检查磁盘空间或权限", &e));
        }
        package.finalize();
        (format!("static/components/{}/{}", package.package_id, entry), true)
    } else {
        let stored_file = format!("{}.{}", hash, ext);
        let file_path = dir.join(&stored_file);

        let mut stored_new_file = false;
        if !file_path.exists() {
            if let Err(e) = std::fs::write(&file_path, bytes) {
                return Ok(err_with_detail(500, "写入本地组件文件失败，请检查磁盘空间或权限", &e.to_string()));
            }
            stored_new_file = true;
        }
        (stored_file, stored_new_file)
    };

    if let Some((id, old_url)) = existing_component {
        let old = old_url.trim().to_string();
        let should_remove_old = if !old.is_empty() && old != stored_file {
            match stored_component_has_other_reference(db, id, &old).await {
                Ok(has_other_reference) => !has_other_reference,
                Err(e) => {
                    if stored_new_file {
                        remove_stored_component(app_state, &stored_file);
                    }
                    return Ok(err_with_detail(500, "清理旧组件文件失败，请稍后重试", &e));
                }
            }
        } else {
            false
        };

        if let Err(e) = sqlx::query("UPDATE remote_component SET url = ?2, component_name = COALESCE(NULLIF(?3, ''), component_name) WHERE id = ?1")
            .bind(id)
            .bind(&stored_file)
            .bind(&component_name)
            .execute(&db.pool)
            .await
        {
            if stored_new_file {
                remove_stored_component(app_state, &stored_file);
            }
            return Ok(err_with_detail(500, "保存组件路径失败，请稍后重试", &e.to_string()));
        }

        if should_remove_old {
            remove_stored_component(app_state, &old);
        }
    } else {
        if let Err(e) = sqlx::query("INSERT INTO remote_component (component_name, url, deleted) VALUES (?1, ?2, 0)")
            .bind(component_name)
            .bind(&stored_file)
            .execute(&db.pool)
            .await
        {
            if stored_new_file {
                remove_stored_component(app_state, &stored_file);
            }
            return Ok(err_with_detail(500, "保存服务器组件信息失败，请稍后重试", &e.to_string()));
        }
    }

    Ok(ok(Value::Bool(true)))
}

/// 将数据库中的组件路径解析成前端可访问的 URL。
fn resolve_script_url(app_state: &AppState, raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return s.to_string();
    }
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("data:") || s.starts_with("blob:") {
        return s.to_string();
    }

    let abs = if s.contains(':') || s.starts_with('/') || s.starts_with('\\') {
        PathBuf::from(s)
    } else if s.contains('/') || s.contains('\\') {
        let p1 = app_state.resource_storage_root().join(s);
        if p1.exists() {
            p1
        } else {
            app_state.files_dir.join(s)
        }
    } else {
        let p1 = app_state.static_storage_dir().join("components").join(s);
        if p1.exists() {
            p1
        } else {
            app_state.files_dir.join("static").join("components").join(s)
        }
    };
    to_asset_url(&abs)
}

/// 判断某个组件文件/包是否仍被其他记录引用。
async fn stored_component_has_other_reference(db: &DbState, current_id: i64, raw: &str) -> Result<bool, String> {
    if let Some(prefix) = component_package_url_prefix(raw) {
        return sqlx::query(
            "SELECT id FROM remote_component \
             WHERE deleted = 0 AND id != ?1 AND REPLACE(TRIM(url), '\\', '/') LIKE ?2 LIMIT 1",
        )
        .bind(current_id)
        .bind(format!("{prefix}%"))
        .fetch_optional(&db.pool)
        .await
        .map(|row| row.is_some())
        .map_err(|e| format!("query component package references failed: {e}"));
    }

    sqlx::query(
        "SELECT id FROM remote_component \
         WHERE deleted = 0 AND id != ?1 AND TRIM(url) = ?2 LIMIT 1",
    )
    .bind(current_id)
    .bind(raw.trim())
    .fetch_optional(&db.pool)
    .await
    .map(|row| row.is_some())
    .map_err(|e| format!("query component file references failed: {e}"))
}

/// 从组件 URL 中提取包目录前缀。
fn component_package_url_prefix(raw: &str) -> Option<String> {
    let normalized = raw.trim().trim_start_matches('/').replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').collect();
    let components_index = parts
        .windows(2)
        .position(|pair| pair[0] == "static" && pair[1] == "components")?;
    let package_index = components_index + 2;
    if package_index >= parts.len() || parts[package_index].is_empty() {
        return None;
    }
    Some(format!("static/components/{}/", parts[package_index]))
}

/// 将组件静态路径解析为本地文件系统路径。
fn resolve_component_storage_path(app_state: &AppState, raw: &str) -> PathBuf {
    let s = raw.trim();
    if s.contains(':') || s.starts_with('/') || s.starts_with('\\') {
        return PathBuf::from(s);
    }
    if s.contains('/') || s.contains('\\') {
        let p1 = app_state.resource_storage_root().join(s);
        if p1.exists() {
            return p1;
        }
        return app_state.files_dir.join(s);
    }
    let p1 = app_state.static_storage_dir().join("components").join(s);
    if p1.exists() {
        return p1;
    }
    app_state.files_dir.join("static").join("components").join(s)
}

/// 删除组件文件或组件包目录。
fn remove_stored_component(app_state: &AppState, raw: &str) {
    let path = resolve_component_storage_path(app_state, raw);
    let normalized = raw.replace('\\', "/");
    let components_dir = app_state.static_storage_dir().join("components");
    if normalized.contains('/') {
        if let Ok(rel) = path.strip_prefix(&components_dir) {
            if let Some(first) = rel.components().next() {
                let package_dir = components_dir.join(first.as_os_str());
                let _ = std::fs::remove_dir_all(package_dir);
                return;
            }
        }
    }
    let _ = std::fs::remove_file(path);
}

/// 解析前端上传的 data URL。
fn parse_data_url(data_url: &str, fallback_mime: Option<&str>) -> Result<(String, Vec<u8>), String> {
    let s = data_url.trim();
    if !s.starts_with("data:") {
        return Err("not data url".to_string());
    }
    let Some((meta, b64)) = s.split_once(',') else {
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

/// 从文件名推导组件展示名。
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

/// 扫描组件 zip 包，选出最适合作为入口的脚本。
fn scan_zip_for_entry_script(zip_bytes: &[u8]) -> Result<String, String> {
    let cursor = Cursor::new(zip_bytes);
    let mut archive = ZipArchive::new(cursor).map_err(|e| format!("zip open failed: {e}"))?;

    let mut best: Option<(i32, usize)> = None;
    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(|e| format!("zip read entry failed: {e}"))?;
        if file.is_dir() {
            continue;
        }
        let Some(rel) = safe_zip_rel_path(file.name()) else {
            continue;
        };
        let lower = rel.to_lowercase();
        if !lower.ends_with(".js") && !lower.ends_with(".mjs") {
            continue;
        }
        let priority = component_entry_priority(&lower);
        match best {
            Some((best_priority, _)) if best_priority >= priority => {}
            _ => best = Some((priority, i)),
        }
    }

    let Some((_, idx)) = best else {
        return Err("zip has no .js or .mjs entry".to_string());
    };
    let file = archive.by_index(idx).map_err(|e| format!("zip read entry failed: {e}"))?;
    safe_zip_rel_path(file.name()).ok_or_else(|| "zip entry path is not safe".to_string())
}

/// 计算组件入口脚本优先级。
fn component_entry_priority(path: &str) -> i32 {
    let depth = path.split('/').count();
    if depth == 1 {
        return 30;
    }
    if path.starts_with("assets/") || path.starts_with("chunks/") {
        return 10;
    }
    if path.contains("/assets/") || path.contains("/chunks/") {
        return 15;
    }
    20
}

/// 安全解压组件 zip 包。
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
        let mut out_file = std::fs::File::create(&out_path).map_err(|e| format!("zip create file failed: {e}"))?;
        std::io::copy(&mut file, &mut out_file).map_err(|e| format!("zip write file failed: {e}"))?;
    }
    Ok(())
}

/// 过滤 zip 条目路径，阻止绝对路径、目录穿越和系统垃圾文件。
fn safe_zip_rel_path(name: &str) -> Option<String> {
    let normalized = name.replace('\\', "/");
    if normalized.starts_with("__MACOSX/") || normalized.ends_with("/.DS_Store") {
        return None;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::db::DbState;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::time::{SystemTime, UNIX_EPOCH};

    async fn build_test_env() -> (DbState, AppState) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory failed");

        datavisool_db::run_base_migrations(&pool).await.expect("run base migrations failed");
        datavisool_db::run_custom_migrations(&pool, sqlx::migrate!("./migrations_desktop"))
            .await
            .expect("run desktop migrations failed");

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("datavisool-remote-component-route-test-{unique}"));
        std::fs::create_dir_all(root.join("static/components")).expect("create temp components dir failed");
        let app_state = AppState::new_for_test(root.clone(), root.clone());

        (DbState { pool }, app_state)
    }

    fn js_data_url(source: &[u8]) -> String {
        format!("data:text/javascript;base64,{}", base64::engine::general_purpose::STANDARD.encode(source))
    }

    #[tokio::test]
    async fn upload_update_keeps_shared_hash_file() {
        let (db, app_state) = build_test_env().await;
        let shared_file = app_state.static_storage_dir().join("components/shared.js");
        std::fs::write(&shared_file, b"export default 'shared';").expect("write shared file failed");

        sqlx::query(
            "INSERT INTO remote_component (id, component_name, url, deleted) \
             VALUES (1, 'old-a', 'shared.js', 0), (2, 'old-b', 'shared.js', 0)",
        )
        .execute(&db.pool)
        .await
        .expect("insert remote components failed");

        let response = upload(
            &db,
            &app_state,
            serde_json::json!({
                "id": "1",
                "fileName": "updated.js",
                "dataUrl": js_data_url(b"export default 'updated';"),
                "componentName": "updated"
            }),
        )
        .await
        .expect("upload update failed");

        assert_eq!(response.code, 200);
        assert!(shared_file.exists(), "shared hash file should stay while another record still references it");
    }

    #[tokio::test]
    async fn upload_update_removes_unreferenced_old_file_after_successful_update() {
        let (db, app_state) = build_test_env().await;
        let old_file = app_state.static_storage_dir().join("components/old.js");
        std::fs::write(&old_file, b"export default 'old';").expect("write old file failed");

        sqlx::query(
            "INSERT INTO remote_component (id, component_name, url, deleted) \
             VALUES (1, 'old', 'old.js', 0)",
        )
        .execute(&db.pool)
        .await
        .expect("insert remote component failed");

        let response = upload(
            &db,
            &app_state,
            serde_json::json!({
                "id": "1",
                "fileName": "updated.js",
                "dataUrl": js_data_url(b"export default 'updated';"),
                "componentName": "updated"
            }),
        )
        .await
        .expect("upload update failed");

        assert_eq!(response.code, 200);
        assert!(!old_file.exists(), "unreferenced old file should be removed");
        let stored_url: String = sqlx::query_scalar("SELECT url FROM remote_component WHERE id = 1")
            .fetch_one(&db.pool)
            .await
            .expect("query updated url failed");
        assert_ne!(stored_url, "old.js");
    }

    #[tokio::test]
    async fn upload_update_keeps_shared_zip_package_dir() {
        let (db, app_state) = build_test_env().await;
        let package_dir = app_state.static_storage_dir().join("components/pkg-old");
        std::fs::create_dir_all(&package_dir).expect("create package dir failed");
        std::fs::write(package_dir.join("entry.js"), b"export default 'entry';").expect("write entry failed");
        std::fs::write(package_dir.join("chunk.js"), b"export default 'chunk';").expect("write chunk failed");

        sqlx::query(
            "INSERT INTO remote_component (id, component_name, url, deleted) VALUES \
             (1, 'old-entry', 'static/components/pkg-old/entry.js', 0), \
             (2, 'old-chunk', 'static/components/pkg-old/chunk.js', 0)",
        )
        .execute(&db.pool)
        .await
        .expect("insert remote components failed");

        let response = upload(
            &db,
            &app_state,
            serde_json::json!({
                "id": "1",
                "fileName": "updated.js",
                "dataUrl": js_data_url(b"export default 'updated';"),
                "componentName": "updated"
            }),
        )
        .await
        .expect("upload update failed");

        assert_eq!(response.code, 200);
        assert!(package_dir.exists(), "zip package dir should stay while another record references an entry in it");
    }
}
