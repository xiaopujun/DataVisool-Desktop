/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 项目发布记录接口。
//!
//! 负责 Pure Release 数据包的创建、覆盖、查询、更新和删除。
//! 发布数据会把项目 JSON 与必要资源依赖组装进 `project_release.data_json`。

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use sqlx::Row;

use crate::app_state::AppState;
use crate::backend::crypto;
use crate::backend::types::{err, err_with_detail, ok, parse_i64_list_param, parse_i64_param, parse_required_i64_param, HttpResponse};
use crate::backend::utils::to_asset_url;
use crate::db::DbState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 发布记录分页查询参数。
struct PageParam {
    current: i64,
    size: i64,
    search_value: Option<String>,
    sort_direction: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 发布记录更新请求体。
struct ReleaseUpdateReq {
    id: Option<String>,
    name: Option<String>,
    state: Option<i64>,
    expiration_date: Option<i64>,
    encrypt: Option<bool>,
    password: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 读取发布数据请求体。
struct ReleaseGetDataReq {
    id: String,
    password: Option<crypto::EncryptResult>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 创建发布记录请求体。
struct ReleaseCreateReq {
    release_type: String,
    project_id: String,
    encrypt: bool,
    password: Option<crypto::EncryptResult>,
    dependencies: Vec<Value>,
    expiration_date: Option<i64>,
}

/// 分页查询发布记录。
pub(crate) async fn page_list(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let page: PageParam = serde_json::from_value(body).map_err(|e| format!("release.pageList: invalid body: {e}"))?;
    let current = if page.current <= 0 { 1 } else { page.current };
    let size = if page.size <= 0 { 10 } else { page.size };
    let offset = (current - 1) * size;

    let total_row = sqlx::query("SELECT COUNT(*) AS cnt FROM project_release WHERE deleted = 0 AND (?1 IS NULL OR name LIKE '%' || ?1 || '%')")
        .bind(page.search_value.clone())
        .fetch_one(&db.pool)
        .await
        .map_err(|e| format!("release.pageList: total query failed: {e}"))?;
    let total: i64 = total_row
        .try_get("cnt")
        .map_err(|e| format!("release.pageList: read total failed: {e}"))?;

    let order = match page.sort_direction.as_deref() {
        Some("asc") => "r.create_time ASC, r.id ASC",
        Some("desc") => "r.create_time DESC, r.id DESC",
        _ => "r.update_time DESC, r.id DESC",
    };
    let query = format!(
        "SELECT r.id, r.name, r.state, r.encrypt, r.expiration_date, r.create_time, r.update_time, p.des \
         FROM project_release r LEFT JOIN project p ON p.id = r.project_id \
         WHERE r.deleted = 0 AND (?1 IS NULL OR r.name LIKE '%' || ?1 || '%') \
         ORDER BY {order} LIMIT ?2 OFFSET ?3"
    );
    let rows = sqlx::query(&query)
    .bind(page.search_value)
    .bind(size)
    .bind(offset)
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("release.pageList: list query failed: {e}"))?;

    let mut records: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("release.pageList: read id failed: {e}"))?;
        let name: Option<String> = row
            .try_get("name")
            .map_err(|e| format!("release.pageList: read name failed: {e}"))?;
        let state_i: i64 = row
            .try_get("state")
            .map_err(|e| format!("release.pageList: read state failed: {e}"))?;
        let encrypt_i: i64 = row
            .try_get("encrypt")
            .map_err(|e| format!("release.pageList: read encrypt failed: {e}"))?;
        let expiration_date: Option<i64> = row
            .try_get("expiration_date")
            .map_err(|e| format!("release.pageList: read expiration_date failed: {e}"))?;
        let update_time: String = row
            .try_get("update_time")
            .map_err(|e| format!("release.pageList: read update_time failed: {e}"))?;
        let create_time: String = row
            .try_get("create_time")
            .map_err(|e| format!("release.pageList: read create_time failed: {e}"))?;
        let description: Option<String> = row
            .try_get("des")
            .map_err(|e| format!("release.pageList: read description failed: {e}"))?;

        records.push(serde_json::json!({
            "id": id.to_string(),
            "name": name.unwrap_or_default(),
            "state": state_i != 0,
            "encrypt": encrypt_i != 0,
            "expirationDate": expiration_date,
            "createTime": create_time,
            "description": description,
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

/// 批量逻辑删除发布记录。
pub(crate) async fn batch_del(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let ids: Vec<String> = serde_json::from_value(body).map_err(|e| format!("release.batchDel: invalid body: {e}"))?;
    if ids.is_empty() {
        return Ok(ok(Value::Bool(true)));
    }

    let parsed = match parse_i64_list_param(&ids, "id") {
        Ok(parsed) => parsed,
        Err(response) => return Ok(response),
    };

    let mut query = String::from("UPDATE project_release SET deleted = 1 WHERE deleted = 0 AND id IN (");
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
        Err(e) => Ok(err_with_detail(500, "删除发布记录失败，请稍后重试", &e.to_string())),
    }
}

/// 更新发布记录元信息。
///
/// 开启加密时会保存解密后的密码，读取发布数据时再校验。
pub(crate) async fn update(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let update_expiration = body.get("expirationDate").is_some();
    let req: ReleaseUpdateReq = serde_json::from_value(body).map_err(|e| format!("release.update: invalid body: {e}"))?;
    let id = match parse_required_i64_param(req.id.as_deref(), "id") {
        Ok(id) => id,
        Err(response) => return Ok(response),
    };

    let mut set_encrypt: Option<i64> = None;
    let mut set_password: Option<Option<String>> = None;
    if let Some(enc) = req.encrypt {
        set_encrypt = Some(if enc { 1 } else { 0 });
        if !enc {
            set_password = Some(None);
        }
    }

    if let Some(pwd_val) = req.password {
        match parse_password_value(pwd_val) {
            Ok(pwd) => {
                set_encrypt = Some(1);
                set_password = Some(Some(pwd));
            }
            Err(e) => return Ok(err_with_detail(400, "请求参数错误：密码不合法", &e)),
        }
    }

    let password_bind: Option<String> = set_password.clone().flatten();
    let changed = sqlx::query(
        "UPDATE project_release SET \
           name = COALESCE(?2, name), \
           state = COALESCE(?3, state), \
           expiration_date = CASE WHEN ?7 THEN ?4 ELSE expiration_date END, \
           encrypt = COALESCE(?5, encrypt), \
           password = COALESCE(?6, password) \
         WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .bind(req.name)
    .bind(req.state)
    .bind(req.expiration_date)
    .bind(set_encrypt)
    .bind(password_bind)
    .bind(update_expiration)
    .execute(&db.pool)
    .await
    .map_err(|e| format!("release.update: update failed: {e}"))?
    .rows_affected();

    if let Some(Some(_)) = set_password {
        let _ = sqlx::query("UPDATE project_release SET encrypt = 1 WHERE id = ?1 AND deleted = 0")
            .bind(id)
            .execute(&db.pool)
            .await;
    }
    if let Some(None) = set_password {
        let _ = sqlx::query("UPDATE project_release SET password = NULL WHERE id = ?1 AND deleted = 0")
            .bind(id)
            .execute(&db.pool)
            .await;
    }

    Ok(ok(Value::Bool(changed > 0)))
}

/// 查询发布记录概要信息。
pub(crate) async fn get_release_info(db: &DbState, id: i64) -> Result<HttpResponse, String> {
    let row = sqlx::query(
        "SELECT id, encrypt, state, expiration_date, create_time, \
          CASE WHEN expiration_date IS NOT NULL AND datetime(create_time, '+' || expiration_date || ' days') <= CURRENT_TIMESTAMP THEN 1 ELSE 0 END AS expired \
         FROM project_release WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .fetch_optional(&db.pool)
    .await
    .map_err(|e| format!("release.getReleaseInfo: query failed: {e}"))?;

    let Some(row) = row else {
        return Ok(err(404, "发布记录不存在或已删除"));
    };

    let encrypt_i: i64 = row
        .try_get("encrypt")
        .map_err(|e| format!("release.getReleaseInfo: read encrypt failed: {e}"))?;
    let state_i: i64 = row
        .try_get("state")
        .map_err(|e| format!("release.getReleaseInfo: read state failed: {e}"))?;
    let expired_i: i64 = row
        .try_get("expired")
        .map_err(|e| format!("release.getReleaseInfo: read expired failed: {e}"))?;

    if state_i == 0 {
        return Ok(err(403, "该发布链接已停用"));
    }
    if expired_i != 0 {
        return Ok(err(410, "该发布链接已过期"));
    }

    Ok(ok(serde_json::json!({
        "encrypt": encrypt_i != 0
    })))
}

/// 读取发布数据。
///
/// 加密发布需要传入密码并通过校验后才返回 dataJson。
pub(crate) async fn get_release_data(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let req: ReleaseGetDataReq = serde_json::from_value(body).map_err(|e| format!("release.getReleaseData: invalid body: {e}"))?;
    let id: i64 = req.id.parse().map_err(|e| format!("release.getReleaseData: invalid id: {e}"))?;

    let row = sqlx::query(
        "SELECT id, name, encrypt, password, data_json, state, \
          CASE WHEN expiration_date IS NOT NULL AND datetime(create_time, '+' || expiration_date || ' days') <= CURRENT_TIMESTAMP THEN 1 ELSE 0 END AS expired \
         FROM project_release WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .fetch_optional(&db.pool)
    .await
    .map_err(|e| format!("release.getReleaseData: query failed: {e}"))?;

    let Some(row) = row else {
        return Ok(err(404, "发布记录不存在或已删除"));
    };

    let state_i: i64 = row
        .try_get("state")
        .map_err(|e| format!("release.getReleaseData: read state failed: {e}"))?;
    let expired_i: i64 = row
        .try_get("expired")
        .map_err(|e| format!("release.getReleaseData: read expired failed: {e}"))?;
    if state_i == 0 {
        return Ok(err(403, "该发布链接已停用"));
    }
    if expired_i != 0 {
        return Ok(err(410, "该发布链接已过期"));
    }

    let encrypt_i: i64 = row
        .try_get("encrypt")
        .map_err(|e| format!("release.getReleaseData: read encrypt failed: {e}"))?;
    let name: Option<String> = row
        .try_get("name")
        .map_err(|e| format!("release.getReleaseData: read name failed: {e}"))?;
    let data_json: Option<String> = row
        .try_get("data_json")
        .map_err(|e| format!("release.getReleaseData: read data_json failed: {e}"))?;
    let stored_password: Option<String> = row
        .try_get("password")
        .map_err(|e| format!("release.getReleaseData: read password failed: {e}"))?;

    if encrypt_i != 0 {
        let Some(enc_pwd) = req.password else {
            return Ok(err(401, "请输入访问密码"));
        };
        let plain = match crypto::decrypt_request(enc_pwd) {
            Ok(v) => v,
            Err(e) => return Ok(err_with_detail(400, "密码解密失败", &e)),
        };
        let Some(stored) = stored_password else {
            return Ok(err(401, "密码校验失败"));
        };
        if stored != plain {
            return Ok(err(401, "密码校验失败"));
        }
    }

    Ok(ok(serde_json::json!({
        "id": id.to_string(),
        "name": name.unwrap_or_default(),
        "dataJson": data_json.unwrap_or_else(|| "{}".to_string())
    })))
}

/// 创建或覆盖发布记录。
///
/// `releaseType=OVERWRITE` 时会覆盖该项目最近一次未删除的发布记录。
pub(crate) async fn release(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: ReleaseCreateReq = serde_json::from_value(body).map_err(|e| format!("release.release: invalid body: {e}"))?;
    let project_id = match parse_i64_param(&req.project_id, "projectId") {
        Ok(project_id) => project_id,
        Err(response) => return Ok(response),
    };

    // 先组装发布包，再根据用户选择决定是否加密保存访问密码。
    let (bundle_json, name_guess) = build_release_bundle(db, app_state, project_id, &req.dependencies).await?;

    let (encrypt_i, password) = if req.encrypt {
        let Some(enc_pwd) = req.password else {
            return Ok(err(400, "请求参数错误：开启加密时必须填写密码"));
        };
        let plain = crypto::decrypt_request(enc_pwd).map_err(|e| format!("release.release: decrypt password failed: {e}"))?;
        (1i64, Some(plain))
    } else {
        (0i64, None)
    };

    let release_type = req.release_type.to_uppercase();
    let mut target_id: Option<i64> = None;
    if release_type == "OVERWRITE" {
        // 覆盖发布只覆盖同一项目最近一次发布，保留其他历史记录。
        let row = sqlx::query("SELECT id FROM project_release WHERE deleted = 0 AND project_id = ?1 ORDER BY update_time DESC LIMIT 1")
            .bind(project_id)
            .fetch_optional(&db.pool)
            .await
            .map_err(|e| format!("release.release: find existing failed: {e}"))?;
        if let Some(row) = row {
            let id: i64 = row.try_get("id").map_err(|e| format!("release.release: read id failed: {e}"))?;
            target_id = Some(id);
        }
    }

    let id = if let Some(id) = target_id {
        let changed = sqlx::query(
            "UPDATE project_release SET \
              name = ?2, \
              state = 1, \
              encrypt = ?3, \
              password = ?4, \
              data_json = ?5, \
              expiration_date = ?6 \
             WHERE id = ?1 AND deleted = 0",
        )
        .bind(id)
        .bind(name_guess)
        .bind(encrypt_i)
        .bind(password)
        .bind(bundle_json)
        .bind(req.expiration_date)
        .execute(&db.pool)
        .await
        .map_err(|e| format!("release.release: overwrite update failed: {e}"))?
        .rows_affected();
        if changed == 0 {
            return Ok(err(500, "覆盖发布失败，请稍后重试"));
        }
        id
    } else {
        let result = sqlx::query(
            "INSERT INTO project_release (name, state, encrypt, password, data_json, project_id, expiration_date, deleted) \
             VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, 0)",
        )
        .bind(name_guess)
        .bind(encrypt_i)
        .bind(password)
        .bind(bundle_json)
        .bind(project_id)
        .bind(req.expiration_date)
        .execute(&db.pool)
        .await
        .map_err(|e| format!("release.release: insert failed: {e}"))?;
        result.last_insert_rowid()
    };

    Ok(ok(Value::String(id.to_string())))
}

/// 组装发布数据包。
///
/// 当前包包含入口项目 JSON、依赖图、组件 URL 和字体 URL。
async fn build_release_bundle(db: &DbState, app_state: &AppState, project_id: i64, dependencies: &[Value]) -> Result<(String, String), String> {
    let mut main: Option<&Value> = None;
    for dep in dependencies {
        if dep.get("id").and_then(|v| v.as_str()).and_then(|s| s.parse::<i64>().ok()) == Some(project_id) {
            main = Some(dep);
            break;
        }
    }
    if main.is_none() {
        main = dependencies.first();
    }

    let name_guess = if let Some(n) = main.and_then(|v| v.get("name")).and_then(|v| v.as_str()).map(|s| s.to_string()) {
        n
    } else {
        let row = sqlx::query("SELECT name FROM project WHERE id = ?1 AND deleted = 0")
            .bind(project_id)
            .fetch_optional(&db.pool)
            .await
            .map_err(|e| format!("release.release: query project name failed: {e}"))?;
        row.and_then(|r| r.try_get::<String, _>("name").ok())
            .unwrap_or_else(|| "未命名项目".to_string())
    };

    let project_json = main
        .and_then(|v| v.get("projectJson"))
        .and_then(|v| v.as_str())
        .unwrap_or("{}")
        .to_string();

    // dependencies 来自前端发布面板，先收集真实需要的字体和组件依赖。
    let mut font_names: Vec<String> = Vec::new();
    let mut component_files: Vec<String> = Vec::new();
    for dep in dependencies {
        if let Some(arr) = dep.get("fonts").and_then(|v| v.as_array()) {
            for it in arr {
                if let Some(s) = it.as_str() {
                    if !font_names.contains(&s.to_string()) {
                        font_names.push(s.to_string());
                    }
                }
            }
        }
        if let Some(arr) = dep.get("components").and_then(|v| v.as_array()) {
            for it in arr {
                if let Some(s) = it.as_str() {
                    if !component_files.contains(&s.to_string()) {
                        component_files.push(s.to_string());
                    }
                }
            }
        }
    }

    let fonts = if font_names.is_empty() {
        Vec::<Value>::new()
    } else {
        let rows = sqlx::query("SELECT name, url FROM font WHERE deleted = 0")
            .fetch_all(&db.pool)
            .await
            .map_err(|e| format!("release.release: query font failed: {e}"))?;
        let mut out: Vec<Value> = Vec::new();
        for row in rows {
            let name: String = row
                .try_get("name")
                .map_err(|e| format!("release.release: read font name failed: {e}"))?;
            if !font_names.contains(&name) {
                continue;
            }
            let url: String = row
                .try_get("url")
                .map_err(|e| format!("release.release: read font url failed: {e}"))?;
            out.push(serde_json::json!({
                "name": name,
                "value": resolve_font_asset_url(app_state, &url)
            }));
        }
        out
    };

    let components = resolve_component_urls(db, app_state, &component_files).await?;
    let bundle = serde_json::json!({
        "projectJson": project_json,
        "dependencies": dependencies,
        "components": components,
        "fonts": fonts,
        "images": [],
        "dynamicScripts": []
    });
    let s = serde_json::to_string(&bundle).map_err(|e| format!("release.release: serialize bundle failed: {e}"))?;
    Ok((s, name_guess))
}

/// 解析发布依赖中的组件 URL。
async fn resolve_component_urls(db: &DbState, app_state: &AppState, component_files: &[String]) -> Result<Vec<Value>, String> {
    if component_files.is_empty() {
        return Ok(vec![]);
    }
    let mut out: Vec<Value> = Vec::new();
    for file in component_files {
        let row = sqlx::query("SELECT url FROM remote_component WHERE deleted = 0 AND url LIKE '%' || ?1 ORDER BY update_time DESC LIMIT 1")
            .bind(file)
            .fetch_optional(&db.pool)
            .await
            .map_err(|e| format!("release.release: query remote_component failed: {e}"))?;
        let Some(row) = row else { continue };
        let raw: String = row
            .try_get("url")
            .map_err(|e| format!("release.release: read remote_component url failed: {e}"))?;
        let resolved = resolve_component_asset_url(app_state, &raw);
        out.push(serde_json::json!({ "value": resolved }));
    }
    Ok(out)
}

/// 将字体路径解析为发布运行时可访问的 asset URL。
fn resolve_font_asset_url(app_state: &AppState, raw: &str) -> String {
    let s = raw.trim();
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("data:") || s.starts_with("blob:") {
        return s.to_string();
    }
    if s.contains(':') || s.starts_with('/') || s.starts_with('\\') {
        return to_asset_url(&PathBuf::from(s));
    }
    if s.contains('/') || s.contains('\\') {
        let p1 = app_state.resource_storage_root().join(s);
        if p1.exists() {
            return to_asset_url(&p1);
        }
        return to_asset_url(&app_state.files_dir.join(s));
    }
    let p1 = app_state.static_storage_dir().join("font").join(s);
    if p1.exists() {
        return to_asset_url(&p1);
    }
    to_asset_url(&app_state.files_dir.join("static").join("font").join(s))
}

/// 将组件路径解析为发布运行时可访问的 asset URL。
fn resolve_component_asset_url(app_state: &AppState, raw: &str) -> String {
    let s = raw.trim();
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("data:") || s.starts_with("blob:") {
        return s.to_string();
    }
    if s.contains(':') || s.starts_with('/') || s.starts_with('\\') {
        return to_asset_url(&PathBuf::from(s));
    }
    if s.contains('/') || s.contains('\\') {
        let p1 = app_state.resource_storage_root().join(s);
        if p1.exists() {
            return to_asset_url(&p1);
        }
        return to_asset_url(&app_state.files_dir.join(s));
    }
    let p1 = app_state.static_storage_dir().join("components").join(s);
    if p1.exists() {
        return to_asset_url(&p1);
    }
    to_asset_url(&app_state.files_dir.join("static").join("components").join(s))
}

/// 兼容明文和加密两种密码输入格式。
fn parse_password_value(v: Value) -> Result<String, String> {
    if let Some(s) = v.as_str() {
        return Ok(s.to_string());
    }
    let enc: crypto::EncryptResult = serde_json::from_value(v).map_err(|e| format!("parse password failed: {e}"))?;
    crypto::decrypt_request(enc).map_err(|e| format!("decrypt password failed: {e}"))
}
