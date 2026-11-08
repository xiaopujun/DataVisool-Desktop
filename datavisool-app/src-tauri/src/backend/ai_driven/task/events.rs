/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 持久化 Timeline Event 到 Tauri 增量通知的稳定协议边界。

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::backend::ai_driven::thread::AgentTimelineEvent;

pub(crate) const AI_DRIVEN_TASK_EVENT_CHANNEL: &str = "datavisool:ai-driven-task-event";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiDrivenTaskEventEnvelope {
    pub event_id: String,
    pub thread_id: String,
    pub turn_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    pub run_id: String,
    pub request_id: String,
    pub project_id: String,
    pub sequence: u64,
    pub thread_sequence: u64,
    pub kind: String,
    pub payload: Value,
    pub dedupe_key: String,
    pub schema_version: u32,
    pub created_at: i64,
}

#[derive(Clone)]
pub(crate) struct AgentTaskEventPublisher {
    app_handle: AppHandle,
    request_id: String,
    project_id: String,
}

impl AgentTaskEventPublisher {
    pub(crate) fn new(app_handle: AppHandle, request_id: impl Into<String>, project_id: i64) -> Self {
        Self { app_handle, request_id: request_id.into(), project_id: project_id.to_string() }
    }

    pub(crate) fn envelope(&self, event: AgentTimelineEvent) -> AiDrivenTaskEventEnvelope {
        AiDrivenTaskEventEnvelope::from_event(event, self.request_id.clone(), self.project_id.clone())
    }

    /// 事件已先写入 SQLite；通知失败由前端通过 cursor 回放补齐，不影响 Agent。
    pub(crate) fn publish(&self, event: AgentTimelineEvent) {
        let _ = self.app_handle.emit(AI_DRIVEN_TASK_EVENT_CHANNEL, self.envelope(event));
    }
}

impl AiDrivenTaskEventEnvelope {
    pub(crate) fn from_event(event: AgentTimelineEvent, request_id: String, project_id: String) -> Self {
        Self {
            event_id: event.event_id,
            thread_id: event.thread_id,
            turn_id: event.turn_id,
            item_id: event.item_id,
            run_id: event.run_id,
            request_id,
            project_id,
            sequence: event.sequence,
            thread_sequence: event.thread_sequence,
            kind: event.kind,
            payload: event.payload,
            dedupe_key: event.dedupe_key,
            schema_version: event.schema_version,
            created_at: event.created_at,
        }
    }
}
