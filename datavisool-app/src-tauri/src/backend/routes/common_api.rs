/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 公共 API 管理接口。
//!
//! 负责维护项目可复用的 HTTP API 配置，包括分页、详情、新增、编辑、复制、
//! 逻辑删除和快速搜索。请求参数与响应结构保持前端既有协议。

use serde::Deserialize;
use serde_json::Value;
use sqlx::Row;

use crate::backend::types::{err, err_with_detail, ok, parse_i64_list_param, parse_required_i64_param, HttpResponse};
use crate::db::DbState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 公共 API 分页查询参数。
struct PageParam {
    current: i64,
    size: i64,
    search_value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 公共 API 新增/编辑请求体。
struct CommonApiItem {
    id: Option<String>,
    name: Option<String>,
    url: Option<String>,
    method: Option<String>,
    params: Option<String>,
    headers: Option<String>,
}

/// 分页查询公共 API。
///
/// 返回前端表格需要的 records/total/size/current 结构。
pub(crate) async fn page_list(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let page: PageParam = serde_json::from_value(body).map_err(|e| format!("commonApi.pageList: invalid body: {e}"))?;
    let current = if page.current <= 0 { 1 } else { page.current };
    let size = if page.size <= 0 { 15 } else { page.size };
    let offset = (current - 1) * size;

    // 分页查询拆成 total 和 records 两次 SQL，保持前端分页总数准确。
    let total_row = sqlx::query("SELECT COUNT(*) AS cnt FROM common_api WHERE deleted = 0 AND (?1 IS NULL OR name LIKE '%' || ?1 || '%')")
        .bind(page.search_value.clone())
        .fetch_one(&db.pool)
        .await
        .map_err(|e| format!("commonApi.pageList: total query failed: {e}"))?;
    let total: i64 = total_row
        .try_get("cnt")
        .map_err(|e| format!("commonApi.pageList: read total failed: {e}"))?;

    let rows = sqlx::query(
        "SELECT id, name, url, method, params, headers, create_time, update_time \
         FROM common_api \
         WHERE deleted = 0 AND (?1 IS NULL OR name LIKE '%' || ?1 || '%') \
         ORDER BY update_time DESC \
         LIMIT ?2 OFFSET ?3",
    )
    .bind(page.search_value)
    .bind(size)
    .bind(offset)
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("commonApi.pageList: list query failed: {e}"))?;

    let mut records: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("commonApi.pageList: read id failed: {e}"))?;
        let name: Option<String> = row
            .try_get("name")
            .map_err(|e| format!("commonApi.pageList: read name failed: {e}"))?;
        let url: String = row
            .try_get("url")
            .map_err(|e| format!("commonApi.pageList: read url failed: {e}"))?;
        let method: String = row
            .try_get("method")
            .map_err(|e| format!("commonApi.pageList: read method failed: {e}"))?;
        let params: Option<String> = row
            .try_get("params")
            .map_err(|e| format!("commonApi.pageList: read params failed: {e}"))?;
        let headers: Option<String> = row
            .try_get("headers")
            .map_err(|e| format!("commonApi.pageList: read headers failed: {e}"))?;
        let create_time: String = row
            .try_get("create_time")
            .map_err(|e| format!("commonApi.pageList: read create_time failed: {e}"))?;
        let update_time: String = row
            .try_get("update_time")
            .map_err(|e| format!("commonApi.pageList: read update_time failed: {e}"))?;

        records.push(serde_json::json!({
            "id": id.to_string(),
            "name": name.unwrap_or_default(),
            "url": url,
            "method": method,
            "params": params.unwrap_or_else(|| "{}".to_string()),
            "headers": headers.unwrap_or_else(|| "{}".to_string()),
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

/// 查询单个公共 API 详情。
pub(crate) async fn get_common_api(db: &DbState, id: i64) -> Result<HttpResponse, String> {
    let row = sqlx::query("SELECT id, name, url, method, params, headers FROM common_api WHERE id = ?1 AND deleted = 0")
        .bind(id)
        .fetch_optional(&db.pool)
        .await
        .map_err(|e| format!("commonApi.get: query failed: {e}"))?;

    let Some(row) = row else {
        return Ok(err(404, "公共API不存在"));
    };

    let id: i64 = row.try_get("id").map_err(|e| format!("commonApi.get: read id failed: {e}"))?;
    let name: Option<String> = row
        .try_get("name")
        .map_err(|e| format!("commonApi.get: read name failed: {e}"))?;
    let url: String = row.try_get("url").map_err(|e| format!("commonApi.get: read url failed: {e}"))?;
    let method: String = row
        .try_get("method")
        .map_err(|e| format!("commonApi.get: read method failed: {e}"))?;
    let params: Option<String> = row
        .try_get("params")
        .map_err(|e| format!("commonApi.get: read params failed: {e}"))?;
    let headers: Option<String> = row
        .try_get("headers")
        .map_err(|e| format!("commonApi.get: read headers failed: {e}"))?;

    Ok(ok(serde_json::json!({
        "id": id.to_string(),
        "name": name.unwrap_or_default(),
        "url": url,
        "method": method,
        "params": params.unwrap_or_else(|| "{}".to_string()),
        "headers": headers.unwrap_or_else(|| "{}".to_string())
    })))
}

/// 新增公共 API。
///
/// url 和 method 是执行公共 API 时的最小必要字段，因此在入库前做非空校验。
pub(crate) async fn create(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let req: CommonApiItem = serde_json::from_value(body).map_err(|e| format!("commonApi.create: invalid body: {e}"))?;

    let url = req.url.unwrap_or_default().trim().to_string();
    let method = req.method.unwrap_or_else(|| "GET".to_string()).trim().to_string();
    if url.is_empty() {
        return Ok(err(400, "请求参数错误：url 不能为空"));
    }
    if method.is_empty() {
        return Ok(err(400, "请求参数错误：method 不能为空"));
    }

    let name = req.name.unwrap_or_default();
    // params/headers 以 JSON 字符串形式存储，空值默认成空对象。
    let params = req.params.unwrap_or_else(|| "{}".to_string());
    let headers = req.headers.unwrap_or_else(|| "{}".to_string());

    let result = sqlx::query("INSERT INTO common_api (name, url, method, params, headers, deleted) VALUES (?1, ?2, ?3, ?4, ?5, 0)")
        .bind(name)
        .bind(url)
        .bind(method)
        .bind(params)
        .bind(headers)
        .execute(&db.pool)
        .await;

    match result {
        Ok(r) => Ok(ok(Value::String(r.last_insert_rowid().to_string()))),
        Err(e) => Ok(err_with_detail(500, "新增公共API失败，请稍后重试", &e.to_string())),
    }
}

/// 更新公共 API。
///
/// 更新时会顺手补齐历史数据缺失的 ref_key，保证导入导出能稳定按引用键匹配。
pub(crate) async fn update(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let req: CommonApiItem = serde_json::from_value(body).map_err(|e| format!("commonApi.update: invalid body: {e}"))?;
    let id = match parse_required_i64_param(req.id.as_deref(), "id") {
        Ok(id) => id,
        Err(response) => return Ok(response),
    };

    let url = req.url.unwrap_or_default().trim().to_string();
    let method = req.method.unwrap_or_else(|| "GET".to_string()).trim().to_string();
    if url.is_empty() {
        return Ok(err(400, "请求参数错误：url 不能为空"));
    }
    if method.is_empty() {
        return Ok(err(400, "请求参数错误：method 不能为空"));
    }

    let name = req.name.unwrap_or_default();
    let params = req.params.unwrap_or_else(|| "{}".to_string());
    let headers = req.headers.unwrap_or_else(|| "{}".to_string());

    // ref_key 是跨项目导入导出引用公共 API 的稳定键，历史空值在编辑时自动补齐。
    // TODO(legacy-compat): 确认现有记录均有 ref_key 后，移除历史空值补齐分支。
    let changed = sqlx::query(
        "UPDATE common_api \
         SET name = ?2, url = ?3, method = ?4, params = ?5, headers = ?6, \
             ref_key = COALESCE(NULLIF(ref_key, ''), lower(hex(randomblob(16)))) \
         WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .bind(name)
    .bind(url)
    .bind(method)
    .bind(params)
    .bind(headers)
    .execute(&db.pool)
    .await;

    match changed {
        Ok(r) if r.rows_affected() == 0 => Ok(err(404, "公共API不存在")),
        Ok(_) => Ok(ok(Value::Bool(true))),
        Err(e) => Ok(err_with_detail(500, "更新公共API失败，请稍后重试", &e.to_string())),
    }
}

/// 批量逻辑删除公共 API。
pub(crate) async fn batch_del(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let ids: Vec<String> = serde_json::from_value(body).map_err(|e| format!("commonApi.batchDel: invalid body: {e}"))?;
    if ids.is_empty() {
        return Ok(ok(Value::Bool(true)));
    }
    let parsed = match parse_i64_list_param(&ids, "id") {
        Ok(parsed) => parsed,
        Err(response) => return Ok(response),
    };

    // SQLite 不支持直接绑定 Vec，这里按 ID 数量构造占位符列表。
    let mut query = String::from("UPDATE common_api SET deleted = 1 WHERE deleted = 0 AND id IN (");
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

    let changed = q.execute(&db.pool).await;
    match changed {
        Ok(r) => Ok(ok(Value::Bool(r.rows_affected() > 0))),
        Err(e) => Ok(err_with_detail(500, "删除公共API失败，请稍后重试", &e.to_string())),
    }
}

/// 复制公共 API。
///
/// 复制只克隆业务配置，不复用原记录 id/ref_key。
pub(crate) async fn copy(db: &DbState, id: i64) -> Result<HttpResponse, String> {
    let row = sqlx::query("SELECT name, url, method, params, headers FROM common_api WHERE id = ?1 AND deleted = 0")
        .bind(id)
        .fetch_optional(&db.pool)
        .await
        .map_err(|e| format!("commonApi.copy: query source failed: {e}"))?;

    let Some(row) = row else {
        return Ok(err(404, "公共API不存在"));
    };

    let name: Option<String> = row
        .try_get("name")
        .map_err(|e| format!("commonApi.copy: read name failed: {e}"))?;
    let url: String = row
        .try_get("url")
        .map_err(|e| format!("commonApi.copy: read url failed: {e}"))?;
    let method: String = row
        .try_get("method")
        .map_err(|e| format!("commonApi.copy: read method failed: {e}"))?;
    let params: Option<String> = row
        .try_get("params")
        .map_err(|e| format!("commonApi.copy: read params failed: {e}"))?;
    let headers: Option<String> = row
        .try_get("headers")
        .map_err(|e| format!("commonApi.copy: read headers failed: {e}"))?;

    let name = format!("{} - 副本", name.unwrap_or_default());
    let params = params.unwrap_or_else(|| "{}".to_string());
    let headers = headers.unwrap_or_else(|| "{}".to_string());

    let result = sqlx::query("INSERT INTO common_api (name, url, method, params, headers, deleted) VALUES (?1, ?2, ?3, ?4, ?5, 0)")
        .bind(name)
        .bind(url)
        .bind(method)
        .bind(params)
        .bind(headers)
        .execute(&db.pool)
        .await;

    match result {
        Ok(r) => Ok(ok(Value::String(r.last_insert_rowid().to_string()))),
        Err(e) => Ok(err_with_detail(500, "复制公共API失败，请稍后重试", &e.to_string())),
    }
}

/// 公共 API 快速搜索。
///
/// 用于选择器等轻量场景，只返回 id/name，并限制最多 50 条。
pub(crate) async fn quick_search(db: &DbState, full_url: &str) -> Result<HttpResponse, String> {
    let keyword = get_query_value(full_url, "search").unwrap_or_default().trim().to_string();
    let keyword = if keyword.is_empty() { None } else { Some(keyword) };

    let rows = sqlx::query(
        "SELECT id, name FROM common_api WHERE deleted = 0 AND (?1 IS NULL OR name LIKE '%' || ?1 || '%') \
         ORDER BY update_time DESC LIMIT 50",
    )
    .bind(keyword)
    .fetch_all(&db.pool)
    .await;

    match rows {
        Ok(rows) => {
            let mut items: Vec<Value> = Vec::with_capacity(rows.len());
            for row in rows {
                let id: i64 = row
                    .try_get("id")
                    .map_err(|e| format!("commonApi.quickSearch: read id failed: {e}"))?;
                let name: Option<String> = row
                    .try_get("name")
                    .map_err(|e| format!("commonApi.quickSearch: read name failed: {e}"))?;
                items.push(serde_json::json!({
                    "id": id.to_string(),
                    "name": name.unwrap_or_default()
                }));
            }
            Ok(ok(Value::Array(items)))
        }
        Err(e) => Ok(err_with_detail(500, "查询公共API失败，请稍后重试", &e.to_string())),
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

#[cfg(test)]
mod tests {
    use super::*;
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

    #[tokio::test]
    async fn common_api_create_populates_ref_key() {
        let db = build_test_db().await;

        let response = create(
            &db,
            serde_json::json!({
                "name": "天气接口",
                "url": "https://example.com/weather",
                "method": "GET",
                "params": "{}",
                "headers": "{}"
            }),
        )
        .await
        .expect("create common api failed");

        assert_eq!(response.code, 200);
        let id = response.data.as_str().expect("create response data should be id string");

        let ref_key: Option<String> = sqlx::query_scalar("SELECT ref_key FROM common_api WHERE id = ?1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .expect("query ref_key failed");
        assert!(ref_key.as_ref().is_some_and(|value| !value.trim().is_empty()));
    }

    #[tokio::test]
    async fn common_api_update_backfills_missing_ref_key() {
        let db = build_test_db().await;

        let inserted = sqlx::query("INSERT INTO common_api (name, url, method, params, headers, deleted) VALUES (?1, ?2, ?3, ?4, ?5, 0)")
            .bind("旧接口")
            .bind("https://example.com/legacy")
            .bind("GET")
            .bind("{}")
            .bind("{}")
            .execute(&db.pool)
            .await
            .expect("insert common_api failed");
        let id = inserted.last_insert_rowid();

        sqlx::query("UPDATE common_api SET ref_key = NULL WHERE id = ?1")
            .bind(id)
            .execute(&db.pool)
            .await
            .expect("clear ref_key failed");

        let response = update(
            &db,
            serde_json::json!({
                "id": id.to_string(),
                "name": "更新后接口",
                "url": "https://example.com/updated",
                "method": "POST",
                "params": "{\"city\":\"shanghai\"}",
                "headers": "{\"Authorization\":\"token\"}"
            }),
        )
        .await
        .expect("update common api failed");

        assert_eq!(response.code, 200);
        assert_eq!(response.data, Value::Bool(true));

        let ref_key: Option<String> = sqlx::query_scalar("SELECT ref_key FROM common_api WHERE id = ?1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .expect("query ref_key failed");
        assert!(ref_key.as_ref().is_some_and(|value| !value.trim().is_empty()));
    }

    #[tokio::test]
    async fn common_api_update_returns_not_found_for_deleted_record() {
        let db = build_test_db().await;

        let inserted = sqlx::query("INSERT INTO common_api (name, url, method, params, headers, deleted) VALUES (?1, ?2, ?3, ?4, ?5, 0)")
            .bind("已删除接口")
            .bind("https://example.com/deleted")
            .bind("GET")
            .bind("{}")
            .bind("{}")
            .execute(&db.pool)
            .await
            .expect("insert common_api failed");
        let id = inserted.last_insert_rowid();

        sqlx::query("UPDATE common_api SET deleted = 1 WHERE id = ?1")
            .bind(id)
            .execute(&db.pool)
            .await
            .expect("soft delete common_api failed");

        let response = update(
            &db,
            serde_json::json!({
                "id": id.to_string(),
                "name": "不应保存",
                "url": "https://example.com/updated",
                "method": "POST",
                "params": "{}",
                "headers": "{}"
            }),
        )
        .await
        .expect("update common api failed");

        assert_eq!(response.code, 404);
        assert_eq!(response.msg, "公共API不存在");
    }
}
