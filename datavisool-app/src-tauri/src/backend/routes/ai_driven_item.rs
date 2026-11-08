/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Item 最终快照查询接口。

use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::ai_driven::thread::{ListItemsQuery, SqliteThreadRepository};
use crate::backend::types::{err, ok, HttpResponse};
use crate::db::DbState;

use super::ai_driven_history_common::{parse_body, parse_project_id, repository_error, required_trimmed};

const DEFAULT_PAGE_SIZE: u32 = 50;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListRequest {
    project_id: String,
    thread_id: String,
    turn_id: String,
    cursor: Option<u64>,
    #[serde(default = "default_page_size")]
    limit: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadRequest {
    project_id: String,
    thread_id: String,
    turn_id: String,
    item_id: String,
}

pub(crate) async fn list(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let request: ListRequest = match parse_body(body, "Item 列表") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let (project_id, thread_id, turn_id) = match scope(&request.project_id, &request.thread_id, &request.turn_id) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let repository = SqliteThreadRepository::new(db.pool.clone());
    match repository
        .list_items(ListItemsQuery {
            project_id,
            thread_id,
            turn_id,
            after_item_sequence: request.cursor.unwrap_or(0),
            limit: request.limit,
        })
        .await
    {
        Ok(page) => Ok(ok(json!({ "data": page.data, "nextCursor": page.next_cursor }))),
        Err(error) => Ok(repository_error(error)),
    }
}

pub(crate) async fn read(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let request: ReadRequest = match parse_body(body, "Item 读取") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let (project_id, thread_id, turn_id) = match scope(&request.project_id, &request.thread_id, &request.turn_id) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let item_id = match required_trimmed(&request.item_id, "itemId") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let repository = SqliteThreadRepository::new(db.pool.clone());
    match repository.find_item(project_id, &thread_id, &turn_id, &item_id).await {
        Ok(Some(item)) => Ok(ok(json!(item))),
        Ok(None) => Ok(err(404, "Item 不存在")),
        Err(error) => Ok(repository_error(error)),
    }
}

fn scope(project_id: &str, thread_id: &str, turn_id: &str) -> Result<(i64, String, String), HttpResponse> {
    Ok((parse_project_id(project_id)?, required_trimmed(thread_id, "threadId")?, required_trimmed(turn_id, "turnId")?))
}

fn default_page_size() -> u32 {
    DEFAULT_PAGE_SIZE
}
