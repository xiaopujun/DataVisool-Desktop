/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! SQLite 查询：按 Turn 隔离 Agent 工作记忆。

use super::conversation_store::{message_from_row, require_text, to_u64, validate_policy, ConversationMemoryError, SqliteConversationMemoryBackend};
use super::policy::{ConversationWindow, ConversationWindowPolicy};

impl SqliteConversationMemoryBackend {
    pub(crate) async fn load_turn_window_inner(
        &self,
        project_id: i64,
        thread_id: &str,
        turn_id: &str,
        policy: ConversationWindowPolicy,
    ) -> Result<ConversationWindow, ConversationMemoryError> {
        validate_policy(policy)?;
        require_text("turn_id", turn_id)?;
        if !self.ensure_read_scope(project_id, thread_id).await? {
            return Ok(empty_window());
        }
        let turn_exists =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ai_agent_turn WHERE turn_id = ?1 AND thread_id = ?2 AND project_id = ?3")
                .bind(turn_id)
                .bind(thread_id)
                .bind(project_id)
                .fetch_one(&self.pool)
                .await?;
        if turn_exists != 1 {
            return Err(ConversationMemoryError::ScopeMismatch(turn_id.to_string()));
        }

        let available_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ai_agent_conversation_message WHERE conversation_id = ?1 AND turn_id = ?2")
                .bind(thread_id)
                .bind(turn_id)
                .fetch_one(&self.pool)
                .await?;
        let rows = sqlx::query(
            "SELECT id, conversation_id, sequence, turn_id, source_item_id, role, content_json, estimated_tokens, artifact_refs_json, created_at \
             FROM ai_agent_conversation_message \
             WHERE conversation_id = ?1 AND turn_id = ?2 \
             ORDER BY sequence DESC LIMIT ?3",
        )
        .bind(thread_id)
        .bind(turn_id)
        .bind(i64::from(policy.max_messages))
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::with_capacity(rows.len());
        let mut message_tokens = 0_u64;
        for row in rows {
            let message = message_from_row(row)?;
            let next_total = message_tokens.saturating_add(message.estimated_tokens);
            if next_total > policy.max_estimated_tokens {
                break;
            }
            message_tokens = next_total;
            messages.push(message);
        }
        messages.reverse();
        let available_count = to_u64(available_count, "turn message count")?;
        let was_truncated = available_count > messages.len() as u64;
        let truncated_through_sequence = if was_truncated {
            messages.first().map(|message| message.sequence.saturating_sub(1))
        } else {
            None
        };
        Ok(ConversationWindow { summary: None, messages, estimated_tokens: message_tokens, was_truncated, truncated_through_sequence })
    }

    pub(crate) async fn clear_turn_inner(&self, project_id: i64, thread_id: &str, turn_id: &str) -> Result<(), ConversationMemoryError> {
        require_text("turn_id", turn_id)?;
        if !self.ensure_read_scope(project_id, thread_id).await? {
            return Ok(());
        }
        let turn_exists =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ai_agent_turn WHERE turn_id = ?1 AND thread_id = ?2 AND project_id = ?3")
                .bind(turn_id)
                .bind(thread_id)
                .bind(project_id)
                .fetch_one(&self.pool)
                .await?;
        if turn_exists != 1 {
            return Err(ConversationMemoryError::ScopeMismatch(turn_id.to_string()));
        }
        sqlx::query(
            "DELETE FROM ai_agent_conversation_message WHERE conversation_id = ?1 AND turn_id = ?2 \
             AND EXISTS (SELECT 1 FROM ai_agent_conversation WHERE conversation_id = ?1 AND project_id = ?3)",
        )
        .bind(thread_id)
        .bind(turn_id)
        .bind(project_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn empty_window() -> ConversationWindow {
    ConversationWindow {
        summary: None,
        messages: Vec::new(),
        estimated_tokens: 0,
        was_truncated: false,
        truncated_through_sequence: None,
    }
}
