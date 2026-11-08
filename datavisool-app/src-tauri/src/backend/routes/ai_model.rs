/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 模型配置接口。
//!
//! 负责 AI 模型的分页查询、下拉列表、创建、更新与批量删除。
//! 其中 API Key 采用本地密钥加密落盘，并在读取历史数据时做懒回写，
//! 管理当前格式的模型配置。

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

use aes::cipher::{BlockEncryptMut, InvalidLength, KeyIvInit};
use aes::Aes256;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use block_padding::Pkcs7;
use cbc::Encryptor;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;

use crate::app_state::AppState;
use crate::backend::ai_model_catalog::list_ai_model_catalog_providers;
use crate::backend::ai_model_provider::{normalize_provider_key_for_storage, provider_display_name, provider_family};
use crate::backend::types::{err, err_with_detail, ok, parse_i64_list_param, parse_required_i64_param, HttpResponse};
use crate::db::DbState;

/// AI 模型分页查询参数。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageParam {
    current: i64,
    size: i64,
    search_value: Option<String>,
}

/// AI 模型新增/编辑请求体。
///
/// 字段全部做成可选，方便同一个结构同时承接“创建”和“更新”两个表单场景。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiModelItem {
    id: Option<String>,
    name: Option<String>,
    provider: Option<String>,
    base_url: Option<String>,
    api_key: Option<String>,
    description: Option<String>,
    temperature: Option<f64>,
    max_tokens: Option<i64>,
    enabled: Option<i64>,
    is_default: Option<i64>,
    clear_api_key: Option<i64>,
}

/// 用于下拉选择的精简模型项。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiModelSelectItem {
    id: String,
    name: String,
    provider: String,
    provider_label: String,
    provider_family: String,
    label: String,
    max_tokens: Option<i64>,
    is_default: i64,
}

/// 前端轻量读取已配置模型时使用的最小模型项。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiModelConfiguredListItem {
    /// 模型配置记录 id。
    id: String,
    /// 用户在 AI 模型管理中维护的自定义模型名称。
    name: String,
    /// 模型厂商标识。
    provider: String,
}

/// 将必填字段做 `trim` 后校验，统一返回 400。
fn required_trimmed(value: Option<String>, field: &str) -> Result<String, HttpResponse> {
    let value = value.unwrap_or_default().trim().to_string();
    if value.is_empty() {
        Err(err(400, format!("请求参数错误：{field} 不能为空")))
    } else {
        Ok(value)
    }
}

/// 将可选字符串做 `trim`，空字符串视为 `None`。
fn optional_trimmed(value: Option<String>) -> Option<String> {
    value.map(|item| item.trim().to_string()).filter(|item| !item.is_empty())
}

/// 将开关类字段规整为 `0/1`。
fn normalize_switch_flag(value: Option<i64>, default_value: i64) -> i64 {
    if value.unwrap_or(default_value) == 0 {
        0
    } else {
        1
    }
}

/// 规整温度范围，避免前端或历史数据写入异常值。
fn normalize_temperature(value: Option<f64>) -> f64 {
    let value = value.unwrap_or(0.7);
    if value.is_finite() {
        value.clamp(0.0, 2.0)
    } else {
        0.7
    }
}

/// 规整 `max_tokens`，非正数视为未设置。
fn normalize_max_tokens(value: Option<i64>) -> Option<i64> {
    value.and_then(|item| if item > 0 { Some(item) } else { None })
}

/// 组合模型展示名。
///
/// 优先使用用户自定义名称；如果为空，则回退到 provider 的展示名。
/// 默认项会额外标记“默认”，方便列表与下拉框直接复用。
fn build_model_label(name: &str, provider: &str, is_default: i64) -> String {
    let display_name = if name.trim().is_empty() {
        provider_display_name(provider).trim().to_string()
    } else {
        name.trim().to_string()
    };

    let provider = provider.trim();
    let detail = if provider.is_empty() {
        String::new()
    } else {
        provider_display_name(provider)
    };

    let mut label = if detail.is_empty() {
        display_name.to_string()
    } else {
        format!("{display_name} · {detail}")
    };

    if is_default == 1 {
        label.push_str("（默认）");
    }

    label
}

/// API Key 落盘后的加密格式前缀。
const AI_MODEL_API_KEY_ENCRYPTION_PREFIX: &str = "enc:v1:";

/// 进程内缓存的 API Key 加密密钥。
///
/// 首次访问时会从 `AppState` 指定的路径加载；如果文件不存在，则自动生成并写入。
static AI_MODEL_API_KEY_SECRET: OnceLock<Vec<u8>> = OnceLock::new();

/// 获取 API Key 密钥文件路径。
///
/// 路径由 `AppState` 统一管理，避免在不同路由里重复拼接同一条目录结构。
fn ai_model_api_key_secret_path(app_state: &AppState) -> PathBuf {
    app_state.ai_model_api_key_secret_file()
}

/// 读取或创建本地 API Key 加密密钥，并缓存到进程内。
///
/// 密钥固定为 32 字节，满足 AES-256 的 key 长度要求。
fn load_or_create_ai_model_api_key_secret(app_state: &AppState) -> Result<&'static [u8], String> {
    if let Some(secret) = AI_MODEL_API_KEY_SECRET.get() {
        return Ok(secret.as_slice());
    }

    let path = ai_model_api_key_secret_path(app_state);
    let parent = path
        .parent()
        .ok_or_else(|| "aiModel.secretKey: invalid secret key path".to_string())?;
    fs::create_dir_all(parent).map_err(|e| format!("aiModel.secretKey: create parent dir failed: {e}"))?;

    let secret = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            // 文件不存在时生成新密钥，首次落盘后供后续启动复用。
            let mut bytes = vec![0u8; 32];
            OsRng.fill_bytes(&mut bytes);

            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(&bytes)
                        .map_err(|e| format!("aiModel.secretKey: write secret key failed: {e}"))?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let permissions = fs::Permissions::from_mode(0o600);
                        fs::set_permissions(&path, permissions).map_err(|e| format!("aiModel.secretKey: set file permissions failed: {e}"))?;
                    }
                    bytes
                }
                Err(open_err) if open_err.kind() == ErrorKind::AlreadyExists => {
                    // 并发创建场景下，其他进程已经写好了，直接复读即可。
                    fs::read(&path).map_err(|e| format!("aiModel.secretKey: read existing secret key failed: {e}"))?
                }
                Err(open_err) => {
                    return Err(format!("aiModel.secretKey: create secret key file failed: {open_err}"));
                }
            }
        }
        Err(err) => {
            return Err(format!("aiModel.secretKey: read secret key failed: {err}"));
        }
    };

    // AES-256 的密钥长度必须严格为 32 字节。
    if secret.len() != 32 {
        return Err(format!("aiModel.secretKey: invalid secret key length {}, expected 32", secret.len()));
    }

    let _ = AI_MODEL_API_KEY_SECRET.set(secret);
    AI_MODEL_API_KEY_SECRET
        .get()
        .map(|value| value.as_slice())
        .ok_or_else(|| "aiModel.secretKey: initialize secret key cache failed".to_string())
}

/// 对 API Key 做加密落盘。
///
/// 已经带有加密前缀的值会直接透传，避免重复加密。
fn encrypt_ai_model_api_key(app_state: &AppState, plain_text: &str) -> Result<String, String> {
    let trimmed = plain_text.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.starts_with(AI_MODEL_API_KEY_ENCRYPTION_PREFIX) {
        return Ok(trimmed.to_string());
    }

    let secret = load_or_create_ai_model_api_key_secret(app_state)?;
    // 每次都使用随机 IV，保证相同明文每次加密结果都不同。
    let mut iv = [0u8; 16];
    OsRng.fill_bytes(&mut iv);

    let plain_bytes = trimmed.as_bytes();
    let block_size = 16usize;
    let padded_len = ((plain_bytes.len() + block_size) / block_size) * block_size;
    let mut buf = vec![0u8; padded_len];
    buf[..plain_bytes.len()].copy_from_slice(plain_bytes);

    let cipher = Encryptor::<Aes256>::new_from_slices(secret, &iv)
        .map_err(|e: InvalidLength| format!("aiModel.encryptApiKey: invalid key/iv length: {e}"))?
        .encrypt_padded_mut::<Pkcs7>(&mut buf, plain_bytes.len())
        .map_err(|e| format!("aiModel.encryptApiKey: encrypt failed: {e}"))?;

    Ok(format!("{}{}:{}", AI_MODEL_API_KEY_ENCRYPTION_PREFIX, STANDARD.encode(iv), STANDARD.encode(cipher)))
}

/// 将表单中的 API Key 规范化为可入库格式。
///
/// 空值保留为空；非空值统一加密后再落库。
fn normalize_api_key_for_storage(app_state: &AppState, value: Option<String>) -> Result<Option<String>, String> {
    match optional_trimmed(value) {
        Some(api_key) => Ok(Some(encrypt_ai_model_api_key(app_state, &api_key)?)),
        None => Ok(None),
    }
}


/// 分页查询 AI 模型。
///
/// 返回结构保持和前端 `pageList` 协议一致，`apiKey` 始终脱敏为空字符串，
/// 仅返回密钥是否存在。
pub(crate) async fn page_list(db: &DbState, _app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let page: PageParam = serde_json::from_value(body).map_err(|e| format!("aiModel.pageList: invalid body: {e}"))?;
    let current = if page.current <= 0 { 1 } else { page.current };
    let size = if page.size <= 0 { 12 } else { page.size };
    let offset = (current - 1) * size;

    let keyword = page
        .search_value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty());

    let total_row = sqlx::query(
        "SELECT COUNT(*) AS cnt \
         FROM ai_model \
         WHERE deleted = 0 \
           AND (?1 IS NULL \
             OR name LIKE '%' || ?1 || '%' \
             OR provider LIKE '%' || ?1 || '%' \
             OR base_url LIKE '%' || ?1 || '%' \
             OR description LIKE '%' || ?1 || '%')",
    )
    .bind(keyword.clone())
    .fetch_one(&db.pool)
    .await
    .map_err(|e| format!("aiModel.pageList: total query failed: {e}"))?;

    let total: i64 = total_row
        .try_get("cnt")
        .map_err(|e| format!("aiModel.pageList: read total failed: {e}"))?;

    let rows = sqlx::query(
        "SELECT id, name, provider, base_url, api_key, description, temperature, \
                max_tokens, enabled, is_default, create_time, update_time \
         FROM ai_model \
         WHERE deleted = 0 \
           AND (?1 IS NULL \
             OR name LIKE '%' || ?1 || '%' \
             OR provider LIKE '%' || ?1 || '%' \
             OR base_url LIKE '%' || ?1 || '%' \
             OR description LIKE '%' || ?1 || '%') \
         ORDER BY is_default DESC, enabled DESC, update_time DESC, id DESC \
         LIMIT ?2 OFFSET ?3",
    )
    .bind(keyword)
    .bind(size)
    .bind(offset)
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("aiModel.pageList: list query failed: {e}"))?;

    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("aiModel.pageList: read id failed: {e}"))?;
        let name: String = row
            .try_get("name")
            .map_err(|e| format!("aiModel.pageList: read name failed: {e}"))?;
        let provider: String = row
            .try_get("provider")
            .map_err(|e| format!("aiModel.pageList: read provider failed: {e}"))?;
        let base_url: Option<String> = row
            .try_get("base_url")
            .map_err(|e| format!("aiModel.pageList: read base_url failed: {e}"))?;
        let api_key: Option<String> = row
            .try_get("api_key")
            .map_err(|e| format!("aiModel.pageList: read api_key failed: {e}"))?;
        let description: Option<String> = row
            .try_get("description")
            .map_err(|e| format!("aiModel.pageList: read description failed: {e}"))?;
        let temperature: f64 = row
            .try_get("temperature")
            .map_err(|e| format!("aiModel.pageList: read temperature failed: {e}"))?;
        let max_tokens: Option<i64> = row
            .try_get("max_tokens")
            .map_err(|e| format!("aiModel.pageList: read max_tokens failed: {e}"))?;
        let enabled: i64 = row
            .try_get("enabled")
            .map_err(|e| format!("aiModel.pageList: read enabled failed: {e}"))?;
        let is_default: i64 = row
            .try_get("is_default")
            .map_err(|e| format!("aiModel.pageList: read is_default failed: {e}"))?;
        let create_time: String = row
            .try_get("create_time")
            .map_err(|e| format!("aiModel.pageList: read create_time failed: {e}"))?;
        let update_time: String = row
            .try_get("update_time")
            .map_err(|e| format!("aiModel.pageList: read update_time failed: {e}"))?;
        let provider = normalize_provider_key_for_storage(&provider);
        let provider_label = provider_display_name(&provider);
        let provider_family_value = provider_family(&provider);

        let has_api_key = api_key.as_ref().map(|value| !value.trim().is_empty()).unwrap_or(false);

        records.push(json!({
            "id": id.to_string(),
            "name": name,
            "provider": provider,
            "providerLabel": provider_label,
            "providerFamily": provider_family_value,
            "baseUrl": base_url.unwrap_or_default(),
            "apiKey": "",
            "hasApiKey": has_api_key,
            "description": description.unwrap_or_default(),
            "temperature": temperature,
            "maxTokens": max_tokens,
            "enabled": enabled,
            "isDefault": is_default,
            "createTime": create_time,
            "updateTime": update_time,
        }));
    }

    Ok(ok(json!({
        "records": records,
        "total": total,
        "size": size,
        "current": current
    })))
}

/// 获取可用于下拉选择的 AI 模型列表。
pub(crate) async fn list(db: &DbState) -> Result<HttpResponse, String> {
    let rows = sqlx::query(
        "SELECT id, name, provider, max_tokens, is_default \
         FROM ai_model \
         WHERE deleted = 0 AND enabled = 1 \
         ORDER BY is_default DESC, update_time DESC, id DESC",
    )
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("aiModel.list: query failed: {e}"))?;

    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.try_get("id").map_err(|e| format!("aiModel.list: read id failed: {e}"))?;
        let name: String = row
            .try_get("name")
            .map_err(|e| format!("aiModel.list: read name failed: {e}"))?;
        let provider: String = row
            .try_get("provider")
            .map_err(|e| format!("aiModel.list: read provider failed: {e}"))?;
        let max_tokens: Option<i64> = row
            .try_get("max_tokens")
            .map_err(|e| format!("aiModel.list: read max_tokens failed: {e}"))?;
        let is_default: i64 = row
            .try_get("is_default")
            .map_err(|e| format!("aiModel.list: read is_default failed: {e}"))?;
        let provider = normalize_provider_key_for_storage(&provider);

        records.push(AiModelSelectItem {
            id: id.to_string(),
            name: name.clone(),
            provider: provider.clone(),
            provider_label: provider_display_name(&provider),
            provider_family: provider_family(&provider).to_string(),
            label: build_model_label(&name, &provider, is_default),
            max_tokens,
            is_default,
        });
    }

    Ok(ok(json!(records)))
}

/// 获取已配置 AI 模型的轻量列表。
///
/// 该接口只返回前端选择配置所需的 id、名称和厂商，避免暴露 API Key、URL、默认项等完整管理字段。
pub(crate) async fn configured_list(db: &DbState) -> Result<HttpResponse, String> {
    let rows = sqlx::query(
        "SELECT id, name, provider \
         FROM ai_model \
         WHERE deleted = 0 \
         ORDER BY is_default DESC, enabled DESC, update_time DESC, id DESC",
    )
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("aiModel.configuredList: query failed: {e}"))?;

    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("aiModel.configuredList: read id failed: {e}"))?;
        let name: String = row
            .try_get("name")
            .map_err(|e| format!("aiModel.configuredList: read name failed: {e}"))?;
        let provider: String = row
            .try_get("provider")
            .map_err(|e| format!("aiModel.configuredList: read provider failed: {e}"))?;
        let provider = normalize_provider_key_for_storage(&provider);

        records.push(AiModelConfiguredListItem { id: id.to_string(), name, provider });
    }

    Ok(ok(json!(records)))
}

/// 新增 AI 模型。
///
/// 默认项创建成功后会自动启用，并清空其他记录的默认标记，保证默认模型唯一。
pub(crate) async fn create(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: AiModelItem = serde_json::from_value(body).map_err(|e| format!("aiModel.create: invalid body: {e}"))?;

    let name = match required_trimmed(req.name, "name") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let provider = match required_trimmed(req.provider, "provider") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let provider = normalize_provider_key_for_storage(&provider);
    let base_url = optional_trimmed(req.base_url);
    let api_key = normalize_api_key_for_storage(app_state, req.api_key).map_err(|e| format!("aiModel.create: normalize api key failed: {e}"))?;
    let description = optional_trimmed(req.description);
    let mut enabled = normalize_switch_flag(req.enabled, 1);
    let is_default = normalize_switch_flag(req.is_default, 0);
    let temperature = normalize_temperature(req.temperature);
    let max_tokens = normalize_max_tokens(req.max_tokens);
    if is_default == 1 {
        // 默认模型必须可用，否则前端选中后会变成不可用状态。
        enabled = 1;
    }

    let mut tx = db
        .pool
        .begin()
        .await
        .map_err(|e| format!("aiModel.create: begin transaction failed: {e}"))?;

    if is_default == 1 {
        // 新默认项落库前，先清空其他记录的默认标记，避免出现多个默认项。
        sqlx::query("UPDATE ai_model SET is_default = 0 WHERE deleted = 0")
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("aiModel.create: clear default failed: {e}"))?;
    }

    let result = sqlx::query(
        "INSERT INTO ai_model \
            (name, provider, base_url, api_key, description, temperature, max_tokens, enabled, is_default, deleted) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
    )
    .bind(name)
    .bind(provider)
    .bind(base_url)
    .bind(api_key)
    .bind(description)
    .bind(temperature)
    .bind(max_tokens)
    .bind(enabled)
    .bind(is_default)
    .execute(&mut *tx)
    .await;

    match result {
        Ok(result) => {
            tx.commit().await.map_err(|e| format!("aiModel.create: commit failed: {e}"))?;
            Ok(ok(Value::String(result.last_insert_rowid().to_string())))
        }
        Err(e) => Ok(err_with_detail(500, "新增 AI 模型失败，请稍后重试", &e.to_string())),
    }
}

/// 返回 AI 模型目录信息，供前端展示 provider 能力范围。
pub(crate) async fn catalog() -> Result<HttpResponse, String> {
    match list_ai_model_catalog_providers() {
        Ok(providers) => Ok(ok(json!(providers))),
        Err(detail) => Ok(err(500, detail)),
    }
}

/// 更新 AI 模型。
///
/// 未提交新密钥时保留当前已加密的 API Key。
pub(crate) async fn update(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: AiModelItem = serde_json::from_value(body).map_err(|e| format!("aiModel.update: invalid body: {e}"))?;
    let id = match parse_required_i64_param(req.id.as_deref(), "id") {
        Ok(id) => id,
        Err(response) => return Ok(response),
    };

    let name = match required_trimmed(req.name, "name") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let provider = match required_trimmed(req.provider, "provider") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let provider = normalize_provider_key_for_storage(&provider);
    let base_url = optional_trimmed(req.base_url);
    let clear_api_key = normalize_switch_flag(req.clear_api_key, 0);
    let api_key = normalize_api_key_for_storage(app_state, req.api_key).map_err(|e| format!("aiModel.update: normalize api key failed: {e}"))?;
    let description = optional_trimmed(req.description);
    let mut enabled = normalize_switch_flag(req.enabled, 1);
    let is_default = normalize_switch_flag(req.is_default, 0);
    let temperature = normalize_temperature(req.temperature);
    let max_tokens = normalize_max_tokens(req.max_tokens);
    if is_default == 1 {
        // 默认模型必须可用，否则前端会出现“默认但不可选”的异常状态。
        enabled = 1;
    }

    let mut tx = db
        .pool
        .begin()
        .await
        .map_err(|e| format!("aiModel.update: begin transaction failed: {e}"))?;

    if is_default == 1 {
        // 更新成默认项时，同样先清除其他记录的默认标记。
        sqlx::query("UPDATE ai_model SET is_default = 0 WHERE deleted = 0 AND id != ?1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("aiModel.update: clear default failed: {e}"))?;
    }

    let existing_api_key: Option<String> = match sqlx::query("SELECT api_key FROM ai_model WHERE id = ?1 AND deleted = 0 LIMIT 1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| format!("aiModel.update: query existing api key failed: {e}"))?
    {
        Some(row) => row
            .try_get("api_key")
            .map_err(|e| format!("aiModel.update: read existing api key failed: {e}"))?,
        None => None,
    };

    let next_api_key = if clear_api_key == 1 {
        None
    } else if api_key.is_some() {
        api_key
    } else {
        match existing_api_key {
            Some(existing) if !existing.trim().is_empty() && !existing.starts_with(AI_MODEL_API_KEY_ENCRYPTION_PREFIX) => {
                return Err("aiModel.update: API Key 格式不受支持，请重新配置".to_string());
            }
            other => other,
        }
    };

    let result = sqlx::query(
        "UPDATE ai_model \
         SET name = ?2, provider = ?3, base_url = ?4, api_key = ?5, description = ?6, \
             temperature = ?7, max_tokens = ?8, enabled = ?9, is_default = ?10 \
         WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .bind(name)
    .bind(provider)
    .bind(base_url)
    .bind(next_api_key)
    .bind(description)
    .bind(temperature)
    .bind(max_tokens)
    .bind(enabled)
    .bind(is_default)
    .execute(&mut *tx)
    .await;

    match result {
        Ok(result) => {
            tx.commit().await.map_err(|e| format!("aiModel.update: commit failed: {e}"))?;
            Ok(ok(Value::Bool(result.rows_affected() > 0)))
        }
        Err(e) => Ok(err_with_detail(500, "更新 AI 模型失败，请稍后重试", &e.to_string())),
    }
}

/// 批量删除 AI 模型。
///
/// 这里采用逻辑删除，并同步清空默认标记，避免被删记录继续参与默认选择。
pub(crate) async fn batch_del(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let ids: Vec<String> = serde_json::from_value(body).map_err(|e| format!("aiModel.batchDel: invalid body: {e}"))?;
    if ids.is_empty() {
        return Ok(ok(Value::Bool(true)));
    }

    let parsed_ids = match parse_i64_list_param(&ids, "id") {
        Ok(parsed_ids) => parsed_ids,
        Err(response) => return Ok(response),
    };

    let mut query = String::from("UPDATE ai_model SET deleted = 1, is_default = 0 WHERE deleted = 0 AND id IN (");
    for index in 0..parsed_ids.len() {
        if index > 0 {
            query.push(',');
        }
        query.push('?');
        query.push_str(&(index + 1).to_string());
    }
    query.push(')');

    let mut exec = sqlx::query(&query);
    for id in parsed_ids {
        exec = exec.bind(id);
    }

    match exec.execute(&db.pool).await {
        Ok(result) => Ok(ok(Value::Bool(result.rows_affected() > 0))),
        Err(e) => Ok(err_with_detail(500, "删除 AI 模型失败，请稍后重试", &e.to_string())),
    }
}
