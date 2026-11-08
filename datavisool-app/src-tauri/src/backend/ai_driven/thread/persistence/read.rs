/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Thread、Turn、Item、TaskRun 与 Event 的作用域读取。

use sqlx::Row;

use crate::backend::ai_driven::thread::error::ThreadRepositoryError;
use crate::backend::ai_driven::thread::state::{
    AgentItem, AgentTaskRunRef, AgentThreadSummary, AgentTimelineEvent, AgentTurn, ListItemsQuery, ListRunEventsQuery, ListThreadEventsQuery,
    ListThreadsQuery, ListTurnsQuery, ThreadCursor, ThreadPage,
};

use super::{event_from_row, item_from_row, run_from_row, thread_summary_from_row, turn_from_row, SqliteThreadRepository};

const THREAD_SUMMARY_SELECT: &str = concat!(
    "SELECT th.thread_id, th.project_id, th.title, th.title_source, th.preview, th.status, th.create_request_id, ",
    "th.row_version, th.created_at, th.updated_at, th.last_activity_at, th.archived_at, ",
    "active.status AS runtime_status, active.turn_id AS active_turn_id, ",
    "(SELECT r.run_id FROM ai_agent_task_run r WHERE r.turn_id = active.turn_id ORDER BY r.attempt_no DESC LIMIT 1) AS current_run_id ",
    "FROM ai_agent_thread th LEFT JOIN ai_agent_turn active ON active.turn_id = (",
    "SELECT candidate.turn_id FROM ai_agent_turn candidate WHERE candidate.thread_id = th.thread_id ",
    "AND candidate.status IN ('created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation') LIMIT 1) "
);

const TURN_SELECT: &str = concat!(
    "SELECT turn_id, project_id, thread_id, turn_sequence, request_id, status, request_json, COALESCE(input_summary, '') AS input_summary, model_config_id, ",
    "runtime_model_id, provider, entry_digest, usage_json, last_artifact_id, last_artifact_revision, error_code, error_message, ",
    "row_version, created_at, started_at, updated_at, ended_at, ",
    "(SELECT latest_run.limits_json FROM ai_agent_task_run latest_run WHERE latest_run.turn_id = ai_agent_turn.turn_id ",
    "ORDER BY latest_run.attempt_no DESC LIMIT 1) AS limits_json FROM ai_agent_turn "
);

const ITEM_SELECT: &str = concat!(
    "SELECT item_id, turn_id, item_sequence, source_run_id, logical_key, item_type, status, schema_version, payload_json, ",
    "last_event_sequence, row_version, created_at, updated_at, completed_at FROM ai_agent_item "
);

const EVENT_SELECT: &str = concat!(
    "SELECT id, thread_id, turn_id, item_id, run_id, sequence, thread_sequence, kind, payload_json, dedupe_key, schema_version, created_at ",
    "FROM ai_agent_task_event "
);

const RUN_SELECT: &str = concat!(
    "SELECT r.task_id, r.run_id, r.request_id, turn.entry_digest, turn.project_id, turn.thread_id, r.turn_id, r.attempt_no, r.client_session_id, ",
    "r.status, r.task_type, r.limits_json, r.pending_json, r.usage_json, r.last_artifact_id, r.last_artifact_revision, ",
    "r.error_code, r.error_message, r.project_snapshot_ref, r.rig_version, r.rig_snapshot_ref, r.created_at, r.started_at, r.updated_at, r.ended_at ",
    "FROM ai_agent_task_run r JOIN ai_agent_turn turn ON turn.turn_id = r.turn_id ",
    "JOIN project active_project ON active_project.id = turn.project_id AND active_project.deleted = 0 "
);

impl SqliteThreadRepository {
    pub(crate) async fn find_thread(&self, project_id: i64, thread_id: &str) -> Result<Option<AgentThreadSummary>, ThreadRepositoryError> {
        let query = format!(
            "{THREAD_SUMMARY_SELECT} WHERE th.project_id = ?1 AND th.thread_id = ?2 \
             AND EXISTS (SELECT 1 FROM project active_project WHERE active_project.id = th.project_id AND active_project.deleted = 0)"
        );
        sqlx::query(&query)
            .bind(project_id)
            .bind(thread_id)
            .fetch_optional(&self.pool)
            .await?
            .map(thread_summary_from_row)
            .transpose()
    }

    pub(crate) async fn list_threads(&self, query: ListThreadsQuery) -> Result<ThreadPage<AgentThreadSummary, ThreadCursor>, ThreadRepositoryError> {
        let status = if query.archived { "archived" } else { "active" };
        let cursor_time = query.cursor.as_ref().map(|cursor| cursor.last_activity_at);
        let cursor_id = query.cursor.as_ref().map(|cursor| cursor.thread_id.as_str());
        let sql = format!(
            "{THREAD_SUMMARY_SELECT} WHERE th.project_id = ?1 AND th.status = ?2 \
             AND EXISTS (SELECT 1 FROM project active_project WHERE active_project.id = th.project_id AND active_project.deleted = 0) \
             AND EXISTS (SELECT 1 FROM ai_agent_turn listed_turn WHERE listed_turn.thread_id = th.thread_id) \
             AND (?3 IS NULL OR th.last_activity_at < ?3 OR (th.last_activity_at = ?3 AND th.thread_id > ?4)) \
             ORDER BY th.last_activity_at DESC, th.thread_id ASC LIMIT ?5"
        );
        let limit = query.limit.clamp(1, 100);
        let rows = sqlx::query(&sql)
            .bind(query.project_id)
            .bind(status)
            .bind(cursor_time)
            .bind(cursor_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?;
        let data = rows.into_iter().map(thread_summary_from_row).collect::<Result<Vec<_>, _>>()?;
        let next_cursor = (data.len() == limit as usize).then(|| {
            let last = data.last().expect("non-empty full page");
            ThreadCursor { last_activity_at: last.thread.last_activity_at, thread_id: last.thread.thread_id.clone() }
        });
        Ok(ThreadPage { data, next_cursor })
    }

    pub(crate) async fn find_turn(&self, project_id: i64, thread_id: &str, turn_id: &str) -> Result<Option<AgentTurn>, ThreadRepositoryError> {
        let query = format!(
            "{TURN_SELECT} WHERE project_id = ?1 AND thread_id = ?2 AND turn_id = ?3 \
             AND EXISTS (SELECT 1 FROM project active_project WHERE active_project.id = project_id AND active_project.deleted = 0)"
        );
        sqlx::query(&query)
            .bind(project_id)
            .bind(thread_id)
            .bind(turn_id)
            .fetch_optional(&self.pool)
            .await?
            .map(turn_from_row)
            .transpose()
    }

    pub(crate) async fn list_turns(&self, query: ListTurnsQuery) -> Result<ThreadPage<AgentTurn, u64>, ThreadRepositoryError> {
        self.ensure_thread_read_scope(query.project_id, &query.thread_id).await?;
        let sql = format!("{TURN_SELECT} WHERE project_id = ?1 AND thread_id = ?2 AND turn_sequence > ?3 ORDER BY turn_sequence ASC LIMIT ?4");
        let limit = query.limit.clamp(1, 100);
        let rows = sqlx::query(&sql)
            .bind(query.project_id)
            .bind(query.thread_id)
            .bind(i64::try_from(query.after_turn_sequence).unwrap_or(i64::MAX))
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?;
        let data = rows.into_iter().map(turn_from_row).collect::<Result<Vec<_>, _>>()?;
        let next_cursor = (data.len() == limit as usize).then(|| data.last().expect("non-empty full page").turn_sequence);
        Ok(ThreadPage { data, next_cursor })
    }

    pub(crate) async fn find_item(
        &self,
        project_id: i64,
        thread_id: &str,
        turn_id: &str,
        item_id: &str,
    ) -> Result<Option<AgentItem>, ThreadRepositoryError> {
        self.ensure_turn_read_scope(project_id, thread_id, turn_id).await?;
        let query = format!("{ITEM_SELECT} WHERE turn_id = ?1 AND item_id = ?2");
        sqlx::query(&query)
            .bind(turn_id)
            .bind(item_id)
            .fetch_optional(&self.pool)
            .await?
            .map(item_from_row)
            .transpose()
    }

    pub(crate) async fn list_items(&self, query: ListItemsQuery) -> Result<ThreadPage<AgentItem, u64>, ThreadRepositoryError> {
        self.ensure_turn_read_scope(query.project_id, &query.thread_id, &query.turn_id)
            .await?;
        let sql = format!("{ITEM_SELECT} WHERE turn_id = ?1 AND item_sequence > ?2 ORDER BY item_sequence ASC LIMIT ?3");
        let limit = query.limit.clamp(1, 200);
        let rows = sqlx::query(&sql)
            .bind(query.turn_id)
            .bind(i64::try_from(query.after_item_sequence).unwrap_or(i64::MAX))
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?;
        let data = rows.into_iter().map(item_from_row).collect::<Result<Vec<_>, _>>()?;
        let next_cursor = (data.len() == limit as usize).then(|| data.last().expect("non-empty full page").item_sequence);
        Ok(ThreadPage { data, next_cursor })
    }

    pub(crate) async fn list_thread_events(
        &self,
        query: ListThreadEventsQuery,
    ) -> Result<ThreadPage<AgentTimelineEvent, u64>, ThreadRepositoryError> {
        self.ensure_thread_read_scope(query.project_id, &query.thread_id).await?;
        let sql = format!("{EVENT_SELECT} WHERE thread_id = ?1 AND thread_sequence > ?2 ORDER BY thread_sequence ASC LIMIT ?3");
        let limit = query.limit.clamp(1, 500);
        let rows = sqlx::query(&sql)
            .bind(query.thread_id)
            .bind(i64::try_from(query.after_thread_sequence).unwrap_or(i64::MAX))
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?;
        event_page(rows, limit, |event| event.thread_sequence)
    }

    pub(crate) async fn list_run_events(&self, query: ListRunEventsQuery) -> Result<ThreadPage<AgentTimelineEvent, u64>, ThreadRepositoryError> {
        self.ensure_run_read_scope(query.project_id, &query.thread_id, &query.turn_id, &query.run_id)
            .await?;
        let sql = format!("{EVENT_SELECT} WHERE run_id = ?1 AND sequence > ?2 ORDER BY sequence ASC LIMIT ?3");
        let limit = query.limit.clamp(1, 500);
        let rows = sqlx::query(&sql)
            .bind(query.run_id)
            .bind(i64::try_from(query.after_sequence).unwrap_or(i64::MAX))
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?;
        event_page(rows, limit, |event| event.sequence)
    }

    pub(crate) async fn find_task_run_by_run_id(&self, run_id: &str) -> Result<Option<AgentTaskRunRef>, ThreadRepositoryError> {
        let query = format!("{RUN_SELECT} WHERE r.run_id = ?1");
        sqlx::query(&query)
            .bind(run_id)
            .fetch_optional(&self.pool)
            .await?
            .map(run_from_row)
            .transpose()
    }

    pub(crate) async fn find_task_run_by_request_id(&self, request_id: &str) -> Result<Option<AgentTaskRunRef>, ThreadRepositoryError> {
        let query = format!("{RUN_SELECT} WHERE r.request_id = ?1 ORDER BY r.attempt_no DESC LIMIT 1");
        sqlx::query(&query)
            .bind(request_id)
            .fetch_optional(&self.pool)
            .await?
            .map(run_from_row)
            .transpose()
    }

    pub(super) async fn ensure_thread_read_scope(&self, project_id: i64, thread_id: &str) -> Result<(), ThreadRepositoryError> {
        let stored_project = sqlx::query_scalar::<_, i64>(
            "SELECT thread.project_id FROM ai_agent_thread thread \
             JOIN project active_project ON active_project.id = thread.project_id AND active_project.deleted = 0 \
             WHERE thread.thread_id = ?1",
        )
        .bind(thread_id)
        .fetch_optional(&self.pool)
        .await?;
        match stored_project {
            None => Err(ThreadRepositoryError::NotFound { entity: "thread", key: thread_id.to_string() }),
            Some(stored_project) if stored_project == project_id => Ok(()),
            Some(_) => Err(ThreadRepositoryError::ScopeMismatch { entity: "thread", key: thread_id.to_string() }),
        }
    }

    pub(super) async fn ensure_turn_read_scope(&self, project_id: i64, thread_id: &str, turn_id: &str) -> Result<(), ThreadRepositoryError> {
        let row = sqlx::query(
            "SELECT turn.project_id, turn.thread_id FROM ai_agent_turn turn \
             JOIN project active_project ON active_project.id = turn.project_id AND active_project.deleted = 0 \
             WHERE turn.turn_id = ?1",
        )
        .bind(turn_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Err(ThreadRepositoryError::NotFound { entity: "turn", key: turn_id.to_string() });
        };
        let stored_project: i64 = row.try_get("project_id")?;
        let stored_thread: String = row.try_get("thread_id")?;
        if stored_project == project_id && stored_thread == thread_id {
            Ok(())
        } else {
            Err(ThreadRepositoryError::ScopeMismatch { entity: "turn", key: turn_id.to_string() })
        }
    }

    async fn ensure_run_read_scope(&self, project_id: i64, thread_id: &str, turn_id: &str, run_id: &str) -> Result<(), ThreadRepositoryError> {
        self.ensure_turn_read_scope(project_id, thread_id, turn_id).await?;
        let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM ai_agent_task_run WHERE turn_id = ?1 AND run_id = ?2")
            .bind(turn_id)
            .bind(run_id)
            .fetch_one(&self.pool)
            .await?;
        if exists == 1 {
            Ok(())
        } else {
            Err(ThreadRepositoryError::ScopeMismatch { entity: "task run", key: run_id.to_string() })
        }
    }
}

fn event_page(
    rows: Vec<sqlx::sqlite::SqliteRow>,
    limit: u32,
    cursor: impl Fn(&AgentTimelineEvent) -> u64,
) -> Result<ThreadPage<AgentTimelineEvent, u64>, ThreadRepositoryError> {
    let data = rows.into_iter().map(event_from_row).collect::<Result<Vec<_>, _>>()?;
    let next_cursor = (data.len() == limit as usize).then(|| cursor(data.last().expect("non-empty full page")));
    Ok(ThreadPage { data, next_cursor })
}
