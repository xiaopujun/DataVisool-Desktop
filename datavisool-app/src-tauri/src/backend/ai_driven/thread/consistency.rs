/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 逻辑外键与关键时间线不变量的只读巡检。

use sqlx::Row;

use super::error::ThreadRepositoryError;
use super::persistence::SqliteThreadRepository;
use super::state::ConsistencyIssue;

impl SqliteThreadRepository {
    pub(crate) async fn audit_consistency(&self) -> Result<Vec<ConsistencyIssue>, ThreadRepositoryError> {
        let mut issues = Vec::new();
        collect(
            &self.pool,
            &mut issues,
            "thread.project",
            "SELECT thread.thread_id AS entity_id, 'Thread references a missing project' AS detail \
             FROM ai_agent_thread thread LEFT JOIN project ON project.id = thread.project_id \
             WHERE project.id IS NULL",
        )
        .await?;
        collect(
            &self.pool,
            &mut issues,
            "turn.thread_scope",
            "SELECT turn.turn_id AS entity_id, 'Turn has no parent Thread or its project differs' AS detail \
             FROM ai_agent_turn turn LEFT JOIN ai_agent_thread thread ON thread.thread_id = turn.thread_id \
             WHERE thread.thread_id IS NULL OR thread.project_id <> turn.project_id",
        )
        .await?;
        collect(
            &self.pool,
            &mut issues,
            "item.turn",
            "SELECT item.item_id AS entity_id, 'Item references a missing Turn' AS detail \
             FROM ai_agent_item item LEFT JOIN ai_agent_turn turn ON turn.turn_id = item.turn_id WHERE turn.turn_id IS NULL",
        )
        .await?;
        collect(
            &self.pool,
            &mut issues,
            "item.run_scope",
            "SELECT item.item_id AS entity_id, 'Item source Run is missing or belongs to another Turn' AS detail \
             FROM ai_agent_item item LEFT JOIN ai_agent_task_run run ON run.run_id = item.source_run_id \
             WHERE item.source_run_id IS NOT NULL AND (run.run_id IS NULL OR run.turn_id <> item.turn_id)",
        )
        .await?;
        collect(
            &self.pool,
            &mut issues,
            "item.last_event",
            "SELECT item.item_id AS entity_id, 'Item last_event_sequence does not identify an Event in the same Turn' AS detail \
             FROM ai_agent_item item LEFT JOIN ai_agent_task_event event \
               ON event.item_id = item.item_id AND event.turn_id = item.turn_id AND event.thread_sequence = item.last_event_sequence \
             WHERE item.last_event_sequence > 0 AND event.id IS NULL",
        )
        .await?;
        collect(
            &self.pool,
            &mut issues,
            "run.turn",
            "SELECT run.run_id AS entity_id, 'TaskRun references a missing Turn' AS detail \
             FROM ai_agent_task_run run LEFT JOIN ai_agent_turn turn ON turn.turn_id = run.turn_id WHERE turn.turn_id IS NULL",
        )
        .await?;
        collect(
            &self.pool,
            &mut issues,
            "run.waiting_item_scope",
            "SELECT run.run_id AS entity_id, 'Waiting TaskRun pending Item is missing or inconsistent' AS detail \
             FROM ai_agent_task_run run \
             LEFT JOIN ai_agent_turn turn ON turn.turn_id = run.turn_id \
             LEFT JOIN ai_agent_item item ON item.item_id = json_extract(run.pending_json, '$.waitingItemId') \
             WHERE run.status IN ('waitingFrontendTool','waitingUserInput','waitingUserConfirmation') AND (\
               run.pending_json IS NULL OR json_extract(run.pending_json, '$.waitingItemId') IS NULL \
               OR turn.status <> run.status OR item.item_id IS NULL OR item.turn_id <> run.turn_id \
               OR item.source_run_id IS NULL OR item.source_run_id <> run.run_id OR item.status <> 'waiting' \
               OR (run.status = 'waitingFrontendTool' AND item.item_type <> 'frontendTool') \
               OR (run.status = 'waitingUserInput' AND item.item_type <> 'userInput') \
               OR (run.status = 'waitingUserConfirmation' AND item.item_type <> 'approval')\
             )",
        )
        .await?;
        collect(
            &self.pool,
            &mut issues,
            "event.scope",
            "SELECT event.id AS entity_id, 'Event Thread, Turn or Run scope is inconsistent' AS detail \
             FROM ai_agent_task_event event \
             LEFT JOIN ai_agent_turn turn ON turn.turn_id = event.turn_id \
             LEFT JOIN ai_agent_task_run run ON run.run_id = event.run_id \
             WHERE turn.turn_id IS NULL OR turn.thread_id <> event.thread_id OR run.run_id IS NULL OR run.turn_id <> event.turn_id",
        )
        .await?;
        collect(
            &self.pool,
            &mut issues,
            "event.item_scope",
            "SELECT event.id AS entity_id, 'Event Item is missing or belongs to another Turn' AS detail \
             FROM ai_agent_task_event event LEFT JOIN ai_agent_item item ON item.item_id = event.item_id \
             WHERE event.item_id IS NOT NULL AND (item.item_id IS NULL OR item.turn_id <> event.turn_id)",
        )
        .await?;
        collect(
            &self.pool,
            &mut issues,
            "inbox.item_scope",
            "SELECT inbox.input_id AS entity_id, 'Inbox user Item is missing or belongs to another Turn' AS detail \
             FROM ai_agent_turn_input inbox LEFT JOIN ai_agent_item item ON item.item_id = inbox.user_item_id \
             WHERE item.item_id IS NULL OR item.turn_id <> inbox.turn_id OR item.item_type <> 'userMessage' OR item.status <> 'completed'",
        )
        .await?;
        collect(
            &self.pool,
            &mut issues,
            "conversation.thread_scope",
            "SELECT conversation.conversation_id AS entity_id, 'Conversation Thread or project scope is inconsistent' AS detail \
             FROM ai_agent_conversation conversation LEFT JOIN ai_agent_thread thread ON thread.thread_id = conversation.thread_id \
             WHERE thread.thread_id IS NULL OR thread.project_id <> conversation.project_id \
                OR conversation.conversation_id <> conversation.thread_id",
        )
        .await?;
        collect(
            &self.pool,
            &mut issues,
            "attachment.claim_scope",
            "SELECT attachment.attachment_id AS entity_id, 'Claimed attachment has a missing or cross-project Thread/Turn' AS detail \
             FROM ai_agent_attachment attachment \
             LEFT JOIN ai_agent_thread thread ON thread.thread_id = attachment.claimed_thread_id \
             LEFT JOIN ai_agent_turn turn ON turn.turn_id = attachment.claimed_turn_id \
             WHERE attachment.claimed_thread_id IS NOT NULL AND (\
               thread.thread_id IS NULL OR thread.project_id <> attachment.project_id OR turn.turn_id IS NULL \
               OR turn.project_id <> attachment.project_id OR turn.thread_id <> attachment.claimed_thread_id\
             )",
        )
        .await?;
        Ok(issues)
    }
}

async fn collect(pool: &sqlx::SqlitePool, issues: &mut Vec<ConsistencyIssue>, kind: &str, query: &str) -> Result<(), ThreadRepositoryError> {
    for row in sqlx::query(query).fetch_all(pool).await? {
        issues.push(ConsistencyIssue { kind: kind.to_string(), entity_id: row.try_get("entity_id")?, detail: row.try_get("detail")? });
    }
    Ok(())
}
