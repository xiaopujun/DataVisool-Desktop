/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Turn 作用域的 Rig 工作记忆适配器构造。
//!
//! Thread Conversation 仍然使用稳定的 `conversation_id == thread_id`。Agent
//! 工作记忆复用同一张消息表，但通过 `turn_id` 隔离每个任务的模型轨迹。

use std::sync::Arc;

use super::{ConversationMemoryAdapter, ConversationMemoryBackend, ConversationWindowPolicy};

/// Rig 在一次 Task 运行中使用的稳定 key，不再把 Thread 当作工作记忆 key。
pub(crate) fn task_conversation_key(turn_id: &str) -> String {
    format!("turn:{turn_id}")
}

/// 创建只读写指定 Turn 工作记忆的适配器。
pub(crate) fn new_task_memory(
    project_id: i64,
    backend: Arc<dyn ConversationMemoryBackend>,
    policy: ConversationWindowPolicy,
    thread_id: impl Into<String>,
    turn_id: impl Into<String>,
) -> ConversationMemoryAdapter {
    ConversationMemoryAdapter::new(project_id, backend, policy).with_turn_scope(thread_id.into(), turn_id.into())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rig_core::completion::Message;
    use rig_core::memory::ConversationMemory;
    use serde_json::Value;
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;
    use crate::backend::ai_driven::memory::conversation_store::ListConversationMessages;
    use crate::backend::ai_driven::memory::{ConversationMemoryBackend, SqliteConversationMemoryBackend};

    const MIGRATION: &str = include_str!("../../../../migrations_desktop/1014_desktop_ai_thread_turn_item.sql");

    async fn backend() -> SqliteConversationMemoryBackend {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect memory database");
        sqlx::raw_sql(MIGRATION).execute(&pool).await.expect("apply Agent migration");
        sqlx::raw_sql(
            "INSERT INTO ai_agent_thread \
                (thread_id, project_id, title_source, status, create_request_id, created_at, updated_at, last_activity_at) VALUES \
                ('thread-isolation', 1, 'auto', 'active', 'create-isolation', 1, 1, 1); \
             INSERT INTO ai_agent_turn \
                (turn_id, project_id, thread_id, turn_sequence, request_id, status, request_json, input_summary, model_config_id, runtime_model_id, entry_digest, created_at, updated_at, ended_at) VALUES \
                ('turn-old', 1, 'thread-isolation', 1, 'request-old', 'completed', '{}', 'old', 'config', 'model', 'digest-old', 1, 1, 1), \
                ('turn-new', 1, 'thread-isolation', 2, 'request-new', 'created', '{}', 'new', 'config', 'model', 'digest-new', 2, 2, NULL);",
        )
        .execute(&pool)
        .await
        .expect("seed Thread and Turns");
        SqliteConversationMemoryBackend::new(pool)
    }

    #[tokio::test]
    async fn isolates_same_thread_turns_and_round_trips_same_turn_memory() {
        let sqlite_backend = backend().await;
        let backend: Arc<dyn ConversationMemoryBackend> = Arc::new(sqlite_backend.clone());
        let policy = ConversationWindowPolicy::default();
        let old = new_task_memory(1, backend.clone(), policy, "thread-isolation", "turn-old");
        let old_key = task_conversation_key("turn-old");
        old.append(&old_key, vec![Message::assistant("old tool trace"), Message::user("old request")])
            .await
            .expect("persist old Turn memory");

        let new = new_task_memory(1, backend.clone(), policy, "thread-isolation", "turn-new");
        let new_key = task_conversation_key("turn-new");
        assert!(new.load(&new_key).await.expect("load new Turn memory").is_empty());

        new.append(&new_key, vec![Message::user("new request")])
            .await
            .expect("persist new Turn memory");
        let loaded = new.load(&new_key).await.expect("load new Turn memory");
        assert_eq!(loaded, vec![Message::user("new request")]);

        let restored = new_task_memory(1, backend, policy, "thread-isolation", "turn-new")
            .load(&new_key)
            .await
            .expect("reload new Turn memory");
        assert_eq!(restored, loaded, "a later attempt must recover the same Turn memory");

        let all_messages = sqlite_backend
            .list_messages(ListConversationMessages {
                project_id: 1,
                thread_id: "thread-isolation".to_string(),
                after_sequence: 0,
                through_sequence: None,
                limit: 10,
            })
            .await
            .expect("list complete Thread history");
        assert_eq!(all_messages.len(), 3, "Thread history keeps both Turns");
        assert_eq!(
            all_messages
                .iter()
                .filter_map(|message| message.turn_id.as_deref())
                .collect::<Vec<_>>(),
            vec!["turn-old", "turn-old", "turn-new"]
        );
        assert!(all_messages.iter().all(|message| message.conversation_id == "thread-isolation"));
        assert!(all_messages
            .iter()
            .all(|message| { message.content.get("schemaVersion").and_then(Value::as_u64) == Some(2) }));

        new.clear(&new_key).await.expect("clear only new Turn memory");
        assert!(new.load(&new_key).await.expect("reload cleared new Turn memory").is_empty());
        assert_eq!(old.load(&old_key).await.expect("old Turn memory remains").len(), 2);
    }

    #[tokio::test]
    async fn rejects_thread_key_for_task_memory() {
        let sqlite_backend = backend().await;
        let backend: Arc<dyn ConversationMemoryBackend> = Arc::new(sqlite_backend);
        let memory = new_task_memory(1, backend, ConversationWindowPolicy::default(), "thread-isolation", "turn-new");
        let error = memory
            .load("thread-isolation")
            .await
            .expect_err("Task memory must not accept a Thread key");
        assert!(error.to_string().contains("task memory key"));
    }
}
