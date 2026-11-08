/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Thread 创建、重命名、归档与恢复。

use sqlx::{Row, Sqlite, Transaction};

use crate::backend::ai_driven::thread::error::ThreadRepositoryError;
use crate::backend::ai_driven::thread::id::new_thread_id;
use crate::backend::ai_driven::thread::lifecycle::ensure_thread_transition;
use crate::backend::ai_driven::thread::state::{AgentThread, CreateThreadInput, SetThreadArchivedInput, ThreadStatus, UpdateThreadTitleInput};

use super::{thread_from_row, SqliteThreadRepository};

const THREAD_SELECT: &str = concat!(
    "SELECT thread_id, project_id, title, title_source, preview, status, create_request_id, row_version, created_at, updated_at, ",
    "last_activity_at, archived_at FROM ai_agent_thread "
);
const MAX_THREAD_TITLE_CHARS: usize = 60;

impl SqliteThreadRepository {
    pub(crate) async fn create_thread(&self, input: CreateThreadInput) -> Result<AgentThread, ThreadRepositoryError> {
        validate_create_thread(&input)?;
        let mut transaction = self.pool.begin().await?;
        ensure_project(&mut transaction, input.project_id).await?;
        if let Some(existing) = find_by_create_request(&mut transaction, input.project_id, &input.create_request_id).await? {
            transaction.commit().await?;
            return Ok(existing);
        }

        let thread_id = new_thread_id();
        let result = sqlx::query(
            "INSERT INTO ai_agent_thread (\
                thread_id, project_id, title, title_source, preview, status, create_request_id, next_turn_sequence, next_event_sequence, \
                row_version, created_at, updated_at, last_activity_at, archived_at\
             ) VALUES (?1, ?2, ?3, 'auto', NULL, 'active', ?4, 1, 1, 1, ?5, ?5, ?5, NULL)",
        )
        .bind(&thread_id)
        .bind(input.project_id)
        .bind(input.title.as_deref().map(str::trim))
        .bind(&input.create_request_id)
        .bind(input.created_at)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = result {
            if is_unique_violation(&error) {
                if let Some(existing) = find_by_create_request(&mut transaction, input.project_id, &input.create_request_id).await? {
                    transaction.commit().await?;
                    return Ok(existing);
                }
            }
            return Err(error.into());
        }
        let created = find_thread_record(&mut transaction, input.project_id, &thread_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "thread", key: thread_id })?;
        transaction.commit().await?;
        Ok(created)
    }

    pub(crate) async fn update_thread_title(&self, input: UpdateThreadTitleInput) -> Result<AgentThread, ThreadRepositoryError> {
        validate_title(Some(&input.title))?;
        let expected_version = i64::try_from(input.expected_version)
            .map_err(|_| ThreadRepositoryError::InvalidRecord("thread version exceeds SQLite INTEGER".to_string()))?;
        let result = sqlx::query(
            "UPDATE ai_agent_thread SET title = ?1, title_source = 'user', row_version = row_version + 1, updated_at = ?2 \
             WHERE project_id = ?3 AND thread_id = ?4 AND row_version = ?5 \
               AND EXISTS (SELECT 1 FROM project active_project \
                           WHERE active_project.id = ai_agent_thread.project_id AND active_project.deleted = 0)",
        )
        .bind(input.title.trim())
        .bind(input.updated_at)
        .bind(input.project_id)
        .bind(&input.thread_id)
        .bind(expected_version)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return self
                .thread_write_miss(input.project_id, &input.thread_id, input.expected_version)
                .await;
        }
        self.find_thread_record(input.project_id, &input.thread_id).await
    }

    pub(crate) async fn set_thread_archived(&self, input: SetThreadArchivedInput) -> Result<AgentThread, ThreadRepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let current = find_thread_record(&mut transaction, input.project_id, &input.thread_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "thread", key: input.thread_id.clone() })?;
        if current.row_version != input.expected_version {
            return Err(ThreadRepositoryError::conflict("agent.thread.version_conflict", "Thread 已被其他操作更新"));
        }
        let next = if input.archived { ThreadStatus::Archived } else { ThreadStatus::Active };
        ensure_thread_transition(current.status, next)?;
        if current.status == next {
            transaction.commit().await?;
            return Ok(current);
        }
        if input.archived {
            let active_turns = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(1) FROM ai_agent_turn WHERE thread_id = ?1 \
                 AND status IN ('created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation')",
            )
            .bind(&input.thread_id)
            .fetch_one(&mut *transaction)
            .await?;
            if active_turns > 0 {
                return Err(ThreadRepositoryError::conflict("agent.thread.active_turn", "存在活动 Turn，不能归档 Thread"));
            }
        }
        let archived_at = input.archived.then_some(input.updated_at);
        let result = sqlx::query(
            "UPDATE ai_agent_thread SET status = ?1, archived_at = ?2, row_version = row_version + 1, updated_at = ?3 \
             WHERE project_id = ?4 AND thread_id = ?5 AND row_version = ?6 AND status = ?7 \
             AND EXISTS (SELECT 1 FROM project active_project \
                         WHERE active_project.id = ai_agent_thread.project_id AND active_project.deleted = 0) \
             AND (?1 = 'active' OR NOT EXISTS (SELECT 1 FROM ai_agent_turn active \
                  WHERE active.thread_id = ai_agent_thread.thread_id \
                  AND active.status IN ('created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation')))",
        )
        .bind(next.as_str())
        .bind(archived_at)
        .bind(input.updated_at)
        .bind(input.project_id)
        .bind(&input.thread_id)
        .bind(i64::try_from(input.expected_version).unwrap_or(i64::MAX))
        .bind(current.status.as_str())
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return Err(ThreadRepositoryError::conflict("agent.thread.concurrent_update", "Thread 归档状态已变化"));
        }
        let updated = find_thread_record(&mut transaction, input.project_id, &input.thread_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "thread", key: input.thread_id })?;
        transaction.commit().await?;
        Ok(updated)
    }

    async fn find_thread_record(&self, project_id: i64, thread_id: &str) -> Result<AgentThread, ThreadRepositoryError> {
        let query = format!(
            "{THREAD_SELECT} WHERE project_id = ?1 AND thread_id = ?2 \
             AND EXISTS (SELECT 1 FROM project active_project \
                         WHERE active_project.id = ai_agent_thread.project_id AND active_project.deleted = 0)"
        );
        sqlx::query(&query)
            .bind(project_id)
            .bind(thread_id)
            .fetch_optional(&self.pool)
            .await?
            .map(thread_from_row)
            .transpose()?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "thread", key: thread_id.to_string() })
    }

    async fn thread_write_miss<T>(&self, project_id: i64, thread_id: &str, expected_version: u64) -> Result<T, ThreadRepositoryError> {
        let row = sqlx::query(
            "SELECT thread.project_id, thread.row_version FROM ai_agent_thread thread \
             JOIN project active_project ON active_project.id = thread.project_id AND active_project.deleted = 0 \
             WHERE thread.thread_id = ?1",
        )
        .bind(thread_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Err(ThreadRepositoryError::NotFound { entity: "thread", key: thread_id.to_string() });
        };
        if row.try_get::<i64, _>("project_id")? != project_id {
            return Err(ThreadRepositoryError::ScopeMismatch { entity: "thread", key: thread_id.to_string() });
        }
        Err(ThreadRepositoryError::conflict(
            "agent.thread.version_conflict",
            format!("Thread 版本已变化，expected={expected_version}"),
        ))
    }
}

async fn ensure_project(transaction: &mut Transaction<'_, Sqlite>, project_id: i64) -> Result<(), ThreadRepositoryError> {
    let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM project WHERE id = ?1 AND deleted = 0")
        .bind(project_id)
        .fetch_one(&mut **transaction)
        .await?;
    if exists == 1 {
        Ok(())
    } else {
        Err(ThreadRepositoryError::NotFound { entity: "project", key: project_id.to_string() })
    }
}

async fn find_by_create_request(
    transaction: &mut Transaction<'_, Sqlite>,
    project_id: i64,
    create_request_id: &str,
) -> Result<Option<AgentThread>, ThreadRepositoryError> {
    let query = format!("{THREAD_SELECT} WHERE project_id = ?1 AND create_request_id = ?2");
    sqlx::query(&query)
        .bind(project_id)
        .bind(create_request_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(thread_from_row)
        .transpose()
}

async fn find_thread_record(
    transaction: &mut Transaction<'_, Sqlite>,
    project_id: i64,
    thread_id: &str,
) -> Result<Option<AgentThread>, ThreadRepositoryError> {
    let query = format!(
        "{THREAD_SELECT} WHERE project_id = ?1 AND thread_id = ?2 \
         AND EXISTS (SELECT 1 FROM project active_project \
                     WHERE active_project.id = ai_agent_thread.project_id AND active_project.deleted = 0)"
    );
    sqlx::query(&query)
        .bind(project_id)
        .bind(thread_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(thread_from_row)
        .transpose()
}

fn validate_create_thread(input: &CreateThreadInput) -> Result<(), ThreadRepositoryError> {
    if input.project_id <= 0 {
        return Err(ThreadRepositoryError::InvalidRecord("project_id must be positive".to_string()));
    }
    if input.create_request_id.trim().is_empty() {
        return Err(ThreadRepositoryError::InvalidRecord("create_request_id must not be blank".to_string()));
    }
    validate_title(input.title.as_deref())?;
    Ok(())
}

fn validate_title(title: Option<&str>) -> Result<(), ThreadRepositoryError> {
    let Some(title) = title else {
        return Ok(());
    };
    let title = title.trim();
    if title.is_empty() {
        return Err(ThreadRepositoryError::InvalidRecord("thread title must not be blank".to_string()));
    }
    if title.chars().count() > MAX_THREAD_TITLE_CHARS {
        return Err(ThreadRepositoryError::InvalidRecord(format!(
            "thread title must not exceed {MAX_THREAD_TITLE_CHARS} characters"
        )));
    }
    Ok(())
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
}
