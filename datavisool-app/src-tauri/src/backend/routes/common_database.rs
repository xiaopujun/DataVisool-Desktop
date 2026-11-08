/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 公共数据库数据源接口。
//!
//! 负责数据库连接配置的 CRUD、复制、连接测试和 SQL 执行。
//! 密码读取与配置写入类接口使用前后端加密协议，避免敏感信息以明文跨桥传输。

use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use sqlx::Row;

use crate::backend::crypto::{decrypt_request, encrypt_response, EncryptResult};
use crate::backend::types::{err, err_with_detail, ok, parse_i64_list_param, parse_i64_param, parse_required_i64_param, HttpResponse};
use crate::db::DbState;

mod extended;
mod policy;
mod query;
#[cfg(test)]
mod driver_tests;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 公共数据库分页查询参数。
struct PageParam {
    current: i64,
    size: i64,
    search_value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 公共数据库连接配置请求体。
struct DataSource {
    id: Option<String>,
    name: Option<String>,
    r#type: Option<String>,
    username: Option<String>,
    password: Option<String>,
    ip: Option<String>,
    port: Option<StringOrNumber>,
    database_name: Option<String>,
    url: Option<String>,
    des: Option<String>,
    allow_sql_script: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
/// 兼容前端把端口传成字符串或数字的历史情况。
/// TODO(legacy-compat): 统一前后端端口类型后，移除旧输入类型兼容。
enum StringOrNumber {
    Str(String),
    Num(i64),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 获取敏感数据库配置时的加密请求。
struct GetReq {
    id: String,
    key: String,
}

/// 分页查询公共数据库连接。
///
/// 列表接口不返回密码，只返回连接概要和执行权限。
pub(crate) async fn page_list(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let page: PageParam = serde_json::from_value(body).map_err(|e| format!("commonDatabase.pageList: invalid body: {e}"))?;
    let current = if page.current <= 0 { 1 } else { page.current };
    let size = if page.size <= 0 { 15 } else { page.size };
    let offset = (current - 1) * size;

    let total_row = sqlx::query("SELECT COUNT(*) AS cnt FROM common_database WHERE deleted = 0 AND (?1 IS NULL OR name LIKE '%' || ?1 || '%' OR type LIKE '%' || ?1 || '%' OR des LIKE '%' || ?1 || '%')")
        .bind(page.search_value.clone())
        .fetch_one(&db.pool)
        .await
        .map_err(|e| format!("commonDatabase.pageList: total query failed: {e}"))?;
    let total: i64 = total_row
        .try_get("cnt")
        .map_err(|e| format!("commonDatabase.pageList: read total failed: {e}"))?;

    let rows = sqlx::query(
        "SELECT id, name, type, ip, port, database_name, username, des, allow_sql_script, create_time, update_time \
         FROM common_database \
         WHERE deleted = 0 AND (?1 IS NULL OR name LIKE '%' || ?1 || '%' OR type LIKE '%' || ?1 || '%' OR des LIKE '%' || ?1 || '%') \
         ORDER BY update_time DESC \
         LIMIT ?2 OFFSET ?3",
    )
    .bind(page.search_value)
    .bind(size)
    .bind(offset)
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("commonDatabase.pageList: list query failed: {e}"))?;

    let mut records: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("commonDatabase.pageList: read id failed: {e}"))?;
        let name: Option<String> = row
            .try_get("name")
            .map_err(|e| format!("commonDatabase.pageList: read name failed: {e}"))?;
        let ty: String = row
            .try_get("type")
            .map_err(|e| format!("commonDatabase.pageList: read type failed: {e}"))?;
        let ip: String = row
            .try_get("ip")
            .map_err(|e| format!("commonDatabase.pageList: read ip failed: {e}"))?;
        let port: String = row
            .try_get("port")
            .map_err(|e| format!("commonDatabase.pageList: read port failed: {e}"))?;
        let database_name: String = row
            .try_get("database_name")
            .map_err(|e| format!("commonDatabase.pageList: read database_name failed: {e}"))?;
        let username: String = row
            .try_get("username")
            .map_err(|e| format!("commonDatabase.pageList: read username failed: {e}"))?;
        let des: Option<String> = row
            .try_get("des")
            .map_err(|e| format!("commonDatabase.pageList: read des failed: {e}"))?;
        let allow_sql_script: i64 = row
            .try_get("allow_sql_script")
            .map_err(|e| format!("commonDatabase.pageList: read allow_sql_script failed: {e}"))?;
        let create_time: String = row
            .try_get("create_time")
            .map_err(|e| format!("commonDatabase.pageList: read create_time failed: {e}"))?;
        let update_time: String = row
            .try_get("update_time")
            .map_err(|e| format!("commonDatabase.pageList: read update_time failed: {e}"))?;
        records.push(serde_json::json!({
            "id": id.to_string(),
            "name": name.unwrap_or_default(),
            "type": ty,
            "ip": ip,
            "port": port,
            "databaseName": database_name,
            "username": username,
            "des": des,
            "allowSqlScript": allow_sql_script != 0,
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

/// 查询所有可选公共数据库连接。
pub(crate) async fn list(db: &DbState) -> Result<HttpResponse, String> {
    let rows = sqlx::query("SELECT id, name, type, allow_sql_script FROM common_database WHERE deleted = 0 ORDER BY update_time DESC")
        .fetch_all(&db.pool)
        .await
        .map_err(|e| format!("commonDatabase.list: query failed: {e}"))?;

    let mut items: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("commonDatabase.list: read id failed: {e}"))?;
        let name: Option<String> = row
            .try_get("name")
            .map_err(|e| format!("commonDatabase.list: read name failed: {e}"))?;
        let ty: String = row
            .try_get("type")
            .map_err(|e| format!("commonDatabase.list: read type failed: {e}"))?;
        let allow_sql_script: i64 = row
            .try_get("allow_sql_script")
            .map_err(|e| format!("commonDatabase.list: read allow_sql_script failed: {e}"))?;
        items.push(serde_json::json!({
            "id": id.to_string(),
            "name": name.unwrap_or_default(),
            "type": ty,
            "allowSqlScript": allow_sql_script != 0
        }));
    }
    Ok(ok(Value::Array(items)))
}

/// 获取单个公共数据库连接详情。
///
/// 密码字段会通过前端传入的公钥重新加密后返回。
pub(crate) async fn get(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let req: GetReq = serde_json::from_value(body).map_err(|e| format!("commonDatabase.get: invalid body: {e}"))?;
    let id = match parse_i64_param(&req.id, "id") {
        Ok(id) => id,
        Err(response) => return Ok(response),
    };

    let row = sqlx::query(
        "SELECT id, name, type, ip, port, database_name, username, password, url, url_is_custom, des, allow_sql_script \
         FROM common_database WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .fetch_optional(&db.pool)
    .await
    .map_err(|e| format!("commonDatabase.get: query failed: {e}"))?;

    let Some(row) = row else {
        return Ok(ok(Value::Null));
    };

    let id: i64 = row
        .try_get("id")
        .map_err(|e| format!("commonDatabase.get: read id failed: {e}"))?;
    let name: Option<String> = row
        .try_get("name")
        .map_err(|e| format!("commonDatabase.get: read name failed: {e}"))?;
    let ty: String = row
        .try_get("type")
        .map_err(|e| format!("commonDatabase.get: read type failed: {e}"))?;
    let ip: String = row
        .try_get("ip")
        .map_err(|e| format!("commonDatabase.get: read ip failed: {e}"))?;
    let port: String = row
        .try_get("port")
        .map_err(|e| format!("commonDatabase.get: read port failed: {e}"))?;
    let database_name: String = row
        .try_get("database_name")
        .map_err(|e| format!("commonDatabase.get: read database_name failed: {e}"))?;
    let username: String = row
        .try_get("username")
        .map_err(|e| format!("commonDatabase.get: read username failed: {e}"))?;
    let password: String = row
        .try_get("password")
        .map_err(|e| format!("commonDatabase.get: read password failed: {e}"))?;
    let url: Option<String> = row
        .try_get("url")
        .map_err(|e| format!("commonDatabase.get: read url failed: {e}"))?;
    let des: Option<String> = row
        .try_get("des")
        .map_err(|e| format!("commonDatabase.get: read des failed: {e}"))?;
    let allow_sql_script: i64 = row
        .try_get("allow_sql_script")
        .map_err(|e| format!("commonDatabase.get: read allow_sql_script failed: {e}"))?;

    let custom_flag: Option<i64> = row.try_get("url_is_custom").map_err(|e| format!("commonDatabase.get: read url_is_custom failed: {e}"))?;
    let url = if custom_flag.is_some() {
        url.unwrap_or_default()
    } else {
        policy::custom_url(url.as_deref(), &ty, &ip, &port, &database_name, &username, &password)
    };

    // 敏感字段只放进加密响应里，避免 Tauri bridge 上直接暴露明文密码。
    let plain = serde_json::json!({
        "id": id.to_string(),
        "name": name.unwrap_or_default(),
        "type": ty,
        "ip": ip,
        "port": port,
        "databaseName": database_name,
        "username": username,
        "password": password,
        "url": url,
        "des": des,
        "allowSqlScript": allow_sql_script != 0
    })
    .to_string();

    let enc = encrypt_response(&req.key, &plain)?;
    Ok(ok(serde_json::to_value(enc).map_err(|e| format!("commonDatabase.get: serialize encrypt result failed: {e}"))?))
}

/// 新增公共数据库连接。
pub(crate) async fn add(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let enc: EncryptResult = serde_json::from_value(body).map_err(|e| format!("commonDatabase.add: invalid body: {e}"))?;
    let plain = decrypt_request(enc)?;
    let ds: DataSource = serde_json::from_str(&plain).map_err(|e| format!("commonDatabase.add: decrypt ok but json invalid: {e}"))?;
    if let Some(message) = policy::validate(&ds) {
        return Ok(err(400, message));
    }

    let ty = ds.r#type.unwrap_or_else(|| "MySQL".to_string());
    let ip = ds.ip.unwrap_or_default();
    let port = string_or_number_to_string(ds.port);
    let database_name = ds.database_name.unwrap_or_default();
    let username = ds.username.unwrap_or_default();
    let password = ds.password.unwrap_or_default();
    let allow_sql_script = ds.allow_sql_script.unwrap_or(false);
    let url = ds.url.unwrap_or_default().trim().to_string();
    let url_is_custom = i64::from(!url.is_empty());

    let result = sqlx::query(
        "INSERT INTO common_database (name, type, ip, port, database_name, username, password, url, des, allow_sql_script, url_is_custom, deleted) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
    )
    .bind(ds.name.unwrap_or_default())
    .bind(ty)
    .bind(ip)
    .bind(port)
    .bind(database_name)
    .bind(username)
    .bind(password)
    .bind(url)
    .bind(ds.des)
    .bind(bool_to_db_i64(allow_sql_script))
    .bind(url_is_custom)
    .execute(&db.pool)
    .await
    .map_err(|e| format!("commonDatabase.add: insert failed: {e}"))?;

    Ok(ok(Value::String(result.last_insert_rowid().to_string())))
}

/// 更新公共数据库连接。
///
/// 更新时会补齐历史数据缺失的 ref_key，供项目导入导出稳定引用。
/// TODO(legacy-compat): 确认现有记录均有 ref_key 后，移除历史空值补齐分支。
pub(crate) async fn update(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let enc: EncryptResult = serde_json::from_value(body).map_err(|e| format!("commonDatabase.update: invalid body: {e}"))?;
    let plain = decrypt_request(enc)?;
    let ds: DataSource = serde_json::from_str(&plain).map_err(|e| format!("commonDatabase.update: decrypt ok but json invalid: {e}"))?;
    if let Some(message) = policy::validate(&ds) {
        return Ok(err(400, message));
    }

    let id = match parse_required_i64_param(ds.id.as_deref(), "id") {
        Ok(id) => id,
        Err(response) => return Ok(response),
    };

    let ty = ds.r#type.unwrap_or_else(|| "MySQL".to_string());
    let ip = ds.ip.unwrap_or_default();
    let port = string_or_number_to_string(ds.port);
    let database_name = ds.database_name.unwrap_or_default();
    let username = ds.username.unwrap_or_default();
    let password = ds.password.unwrap_or_default();
    let allow_sql_script = ds.allow_sql_script.unwrap_or(false);
    let url = ds.url.unwrap_or_default().trim().to_string();
    let url_is_custom = i64::from(!url.is_empty());

    let changed = sqlx::query(
        "UPDATE common_database SET \
           name = ?2, \
           type = ?3, \
           ip = ?4, \
           port = ?5, \
           database_name = ?6, \
           username = ?7, \
           password = ?8, \
           url = ?9, \
           des = ?10, \
           allow_sql_script = ?11, \
           url_is_custom = ?12, \
           ref_key = COALESCE(NULLIF(ref_key, ''), lower(hex(randomblob(16)))) \
         WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .bind(ds.name.unwrap_or_default())
    .bind(ty)
    .bind(ip)
    .bind(port)
    .bind(database_name)
    .bind(username)
    .bind(password)
    .bind(url)
    .bind(ds.des)
    .bind(bool_to_db_i64(allow_sql_script))
    .bind(url_is_custom)
    .execute(&db.pool)
    .await
    .map_err(|e| format!("commonDatabase.update: update failed: {e}"))?
    .rows_affected();

    Ok(ok(Value::Bool(changed > 0)))
}

/// 批量逻辑删除公共数据库连接。
pub(crate) async fn batch_del(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let ids: Vec<String> = serde_json::from_value(body).map_err(|e| format!("commonDatabase.batchDel: invalid body: {e}"))?;
    if ids.is_empty() {
        return Ok(ok(Value::Bool(true)));
    }
    let parsed = match parse_i64_list_param(&ids, "id") {
        Ok(parsed) => parsed,
        Err(response) => return Ok(response),
    };

    let mut query = String::from("UPDATE common_database SET deleted = 1 WHERE deleted = 0 AND id IN (");
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
        .map_err(|e| format!("commonDatabase.batchDel: delete failed: {e}"))?
        .rows_affected();
    Ok(ok(Value::Bool(changed > 0)))
}

/// 复制公共数据库连接。
///
/// 复制会保留连接参数但创建新记录，不复用原记录 id/ref_key。
pub(crate) async fn copy(db: &DbState, id: i64) -> Result<HttpResponse, String> {
    let row = sqlx::query(
        "SELECT name, type, ip, port, database_name, username, password, url, url_is_custom, des, allow_sql_script \
         FROM common_database WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .fetch_optional(&db.pool)
    .await
    .map_err(|e| format!("commonDatabase.copy: query source failed: {e}"))?;

    let Some(row) = row else {
        return Ok(ok(Value::Bool(false)));
    };

    let name: Option<String> = row
        .try_get("name")
        .map_err(|e| format!("commonDatabase.copy: read name failed: {e}"))?;
    let ty: String = row
        .try_get("type")
        .map_err(|e| format!("commonDatabase.copy: read type failed: {e}"))?;
    let ip: String = row
        .try_get("ip")
        .map_err(|e| format!("commonDatabase.copy: read ip failed: {e}"))?;
    let port: String = row
        .try_get("port")
        .map_err(|e| format!("commonDatabase.copy: read port failed: {e}"))?;
    let database_name: String = row
        .try_get("database_name")
        .map_err(|e| format!("commonDatabase.copy: read database_name failed: {e}"))?;
    let username: String = row
        .try_get("username")
        .map_err(|e| format!("commonDatabase.copy: read username failed: {e}"))?;
    let password: String = row
        .try_get("password")
        .map_err(|e| format!("commonDatabase.copy: read password failed: {e}"))?;
    let url: Option<String> = row
        .try_get("url")
        .map_err(|e| format!("commonDatabase.copy: read url failed: {e}"))?;
    let des: Option<String> = row
        .try_get("des")
        .map_err(|e| format!("commonDatabase.copy: read des failed: {e}"))?;
    let allow_sql_script: i64 = row
        .try_get("allow_sql_script")
        .map_err(|e| format!("commonDatabase.copy: read allow_sql_script failed: {e}"))?;

    let url_is_custom: Option<i64> = row.try_get("url_is_custom").map_err(|e| format!("commonDatabase.copy: read url_is_custom failed: {e}"))?;

    let result = sqlx::query(
        "INSERT INTO common_database (name, type, ip, port, database_name, username, password, url, des, allow_sql_script, url_is_custom, deleted) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
    )
    .bind(format!("{} - 副本", name.unwrap_or_default()))
    .bind(ty)
    .bind(ip)
    .bind(port)
    .bind(database_name)
    .bind(username)
    .bind(password)
    .bind(url.unwrap_or_default())
    .bind(des)
    .bind(allow_sql_script)
    .bind(url_is_custom)
    .execute(&db.pool)
    .await
    .map_err(|e| format!("commonDatabase.copy: insert failed: {e}"))?;

    Ok(ok(Value::String(result.last_insert_rowid().to_string())))
}

/// 测试公共数据库连接是否可用。
pub(crate) async fn test(db: &DbState, id: i64) -> Result<HttpResponse, String> {
    let row = sqlx::query(
        "SELECT type, ip, port, database_name, username, password, url \
         FROM common_database WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .fetch_optional(&db.pool)
    .await
    .map_err(|e| format!("commonDatabase.test: query dataSource failed: {e}"))?;

    let Some(row) = row else {
        return Ok(err(404, "数据源不存在"));
    };

    let ty: String = row
        .try_get("type")
        .map_err(|e| format!("commonDatabase.test: read type failed: {e}"))?;
    let ip: String = row
        .try_get("ip")
        .map_err(|e| format!("commonDatabase.test: read ip failed: {e}"))?;
    let port: String = row
        .try_get("port")
        .map_err(|e| format!("commonDatabase.test: read port failed: {e}"))?;
    let database_name: String = row
        .try_get("database_name")
        .map_err(|e| format!("commonDatabase.test: read database_name failed: {e}"))?;
    let username: String = row
        .try_get("username")
        .map_err(|e| format!("commonDatabase.test: read username failed: {e}"))?;
    let password: String = row
        .try_get("password")
        .map_err(|e| format!("commonDatabase.test: read password failed: {e}"))?;
    let url: Option<String> = row
        .try_get("url")
        .map_err(|e| format!("commonDatabase.test: read url failed: {e}"))?;
    if datavisool_datasource::supports(&ty) {
        let config = datavisool_datasource::ConnectionConfig {
            host: ip, port, database: database_name, username, password, url,
        };
        return Ok(extended::test(&ty, config).await);
    }
    let url = build_url(url, &ty, &ip, &port, &database_name, &username, &password);

    if ty.eq_ignore_ascii_case("MySQL") {
        let pool = match sqlx::mysql::MySqlPoolOptions::new().max_connections(1).connect(&url).await {
            Ok(p) => p,
            Err(e) => {
                return Ok(err_with_detail(500, "连接数据库失败，请检查 IP/端口/账号密码/白名单", &e.to_string()));
            }
        };

        if let Err(e) = sqlx::query("SELECT 1").execute(&pool).await {
            return Ok(err_with_detail(500, "数据库连通性校验失败，请检查账号权限与网络", &e.to_string()));
        }

        return Ok(ok(Value::Bool(true)));
    }
    if ty.eq_ignore_ascii_case("PostgresSQL") || ty.eq_ignore_ascii_case("PostgreSQL") {
        let pool = match sqlx::postgres::PgPoolOptions::new().max_connections(1).connect(&url).await {
            Ok(p) => p,
            Err(e) => {
                return Ok(err_with_detail(500, "连接数据库失败，请检查 IP/端口/账号密码/白名单", &e.to_string()));
            }
        };

        if let Err(e) = sqlx::query("SELECT 1").execute(&pool).await {
            return Ok(err_with_detail(500, "数据库连通性校验失败，请检查账号权限与网络", &e.to_string()));
        }
        return Ok(ok(Value::Bool(true)));
    }

    Ok(err(400, format!("不支持的数据源类型：{ty}")))
}

/// 执行公共数据库 SQL。
///
/// 未开启脚本执行时只允许 SELECT/WITH 查询语句。
pub(crate) async fn executor_execute(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    #[derive(Debug, Deserialize)]
    struct ExecReq {
        id: String,
        statement: String,
    }
    let req: ExecReq = serde_json::from_value(body).map_err(|e| format!("db.executor.execute: invalid body: {e}"))?;
    let id = match parse_i64_param(&req.id, "id") {
        Ok(id) => id,
        Err(response) => return Ok(response),
    };

    let sql = base64_decode_utf8(&req.statement)?;

    let row = sqlx::query(
        "SELECT type, ip, port, database_name, username, password, url, allow_sql_script \
         FROM common_database WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .fetch_optional(&db.pool)
    .await
    .map_err(|e| format!("db.executor.execute: query dataSource failed: {e}"))?;

    let Some(row) = row else {
        return Ok(err(404, "数据源不存在"));
    };

    let ty: String = row
        .try_get("type")
        .map_err(|e| format!("db.executor.execute: read type failed: {e}"))?;
    let ip: String = row
        .try_get("ip")
        .map_err(|e| format!("db.executor.execute: read ip failed: {e}"))?;
    let port: String = row
        .try_get("port")
        .map_err(|e| format!("db.executor.execute: read port failed: {e}"))?;
    let database_name: String = row
        .try_get("database_name")
        .map_err(|e| format!("db.executor.execute: read database_name failed: {e}"))?;
    let username: String = row
        .try_get("username")
        .map_err(|e| format!("db.executor.execute: read username failed: {e}"))?;
    let password: String = row
        .try_get("password")
        .map_err(|e| format!("db.executor.execute: read password failed: {e}"))?;
    let url: Option<String> = row
        .try_get("url")
        .map_err(|e| format!("db.executor.execute: read url failed: {e}"))?;
    let allow_sql_script: i64 = row
        .try_get("allow_sql_script")
        .map_err(|e| format!("db.executor.execute: read allow_sql_script failed: {e}"))?;
    let allow_sql_script = allow_sql_script != 0;
    // 默认只允许只读查询，避免普通数据源被用作写库入口。
    let is_readonly = if datavisool_datasource::supports(&ty) {
        datavisool_datasource::is_readonly_query(&ty, &sql)
    } else {
        is_readonly_sql_statement(&sql)
    };
    if !allow_sql_script && !is_readonly {
        return Ok(err(400, "仅允许执行 SELECT/WITH 查询语句"));
    }
    if datavisool_datasource::supports(&ty) {
        let config = datavisool_datasource::ConnectionConfig {
            host: ip, port, database: database_name, username, password, url,
        };
        return Ok(extended::query(&ty, config, sql).await);
    }
    let url = build_url(url, &ty, &ip, &port, &database_name, &username, &password);

    if ty.eq_ignore_ascii_case("MySQL") {
        let pool = match sqlx::mysql::MySqlPoolOptions::new().max_connections(1).connect(&url).await {
            Ok(p) => p,
            Err(e) => {
                return Ok(err_with_detail(500, "连接数据库失败，请检查 IP/端口/账号密码/白名单", &e.to_string()));
            }
        };
        let result = match query::mysql(&pool, &sql, allow_sql_script).await {
            Ok(value) => value,
            Err(e) => return Ok(err_with_detail(500, sql_execution_error_message(allow_sql_script), &e.to_string())),
        };
        return Ok(ok(result));
    }
    if ty.eq_ignore_ascii_case("PostgresSQL") || ty.eq_ignore_ascii_case("PostgreSQL") {
        let pool = match sqlx::postgres::PgPoolOptions::new().max_connections(1).connect(&url).await {
            Ok(p) => p,
            Err(e) => {
                return Ok(err_with_detail(500, "连接数据库失败，请检查 IP/端口/账号密码/白名单", &e.to_string()));
            }
        };
        let result = match query::postgres(&pool, &sql, allow_sql_script).await {
            Ok(value) => value,
            Err(e) => return Ok(err_with_detail(500, sql_execution_error_message(allow_sql_script), &e.to_string())),
        };
        return Ok(ok(result));
    }

    Ok(err(400, format!("不支持的数据源类型：{ty}")))
}

/// 根据数据源配置拼接 SQLx 连接 URL。
fn build_url(url: Option<String>, ty: &str, ip: &str, port: &str, database_name: &str, username: &str, password: &str) -> String {
    let custom = policy::custom_url(url.as_deref(), ty, ip, port, database_name, username, password);
    if !custom.is_empty() {
        return custom;
    }
    policy::generated_url(ty, ip, port, database_name, username, password)
}

/// 解码 base64 字符串并按 UTF-8 返回。
fn base64_decode_utf8(input: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| format!("db.executor.execute: statement base64 decode failed: {e}"))?;
    String::from_utf8(bytes).map_err(|e| format!("db.executor.execute: statement utf8 decode failed: {e}"))
}

/// 判断 SQL 是否属于只读语句。
fn is_readonly_sql_statement(sql: &str) -> bool {
    let lower = sql.trim_start().to_lowercase();
    lower.starts_with("select") || lower.starts_with("with")
}

/// 根据权限配置返回 SQL 执行错误提示。
fn sql_execution_error_message(allow_sql_script: bool) -> &'static str {
    if allow_sql_script {
        "执行 SQL 失败，请检查 SQL 语法与权限"
    } else {
        "执行 SQL 失败，请检查 SQL 语法与权限（仅支持 SELECT/WITH）"
    }
}

/// 将布尔配置转换成 SQLite 中使用的 0/1。
fn bool_to_db_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

/// 将兼容的端口输入转换成字符串。
fn string_or_number_to_string(input: Option<StringOrNumber>) -> String {
    match input {
        Some(StringOrNumber::Str(s)) => s,
        Some(StringOrNumber::Num(n)) => n.to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::crypto::{encrypt_response, server_public_key_pem};
    use crate::db::DbState;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn build_test_db() -> DbState {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory failed");

        datavisool_db::run_base_migrations(&pool).await.expect("run base migrations failed");
        datavisool_db::run_custom_migrations(&pool, sqlx::migrate!("./migrations_desktop"))
            .await
            .expect("run desktop migrations failed");

        DbState { pool }
    }

    fn encrypt_request_body(plain: Value) -> Value {
        let public_key_b64 = server_public_key_pem()
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect::<String>();
        serde_json::to_value(encrypt_response(&public_key_b64, &plain.to_string()).expect("encrypt request body failed"))
            .expect("serialize encrypted request failed")
    }

    #[tokio::test]
    async fn common_database_add_populates_ref_key() {
        let db = build_test_db().await;

        let response = add(
            &db,
            encrypt_request_body(serde_json::json!({
                "name": "主数据库",
                "type": "MySQL",
                "ip": "127.0.0.1",
                "port": "3306",
                "databaseName": "demo",
                "username": "root",
                "password": "secret",
                "url": "",
                "des": "说明",
                "allowSqlScript": true
            })),
        )
        .await
        .expect("add common_database failed");

        assert_eq!(response.code, 200);
        let id = response.data.as_str().expect("add response data should be id string");

        let row = sqlx::query("SELECT ref_key, allow_sql_script FROM common_database WHERE id = ?1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .expect("query common_database failed");
        let ref_key: Option<String> = row.try_get("ref_key").expect("read ref_key failed");
        let allow_sql_script: i64 = row.try_get("allow_sql_script").expect("read allow_sql_script failed");
        assert!(ref_key.as_ref().is_some_and(|value| !value.trim().is_empty()));
        assert_eq!(allow_sql_script, 1);
    }

    #[tokio::test]
    async fn common_database_update_backfills_missing_ref_key() {
        let db = build_test_db().await;

        let inserted = sqlx::query(
            "INSERT INTO common_database (name, type, ip, port, database_name, username, password, url, des, deleted) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
        )
        .bind("旧数据库")
        .bind("MySQL")
        .bind("127.0.0.1")
        .bind("3306")
        .bind("legacy_db")
        .bind("root")
        .bind("secret")
        .bind("mysql://root:secret@127.0.0.1:3306/legacy_db")
        .bind("legacy")
        .execute(&db.pool)
        .await
        .expect("insert common_database failed");
        let id = inserted.last_insert_rowid();

        sqlx::query("UPDATE common_database SET ref_key = NULL WHERE id = ?1")
            .bind(id)
            .execute(&db.pool)
            .await
            .expect("clear ref_key failed");

        let response = update(
            &db,
            encrypt_request_body(serde_json::json!({
                "id": id.to_string(),
                "name": "更新数据库",
                "type": "PostgreSQL",
                "ip": "127.0.0.1",
                "port": "5432",
                "databaseName": "new_db",
                "username": "postgres",
                "password": "secret",
                "url": "",
                "des": "updated",
                "allowSqlScript": true
            })),
        )
        .await
        .expect("update common_database failed");

        assert_eq!(response.code, 200);
        assert_eq!(response.data, Value::Bool(true));

        let row = sqlx::query("SELECT ref_key, allow_sql_script FROM common_database WHERE id = ?1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .expect("query common_database failed");
        let ref_key: Option<String> = row.try_get("ref_key").expect("read ref_key failed");
        let allow_sql_script: i64 = row.try_get("allow_sql_script").expect("read allow_sql_script failed");
        assert!(ref_key.as_ref().is_some_and(|value| !value.trim().is_empty()));
        assert_eq!(allow_sql_script, 1);
    }
}

#[cfg(test)]
mod regression_tests;
