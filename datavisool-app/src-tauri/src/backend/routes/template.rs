/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! template 相关接口（模板库）。
//!
//! 负责模板的查询、详情读取、创建、批量软删除，以及模板封面图的本地静态资源落盘。

use std::path::{Path, PathBuf};

use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use sqlx::Row;

use crate::app_state::AppState;
use crate::backend::types::{err, err_with_detail, ok, parse_i64_list_param, HttpResponse};
use crate::backend::utils::{sanitize_error_message, to_asset_url};
use crate::db::DbState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 模板列表查询请求。
struct TemplateListReq {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 创建模板请求体。
struct TemplateCreateReq {
    name: String,
    description: Option<String>,
    template_json: String,
    cover_file: Option<TemplateCoverFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 前端上传的模板封面文件。
struct TemplateCoverFile {
    file_name: String,
    data_url: String,
    mime: Option<String>,
}

/// 查询模板库列表，支持按名称模糊搜索。
pub(crate) async fn get_all_template(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: TemplateListReq = serde_json::from_value(body).unwrap_or(TemplateListReq { name: None });
    let search = req.name.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());

    // 列表接口只返回模板元信息，模板 JSON 由详情接口单独读取。
    let rows = sqlx::query(
        "SELECT id, name, description, cover, create_time, update_time \
         FROM template \
         WHERE deleted = 0 AND (?1 IS NULL OR name LIKE '%' || ?1 || '%') \
         ORDER BY update_time DESC",
    )
    .bind(search)
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("template.getAllTemplate: query failed: {e}"))?;

    let mut out: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("template.getAllTemplate: read id failed: {e}"))?;
        let name: String = row
            .try_get("name")
            .map_err(|e| format!("template.getAllTemplate: read name failed: {e}"))?;
        let description: Option<String> = row
            .try_get("description")
            .map_err(|e| format!("template.getAllTemplate: read description failed: {e}"))?;
        let cover: Option<String> = row
            .try_get("cover")
            .map_err(|e| format!("template.getAllTemplate: read cover failed: {e}"))?;
        let create_time: String = row
            .try_get("create_time")
            .map_err(|e| format!("template.getAllTemplate: read create_time failed: {e}"))?;
        let update_time: String = row
            .try_get("update_time")
            .map_err(|e| format!("template.getAllTemplate: read update_time failed: {e}"))?;

        out.push(serde_json::json!({
            "id": id.to_string(),
            "name": name,
            "description": description.unwrap_or_default(),
            "coverUrl": cover.as_deref().map(|v| resolve_cover_url(app_state, v)).unwrap_or_default(),
            "createTime": create_time,
            "updateTime": update_time
        }));
    }

    Ok(ok(Value::Array(out)))
}

/// 查询单个模板详情。
pub(crate) async fn get_template(db: &DbState, app_state: &AppState, id: i64) -> Result<HttpResponse, String> {
    let row = sqlx::query(
        "SELECT id, name, description, cover, template_json, create_time, update_time \
         FROM template \
         WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .fetch_optional(&db.pool)
    .await
    .map_err(|e| format!("template.getTemplate: query failed: {e}"))?;

    let Some(row) = row else {
        return Ok(err(404, "模板不存在或已删除"));
    };

    let id: i64 = row
        .try_get("id")
        .map_err(|e| format!("template.getTemplate: read id failed: {e}"))?;
    let name: String = row
        .try_get("name")
        .map_err(|e| format!("template.getTemplate: read name failed: {e}"))?;
    let description: Option<String> = row
        .try_get("description")
        .map_err(|e| format!("template.getTemplate: read description failed: {e}"))?;
    let cover: Option<String> = row
        .try_get("cover")
        .map_err(|e| format!("template.getTemplate: read cover failed: {e}"))?;
    let template_json: String = row
        .try_get("template_json")
        .map_err(|e| format!("template.getTemplate: read template_json failed: {e}"))?;
    let create_time: String = row
        .try_get("create_time")
        .map_err(|e| format!("template.getTemplate: read create_time failed: {e}"))?;
    let update_time: String = row
        .try_get("update_time")
        .map_err(|e| format!("template.getTemplate: read update_time failed: {e}"))?;

    Ok(ok(serde_json::json!({
        "id": id.to_string(),
        "name": name,
        "description": description.unwrap_or_default(),
        "coverUrl": cover.as_deref().map(|v| resolve_cover_url(app_state, v)).unwrap_or_default(),
        "templateJson": template_json,
        "createTime": create_time,
        "updateTime": update_time
    })))
}

/// 创建模板，并在提供封面时保存封面静态资源。
pub(crate) async fn create(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: TemplateCreateReq = serde_json::from_value(body).map_err(|e| format!("template.create: invalid body: {e}"))?;
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Ok(err(400, "请求参数错误：模板名称不能为空"));
    }
    if req.template_json.trim().is_empty() {
        return Ok(err(400, "请求参数错误：模板内容不能为空"));
    }
    if let Err(e) = serde_json::from_str::<Value>(&req.template_json) {
        return Ok(err_with_detail(400, "请求参数错误：模板内容不是合法 JSON", e.to_string()));
    }

    // 封面文件先完成校验和落盘，再写入模板记录，避免记录指向非法文件。
    let cover = match req.cover_file {
        Some(file) => match save_cover_file(app_state, file) {
            Ok(v) => Some(v),
            Err(e) => return Ok(err_with_detail(400, "请求参数错误：封面图数据不合法", e)),
        },
        None => None,
    };

    let description = req.description.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());

    match sqlx::query(
        "INSERT INTO template (name, description, cover, template_json, deleted) \
         VALUES (?1, ?2, ?3, ?4, 0)",
    )
    .bind(name)
    .bind(description)
    .bind(cover)
    .bind(req.template_json)
    .execute(&db.pool)
    .await
    {
        Ok(_) => Ok(ok(Value::Bool(true))),
        Err(e) => Ok(err_with_detail(500, "保存模板失败，请稍后重试", sanitize_error_message(&e.to_string()))),
    }
}

/// 批量软删除模板。
pub(crate) async fn batch_del(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let ids: Vec<String> = serde_json::from_value(body).map_err(|e| format!("template.batchDel: invalid body: {e}"))?;
    if ids.is_empty() {
        return Ok(ok(Value::Bool(true)));
    }

    let parsed = match parse_i64_list_param(&ids, "id") {
        Ok(parsed) => parsed,
        Err(response) => return Ok(response),
    };

    // SQLite 不支持直接绑定数组，这里按 id 数量构造 IN 占位符。
    let mut query = String::from("UPDATE template SET deleted = 1 WHERE deleted = 0 AND id IN (");
    for index in 0..parsed.len() {
        if index > 0 {
            query.push(',');
        }
        query.push('?');
        query.push_str(&(index + 1).to_string());
    }
    query.push(')');

    let mut q = sqlx::query(&query);
    for id in parsed {
        q = q.bind(id);
    }

    match q.execute(&db.pool).await {
        Ok(r) => Ok(ok(Value::Bool(r.rows_affected() > 0))),
        Err(e) => Ok(err_with_detail(500, "删除模板失败，请稍后重试", sanitize_error_message(&e.to_string()))),
    }
}

/// 保存模板封面文件，并返回数据库中存储的文件名。
fn save_cover_file(app_state: &AppState, file: TemplateCoverFile) -> Result<String, String> {
    let (mime, bytes) = parse_data_url(&file.data_url, file.mime.as_deref())?;
    let ext = file_extension(&file.file_name).or_else(|| mime_to_ext(&mime));
    let Some(ext) = ext else {
        return Err("无法识别封面图类型".to_string());
    };
    if !is_allowed_image_ext(&ext) {
        return Err("仅支持 bmp/jpg/jpeg/png/gif/svg/webp 图片".to_string());
    }

    // 用内容 hash 命名可以天然去重，并避免原始文件名中的路径或特殊字符风险。
    let hash = crate::backend::utils::sha256_hex(&bytes);
    let dir = app_state.static_storage_dir().join("templates").join("covers");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create cover dir failed: {e}"))?;

    let file_name = format!("{}.{}", hash, ext);
    let file_path = dir.join(&file_name);
    if !file_path.exists() {
        std::fs::write(&file_path, bytes).map_err(|e| format!("write cover failed: {e}"))?;
    }
    Ok(file_name)
}

/// 将数据库中的封面值转换为前端可访问的 asset URL。
fn resolve_cover_url(app_state: &AppState, raw: &str) -> String {
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

    let p1 = app_state.static_storage_dir().join("templates").join("covers").join(s);
    if p1.exists() {
        return to_asset_url(&p1);
    }
    to_asset_url(&app_state.files_dir.join("static").join("templates").join("covers").join(s))
}

/// 解析 base64 data URL，返回 MIME 和原始字节。
fn parse_data_url(data_url: &str, fallback_mime: Option<&str>) -> Result<(String, Vec<u8>), String> {
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
        .or(fallback_mime)
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("base64 decode failed: {e}"))?;
    Ok((mime, bytes))
}

/// 从文件名提取扩展名。
fn file_extension(file_name: &str) -> Option<String> {
    Path::new(file_name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty())
}

/// 将图片 MIME 映射为文件扩展名。
fn mime_to_ext(mime: &str) -> Option<String> {
    match mime.to_ascii_lowercase().as_str() {
        "image/bmp" => Some("bmp".to_string()),
        "image/jpeg" | "image/jpg" => Some("jpg".to_string()),
        "image/png" => Some("png".to_string()),
        "image/gif" => Some("gif".to_string()),
        "image/svg+xml" => Some("svg".to_string()),
        "image/webp" => Some("webp".to_string()),
        _ => None,
    }
}

/// 判断扩展名是否属于模板封面允许的图片类型。
fn is_allowed_image_ext(ext: &str) -> bool {
    matches!(ext.to_ascii_lowercase().as_str(), "bmp" | "jpg" | "jpeg" | "png" | "gif" | "svg" | "webp")
}
