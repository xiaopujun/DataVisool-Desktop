/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Turn、首个 User Item、TaskRun 与初始 Event 的原子创建。

use serde_json::{json, Value};
use sqlx::{Row, Sqlite, Transaction};

use crate::backend::ai_driven::thread::error::ThreadRepositoryError;
use crate::backend::ai_driven::thread::id::{new_event_id, new_item_id, new_run_id, new_task_id, new_turn_id};
use crate::backend::ai_driven::thread::state::{
    AgentItem, AgentTaskRunRef, AgentTimelineEvent, AgentTurn, AttachmentClaim, CreateTurnInput, CreateTurnResult,
};

use super::{allocate_event_sequences, insert_event, item_from_row, run_from_row, turn_from_row, SqliteThreadRepository};

const TURN_SELECT: &str = concat!(
    "SELECT turn_id, project_id, thread_id, turn_sequence, request_id, status, request_json, COALESCE(input_summary, '') AS input_summary, ",
    "model_config_id, runtime_model_id, provider, entry_digest, usage_json, last_artifact_id, last_artifact_revision, error_code, error_message, ",
    "row_version, created_at, started_at, updated_at, ended_at FROM ai_agent_turn "
);
const ITEM_SELECT: &str = concat!(
    "SELECT item_id, turn_id, item_sequence, source_run_id, logical_key, item_type, status, schema_version, payload_json, ",
    "last_event_sequence, row_version, created_at, updated_at, completed_at FROM ai_agent_item "
);
const RUN_SELECT: &str = concat!(
    "SELECT r.task_id, r.run_id, r.request_id, turn.entry_digest, turn.project_id, turn.thread_id, r.turn_id, r.attempt_no, r.client_session_id, ",
    "r.status, r.task_type, r.limits_json, r.pending_json, r.usage_json, r.last_artifact_id, r.last_artifact_revision, ",
    "r.error_code, r.error_message, r.project_snapshot_ref, r.rig_version, r.rig_snapshot_ref, r.created_at, r.started_at, r.updated_at, r.ended_at ",
    "FROM ai_agent_task_run r JOIN ai_agent_turn turn ON turn.turn_id = r.turn_id "
);

impl SqliteThreadRepository {
    /// 原子创建一个 Turn、首条用户消息、TaskRun 和初始时间线事件。
    ///
    /// 同一 `request_id` 的重复提交会回放已创建结果，避免产生重复 Turn。
    pub(crate) async fn create_turn(&self, input: CreateTurnInput) -> Result<CreateTurnResult, ThreadRepositoryError> {
        // 1. 在开启事务前校验输入的必填字段、任务类型和附件声明等静态约束。
        validate_create_turn(&input)?;

        // 2. 开启事务，确保 Turn、TaskRun、首条消息和事件要么全部创建成功，要么全部回滚。
        let mut transaction = self.pool.begin().await?;

        // 3. 先按 requestId 查找既有 Turn，处理客户端重试带来的幂等请求。
        if let Some(existing) = find_turn_by_request(&mut transaction, &input.request_id).await? {
            // 3.1 确认当前 Thread 仍属于目标项目且可用，再校验入口摘要并回放既有结果。
            ensure_active_thread(&mut transaction, input.project_id, &input.thread_id).await?;
            let replayed = replay_existing(&mut transaction, &input, existing).await?;

            // 3.2 读取成功后提交只读事务，并返回既有记录而不再创建新数据。
            transaction.commit().await?;
            return Ok(replayed);
        }

        // 4. 新建 Turn 前确认 Thread 归属正确、项目未删除且 Thread 未归档。
        ensure_active_thread(&mut transaction, input.project_id, &input.thread_id).await?;

        // 5. 生成本轮 Turn 标识，并在认领前校验每个临时附件是否可供本任务使用。
        let turn_id = new_turn_id();
        validate_attachments(&mut transaction, &input, &turn_id).await?;

        // 6. 原子递增 Thread 的序号并取得本 Turn 的顺序；更新失败说明 Thread 状态已变化。
        let turn_sequence = sqlx::query_scalar::<_, i64>(
            "UPDATE ai_agent_thread SET next_turn_sequence = next_turn_sequence + 1, row_version = row_version + 1, \
                 updated_at = MAX(updated_at, ?1), last_activity_at = MAX(last_activity_at, ?1) \
             WHERE project_id = ?2 AND thread_id = ?3 AND status = 'active' \
             RETURNING next_turn_sequence - 1",
        )
        .bind(input.created_at)
        .bind(input.project_id)
        .bind(&input.thread_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| ThreadRepositoryError::conflict("agent.thread.archived", "Thread 已归档或作用域不匹配"))?;

        // 7. 为 TaskRun、首条用户消息生成标识，并序列化完整请求快照。
        let task_id = new_task_id();
        let run_id = new_run_id();
        let user_item_id = new_item_id();
        let request_json = serde_json::to_string(&input.request)?;

        // 8. 插入初始状态为 created 的 Turn；唯一键冲突会被转换为业务语义错误。
        let insert_turn = sqlx::query(
            "INSERT INTO ai_agent_turn (\
                turn_id, project_id, thread_id, turn_sequence, request_id, status, request_json, input_summary, model_config_id, \
                runtime_model_id, provider, entry_digest, usage_json, last_artifact_id, last_artifact_revision, error_code, error_message, \
                next_item_sequence, row_version, created_at, started_at, updated_at, ended_at\
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'created', ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, NULL, NULL, NULL, 2, 1, ?12, NULL, ?12, NULL)",
        )
        .bind(&turn_id)
        .bind(input.project_id)
        .bind(&input.thread_id)
        .bind(turn_sequence)
        .bind(&input.request_id)
        .bind(request_json)
        .bind(&input.input_summary)
        .bind(&input.model_config_id)
        .bind(&input.runtime_model_id)
        .bind(input.provider.as_deref())
        .bind(&input.entry_digest)
        .bind(input.created_at)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = insert_turn {
            return Err(map_turn_insert_error(&mut transaction, &input, error).await);
        }

        // 9. 确定项目工作区快照引用，调用方未提供时生成与当前入口摘要绑定的默认引用。
        let project_snapshot_ref = input
            .project_snapshot_ref
            .clone()
            .or_else(|| Some(format!("workspace:project/{}/workspace#entry={}", input.project_id, input.entry_digest)));

        // 10. 插入首次尝试的 TaskRun，并固化本轮执行限制、Rig 版本和工作区快照。
        sqlx::query(
            "INSERT INTO ai_agent_task_run (\
                task_id, run_id, request_id, turn_id, attempt_no, client_session_id, status, task_type, limits_json, pending_json, usage_json, \
                last_artifact_id, last_artifact_revision, error_code, error_message, rig_version, rig_snapshot_ref, project_snapshot_ref, \
                next_event_sequence, created_at, started_at, updated_at, ended_at\
             ) VALUES (?1, ?2, ?3, ?4, 1, ?5, 'created', ?6, ?7, NULL, NULL, NULL, NULL, NULL, NULL, ?8, ?9, ?10, 1, ?11, NULL, ?11, NULL)",
        )
        .bind(&task_id)
        .bind(&run_id)
        .bind(&input.request_id)
        .bind(&turn_id)
        .bind(&input.client_session_id)
        .bind(&input.task_type)
        .bind(serde_json::to_string(&input.limits)?)
        .bind(input.rig_version.as_deref())
        .bind(input.rig_snapshot_ref.as_deref())
        .bind(project_snapshot_ref.as_deref())
        .bind(input.created_at)
        .execute(&mut *transaction)
        .await?;

        // 11. 为 Thread 初始化会话记录；已存在会话保持不变，再确认其项目作用域正确。
        sqlx::query(
            "INSERT INTO ai_agent_conversation (\
                conversation_id, thread_id, project_id, next_message_sequence, summary_json, summary_until_sequence, \
                summary_estimated_tokens, summary_updated_at, created_at, updated_at\
             ) VALUES (?1, ?1, ?2, 1, NULL, 0, 0, NULL, ?3, ?3) \
             ON CONFLICT(thread_id) DO NOTHING",
        )
        .bind(&input.thread_id)
        .bind(input.project_id)
        .bind(input.created_at)
        .execute(&mut *transaction)
        .await?;
        ensure_conversation_scope(&mut transaction, input.project_id, &input.thread_id).await?;

        // 12. 认领已校验的附件，使其正式归属到新建 Turn。
        claim_attachments(&mut transaction, &input, &turn_id).await?;

        // 13. 写入本 Turn 的第一条用户消息，状态直接标记为 completed。
        let logical_key = format!("turn:{turn_id}:user:initial");
        sqlx::query(
            "INSERT INTO ai_agent_item (\
                item_id, turn_id, item_sequence, source_run_id, logical_key, item_type, status, schema_version, payload_json, \
                last_event_sequence, row_version, created_at, updated_at, completed_at\
             ) VALUES (?1, ?2, 1, ?3, ?4, 'userMessage', 'completed', 1, ?5, 0, 1, ?6, ?6, ?6)",
        )
        .bind(&user_item_id)
        .bind(&turn_id)
        .bind(&run_id)
        .bind(&logical_key)
        .bind(serde_json::to_string(&input.user_item_payload)?)
        .bind(input.created_at)
        .execute(&mut *transaction)
        .await?;

        // 14. 依次写入 Turn 创建、用户消息开始和用户消息完成三条初始时间线事件。
        let mut events = Vec::with_capacity(3);
        events.push(
            append_event(
                &mut transaction,
                &input.thread_id,
                &turn_id,
                None,
                &run_id,
                "turn.created",
                json!({ "turnId": turn_id, "requestId": input.request_id, "status": "created" }),
                format!("turn:{turn_id}:created"),
                input.created_at,
            )
            .await?,
        );
        events.push(
            append_event(
                &mut transaction,
                &input.thread_id,
                &turn_id,
                Some(&user_item_id),
                &run_id,
                "item.started",
                item_payload(&user_item_id, "userMessage", "completed", &input.user_item_payload),
                format!("item:{user_item_id}:started"),
                input.created_at,
            )
            .await?,
        );
        events.push(
            append_event(
                &mut transaction,
                &input.thread_id,
                &turn_id,
                Some(&user_item_id),
                &run_id,
                "item.completed",
                item_payload(&user_item_id, "userMessage", "completed", &input.user_item_payload),
                format!("item:{user_item_id}:completed"),
                input.created_at,
            )
            .await?,
        );

        // 15. 将最后一条事件序号回写到用户消息，并在 Thread 尚无标题时生成默认标题。
        let last_event_sequence = events.last().expect("three initial events").thread_sequence;
        sqlx::query("UPDATE ai_agent_item SET last_event_sequence = ?1 WHERE item_id = ?2")
            .bind(i64::try_from(last_event_sequence).unwrap_or(i64::MAX))
            .bind(&user_item_id)
            .execute(&mut *transaction)
            .await?;
        set_default_title(&mut transaction, &input.thread_id, &input.input_summary, input.created_at).await?;

        // 16. 从事务中回读刚创建的 Turn、用户消息和 TaskRun，确保返回数据与数据库一致。
        let turn = find_turn(&mut transaction, &turn_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "turn", key: turn_id.clone() })?;
        let user_item = find_item(&mut transaction, &turn_id, &user_item_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "item", key: user_item_id.clone() })?;
        let run = find_run(&mut transaction, &run_id)
            .await?
            .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "task run", key: run_id.clone() })?;

        // 17. 提交全部写入，并以非回放结果返回本轮创建的完整实体和初始事件。
        transaction.commit().await?;
        Ok(CreateTurnResult { turn, user_item, run, events, replayed: false })
    }
}

async fn replay_existing(
    transaction: &mut Transaction<'_, Sqlite>,
    input: &CreateTurnInput,
    turn: AgentTurn,
) -> Result<CreateTurnResult, ThreadRepositoryError> {
    if turn.project_id != input.project_id || turn.thread_id != input.thread_id {
        return Err(ThreadRepositoryError::ScopeMismatch { entity: "request", key: input.request_id.clone() });
    }
    if turn.entry_digest != input.entry_digest {
        return Err(ThreadRepositoryError::conflict("agent.turn.idempotency_conflict", "requestId 已绑定到不同入口内容"));
    }
    let user_item = find_initial_user_item(transaction, &turn.turn_id)
        .await?
        .ok_or_else(|| ThreadRepositoryError::InvalidRecord("idempotent Turn is missing its initial user Item".to_string()))?;
    let run = find_run_by_turn(transaction, &turn.turn_id)
        .await?
        .ok_or_else(|| ThreadRepositoryError::InvalidRecord("idempotent Turn is missing its TaskRun".to_string()))?;
    let events = sqlx::query(
        "SELECT id, thread_id, turn_id, item_id, run_id, sequence, thread_sequence, kind, payload_json, dedupe_key, schema_version, created_at \
         FROM ai_agent_task_event WHERE turn_id = ?1 ORDER BY thread_sequence ASC LIMIT 3",
    )
    .bind(&turn.turn_id)
    .fetch_all(&mut **transaction)
    .await?
    .into_iter()
    .map(super::event_from_row)
    .collect::<Result<Vec<_>, _>>()?;
    Ok(CreateTurnResult { turn, user_item, run, events, replayed: true })
}

async fn append_event(
    transaction: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
    turn_id: &str,
    item_id: Option<&str>,
    run_id: &str,
    kind: &str,
    payload: Value,
    dedupe_key: String,
    created_at: i64,
) -> Result<AgentTimelineEvent, ThreadRepositoryError> {
    let (run_sequence, thread_sequence) = allocate_event_sequences(transaction, thread_id, run_id, created_at).await?;
    insert_event(
        transaction,
        &new_event_id(),
        thread_id,
        turn_id,
        item_id,
        run_id,
        run_sequence,
        thread_sequence,
        kind,
        &payload,
        &dedupe_key,
        1,
        created_at,
    )
    .await
}

async fn ensure_active_thread(transaction: &mut Transaction<'_, Sqlite>, project_id: i64, thread_id: &str) -> Result<(), ThreadRepositoryError> {
    let row = sqlx::query(
        "SELECT thread.project_id, thread.status FROM ai_agent_thread thread \
         JOIN project active_project ON active_project.id = thread.project_id AND active_project.deleted = 0 \
         WHERE thread.thread_id = ?1",
    )
    .bind(thread_id)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(row) = row else {
        return Err(ThreadRepositoryError::NotFound { entity: "thread", key: thread_id.to_string() });
    };
    if row.try_get::<i64, _>("project_id")? != project_id {
        return Err(ThreadRepositoryError::ScopeMismatch { entity: "thread", key: thread_id.to_string() });
    }
    if row.try_get::<String, _>("status")? != "active" {
        return Err(ThreadRepositoryError::conflict("agent.thread.archived", "归档 Thread 不能创建 Turn"));
    }
    Ok(())
}

async fn validate_attachments(
    transaction: &mut Transaction<'_, Sqlite>,
    input: &CreateTurnInput,
    _turn_id: &str,
) -> Result<(), ThreadRepositoryError> {
    for claim in &input.attachments {
        let row = attachment_row(transaction, &claim.attachment_id).await?;
        validate_attachment_row(&row, input, claim)?;
    }
    Ok(())
}

async fn claim_attachments(transaction: &mut Transaction<'_, Sqlite>, input: &CreateTurnInput, turn_id: &str) -> Result<(), ThreadRepositoryError> {
    for claim in &input.attachments {
        let result = sqlx::query(
            "UPDATE ai_agent_attachment SET status = 'claimed', claimed_thread_id = ?1, claimed_turn_id = ?2, claimed_at = ?3, \
                 updated_at = ?3, last_used_at = ?3 \
             WHERE attachment_id = ?4 AND project_id = ?5 AND upload_session_id = ?6 AND status = 'temporary' \
               AND claimed_thread_id IS NULL AND claimed_turn_id IS NULL AND claimed_at IS NULL AND expire_at > ?3",
        )
        .bind(&input.thread_id)
        .bind(turn_id)
        .bind(input.created_at)
        .bind(&claim.attachment_id)
        .bind(input.project_id)
        .bind(&input.client_session_id)
        .execute(&mut **transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(ThreadRepositoryError::conflict("agent.attachment.claim_conflict", "附件已被认领或已过期"));
        }
    }
    Ok(())
}

async fn attachment_row(transaction: &mut Transaction<'_, Sqlite>, attachment_id: &str) -> Result<sqlx::sqlite::SqliteRow, ThreadRepositoryError> {
    sqlx::query(
        "SELECT attachment_id, project_id, upload_session_id, status, sha256, byte_size, expire_at, claimed_thread_id, claimed_turn_id \
         FROM ai_agent_attachment WHERE attachment_id = ?1",
    )
    .bind(attachment_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| ThreadRepositoryError::NotFound { entity: "attachment", key: attachment_id.to_string() })
}

fn validate_attachment_row(row: &sqlx::sqlite::SqliteRow, input: &CreateTurnInput, claim: &AttachmentClaim) -> Result<(), ThreadRepositoryError> {
    if row.try_get::<i64, _>("project_id")? != input.project_id || row.try_get::<String, _>("upload_session_id")? != input.client_session_id {
        return Err(ThreadRepositoryError::ScopeMismatch { entity: "attachment", key: claim.attachment_id.clone() });
    }
    let status: String = row.try_get("status")?;
    let claimed_thread: Option<String> = row.try_get("claimed_thread_id")?;
    let claimed_turn: Option<String> = row.try_get("claimed_turn_id")?;
    if status != "temporary" || claimed_thread.is_some() || claimed_turn.is_some() || row.try_get::<i64, _>("expire_at")? <= input.created_at {
        return Err(ThreadRepositoryError::conflict("agent.attachment.unavailable", "附件不可用、已过期或已被认领"));
    }
    if let Some(expected) = claim.expected_sha256.as_deref() {
        let stored: String = row.try_get("sha256")?;
        if stored != expected {
            return Err(ThreadRepositoryError::conflict("agent.attachment.content_changed", "附件哈希已变化"));
        }
    }
    if let Some(expected) = claim.expected_byte_size {
        let stored = u64::try_from(row.try_get::<i64, _>("byte_size")?)
            .map_err(|_| ThreadRepositoryError::InvalidRecord("attachment byte_size must not be negative".to_string()))?;
        if stored != expected {
            return Err(ThreadRepositoryError::conflict("agent.attachment.content_changed", "附件大小已变化"));
        }
    }
    Ok(())
}

async fn ensure_conversation_scope(transaction: &mut Transaction<'_, Sqlite>, project_id: i64, thread_id: &str) -> Result<(), ThreadRepositoryError> {
    let stored = sqlx::query_scalar::<_, i64>("SELECT project_id FROM ai_agent_conversation WHERE thread_id = ?1")
        .bind(thread_id)
        .fetch_optional(&mut **transaction)
        .await?;
    if stored == Some(project_id) {
        Ok(())
    } else {
        Err(ThreadRepositoryError::ScopeMismatch { entity: "conversation", key: thread_id.to_string() })
    }
}

async fn set_default_title(
    transaction: &mut Transaction<'_, Sqlite>,
    thread_id: &str,
    input_summary: &str,
    updated_at: i64,
) -> Result<(), ThreadRepositoryError> {
    let title = input_summary.trim().chars().take(60).collect::<String>();
    if title.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE ai_agent_thread SET title = ?1, updated_at = MAX(updated_at, ?2) \
         WHERE thread_id = ?3 AND title IS NULL AND title_source = 'auto'",
    )
    .bind(title)
    .bind(updated_at)
    .bind(thread_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn find_turn_by_request(transaction: &mut Transaction<'_, Sqlite>, request_id: &str) -> Result<Option<AgentTurn>, ThreadRepositoryError> {
    let query = format!("{TURN_SELECT} WHERE request_id = ?1");
    sqlx::query(&query)
        .bind(request_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(turn_from_row)
        .transpose()
}

async fn find_turn(transaction: &mut Transaction<'_, Sqlite>, turn_id: &str) -> Result<Option<AgentTurn>, ThreadRepositoryError> {
    let query = format!("{TURN_SELECT} WHERE turn_id = ?1");
    sqlx::query(&query)
        .bind(turn_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(turn_from_row)
        .transpose()
}

async fn find_item(transaction: &mut Transaction<'_, Sqlite>, turn_id: &str, item_id: &str) -> Result<Option<AgentItem>, ThreadRepositoryError> {
    let query = format!("{ITEM_SELECT} WHERE turn_id = ?1 AND item_id = ?2");
    sqlx::query(&query)
        .bind(turn_id)
        .bind(item_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(item_from_row)
        .transpose()
}

async fn find_initial_user_item(transaction: &mut Transaction<'_, Sqlite>, turn_id: &str) -> Result<Option<AgentItem>, ThreadRepositoryError> {
    let query = format!("{ITEM_SELECT} WHERE turn_id = ?1 AND item_sequence = 1 AND item_type = 'userMessage'");
    sqlx::query(&query)
        .bind(turn_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(item_from_row)
        .transpose()
}

async fn find_run(transaction: &mut Transaction<'_, Sqlite>, run_id: &str) -> Result<Option<AgentTaskRunRef>, ThreadRepositoryError> {
    let query = format!("{RUN_SELECT} WHERE r.run_id = ?1");
    sqlx::query(&query)
        .bind(run_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(run_from_row)
        .transpose()
}

async fn find_run_by_turn(transaction: &mut Transaction<'_, Sqlite>, turn_id: &str) -> Result<Option<AgentTaskRunRef>, ThreadRepositoryError> {
    let query = format!("{RUN_SELECT} WHERE r.turn_id = ?1 ORDER BY r.attempt_no DESC LIMIT 1");
    sqlx::query(&query)
        .bind(turn_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(run_from_row)
        .transpose()
}

fn item_payload(item_id: &str, item_type: &str, status: &str, payload: &Value) -> Value {
    json!({
        "item": {
            "itemId": item_id,
            "itemType": item_type,
            "status": status,
            "payload": payload
        }
    })
}

fn validate_create_turn(input: &CreateTurnInput) -> Result<(), ThreadRepositoryError> {
    if input.project_id <= 0
        || [
            input.thread_id.as_str(),
            input.request_id.as_str(),
            input.model_config_id.as_str(),
            input.runtime_model_id.as_str(),
            input.entry_digest.as_str(),
            input.client_session_id.as_str(),
            input.task_type.as_str(),
        ]
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err(ThreadRepositoryError::InvalidRecord("Turn required fields must not be blank".to_string()));
    }
    let mut attachment_ids = std::collections::HashSet::new();
    if input
        .attachments
        .iter()
        .any(|claim| !attachment_ids.insert(claim.attachment_id.as_str()))
    {
        return Err(ThreadRepositoryError::InvalidRecord("duplicate attachment id".to_string()));
    }
    Ok(())
}

async fn map_turn_insert_error(transaction: &mut Transaction<'_, Sqlite>, input: &CreateTurnInput, error: sqlx::Error) -> ThreadRepositoryError {
    if !error
        .as_database_error()
        .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
    {
        return ThreadRepositoryError::Sql(error);
    }

    let request_exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM ai_agent_turn WHERE request_id = ?1")
        .bind(&input.request_id)
        .fetch_one(&mut **transaction)
        .await
        .is_ok_and(|count| count > 0);
    if request_exists {
        return ThreadRepositoryError::conflict("agent.turn.request_conflict", "requestId 已存在");
    }
    let thread_active = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM ai_agent_turn WHERE thread_id = ?1 \
         AND status IN ('created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation')",
    )
    .bind(&input.thread_id)
    .fetch_one(&mut **transaction)
    .await
    .is_ok_and(|count| count > 0);
    if thread_active {
        return ThreadRepositoryError::conflict("agent.thread.active_turn", "当前 Thread 已有活动 Turn");
    }
    let project_active = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM ai_agent_turn WHERE project_id = ?1 \
         AND status IN ('created','running','verifying','waitingFrontendTool','waitingUserInput','waitingUserConfirmation')",
    )
    .bind(input.project_id)
    .fetch_one(&mut **transaction)
    .await
    .is_ok_and(|count| count > 0);
    if project_active {
        return ThreadRepositoryError::conflict("agent.project.active_turn", "当前项目已有活动 Turn");
    }

    // SQLite 的 UNIQUE 错误文本通常只含列名，不含索引名；列文本仅作查询无法定位时的兜底。
    let message = error.to_string();
    if message.contains("ai_agent_turn.request_id") {
        ThreadRepositoryError::conflict("agent.turn.request_conflict", "requestId 已存在")
    } else if message.contains("ai_agent_turn.project_id") {
        ThreadRepositoryError::conflict("agent.project.active_turn", "当前项目已有活动 Turn")
    } else if message.contains("ai_agent_turn.thread_id") {
        ThreadRepositoryError::conflict("agent.thread.active_turn", "当前 Thread 已有活动 Turn")
    } else {
        ThreadRepositoryError::conflict("agent.turn.unique_conflict", "Turn 唯一约束冲突，请刷新后重试")
    }
}
