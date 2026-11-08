/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Thread 管理与 Thread Event 查询接口。

use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::ai_driven::thread::{
    CreateThreadInput, ListThreadEventsQuery, ListThreadsQuery, SetThreadArchivedInput, SqliteThreadRepository, ThreadCursor, UpdateThreadTitleInput,
};
use crate::backend::types::{err, ok, HttpResponse};
use crate::db::DbState;

use super::ai_driven_history_common::{parse_body, parse_project_id, repository_error, required_trimmed, with_string_project_id};

const DEFAULT_PAGE_SIZE: u32 = 30;
const MAX_TITLE_CHARS: usize = 60;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRequest {
    project_id: String,
    client_request_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadScope {
    project_id: String,
    thread_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadCursorRequest {
    last_activity_at: i64,
    thread_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListRequest {
    project_id: String,
    #[serde(default)]
    archived: bool,
    cursor: Option<ThreadCursorRequest>,
    #[serde(default = "default_page_size")]
    limit: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRequest {
    project_id: String,
    thread_id: String,
    title: String,
    expected_version: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveRequest {
    project_id: String,
    thread_id: String,
    archived: bool,
    expected_version: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsRequest {
    project_id: String,
    thread_id: String,
    #[serde(default)]
    after_thread_sequence: u64,
    #[serde(default = "default_page_size")]
    limit: u32,
}

pub(crate) async fn create(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let request: CreateRequest = match parse_body(body, "Thread 创建") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let project_id = match parse_project_id(&request.project_id) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let client_request_id = match required_trimmed(&request.client_request_id, "clientRequestId") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let repository = SqliteThreadRepository::new(db.pool.clone());
    match repository
        .create_thread(CreateThreadInput { project_id, create_request_id: client_request_id, title: None, created_at: now_ms() })
        .await
    {
        Ok(thread) => Ok(ok(with_string_project_id(thread, project_id))),
        Err(error) => Ok(repository_error(error)),
    }
}

pub(crate) async fn list(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let request: ListRequest = match parse_body(body, "Thread 列表") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let project_id = match parse_project_id(&request.project_id) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let cursor = request
        .cursor
        .map(|cursor| ThreadCursor { last_activity_at: cursor.last_activity_at, thread_id: cursor.thread_id });
    let repository = SqliteThreadRepository::new(db.pool.clone());
    match repository
        .list_threads(ListThreadsQuery { project_id, archived: request.archived, cursor, limit: request.limit })
        .await
    {
        Ok(page) => {
            let data = page
                .data
                .into_iter()
                .map(|thread| with_string_project_id(thread, project_id))
                .collect::<Vec<_>>();
            Ok(ok(json!({ "data": data, "nextCursor": page.next_cursor })))
        }
        Err(error) => Ok(repository_error(error)),
    }
}

pub(crate) async fn read(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let request: ThreadScope = match parse_body(body, "Thread 读取") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let (project_id, thread_id) = match scope(request.project_id, request.thread_id) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let repository = SqliteThreadRepository::new(db.pool.clone());
    match repository.find_thread(project_id, &thread_id).await {
        Ok(Some(thread)) => Ok(ok(with_string_project_id(thread, project_id))),
        Ok(None) => Ok(err(404, "Thread 不存在")),
        Err(error) => Ok(repository_error(error)),
    }
}

pub(crate) async fn update(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let request: UpdateRequest = match parse_body(body, "Thread 重命名") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let (project_id, thread_id) = match scope(request.project_id, request.thread_id) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let title = match required_trimmed(&request.title, "title") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    if title.chars().count() > MAX_TITLE_CHARS {
        return Ok(err(400, format!("Thread 标题不能超过 {MAX_TITLE_CHARS} 个字符")));
    }
    let repository = SqliteThreadRepository::new(db.pool.clone());
    match repository
        .update_thread_title(UpdateThreadTitleInput {
            project_id,
            thread_id,
            title,
            expected_version: request.expected_version,
            updated_at: now_ms(),
        })
        .await
    {
        Ok(thread) => Ok(ok(with_string_project_id(thread, project_id))),
        Err(error) => Ok(repository_error(error)),
    }
}

pub(crate) async fn archive(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let request: ArchiveRequest = match parse_body(body, "Thread 归档") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let (project_id, thread_id) = match scope(request.project_id, request.thread_id) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let repository = SqliteThreadRepository::new(db.pool.clone());
    match repository
        .set_thread_archived(SetThreadArchivedInput {
            project_id,
            thread_id,
            archived: request.archived,
            expected_version: request.expected_version,
            updated_at: now_ms(),
        })
        .await
    {
        Ok(thread) => Ok(ok(with_string_project_id(thread, project_id))),
        Err(error) => Ok(repository_error(error)),
    }
}

pub(crate) async fn events(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let request: EventsRequest = match parse_body(body, "Thread 事件") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let (project_id, thread_id) = match scope(request.project_id, request.thread_id) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let repository = SqliteThreadRepository::new(db.pool.clone());
    match repository
        .list_thread_events(ListThreadEventsQuery {
            project_id,
            thread_id,
            after_thread_sequence: request.after_thread_sequence,
            limit: request.limit,
        })
        .await
    {
        Ok(page) => Ok(ok(json!({ "data": page.data, "nextCursor": page.next_cursor }))),
        Err(error) => Ok(repository_error(error)),
    }
}

fn scope(project_id: String, thread_id: String) -> Result<(i64, String), HttpResponse> {
    Ok((parse_project_id(&project_id)?, required_trimmed(&thread_id, "threadId")?))
}

fn default_page_size() -> u32 {
    DEFAULT_PAGE_SIZE
}

fn now_ms() -> i64 {
    crate::backend::ai_driven::time::now_ms()
}
