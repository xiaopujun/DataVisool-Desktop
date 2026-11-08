/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! SQLite ConversationMemory 后端。

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use super::policy::{ConversationWindow, ConversationWindowPolicy};

pub(crate) type ConversationMemoryFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ConversationMemoryError>> + Send + 'a>>;

#[derive(Debug)]
pub(crate) enum ConversationMemoryError {
    Sql(sqlx::Error),
    Json(serde_json::Error),
    ScopeMismatch(String),
    InvalidRecord(String),
}

impl Display for ConversationMemoryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(error) => write!(formatter, "SQLite conversation memory failed: {error}"),
            Self::Json(error) => write!(formatter, "conversation JSON failed: {error}"),
            Self::ScopeMismatch(conversation_id) => {
                write!(formatter, "conversation scope does not match project/thread: {conversation_id}")
            }
            Self::InvalidRecord(message) => formatter.write_str(message),
        }
    }
}

impl Error for ConversationMemoryError {}

impl From<sqlx::Error> for ConversationMemoryError {
    fn from(error: sqlx::Error) -> Self {
        Self::Sql(error)
    }
}

impl From<serde_json::Error> for ConversationMemoryError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ConversationRole {
    User,
    Assistant,
    Tool,
}

impl ConversationRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversationMessage {
    pub message_id: String,
    pub conversation_id: String,
    pub sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_item_id: Option<String>,
    pub role: ConversationRole,
    pub content: Value,
    pub estimated_tokens: u64,
    pub artifact_refs: Vec<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NewConversationMessage {
    pub project_id: i64,
    pub thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_item_id: Option<String>,
    pub role: ConversationRole,
    pub content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<u64>,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversationSummary {
    pub conversation_id: String,
    pub content: Value,
    pub summarized_until_sequence: u64,
    pub estimated_tokens: u64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveConversationSummary {
    pub project_id: i64,
    pub thread_id: String,
    pub content: Value,
    pub summarized_until_sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<u64>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListConversationMessages {
    pub project_id: i64,
    pub thread_id: String,
    pub after_sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub through_sequence: Option<u64>,
    pub limit: u32,
}

pub(crate) trait ConversationMemoryBackend: Send + Sync {
    fn append_message<'a>(&'a self, input: NewConversationMessage) -> ConversationMemoryFuture<'a, ConversationMessage>;
    fn append_messages<'a>(&'a self, inputs: Vec<NewConversationMessage>) -> ConversationMemoryFuture<'a, Vec<ConversationMessage>>;
    fn list_messages<'a>(&'a self, query: ListConversationMessages) -> ConversationMemoryFuture<'a, Vec<ConversationMessage>>;
    fn load_summary<'a>(&'a self, project_id: i64, thread_id: String) -> ConversationMemoryFuture<'a, Option<ConversationSummary>>;
    fn save_summary<'a>(&'a self, input: SaveConversationSummary) -> ConversationMemoryFuture<'a, ConversationSummary>;
    fn load_window<'a>(
        &'a self,
        project_id: i64,
        thread_id: String,
        policy: ConversationWindowPolicy,
    ) -> ConversationMemoryFuture<'a, ConversationWindow>;
    fn load_turn_window<'a>(
        &'a self,
        project_id: i64,
        thread_id: String,
        turn_id: String,
        policy: ConversationWindowPolicy,
    ) -> ConversationMemoryFuture<'a, ConversationWindow>;
    fn clear<'a>(&'a self, project_id: i64, thread_id: String) -> ConversationMemoryFuture<'a, ()>;
    fn clear_turn<'a>(&'a self, project_id: i64, thread_id: String, turn_id: String) -> ConversationMemoryFuture<'a, ()>;
}

#[derive(Clone)]
pub(crate) struct SqliteConversationMemoryBackend {
    pub(crate) pool: SqlitePool,
}

impl SqliteConversationMemoryBackend {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn append_message_inner(&self, input: NewConversationMessage) -> Result<ConversationMessage, ConversationMemoryError> {
        self.append_messages_inner(vec![input])
            .await?
            .pop()
            .ok_or_else(|| ConversationMemoryError::InvalidRecord("message batch was unexpectedly empty".to_string()))
    }

    async fn append_messages_inner(&self, inputs: Vec<NewConversationMessage>) -> Result<Vec<ConversationMessage>, ConversationMemoryError> {
        let Some(first) = inputs.first() else {
            return Ok(Vec::new());
        };
        let project_id = first.project_id;
        let conversation_id = first.thread_id.clone();
        for input in &inputs {
            validate_message(input)?;
            if input.project_id != project_id || input.thread_id != conversation_id {
                return Err(ConversationMemoryError::InvalidRecord(
                    "all messages in one batch must share project_id and thread_id".to_string(),
                ));
            }
        }
        let mut transaction = self.pool.begin().await?;
        ensure_conversation(&mut transaction, project_id, &conversation_id, first.created_at).await?;

        let mut stored = Vec::with_capacity(inputs.len());
        for input in inputs {
            validate_message_scope(&mut transaction, &input).await?;
            let content_json = serde_json::to_string(&input.content)?;
            let artifact_refs_json = serde_json::to_string(&input.artifact_refs)?;
            let estimated_tokens = input.estimated_tokens.unwrap_or_else(|| estimate_tokens(&content_json));
            let estimated_tokens_i64 = to_i64(estimated_tokens, "message token estimate")?;
            if let Some(source_item_id) = input.source_item_id.as_deref() {
                if let Some(existing) = find_message_by_source_item(&mut transaction, source_item_id).await? {
                    ensure_projected_message_matches(&existing, &input, estimated_tokens)?;
                    stored.push(existing);
                    continue;
                }
            }
            let sequence = sqlx::query_scalar::<_, i64>(
                "UPDATE ai_agent_conversation SET \
                    next_message_sequence = next_message_sequence + 1, updated_at = MAX(updated_at, ?1) \
                 WHERE conversation_id = ?2 AND thread_id = ?2 AND project_id = ?3 \
                 RETURNING next_message_sequence - 1",
            )
            .bind(input.created_at)
            .bind(&conversation_id)
            .bind(&project_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| ConversationMemoryError::ScopeMismatch(conversation_id.clone()))?;

            let message_id = format!("{conversation_id}:{sequence}");
            sqlx::query(
                "INSERT INTO ai_agent_conversation_message (\
                    id, conversation_id, sequence, turn_id, source_item_id, role, content_json, estimated_tokens, artifact_refs_json, created_at\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .bind(&message_id)
            .bind(&conversation_id)
            .bind(sequence)
            .bind(input.turn_id.as_deref())
            .bind(input.source_item_id.as_deref())
            .bind(input.role.as_str())
            .bind(content_json)
            .bind(estimated_tokens_i64)
            .bind(artifact_refs_json)
            .bind(input.created_at)
            .execute(&mut *transaction)
            .await?;

            stored.push(ConversationMessage {
                message_id,
                conversation_id: conversation_id.clone(),
                sequence: to_u64(sequence, "message sequence")?,
                turn_id: input.turn_id,
                source_item_id: input.source_item_id,
                role: input.role,
                content: input.content,
                estimated_tokens,
                artifact_refs: input.artifact_refs,
                created_at: input.created_at,
            });
        }
        transaction.commit().await?;
        Ok(stored)
    }

    async fn list_messages_inner(&self, query: ListConversationMessages) -> Result<Vec<ConversationMessage>, ConversationMemoryError> {
        if !self.ensure_read_scope(query.project_id, &query.thread_id).await? {
            return Ok(Vec::new());
        }
        let after_sequence = to_i64(query.after_sequence, "after message sequence")?;
        let through_sequence = query
            .through_sequence
            .map(|value| to_i64(value, "through message sequence"))
            .transpose()?
            .unwrap_or(i64::MAX);
        if through_sequence < after_sequence {
            return Err(ConversationMemoryError::InvalidRecord("through_sequence must not be less than after_sequence".to_string()));
        }
        let rows = sqlx::query(
            "SELECT id, conversation_id, sequence, turn_id, source_item_id, role, content_json, estimated_tokens, artifact_refs_json, created_at \
             FROM ai_agent_conversation_message \
             WHERE conversation_id = ?1 AND sequence > ?2 AND sequence <= ?3 \
             ORDER BY sequence ASC LIMIT ?4",
        )
        .bind(query.thread_id)
        .bind(after_sequence)
        .bind(through_sequence)
        .bind(i64::from(query.limit.clamp(1, 10_000)))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(message_from_row).collect()
    }

    async fn load_summary_inner(&self, project_id: i64, thread_id: &str) -> Result<Option<ConversationSummary>, ConversationMemoryError> {
        if !self.ensure_read_scope(project_id, thread_id).await? {
            return Ok(None);
        }
        let row = sqlx::query(
            "SELECT conversation_id, summary_json, summary_until_sequence, summary_estimated_tokens, summary_updated_at \
             FROM ai_agent_conversation WHERE conversation_id = ?1 AND summary_json IS NOT NULL",
        )
        .bind(thread_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(summary_from_row).transpose()
    }

    async fn save_summary_inner(&self, input: SaveConversationSummary) -> Result<ConversationSummary, ConversationMemoryError> {
        validate_summary(&input)?;
        let conversation_id = input.thread_id.clone();
        let summary_json = serde_json::to_string(&input.content)?;
        let estimated_tokens = input.estimated_tokens.unwrap_or_else(|| estimate_tokens(&summary_json));
        let summarized_until = to_i64(input.summarized_until_sequence, "summary sequence")?;
        let estimated_tokens_i64 = to_i64(estimated_tokens, "summary token estimate")?;
        let mut transaction = self.pool.begin().await?;

        ensure_conversation(&mut transaction, input.project_id, &conversation_id, input.updated_at).await?;

        let next_sequence = sqlx::query_scalar::<_, i64>(
            "SELECT next_message_sequence FROM ai_agent_conversation \
             WHERE conversation_id = ?1 AND thread_id = ?1 AND project_id = ?2",
        )
        .bind(&conversation_id)
        .bind(&input.project_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| ConversationMemoryError::ScopeMismatch(conversation_id.clone()))?;
        if summarized_until >= next_sequence {
            return Err(ConversationMemoryError::InvalidRecord(format!(
                "summary sequence {} exceeds latest message sequence {}",
                input.summarized_until_sequence,
                next_sequence - 1
            )));
        }

        sqlx::query(
            "UPDATE ai_agent_conversation SET \
                summary_json = ?1, summary_until_sequence = ?2, summary_estimated_tokens = ?3, \
                summary_updated_at = ?4, updated_at = MAX(updated_at, ?4) \
             WHERE conversation_id = ?5 AND thread_id = ?5 AND project_id = ?6 \
               AND summary_until_sequence <= ?2",
        )
        .bind(summary_json)
        .bind(summarized_until)
        .bind(estimated_tokens_i64)
        .bind(input.updated_at)
        .bind(&conversation_id)
        .bind(&input.project_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;

        self.load_summary_inner(input.project_id, &conversation_id)
            .await?
            .ok_or_else(|| ConversationMemoryError::InvalidRecord("summary was not persisted".to_string()))
    }

    async fn load_window_inner(
        &self,
        project_id: i64,
        thread_id: &str,
        policy: ConversationWindowPolicy,
    ) -> Result<ConversationWindow, ConversationMemoryError> {
        validate_policy(policy)?;
        if !self.ensure_read_scope(project_id, thread_id).await? {
            return Ok(ConversationWindow {
                summary: None,
                messages: Vec::new(),
                estimated_tokens: 0,
                was_truncated: false,
                truncated_through_sequence: None,
            });
        }
        let summary = self.load_summary_inner(project_id, thread_id).await?;
        let summarized_until = summary.as_ref().map_or(0, |summary| summary.summarized_until_sequence);
        let summary_tokens = summary.as_ref().map_or(0, |summary| summary.estimated_tokens);
        let remaining_tokens = policy.max_estimated_tokens.saturating_sub(summary_tokens);
        let summarized_until_i64 = to_i64(summarized_until, "summary sequence")?;

        let available_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM ai_agent_conversation_message \
             WHERE conversation_id = ?1 AND sequence > ?2",
        )
        .bind(thread_id)
        .bind(summarized_until_i64)
        .fetch_one(&self.pool)
        .await?;

        let rows = sqlx::query(
            "SELECT id, conversation_id, sequence, turn_id, source_item_id, role, content_json, estimated_tokens, artifact_refs_json, created_at \
             FROM ai_agent_conversation_message \
             WHERE conversation_id = ?1 AND sequence > ?2 \
             ORDER BY sequence DESC LIMIT ?3",
        )
        .bind(thread_id)
        .bind(summarized_until_i64)
        .bind(i64::from(policy.max_messages))
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::with_capacity(rows.len());
        let mut message_tokens = 0_u64;
        for row in rows {
            let message = message_from_row(row)?;
            let next_total = message_tokens.saturating_add(message.estimated_tokens);
            if next_total > remaining_tokens {
                break;
            }
            message_tokens = next_total;
            messages.push(message);
        }
        messages.reverse();
        let available_count = to_u64(available_count, "available message count")?;
        let was_truncated = available_count > messages.len() as u64;
        let truncated_through_sequence = if was_truncated {
            if let Some(first_retained) = messages.first() {
                Some(first_retained.sequence.saturating_sub(1))
            } else {
                sqlx::query_scalar::<_, i64>(
                    "SELECT next_message_sequence - 1 FROM ai_agent_conversation \
                     WHERE conversation_id = ?1 AND thread_id = ?1 AND project_id = ?2",
                )
                .bind(thread_id)
                .bind(project_id)
                .fetch_optional(&self.pool)
                .await?
                .map(|sequence| to_u64(sequence, "latest message sequence"))
                .transpose()?
            }
        } else {
            None
        };

        Ok(ConversationWindow {
            summary,
            was_truncated,
            truncated_through_sequence,
            messages,
            estimated_tokens: summary_tokens.saturating_add(message_tokens),
        })
    }

    async fn clear_inner(&self, project_id: i64, thread_id: &str) -> Result<(), ConversationMemoryError> {
        if !self.ensure_read_scope(project_id, thread_id).await? {
            return Ok(());
        }
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM ai_agent_conversation_message WHERE conversation_id = ?1 \
             AND EXISTS (SELECT 1 FROM ai_agent_conversation \
                         WHERE conversation_id = ?1 AND project_id = ?2)",
        )
        .bind(thread_id)
        .bind(project_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM ai_agent_conversation WHERE conversation_id = ?1 AND thread_id = ?1 AND project_id = ?2")
            .bind(thread_id)
            .bind(project_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn ensure_read_scope(&self, project_id: i64, thread_id: &str) -> Result<bool, ConversationMemoryError> {
        require_project_id(project_id)?;
        require_text("thread_id", thread_id)?;
        let stored_project =
            sqlx::query_scalar::<_, i64>("SELECT project_id FROM ai_agent_conversation WHERE conversation_id = ?1 AND thread_id = ?1")
                .bind(thread_id)
                .fetch_optional(&self.pool)
                .await?;
        match stored_project {
            None => Ok(false),
            Some(stored_project) if stored_project == project_id => Ok(true),
            Some(_) => Err(ConversationMemoryError::ScopeMismatch(thread_id.to_string())),
        }
    }
}

async fn ensure_conversation(
    transaction: &mut Transaction<'_, Sqlite>,
    project_id: i64,
    thread_id: &str,
    updated_at: i64,
) -> Result<(), ConversationMemoryError> {
    sqlx::query(
        "INSERT INTO ai_agent_conversation (conversation_id, thread_id, project_id, created_at, updated_at) \
         SELECT ?1, ?1, ?2, ?3, ?3 \
         WHERE EXISTS (SELECT 1 FROM ai_agent_thread WHERE thread_id = ?1 AND project_id = ?2) \
         ON CONFLICT(thread_id) DO NOTHING",
    )
    .bind(thread_id)
    .bind(project_id)
    .bind(updated_at)
    .execute(&mut **transaction)
    .await?;
    let stored_project = sqlx::query_scalar::<_, i64>("SELECT project_id FROM ai_agent_conversation WHERE conversation_id = ?1 AND thread_id = ?1")
        .bind(thread_id)
        .fetch_optional(&mut **transaction)
        .await?;
    if stored_project == Some(project_id) {
        Ok(())
    } else {
        Err(ConversationMemoryError::ScopeMismatch(thread_id.to_string()))
    }
}

async fn validate_message_scope(transaction: &mut Transaction<'_, Sqlite>, input: &NewConversationMessage) -> Result<(), ConversationMemoryError> {
    let Some(turn_id) = input.turn_id.as_deref() else {
        return Ok(());
    };
    let turn_exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ai_agent_turn WHERE turn_id = ?1 AND thread_id = ?2 AND project_id = ?3")
        .bind(turn_id)
        .bind(&input.thread_id)
        .bind(input.project_id)
        .fetch_one(&mut **transaction)
        .await?;
    if turn_exists != 1 {
        return Err(ConversationMemoryError::ScopeMismatch(input.thread_id.clone()));
    }
    if let Some(source_item_id) = input.source_item_id.as_deref() {
        let item_exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ai_agent_item WHERE item_id = ?1 AND turn_id = ?2")
            .bind(source_item_id)
            .bind(turn_id)
            .fetch_one(&mut **transaction)
            .await?;
        if item_exists != 1 {
            return Err(ConversationMemoryError::ScopeMismatch(source_item_id.to_string()));
        }
    }
    Ok(())
}

async fn find_message_by_source_item(
    transaction: &mut Transaction<'_, Sqlite>,
    source_item_id: &str,
) -> Result<Option<ConversationMessage>, ConversationMemoryError> {
    sqlx::query(
        "SELECT id, conversation_id, sequence, turn_id, source_item_id, role, content_json, estimated_tokens, artifact_refs_json, created_at \
         FROM ai_agent_conversation_message WHERE source_item_id = ?1",
    )
    .bind(source_item_id)
    .fetch_optional(&mut **transaction)
    .await?
    .map(message_from_row)
    .transpose()
}

fn ensure_projected_message_matches(
    existing: &ConversationMessage,
    input: &NewConversationMessage,
    estimated_tokens: u64,
) -> Result<(), ConversationMemoryError> {
    if existing.conversation_id == input.thread_id
        && existing.turn_id == input.turn_id
        && existing.source_item_id == input.source_item_id
        && existing.role == input.role
        && existing.content == input.content
        && existing.estimated_tokens == estimated_tokens
        && existing.artifact_refs == input.artifact_refs
    {
        Ok(())
    } else {
        Err(ConversationMemoryError::InvalidRecord(format!(
            "source Item projection conflicts with persisted message: {}",
            input.source_item_id.as_deref().unwrap_or_default()
        )))
    }
}

impl ConversationMemoryBackend for SqliteConversationMemoryBackend {
    fn append_message<'a>(&'a self, input: NewConversationMessage) -> ConversationMemoryFuture<'a, ConversationMessage> {
        Box::pin(async move { self.append_message_inner(input).await })
    }

    fn append_messages<'a>(&'a self, inputs: Vec<NewConversationMessage>) -> ConversationMemoryFuture<'a, Vec<ConversationMessage>> {
        Box::pin(async move { self.append_messages_inner(inputs).await })
    }

    fn list_messages<'a>(&'a self, query: ListConversationMessages) -> ConversationMemoryFuture<'a, Vec<ConversationMessage>> {
        Box::pin(async move { self.list_messages_inner(query).await })
    }

    fn load_summary<'a>(&'a self, project_id: i64, thread_id: String) -> ConversationMemoryFuture<'a, Option<ConversationSummary>> {
        Box::pin(async move { self.load_summary_inner(project_id, &thread_id).await })
    }

    fn save_summary<'a>(&'a self, input: SaveConversationSummary) -> ConversationMemoryFuture<'a, ConversationSummary> {
        Box::pin(async move { self.save_summary_inner(input).await })
    }

    fn load_window<'a>(
        &'a self,
        project_id: i64,
        thread_id: String,
        policy: ConversationWindowPolicy,
    ) -> ConversationMemoryFuture<'a, ConversationWindow> {
        Box::pin(async move { self.load_window_inner(project_id, &thread_id, policy).await })
    }

    fn load_turn_window<'a>(
        &'a self,
        project_id: i64,
        thread_id: String,
        turn_id: String,
        policy: ConversationWindowPolicy,
    ) -> ConversationMemoryFuture<'a, ConversationWindow> {
        Box::pin(async move { self.load_turn_window_inner(project_id, &thread_id, &turn_id, policy).await })
    }

    fn clear<'a>(&'a self, project_id: i64, thread_id: String) -> ConversationMemoryFuture<'a, ()> {
        Box::pin(async move { self.clear_inner(project_id, &thread_id).await })
    }

    fn clear_turn<'a>(&'a self, project_id: i64, thread_id: String, turn_id: String) -> ConversationMemoryFuture<'a, ()> {
        Box::pin(async move { self.clear_turn_inner(project_id, &thread_id, &turn_id).await })
    }
}

pub(crate) fn message_from_row(row: SqliteRow) -> Result<ConversationMessage, ConversationMemoryError> {
    let role_text: String = row.try_get("role")?;
    let role = ConversationRole::from_str(&role_text)
        .ok_or_else(|| ConversationMemoryError::InvalidRecord(format!("unknown conversation role: {role_text}")))?;
    Ok(ConversationMessage {
        message_id: row.try_get("id")?,
        conversation_id: row.try_get("conversation_id")?,
        sequence: to_u64(row.try_get("sequence")?, "message sequence")?,
        turn_id: row.try_get("turn_id")?,
        source_item_id: row.try_get("source_item_id")?,
        role,
        content: serde_json::from_str(row.try_get::<String, _>("content_json")?.as_str())?,
        estimated_tokens: to_u64(row.try_get("estimated_tokens")?, "message token estimate")?,
        artifact_refs: serde_json::from_str(row.try_get::<String, _>("artifact_refs_json")?.as_str())?,
        created_at: row.try_get("created_at")?,
    })
}

fn summary_from_row(row: SqliteRow) -> Result<ConversationSummary, ConversationMemoryError> {
    Ok(ConversationSummary {
        conversation_id: row.try_get("conversation_id")?,
        content: serde_json::from_str(row.try_get::<String, _>("summary_json")?.as_str())?,
        summarized_until_sequence: to_u64(row.try_get("summary_until_sequence")?, "summary sequence")?,
        estimated_tokens: to_u64(row.try_get("summary_estimated_tokens")?, "summary token estimate")?,
        updated_at: row.try_get("summary_updated_at")?,
    })
}

fn validate_message(input: &NewConversationMessage) -> Result<(), ConversationMemoryError> {
    require_project_id(input.project_id)?;
    require_text("thread_id", &input.thread_id)?;
    if let Some(turn_id) = input.turn_id.as_deref() {
        require_text("turn_id", turn_id)?;
    }
    if let Some(source_item_id) = input.source_item_id.as_deref() {
        require_text("source_item_id", source_item_id)?;
        if input.turn_id.is_none() {
            return Err(ConversationMemoryError::InvalidRecord("source_item_id requires turn_id".to_string()));
        }
    }
    for artifact_ref in &input.artifact_refs {
        require_text("artifact reference", artifact_ref)?;
    }
    if let Some(tokens) = input.estimated_tokens {
        to_i64(tokens, "message token estimate")?;
    }
    Ok(())
}

fn validate_summary(input: &SaveConversationSummary) -> Result<(), ConversationMemoryError> {
    require_project_id(input.project_id)?;
    require_text("thread_id", &input.thread_id)?;
    to_i64(input.summarized_until_sequence, "summary sequence")?;
    if let Some(tokens) = input.estimated_tokens {
        to_i64(tokens, "summary token estimate")?;
    }
    Ok(())
}

pub(crate) fn validate_policy(policy: ConversationWindowPolicy) -> Result<(), ConversationMemoryError> {
    if policy.max_messages == 0 || policy.max_messages > 10_000 {
        return Err(ConversationMemoryError::InvalidRecord("max_messages must be between 1 and 10000".to_string()));
    }
    if policy.max_estimated_tokens == 0 {
        return Err(ConversationMemoryError::InvalidRecord("max_estimated_tokens must be greater than zero".to_string()));
    }
    Ok(())
}

pub(crate) fn require_text(field: &str, value: &str) -> Result<(), ConversationMemoryError> {
    if value.trim().is_empty() {
        return Err(ConversationMemoryError::InvalidRecord(format!("{field} must not be blank")));
    }
    Ok(())
}

fn require_project_id(project_id: i64) -> Result<(), ConversationMemoryError> {
    if project_id <= 0 {
        return Err(ConversationMemoryError::InvalidRecord("project_id must be positive".to_string()));
    }
    Ok(())
}

fn estimate_tokens(serialized_content: &str) -> u64 {
    let chars = serialized_content.chars().count() as u64;
    if chars == 0 {
        0
    } else {
        (chars + 3) / 4
    }
}

fn to_i64(value: u64, field: &str) -> Result<i64, ConversationMemoryError> {
    i64::try_from(value).map_err(|_| ConversationMemoryError::InvalidRecord(format!("{field} exceeds SQLite INTEGER")))
}

pub(crate) fn to_u64(value: i64, field: &str) -> Result<u64, ConversationMemoryError> {
    u64::try_from(value).map_err(|_| ConversationMemoryError::InvalidRecord(format!("{field} must not be negative")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::memory::adapter::ConversationMemoryAdapter;
    use rig_core::completion::Message;
    use rig_core::memory::ConversationMemory;
    use rig_core::message::{ToolCall, ToolFunction};
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    const MIGRATION: &str = include_str!("../../../../migrations_desktop/1014_desktop_ai_thread_turn_item.sql");

    async fn backend() -> SqliteConversationMemoryBackend {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect memory database");
        sqlx::raw_sql(MIGRATION)
            .execute(&pool)
            .await
            .expect("apply agent runtime migration");
        sqlx::raw_sql(
            "INSERT INTO ai_agent_thread \
                (thread_id, project_id, title_source, status, create_request_id, created_at, updated_at, last_activity_at) VALUES \
                ('thread-1', 1, 'auto', 'active', 'create-1', 1, 1, 1), \
                ('thread-auto', 1, 'auto', 'active', 'create-auto', 1, 1, 1), \
                ('thread-bounded', 1, 'auto', 'active', 'create-bounded', 1, 1, 1), \
                ('thread-adapter', 1, 'auto', 'active', 'create-adapter', 1, 1, 1); \
             INSERT INTO ai_agent_turn (turn_id, project_id, thread_id, turn_sequence, request_id, status, request_json, input_summary, model_config_id, runtime_model_id, entry_digest, created_at, updated_at, ended_at) \
             VALUES ('turn-1', 1, 'thread-1', 1, 'request-1', 'completed', '{}', 'input', 'config', 'model', 'digest', 1, 1, 1); \
             INSERT INTO ai_agent_item (item_id, turn_id, item_sequence, item_type, status, schema_version, payload_json, created_at, updated_at, completed_at) \
             VALUES ('item-1', 'turn-1', 1, 'userMessage', 'completed', 1, '{}', 1, 1, 1);",
        )
        .execute(&pool)
        .await
        .expect("seed Agent Threads");
        SqliteConversationMemoryBackend::new(pool)
    }

    fn message(sequence: u64) -> NewConversationMessage {
        NewConversationMessage {
            project_id: 1,
            thread_id: "thread-1".to_string(),
            turn_id: None,
            source_item_id: None,
            role: if sequence % 2 == 0 {
                ConversationRole::Assistant
            } else {
                ConversationRole::User
            },
            content: json!({ "text": format!("message-{sequence}") }),
            estimated_tokens: Some(3),
            artifact_refs: vec![],
            created_at: sequence as i64 * 10,
        }
    }

    #[tokio::test]
    async fn persists_messages_in_conversation_order_and_checks_scope() {
        let backend = backend().await;
        let mut first_input = message(1);
        first_input.turn_id = Some("turn-1".to_string());
        first_input.source_item_id = Some("item-1".to_string());
        let first = backend.append_message(first_input.clone()).await.expect("append first message");
        let replayed = backend
            .append_message(first_input)
            .await
            .expect("replay source Item projection");
        let second = backend.append_message(message(2)).await.expect("append second message");
        assert_eq!((first.sequence, second.sequence), (1, 2));
        assert_eq!(replayed, first);
        assert_eq!(first.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(first.source_item_id.as_deref(), Some("item-1"));
        assert_eq!(first.conversation_id, "thread-1");
        let listed = backend
            .list_messages(ListConversationMessages {
                project_id: 1,
                thread_id: "thread-1".to_string(),
                after_sequence: 0,
                through_sequence: None,
                limit: 10,
            })
            .await
            .expect("list messages");
        assert_eq!(listed.iter().map(|message| message.sequence).collect::<Vec<_>>(), vec![1, 2]);
        let mut wrong_scope = message(3);
        wrong_scope.project_id = 2;
        assert!(matches!(backend.append_message(wrong_scope).await, Err(ConversationMemoryError::ScopeMismatch(_))));
    }

    #[tokio::test]
    async fn loads_recent_window_and_advances_summary_watermark() {
        let backend = backend().await;
        for sequence in 1..=4 {
            backend.append_message(message(sequence)).await.expect("append message");
        }
        let window = backend
            .load_window(1, "thread-1".to_string(), ConversationWindowPolicy { max_messages: 2, max_estimated_tokens: 6 })
            .await
            .expect("load limited window");
        assert_eq!(window.messages.iter().map(|message| message.sequence).collect::<Vec<_>>(), vec![3, 4]);
        assert!(window.was_truncated);
        assert_eq!(window.truncated_through_sequence, Some(2));
        let summary = backend
            .save_summary(SaveConversationSummary {
                project_id: 1,
                thread_id: "thread-1".to_string(),
                content: json!({ "facts": ["first two messages"] }),
                summarized_until_sequence: 2,
                estimated_tokens: Some(2),
                updated_at: 50,
            })
            .await
            .expect("save summary");
        assert_eq!(summary.summarized_until_sequence, 2);
        let window = backend
            .load_window(1, "thread-1".to_string(), ConversationWindowPolicy { max_messages: 4, max_estimated_tokens: 8 })
            .await
            .expect("load summarized window");
        assert_eq!(window.summary.as_ref().map(|summary| summary.summarized_until_sequence), Some(2));
        assert_eq!(window.messages.iter().map(|message| message.sequence).collect::<Vec<_>>(), vec![3, 4]);
        assert_eq!(window.estimated_tokens, 8);
        assert!(!window.was_truncated);
        assert_eq!(window.truncated_through_sequence, None);
    }

    #[tokio::test]
    async fn rig_adapter_automatically_compacts_truncated_sqlite_history() {
        let sqlite_backend = backend().await;
        let backend: Arc<dyn ConversationMemoryBackend> = Arc::new(sqlite_backend.clone());
        let adapter = ConversationMemoryAdapter::new(1, backend, ConversationWindowPolicy { max_messages: 2, max_estimated_tokens: 1_000 });
        adapter
            .append(
                "thread-auto",
                vec![
                    Message::user("first visible request"),
                    Message::assistant("first visible answer"),
                    Message::user("recent request"),
                    Message::assistant("recent answer"),
                ],
            )
            .await
            .expect("append Rig history");
        let loaded = adapter.load("thread-auto").await.expect("load and compact history");
        assert_eq!(loaded.len(), 3, "summary plus two retained messages");
        assert_eq!(loaded[1], Message::user("recent request"));
        assert_eq!(loaded[2], Message::assistant("recent answer"));
        let summary = sqlite_backend
            .load_summary(1, "thread-auto".to_string())
            .await
            .expect("load persisted summary")
            .expect("automatic summary exists");
        assert_eq!(summary.summarized_until_sequence, 2);
        assert_eq!(summary.content.get("schemaVersion").and_then(Value::as_u64), Some(1));
        let text = summary
            .content
            .get("text")
            .and_then(Value::as_str)
            .expect("stable text summary");
        assert!(text.contains("first visible request"));
        assert!(text.contains("first visible answer"));
        let loaded_again = adapter.load("thread-auto").await.expect("reload already compacted history");
        assert_eq!(loaded_again[1], Message::user("recent request"));
        let unchanged_summary = sqlite_backend
            .load_summary(1, "thread-auto".to_string())
            .await
            .expect("reload persisted summary")
            .expect("summary remains present");
        assert_eq!(unchanged_summary, summary, "a second load must not rewrite an unchanged watermark");
        let reloaded = sqlite_backend
            .load_window(1, "thread-auto".to_string(), ConversationWindowPolicy { max_messages: 2, max_estimated_tokens: 1_000 })
            .await
            .expect("reload compacted SQLite window");
        assert!(!reloaded.was_truncated);
    }

    #[tokio::test]
    async fn automatic_compaction_preserves_summary_tool_names_scope_and_length_boundary() {
        let sqlite_backend = backend().await;
        let backend: Arc<dyn ConversationMemoryBackend> = Arc::new(sqlite_backend.clone());
        let policy = ConversationWindowPolicy { max_messages: 1, max_estimated_tokens: 10_000 };
        let adapter = ConversationMemoryAdapter::new(1, backend.clone(), policy);
        adapter
            .append("thread-bounded", vec![Message::user("message represented by the prior summary")])
            .await
            .expect("append prior message");
        sqlite_backend
            .save_summary(SaveConversationSummary {
                project_id: 1,
                thread_id: "thread-bounded".to_string(),
                content: json!({ "text": "prior durable fact" }),
                summarized_until_sequence: 1,
                estimated_tokens: Some(4),
                updated_at: 10,
            })
            .await
            .expect("seed prior summary");
        let secret_argument = "A".repeat(8_000);
        let secret_result = "tool-result-must-not-enter-summary".repeat(500);
        let mut compacted_messages = vec![
            Message::from(ToolCall::new(
                "call-1".to_string(),
                ToolFunction::new("read_artifact".to_string(), json!({ "payload": secret_argument })),
            )),
            Message::tool_result("call-1", secret_result),
        ];
        for index in 0..12 {
            let text = format!("visible-history-{index} {}", "detail ".repeat(200));
            compacted_messages.push(if index % 2 == 0 { Message::user(text) } else { Message::assistant(text) });
        }
        compacted_messages.push(Message::user("latest retained request"));
        adapter
            .append("thread-bounded", compacted_messages)
            .await
            .expect("append messages containing large tool payloads");
        let loaded = adapter.load("thread-bounded").await.expect("compact bounded history");
        assert_eq!(loaded.len(), 2, "summary plus latest message");
        assert_eq!(loaded[1], Message::user("latest retained request"));
        let summary = sqlite_backend
            .load_summary(1, "thread-bounded".to_string())
            .await
            .expect("load automatic summary")
            .expect("summary exists");
        assert_eq!(summary.summarized_until_sequence, 15);
        let text = summary.content.get("text").and_then(Value::as_str).expect("summary text");
        assert!(text.contains("prior durable fact"));
        assert!(text.contains("read_artifact"));
        assert!(text.contains("visible-history-11"), "the newest compacted facts must be retained");
        assert!(!text.contains("visible-history-0"), "old details may be evicted at the hard summary boundary");
        assert!(!text.contains("AAAA"), "tool arguments must not enter the summary");
        assert!(!text.contains("tool-result-must-not-enter-summary"));
        assert!(text.chars().count() <= 4_096, "summary must not cross the hard character boundary");
        let wrong_scope = ConversationMemoryAdapter::new(2, backend, policy);
        let error = wrong_scope
            .load("thread-bounded")
            .await
            .expect_err("cross-project conversation load must fail");
        assert!(error.to_string().contains("scope does not match"));
    }

    #[tokio::test]
    async fn rig_adapter_round_trips_window_summary_and_clear() {
        let sqlite_backend = backend().await;
        let backend: Arc<dyn ConversationMemoryBackend> = Arc::new(sqlite_backend.clone());
        let adapter = ConversationMemoryAdapter::new(1, backend, ConversationWindowPolicy { max_messages: 2, max_estimated_tokens: 100 });
        adapter
            .append("thread-adapter", vec![Message::user("first"), Message::assistant("second"), Message::user("third")])
            .await
            .expect("append through Rig adapter");
        sqlite_backend
            .save_summary(SaveConversationSummary {
                project_id: 1,
                thread_id: "thread-adapter".to_string(),
                content: json!({ "text": "earlier conversation facts" }),
                summarized_until_sequence: 1,
                estimated_tokens: Some(4),
                updated_at: 100,
            })
            .await
            .expect("save summary");
        let loaded = adapter.load("thread-adapter").await.expect("load through Rig adapter");
        assert_eq!(loaded.len(), 3);
        assert!(matches!(&loaded[0], Message::User { .. }));
        assert_eq!(loaded[1], Message::assistant("second"));
        assert_eq!(loaded[2], Message::user("third"));
        adapter.clear("thread-adapter").await.expect("clear through Rig adapter");
        assert!(adapter.load("thread-adapter").await.expect("reload cleared memory").is_empty());
    }
}
