/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! router：把 “HttpRequest(method/url/body)” 分发到各个业务 handler。
//!
//! 这里刻意保持“薄”：
//! - 只做 method/url 匹配
//! - 只做必要的参数抽取（例如 body 是否存在、从 url 尾部解析 id）
//! - 不写 SQL，不组装业务数据结构
//!
//! 这样做的好处：
//! - 新增一个接口时，只需要：routes 里加 handler + router 加一条分发规则
//! - project 这种复杂资源的 SQL/结构不会把整个文件“撑爆”

use std::future::Future;

use serde_json::Value;
use tauri::State;
use tokio::sync::watch;

use crate::app_state::AppState;
use crate::backend::routes;
use crate::backend::types::{err, HttpRequest, HttpResponse};
use crate::backend::utils::parse_id_from_tail;
use crate::backend::FrontendEffectRegistry;
use crate::db::DbState;

const DEFAULT_MISSING_BODY_MESSAGE: &str = "请求参数错误：缺少请求体";

/// 当请求体存在时执行 handler；调用方可传入自定义的缺失请求体提示。
async fn with_required_body<F, Fut>(body: Option<Value>, custom_missing_message: Option<&str>, handler: F) -> Result<HttpResponse, String>
where
    F: FnOnce(Value) -> Fut,
    Fut: Future<Output = Result<HttpResponse, String>>,
{
    let Some(body) = body else {
        return Ok(err(400, custom_missing_message.unwrap_or(DEFAULT_MISSING_BODY_MESSAGE)));
    };
    handler(body).await
}

/// 根据请求方法和 URL 分发到具体业务路由。
pub(crate) async fn dispatch(
    app_handle: &tauri::AppHandle,
    db_state: State<'_, DbState>,
    app_state: State<'_, AppState>,
    _frontend_effects: State<'_, FrontendEffectRegistry>,
    req: HttpRequest,
    _cancel_rx: Option<watch::Receiver<bool>>,
) -> Result<HttpResponse, String> {
    let method = req.method.to_uppercase();
    let url = req.url.as_str();

    // 所有分支都返回统一 HttpResponse，避免前端需要区分 Rust 错误和业务错误。
    match (method.as_str(), url) {
        ("GET", "/api/security/getEncryptKey") => Ok(routes::security::get_encrypt_key()),

        // 1) 环境授权/能力探测（不落库）
        ("GET", "/api/systemAuth/hasAuth") => Ok(routes::system_auth::has_auth()),
        ("GET", "/api/systemAuth/getAuthCode") => Ok(routes::system_auth::get_auth_code()),
        ("GET", "/api/appConfig/get") => routes::app_config::get(app_state.inner()).await,
        ("POST", "/api/appConfig/save") => {
            with_required_body(req.body, None, |body| routes::app_config::save(db_state.inner(), app_state.inner(), body)).await
        }
        ("GET", "/api/aiDriven/settings/get") => routes::ai_driven_settings::get(db_state.inner()).await,
        ("POST", "/api/aiDriven/settings/save") => {
            with_required_body(req.body, None, |body| routes::ai_driven_settings::save(db_state.inner(), body)).await
        }

        // 2) 项目导入/导出：涉及文件系统和 zip 包处理。
        ("POST", "/api/export/exportProject") => {
            with_required_body(req.body, None, |body| routes::export::export_project(db_state.inner(), app_state.inner(), body)).await
        }
        ("POST", "/api/export/exportProjectToPath") => {
            with_required_body(req.body, None, |body| routes::export::export_project_to_path(db_state.inner(), app_state.inner(), body)).await
        }
        ("POST", "/api/import/importProjectFromPath") => {
            with_required_body(req.body, None, |body| routes::import::import_project_from_path(db_state.inner(), app_state.inner(), body)).await
        }

        ("GET", "/api/remote/component/list") => routes::remote_component::list(db_state.inner(), app_state.inner()).await,
        ("POST", "/api/remote/component/pageList") => {
            with_required_body(req.body, None, |body| routes::remote_component::page_list(db_state.inner(), app_state.inner(), body)).await
        }
        ("POST", "/api/remote/component/batchDel") => {
            with_required_body(req.body, None, |body| routes::remote_component::batch_del(db_state.inner(), body)).await
        }
        ("POST", "/api/remote/component/update") => {
            with_required_body(req.body, None, |body| routes::remote_component::update(db_state.inner(), body)).await
        }
        ("POST", "/api/remote/component/upload") => {
            with_required_body(req.body, None, |body| routes::remote_component::upload(db_state.inner(), app_state.inner(), body)).await
        }

        ("GET", u) if u.starts_with("/api/release/getReleaseInfo/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::release::get_release_info(db_state.inner(), id).await
        }
        ("POST", "/api/release/getReleaseData") => {
            with_required_body(req.body, None, |body| routes::release::get_release_data(db_state.inner(), body)).await
        }
        ("POST", "/api/release/release") => {
            with_required_body(req.body, None, |body| routes::release::release(db_state.inner(), app_state.inner(), body)).await
        }
        ("POST", "/api/release/pageList") => with_required_body(req.body, None, |body| routes::release::page_list(db_state.inner(), body)).await,
        ("POST", "/api/release/batchDel") => with_required_body(req.body, None, |body| routes::release::batch_del(db_state.inner(), body)).await,
        ("POST", "/api/release/update") => with_required_body(req.body, None, |body| routes::release::update(db_state.inner(), body)).await,

        ("GET", u) if u.starts_with("/api/commonApi/quickSearch") => routes::common_api::quick_search(db_state.inner(), u).await,
        ("GET", u) if u.starts_with("/api/commonApi/getCommonApi/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::common_api::get_common_api(db_state.inner(), id).await
        }
        ("GET", u) if u.starts_with("/api/commonApi/copy/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::common_api::copy(db_state.inner(), id).await
        }
        ("POST", "/api/commonApi/pageList") => with_required_body(req.body, None, |body| routes::common_api::page_list(db_state.inner(), body)).await,
        ("POST", "/api/commonApi/batchDel") => with_required_body(req.body, None, |body| routes::common_api::batch_del(db_state.inner(), body)).await,
        ("POST", "/api/commonApi/create") => with_required_body(req.body, None, |body| routes::common_api::create(db_state.inner(), body)).await,
        ("POST", "/api/commonApi/update") => with_required_body(req.body, None, |body| routes::common_api::update(db_state.inner(), body)).await,

        ("GET", "/api/aiDriven/model/providers") => routes::ai_driven_model::providers().await,
        ("POST", "/api/aiDriven/attachment/upload") => {
            with_required_body(req.body, None, |body| routes::ai_driven_attachment::upload(db_state.inner(), app_state.inner(), body)).await
        }
        ("POST", "/api/aiDriven/thread/create") => {
            with_required_body(req.body, None, |body| routes::ai_driven_thread::create(db_state.inner(), body)).await
        }
        ("POST", "/api/aiDriven/thread/list") => {
            with_required_body(req.body, None, |body| routes::ai_driven_thread::list(db_state.inner(), body)).await
        }
        ("POST", "/api/aiDriven/thread/read") => {
            with_required_body(req.body, None, |body| routes::ai_driven_thread::read(db_state.inner(), body)).await
        }
        ("POST", "/api/aiDriven/thread/update") => {
            with_required_body(req.body, None, |body| routes::ai_driven_thread::update(db_state.inner(), body)).await
        }
        ("POST", "/api/aiDriven/thread/archive") => {
            with_required_body(req.body, None, |body| routes::ai_driven_thread::archive(db_state.inner(), body)).await
        }
        ("POST", "/api/aiDriven/thread/events") => {
            with_required_body(req.body, None, |body| routes::ai_driven_thread::events(db_state.inner(), body)).await
        }
        ("POST", "/api/aiDriven/turn/list") => with_required_body(req.body, None, |body| routes::ai_driven_turn::list(db_state.inner(), body)).await,
        ("POST", "/api/aiDriven/turn/read") => with_required_body(req.body, None, |body| routes::ai_driven_turn::read(db_state.inner(), body)).await,
        ("POST", "/api/aiDriven/item/list") => with_required_body(req.body, None, |body| routes::ai_driven_item::list(db_state.inner(), body)).await,
        ("POST", "/api/aiDriven/item/read") => with_required_body(req.body, None, |body| routes::ai_driven_item::read(db_state.inner(), body)).await,
        ("POST", "/api/aiDriven/task/submit") => with_required_body(req.body, None, |body| routes::ai_driven_task::submit(app_handle, body)).await,
        ("POST", "/api/aiDriven/task/resume") => with_required_body(req.body, None, |body| routes::ai_driven_task::resume(app_handle, body)).await,
        ("POST", "/api/aiDriven/task/steer") => {
            with_required_body(req.body, None, |body| routes::ai_driven_task::steer(app_handle, db_state.inner(), body)).await
        }
        ("POST", "/api/aiDriven/task/cancel") => {
            with_required_body(req.body, None, |body| routes::ai_driven_task::cancel(db_state.inner(), body)).await
        }
        ("POST", "/api/aiDriven/task/status") => {
            with_required_body(req.body, None, |body| routes::ai_driven_task::status(db_state.inner(), body)).await
        }
        ("POST", "/api/aiDriven/task/events") => {
            with_required_body(req.body, None, |body| routes::ai_driven_task::events(db_state.inner(), body)).await
        }

        ("POST", "/api/aiModel/pageList") => {
            with_required_body(req.body, None, |body| routes::ai_model::page_list(db_state.inner(), app_state.inner(), body)).await
        }
        ("GET", "/api/aiModel/list") => routes::ai_model::list(db_state.inner()).await,
        ("GET", "/api/aiModel/configuredList") => routes::ai_model::configured_list(db_state.inner()).await,
        ("GET", "/api/aiModel/catalog") => routes::ai_model::catalog().await,
        ("POST", "/api/aiModel/batchDel") => with_required_body(req.body, None, |body| routes::ai_model::batch_del(db_state.inner(), body)).await,
        ("POST", "/api/aiModel/create") => {
            with_required_body(req.body, None, |body| routes::ai_model::create(db_state.inner(), app_state.inner(), body)).await
        }
        ("POST", "/api/aiModel/update") => {
            with_required_body(req.body, None, |body| routes::ai_model::update(db_state.inner(), app_state.inner(), body)).await
        }
        ("GET", u) if u.starts_with("/api/dynamicScript/list") => routes::dynamic_script::list(db_state.inner(), app_state.inner(), u).await,
        ("POST", "/api/dynamicScript/pageList") => {
            with_required_body(req.body, None, |body| routes::dynamic_script::page_list(db_state.inner(), app_state.inner(), body)).await
        }
        ("POST", "/api/dynamicScript/batchDel") => {
            with_required_body(req.body, None, |body| routes::dynamic_script::batch_del(db_state.inner(), body)).await
        }
        ("POST", "/api/dynamicScript/update") => {
            with_required_body(req.body, None, |body| routes::dynamic_script::update(db_state.inner(), body)).await
        }
        ("POST", "/api/dynamicScript/saveOrUpdate") => {
            with_required_body(req.body, None, |body| routes::dynamic_script::save_or_update(db_state.inner(), app_state.inner(), body)).await
        }

        ("POST", "/api/video/page") => {
            with_required_body(req.body, None, |body| routes::video::page(db_state.inner(), app_state.inner(), body)).await
        }
        ("GET", u) if u.starts_with("/api/video/delete/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::video::delete(db_state.inner(), id).await
        }
        ("POST", "/api/video/batchDelete") => with_required_body(req.body, None, |body| routes::video::batch_delete(db_state.inner(), body)).await,
        ("POST", "/api/video/update") => with_required_body(req.body, None, |body| routes::video::update(db_state.inner(), body)).await,
        ("POST", "/api/video/upload") => {
            with_required_body(req.body, None, |body| routes::video::upload(db_state.inner(), app_state.inner(), body)).await
        }

        ("POST", "/api/model/page") => {
            with_required_body(req.body, None, |body| routes::three_d_model::page(db_state.inner(), app_state.inner(), body)).await
        }
        ("POST", "/api/model/batchDelete") => {
            with_required_body(req.body, None, |body| routes::three_d_model::batch_delete(db_state.inner(), body)).await
        }
        ("POST", "/api/model/update") => with_required_body(req.body, None, |body| routes::three_d_model::update(db_state.inner(), body)).await,
        ("POST", "/api/model/upload") => {
            with_required_body(req.body, None, |body| routes::three_d_model::upload(db_state.inner(), app_state.inner(), body)).await
        }
        ("GET", u) if u.starts_with("/api/model/get/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::three_d_model::get(db_state.inner(), app_state.inner(), id).await
        }

        ("GET", "/api/commonDatabase/list") => routes::common_database::list(db_state.inner()).await,
        ("POST", "/api/commonDatabase/pageList") => {
            with_required_body(req.body, None, |body| routes::common_database::page_list(db_state.inner(), body)).await
        }
        ("POST", "/api/commonDatabase/batchDel") => {
            with_required_body(req.body, None, |body| routes::common_database::batch_del(db_state.inner(), body)).await
        }
        ("GET", u) if u.starts_with("/api/commonDatabase/copy/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::common_database::copy(db_state.inner(), id).await
        }
        ("GET", u) if u.starts_with("/api/commonDatabase/test/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::common_database::test(db_state.inner(), id).await
        }
        ("POST", "/api/commonDatabase/get") => with_required_body(req.body, None, |body| routes::common_database::get(db_state.inner(), body)).await,
        ("POST", "/api/commonDatabase/add") => with_required_body(req.body, None, |body| routes::common_database::add(db_state.inner(), body)).await,
        ("POST", "/api/commonDatabase/update") => {
            with_required_body(req.body, None, |body| routes::common_database::update(db_state.inner(), body)).await
        }

        ("POST", "/api/db/executor/execute") => {
            with_required_body(req.body, None, |body| routes::common_database::executor_execute(db_state.inner(), body)).await
        }

        ("GET", "/api/font/list") => routes::font::list(db_state.inner(), app_state.inner()).await,
        ("POST", "/api/font/pageList") => {
            with_required_body(req.body, None, |body| routes::font::page_list(db_state.inner(), app_state.inner(), body)).await
        }
        ("POST", "/api/font/update") => with_required_body(req.body, None, |body| routes::font::update(db_state.inner(), body)).await,
        ("POST", "/api/font/batchDel") => with_required_body(req.body, None, |body| routes::font::batch_del(db_state.inner(), body)).await,
        ("POST", "/api/font/upload") => {
            with_required_body(req.body, None, |body| routes::font::upload(db_state.inner(), app_state.inner(), body)).await
        }

        // 3) 核心资源：项目（SQLite）- 精确匹配
        ("POST", "/api/project/pageList") => with_required_body(req.body, None, |body| routes::project::page_list(db_state.inner(), body)).await,
        ("POST", "/api/project/create") => with_required_body(req.body, None, |body| routes::project::create(db_state.inner(), body)).await,
        ("POST", "/api/project/update") => with_required_body(req.body, None, |body| routes::project::update(db_state.inner(), body)).await,
        ("POST", "/api/project/createSubProject") => {
            with_required_body(req.body, None, |body| routes::project::create_sub_project(db_state.inner(), body)).await
        }
        ("GET", u) if u.starts_with("/api/project/quickSearch") => {
            let mut current: Option<i64> = None;
            let mut exclude: Option<i64> = None;
            let mut search: Option<String> = None;
            // 该接口仍使用 query string 传参，路由层只抽取基础参数后交给 project handler。
            if let Some((_, qs)) = u.split_once('?') {
                for kv in qs.split('&') {
                    let Some((k, v)) = kv.split_once('=') else {
                        continue;
                    };
                    let key = urlencoding::decode(k)
                        .ok()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| k.to_string());
                    let val = urlencoding::decode(v)
                        .ok()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| v.to_string());
                    match key.as_str() {
                        "current" => {
                            let t = val.trim();
                            if !t.is_empty() {
                                current = t.parse::<i64>().ok();
                            }
                        }
                        "exclude" => {
                            let t = val.trim();
                            if !t.is_empty() {
                                exclude = t.parse::<i64>().ok();
                            }
                        }
                        "search" => {
                            let t = val.trim().to_string();
                            if !t.is_empty() {
                                search = Some(t);
                            }
                        }
                        _ => {}
                    }
                }
            }
            routes::project::quick_search(db_state.inner(), current, exclude, search).await
        }

        // 3) 核心资源：项目（SQLite）- 前缀匹配 (Dynamic segments)
        ("GET", u) if u.starts_with("/api/project/getProjectData/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::project::get_project_data(db_state.inner(), id).await
        }
        ("GET", u) if u.starts_with("/api/project/getProjectMeta/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::project::get_project_meta(db_state.inner(), id).await
        }
        ("GET", u) if u.starts_with("/api/project/copy/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::project::copy(db_state.inner(), id).await
        }
        ("GET", u) if u.starts_with("/api/project/del/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::project::del(db_state.inner(), id).await
        }
        ("GET", u) if u.starts_with("/api/project/getSubProjectList/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::project::get_sub_project_list(db_state.inner(), id).await
        }
        ("POST", u) if u.starts_with("/api/project/deleteSubProject/") => {
            // 历史接口把父项目 id 和子项目 id 都放在路径段里，这里保留兼容解析。
            // TODO(legacy-compat): 迁移当前前端调用方后，移除此历史路由格式。
            let parts: Vec<&str> = u.split('/').collect();
            if parts.len() < 6 {
                return Ok(err(400, "请求参数错误：参数不完整"));
            }
            let pid = parts[4].parse::<i64>().ok();
            let sub_id = parts[5].parse::<i64>().ok();
            let (Some(pid), Some(sub_id)) = (pid, sub_id) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::project::delete_sub_project(db_state.inner(), pid, sub_id).await
        }

        // 4) 素材/模板等“资源库能力”
        ("POST", "/api/image/upload") => {
            with_required_body(req.body, None, |body| routes::image::upload(db_state.inner(), app_state.inner(), body)).await
        }
        ("GET", "/api/image/getUserUploadedImageList") => routes::image::get_user_uploaded_image_list(db_state.inner(), app_state.inner()).await,
        ("GET", u) if u.starts_with("/api/image/del/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::image::del(db_state.inner(), app_state.inner(), id).await
        }
        ("POST", "/api/template/create") => {
            with_required_body(req.body, None, |body| routes::template::create(db_state.inner(), app_state.inner(), body)).await
        }
        ("POST", "/api/template/getAllTemplate") => match req.body {
            Some(body) => routes::template::get_all_template(db_state.inner(), app_state.inner(), body).await,
            None => routes::template::get_all_template(db_state.inner(), app_state.inner(), serde_json::Value::Null).await,
        },
        ("GET", u) if u.starts_with("/api/template/getTemplate/") => {
            let Some(id) = parse_id_from_tail(u) else {
                return Ok(err(400, "请求参数错误：id 不合法"));
            };
            routes::template::get_template(db_state.inner(), app_state.inner(), id).await
        }
        ("POST", "/api/template/batchDel") => with_required_body(req.body, None, |body| routes::template::batch_del(db_state.inner(), body)).await,

        // 5) 兜底：返回“成功但 code!=200 的业务错误”而不是 Rust panic，
        //    以保持前端 responseHandler 的行为可控、可观测。
        _ => Ok(err(404, format!("unsupported api: {method} {url}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn required_body_uses_default_message_when_no_override_is_provided() {
        let response = with_required_body(None, None, |_body: Value| async { Ok(err(200, "不应执行")) })
            .await
            .expect("missing body should produce an HttpResponse");

        assert_eq!(response.code, 400);
        assert_eq!(response.msg, DEFAULT_MISSING_BODY_MESSAGE);
    }

    #[tokio::test]
    async fn required_body_uses_custom_message_when_provided() {
        let response = with_required_body(None, Some("导入参数缺失"), |_body: Value| async { Ok(err(200, "不应执行")) })
            .await
            .expect("missing body should produce an HttpResponse");

        assert_eq!(response.code, 400);
        assert_eq!(response.msg, "导入参数缺失");
    }
}
