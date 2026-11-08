/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! project 相关接口（项目 CRUD + 分页检索）。
//!
//! 拆分理由：
//! - project 是当前本地后端的核心资源，SQL 与数据结构最多；
//! - 把它独立出来，可以让 router 变得非常薄，只负责分发与参数抽取；
//! - 未来如果继续扩展（例如“场景/素材/图层”等表），也能按资源维度继续拆分文件。

use serde::Deserialize;
use serde_json::Value;
use sqlx::Row;

use crate::backend::types::{err, err_with_detail, ok, HttpResponse};
use crate::db::DbState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 项目分页请求参数。
struct PageParam {
    current: i64,
    size: i64,
    search_value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 项目创建、更新和子项目创建共用的请求体。
struct ProjectInfoManager {
    id: Option<String>,
    name: Option<String>,
    des: Option<String>,
    state: Option<String>,
    pid: Option<String>,
    #[allow(dead_code)]
    create_time: Option<String>,
    #[allow(dead_code)]
    update_time: Option<String>,
    cover: Option<String>,
    data_json: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProjectDeleteOutcome {
    Deleted,
    NotFound,
    ActiveTurn,
}

/// 查询顶层项目分页列表。
pub(crate) async fn page_list(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let page: PageParam = serde_json::from_value(body).map_err(|e| format!("project.pageList: invalid body: {e}"))?;
    let current = if page.current <= 0 { 1 } else { page.current };
    let size = if page.size <= 0 { 15 } else { page.size };
    let offset = (current - 1) * size;

    // 说明：
    // - 查询条件与返回结构刻意对齐前端既有的 pageList 协议（records/total/size/current）
    // - `search_value` 对应前端的 name 模糊搜索
    // - 数据库 id 是 INTEGER，但前端全链路按 string 处理（避免 JS 精度/序列化差异）
    // 分页列表分两次查询：先取总数，再取当前页数据，保持前端分页协议稳定。
    let total_row =
        sqlx::query("SELECT COUNT(*) AS cnt FROM project WHERE deleted = 0 AND pid IS NULL AND (?1 IS NULL OR name LIKE '%' || ?1 || '%')")
            .bind(page.search_value.clone())
            .fetch_one(&db.pool)
            .await
            .map_err(|e| format!("project.pageList: total query failed: {e}"))?;
    let total: i64 = total_row
        .try_get("cnt")
        .map_err(|e| format!("project.pageList: read total failed: {e}"))?;

    let rows = sqlx::query(
        "SELECT id, name, des, state, cover, data_json, create_time, update_time \
         FROM project \
         WHERE deleted = 0 AND pid IS NULL AND (?1 IS NULL OR name LIKE '%' || ?1 || '%') \
         ORDER BY update_time DESC \
         LIMIT ?2 OFFSET ?3",
    )
    .bind(page.search_value)
    .bind(size)
    .bind(offset)
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("project.pageList: list query failed: {e}"))?;

    let mut records: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("project.pageList: read id failed: {e}"))?;
        let name: String = row
            .try_get("name")
            .map_err(|e| format!("project.pageList: read name failed: {e}"))?;
        let des: Option<String> = row
            .try_get("des")
            .map_err(|e| format!("project.pageList: read des failed: {e}"))?;
        let state: Option<String> = row
            .try_get("state")
            .map_err(|e| format!("project.pageList: read state failed: {e}"))?;
        let cover: Option<String> = row
            .try_get("cover")
            .map_err(|e| format!("project.pageList: read cover failed: {e}"))?;
        let data_json: Option<String> = row
            .try_get("data_json")
            .map_err(|e| format!("project.pageList: read data_json failed: {e}"))?;
        let create_time: String = row
            .try_get("create_time")
            .map_err(|e| format!("project.pageList: read create_time failed: {e}"))?;
        let update_time: String = row
            .try_get("update_time")
            .map_err(|e| format!("project.pageList: read update_time failed: {e}"))?;
        records.push(serde_json::json!({
            "id": id.to_string(),
            "name": name,
            "des": des,
            "state": state,
            "cover": cover,
            "dataJson": data_json,
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

/// 创建顶层项目。
pub(crate) async fn create(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let project: ProjectInfoManager = serde_json::from_value(body).map_err(|e| format!("project.create: invalid body: {e}"))?;
    if project.name.as_deref().is_some_and(|name| name.trim().is_empty()) {
        return Ok(err(400, "项目名称不能为空"));
    }
    let name = project.name.unwrap_or_else(|| "未命名项目".to_string());

    let result = sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES (?1, ?2, ?3, NULL, ?4, ?5, 0)")
        .bind(name)
        .bind(project.des)
        .bind(project.state)
        .bind(project.data_json)
        .bind(project.cover)
        .execute(&db.pool)
        .await
        .map_err(|e| format!("project.create: insert failed: {e}"))?;

    Ok(ok(Value::String(result.last_insert_rowid().to_string())))
}

/// 更新项目基础信息和项目 JSON。
pub(crate) async fn update(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let project: ProjectInfoManager = serde_json::from_value(body).map_err(|e| format!("project.update: invalid body: {e}"))?;
    if project.name.as_deref().is_some_and(|name| name.trim().is_empty()) {
        return Ok(err(400, "项目名称不能为空"));
    }
    let Some(id_str) = project.id else {
        return Ok(err(400, "请求参数错误：缺少 id"));
    };
    let id: i64 = id_str
        .parse()
        .map_err(|e| format!("project.update: invalid id `{id_str}`: {e}"))?;

    // 只更新请求中传入的字段，未传字段保留数据库现有值。
    let changed = sqlx::query(
        "UPDATE project SET \
           name = COALESCE(?2, name), \
           des = COALESCE(?3, des), \
           state = COALESCE(?4, state), \
           cover = COALESCE(?5, cover), \
           data_json = COALESCE(?6, data_json) \
         WHERE id = ?1 AND deleted = 0",
    )
    .bind(id)
    .bind(project.name)
    .bind(project.des)
    .bind(project.state)
    .bind(project.cover)
    .bind(project.data_json)
    .execute(&db.pool)
    .await
    .map_err(|e| format!("project.update: update failed: {e}"))?
    .rows_affected();

    Ok(ok(Value::Bool(changed > 0)))
}

/// 获取项目编辑器需要的完整项目数据。
pub(crate) async fn get_project_data(db: &DbState, id: i64) -> Result<HttpResponse, String> {
    let row = sqlx::query("SELECT id, name, data_json FROM project WHERE id = ?1 AND deleted = 0")
        .bind(id)
        .fetch_optional(&db.pool)
        .await
        .map_err(|e| format!("project.getProjectData: query failed: {e}"))?;

    let Some(row) = row else {
        return Ok(ok(Value::Null));
    };

    let id: i64 = row
        .try_get("id")
        .map_err(|e| format!("project.getProjectData: read id failed: {e}"))?;
    let name: String = row
        .try_get("name")
        .map_err(|e| format!("project.getProjectData: read name failed: {e}"))?;
    let data_json: Option<String> = row
        .try_get("data_json")
        .map_err(|e| format!("project.getProjectData: read data_json failed: {e}"))?;
    Ok(ok(serde_json::json!({
        "id": id.to_string(),
        "name": name,
        "dataJson": data_json.unwrap_or_else(|| "{}".to_string())
    })))
}

/// 获取项目元信息，用于轻量场景引用。
pub(crate) async fn get_project_meta(db: &DbState, id: i64) -> Result<HttpResponse, String> {
    let row = sqlx::query("SELECT id, name, pid, data_json FROM project WHERE id = ?1 AND deleted = 0")
        .bind(id)
        .fetch_optional(&db.pool)
        .await
        .map_err(|e| format!("project.getProjectMeta: query failed: {e}"))?;

    let Some(row) = row else {
        return Ok(ok(Value::Null));
    };

    let id: i64 = row
        .try_get("id")
        .map_err(|e| format!("project.getProjectMeta: read id failed: {e}"))?;
    let name: String = row
        .try_get("name")
        .map_err(|e| format!("project.getProjectMeta: read name failed: {e}"))?;
    let pid: Option<i64> = row
        .try_get("pid")
        .map_err(|e| format!("project.getProjectMeta: read pid failed: {e}"))?;
    let data_json: Option<String> = row
        .try_get("data_json")
        .map_err(|e| format!("project.getProjectMeta: read data_json failed: {e}"))?;
    Ok(ok(serde_json::json!({
        "id": id.to_string(),
        "name": name,
        "pid": pid.map(|value| value.to_string()),
        "dataJson": data_json.unwrap_or_else(|| "{}".to_string())
    })))
}

/// 复制项目记录并返回新项目 id。
pub(crate) async fn copy(db: &DbState, id: i64) -> Result<HttpResponse, String> {
    let row = sqlx::query("SELECT name, des, state, pid, data_json, cover FROM project WHERE id = ?1 AND deleted = 0")
        .bind(id)
        .fetch_optional(&db.pool)
        .await
        .map_err(|e| format!("project.copy: query source failed: {e}"))?;

    let Some(row) = row else {
        return Ok(ok(Value::String("".to_string())));
    };

    let name: String = row
        .try_get("name")
        .map_err(|e| format!("project.copy: read name failed: {e}"))?;
    let des: Option<String> = row.try_get("des").map_err(|e| format!("project.copy: read des failed: {e}"))?;
    let project_state: Option<String> = row
        .try_get("state")
        .map_err(|e| format!("project.copy: read state failed: {e}"))?;
    let pid: Option<i64> = row.try_get("pid").map_err(|e| format!("project.copy: read pid failed: {e}"))?;
    let data_json: Option<String> = row
        .try_get("data_json")
        .map_err(|e| format!("project.copy: read data_json failed: {e}"))?;
    let cover: Option<String> = row
        .try_get("cover")
        .map_err(|e| format!("project.copy: read cover failed: {e}"))?;

    // 复制只复制项目表字段，资源文件和外部引用仍复用原项目数据中的引用。
    let result = sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)")
        .bind(format!("{name} - 副本"))
        .bind(des)
        .bind(project_state)
        .bind(pid)
        .bind(data_json)
        .bind(cover)
        .execute(&db.pool)
        .await
        .map_err(|e| format!("project.copy: insert failed: {e}"))?;

    Ok(ok(Value::String(result.last_insert_rowid().to_string())))
}

/// 创建挂在指定父项目下的子项目。
pub(crate) async fn create_sub_project(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let project: ProjectInfoManager = serde_json::from_value(body).map_err(|e| format!("project.createSubProject: invalid body: {e}"))?;
    if project.name.as_deref().is_some_and(|name| name.trim().is_empty()) {
        return Ok(err(400, "项目名称不能为空"));
    }
    let name = project.name.unwrap_or_else(|| "未命名子项目".to_string());
    let Some(pid_str) = project.pid else {
        return Ok(err(400, "请求参数错误：缺少 pid"));
    };
    let pid: i64 = pid_str
        .parse()
        .map_err(|e| format!("project.createSubProject: invalid pid `{pid_str}`: {e}"))?;

    let result = sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)")
        .bind(name)
        .bind(project.des)
        .bind(project.state)
        .bind(pid)
        .bind(project.data_json)
        .bind(project.cover)
        .execute(&db.pool)
        .await
        .map_err(|e| format!("project.createSubProject: insert failed: {e}"))?;

    Ok(ok(Value::String(result.last_insert_rowid().to_string())))
}

/// 查询指定父项目下的子项目列表。
pub(crate) async fn get_sub_project_list(db: &DbState, pid: i64) -> Result<HttpResponse, String> {
    let rows = sqlx::query("SELECT id, name, des FROM project WHERE deleted = 0 AND pid = ?1 ORDER BY update_time DESC")
        .bind(pid)
        .fetch_all(&db.pool)
        .await
        .map_err(|e| format!("project.getSubProjectList: query failed: {e}"))?;

    let mut out: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("project.getSubProjectList: read id failed: {e}"))?;
        let name: String = row
            .try_get("name")
            .map_err(|e| format!("project.getSubProjectList: read name failed: {e}"))?;
        let des: Option<String> = row
            .try_get("des")
            .map_err(|e| format!("project.getSubProjectList: read des failed: {e}"))?;
        out.push(serde_json::json!({
            "id": id.to_string(),
            "name": name,
            "des": des
        }));
    }
    Ok(ok(Value::Array(out)))
}

/// 软删除指定父项目下的一个子项目。
pub(crate) async fn delete_sub_project(db: &DbState, pid: i64, sub_id: i64) -> Result<HttpResponse, String> {
    project_delete_response(soft_delete_project(db, sub_id, Some(pid), "project.deleteSubProject").await?)
}

/// 快速搜索顶层项目，并保证当前选中项目可被回显。
pub(crate) async fn quick_search(db: &DbState, current: Option<i64>, exclude: Option<i64>, search: Option<String>) -> Result<HttpResponse, String> {
    let rows = sqlx::query(
        "SELECT id, name FROM project \
         WHERE deleted = 0 \
           AND (?1 IS NULL OR name LIKE '%' || ?1 || '%') \
           AND (?2 IS NULL OR id != ?2) \
         ORDER BY update_time DESC \
         LIMIT 10",
    )
    .bind(search.clone())
    .bind(exclude)
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("project.quickSearch: query failed: {e}"))?;

    let mut out: Vec<Value> = Vec::with_capacity(rows.len() + 1);
    let mut has_current = false;
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("project.quickSearch: read id failed: {e}"))?;
        let name: String = row
            .try_get("name")
            .map_err(|e| format!("project.quickSearch: read name failed: {e}"))?;
        if Some(id) == current {
            has_current = true;
        }
        out.push(serde_json::json!({ "id": id.to_string(), "name": name }));
    }

    // 搜索结果最多 10 条；当前选中项不在搜索结果内时追加，避免前端选择器丢失标签。
    if let Some(cur) = current {
        if !has_current && Some(cur) != exclude {
            let row = sqlx::query("SELECT id, name FROM project WHERE id = ?1 AND deleted = 0 AND pid IS NULL")
                .bind(cur)
                .fetch_optional(&db.pool)
                .await
                .map_err(|e| format!("project.quickSearch: query current failed: {e}"))?;
            if let Some(row) = row {
                let id: i64 = row
                    .try_get("id")
                    .map_err(|e| format!("project.quickSearch: read current id failed: {e}"))?;
                let name: String = row
                    .try_get("name")
                    .map_err(|e| format!("project.quickSearch: read current name failed: {e}"))?;
                out.push(serde_json::json!({ "id": id.to_string(), "name": name }));
            }
        }
    }

    Ok(ok(Value::Array(out)))
}

/// 软删除项目。
pub(crate) async fn del(db: &DbState, id: i64) -> Result<HttpResponse, String> {
    project_delete_response(soft_delete_project(db, id, None, "project.del").await?)
}

/// 项目与活动 Turn 的删除互斥边界。单个写事务保证并发 submit 与删除只能一方成功。
async fn soft_delete_project(db: &DbState, project_id: i64, parent_id: Option<i64>, operation: &str) -> Result<ProjectDeleteOutcome, String> {
    let mut transaction = db
        .pool
        .begin()
        .await
        .map_err(|error| format!("{operation}: begin transaction failed: {error}"))?;
    let changed = sqlx::query(
        "UPDATE project SET deleted = 1 \
         WHERE id = ?1 AND deleted = 0 AND (?2 IS NULL OR pid = ?2) \
           AND NOT EXISTS (\
             SELECT 1 FROM ai_agent_turn active \
             WHERE active.project_id = project.id \
               AND active.status IN ('created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation')\
           )",
    )
    .bind(project_id)
    .bind(parent_id)
    .execute(&mut *transaction)
    .await
    .map_err(|error| format!("{operation}: update failed: {error}"))?
    .rows_affected();

    let outcome = if changed == 1 {
        ProjectDeleteOutcome::Deleted
    } else {
        let has_active_turn = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(\
               SELECT 1 FROM project scoped \
               JOIN ai_agent_turn active ON active.project_id = scoped.id \
               WHERE scoped.id = ?1 AND scoped.deleted = 0 AND (?2 IS NULL OR scoped.pid = ?2) \
                 AND active.status IN ('created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation')\
             )",
        )
        .bind(project_id)
        .bind(parent_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| format!("{operation}: active Turn query failed: {error}"))?;
        if has_active_turn == 1 {
            ProjectDeleteOutcome::ActiveTurn
        } else {
            ProjectDeleteOutcome::NotFound
        }
    };
    transaction
        .commit()
        .await
        .map_err(|error| format!("{operation}: commit failed: {error}"))?;
    Ok(outcome)
}

fn project_delete_response(outcome: ProjectDeleteOutcome) -> Result<HttpResponse, String> {
    Ok(match outcome {
        ProjectDeleteOutcome::Deleted => ok(Value::Bool(true)),
        ProjectDeleteOutcome::NotFound => ok(Value::Bool(false)),
        ProjectDeleteOutcome::ActiveTurn => err_with_detail(409, "项目存在运行中的 AI 任务，请先停止任务", "agent.project.active_turn"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::thread::{CreateThreadInput, CreateTurnInput, ThreadService};
    use crate::db::DbState;
    use serde_json::json;
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
    async fn project_names_reject_blank_values_without_blocking_partial_updates() {
        let db = build_test_db().await;
        for name in ["", "  \t", "　"] {
            assert_eq!(create(&db, json!({"name": name})).await.unwrap().code, 400);
            assert_eq!(create_sub_project(&db, json!({"name": name, "pid": "1"})).await.unwrap().code, 400);
        }

        let created = create(&db, json!({"name": "测试项目"})).await.unwrap();
        let id = created.data.as_str().unwrap();
        assert_eq!(update(&db, json!({"id": id, "name": "  "})).await.unwrap().code, 400);
        assert_eq!(update(&db, json!({"id": id, "des": "更新描述"})).await.unwrap().code, 200);
        let record = get_project_data(&db, id.parse().unwrap()).await.unwrap();
        assert_eq!(record.data["name"], "测试项目");
    }

    #[tokio::test]
    async fn get_project_meta_returns_name_pid_and_data_json() {
        let db = build_test_db().await;

        let root_id = sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES (?1, '', '0', NULL, ?2, '', 0)")
            .bind("主项目")
            .bind(r#"{"layerManager":{"elemConfigs":{}}}"#)
            .execute(&db.pool)
            .await
            .expect("insert root project failed")
            .last_insert_rowid();

        let child_id = sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES (?1, '', '0', ?2, ?3, '', 0)")
            .bind("子项目")
            .bind(root_id)
            .bind(r#"{"layerManager":{"elemConfigs":{"screen":{"base":{"type":"ScreenReference"},"style":{"screenId":"9"}}}}}"#)
            .execute(&db.pool)
            .await
            .expect("insert child project failed")
            .last_insert_rowid();

        let response = get_project_meta(&db, child_id).await.expect("get project meta failed");

        assert_eq!(response.code, 200);
        assert_eq!(response.msg, "操作成功");
        assert_eq!(response.data["id"], child_id.to_string());
        assert_eq!(response.data["name"], "子项目");
        assert_eq!(response.data["pid"], root_id.to_string());
        assert_eq!(
            response.data["dataJson"],
            serde_json::Value::String(
                r#"{"layerManager":{"elemConfigs":{"screen":{"base":{"type":"ScreenReference"},"style":{"screenId":"9"}}}}}"#.to_string()
            )
        );
    }

    #[tokio::test]
    async fn get_project_meta_returns_null_when_project_missing() {
        let db = build_test_db().await;

        let response = get_project_meta(&db, 999).await.expect("get project meta failed");

        assert_eq!(response.code, 200);
        assert_eq!(response.data, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn delete_rejects_project_with_active_agent_turn() {
        let db = build_test_db().await;
        let project_id = sqlx::query("INSERT INTO project (name, deleted) VALUES ('active-agent-project', 0)")
            .execute(&db.pool)
            .await
            .expect("insert project failed")
            .last_insert_rowid();
        let threads = ThreadService::new(db.pool.clone());
        let thread = threads
            .create_thread(CreateThreadInput { project_id, create_request_id: "delete-active-thread".to_string(), title: None, created_at: 1 })
            .await
            .expect("create Thread failed");
        threads
            .create_turn(CreateTurnInput {
                project_id,
                thread_id: thread.thread_id,
                request_id: "delete-active-turn".to_string(),
                request: json!({"requirement": "keep running"}),
                input_summary: "keep running".to_string(),
                model_config_id: "model-config".to_string(),
                runtime_model_id: "runtime-model".to_string(),
                provider: None,
                entry_digest: "sha256:delete-active-turn".to_string(),
                client_session_id: "client-session".to_string(),
                task_type: "general".to_string(),
                limits: json!({}),
                project_snapshot_ref: None,
                rig_version: None,
                rig_snapshot_ref: None,
                user_item_payload: json!({"text": "keep running"}),
                attachments: Vec::new(),
                created_at: 2,
            })
            .await
            .expect("create active Turn failed");

        let response = del(&db, project_id).await.expect("delete project request failed");
        assert_eq!(response.code, 409);
        assert!(response.msg.contains("agent.project.active_turn"));
        assert_eq!(response.data, Value::Null);
        let deleted = sqlx::query_scalar::<_, i64>("SELECT deleted FROM project WHERE id = ?1")
            .bind(project_id)
            .fetch_one(&db.pool)
            .await
            .expect("read project deleted state failed");
        assert_eq!(deleted, 0);
    }
}
