/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! TaskRun 仓储：以 Thread Repository 为唯一状态与 Event 写入边界。

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::pin::Pin;

use sqlx::{Row, SqlitePool};

use crate::backend::ai_driven::thread::{
    AgentItem, AgentTaskRunRef, AgentTurnInput, AppendTimelineEventInput, ApplyItemEventInput, ClaimTurnRunningInput, CompleteTurnInput,
    ConsumeTurnInput, ConsumedTurnInput, CreateResumeAttemptInput, CreateTurnInput, ListRunEventsQuery, MarkTurnWaitingInput, ResumeTurnInput,
    SqliteThreadRepository, StartItemInput, ThreadRepositoryError, TimelineWriteResult, TimelineWriter, TransitionTurnExecutionStatusInput,
};

use super::identity::validate_persisted_identity;
use super::state::{
    AgentArtifactRef, AgentTaskCheckpointResult, AgentTaskError, AgentTaskEvent, AgentTaskResumeResult, AgentTaskRun, AgentTaskStatus,
    ListTaskEvents, NewAgentTaskEvent, NewAgentTaskRun, SaveAgentTaskCheckpoint,
};
use super::{AgentWorkingCheckpoint, AgentWorkingCheckpointState, SaveAgentWorkingCheckpoint};

pub(crate) type TaskRepositoryFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, TaskRepositoryError>> + Send + 'a>>;

/// Project 工作区恢复所需的最小 Run 历史，按结束时间从新到旧返回。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentWorkspaceHistoryEntry {
    pub turn_id: String,
    pub run_id: String,
    pub status: AgentTaskStatus,
    pub result_revision: Option<u64>,
}

#[derive(Debug)]
pub(crate) enum TaskRepositoryError {
    Thread(ThreadRepositoryError),
    Sql(sqlx::Error),
    Json(serde_json::Error),
    NotFound { entity: &'static str, key: String },
    ScopeMismatch { entity: &'static str, key: String },
    InvalidRecord(String),
}

impl TaskRepositoryError {
    /// Returns whether this error is a transient SQLite write lock conflict.
    ///
    /// Repository calls that go through the thread layer retain the thread
    /// error classification. Direct task-table calls use the same SQLite
    /// result-code check here so callers can apply one retry policy to both
    /// paths. Only SQLITE_BUSY (5) and SQLITE_LOCKED (6), including extended
    /// codes, are considered retryable.
    pub(crate) fn is_transient_write_conflict(&self) -> bool {
        match self {
            Self::Thread(error) => error.is_transient_write_conflict(),
            Self::Sql(error) => is_transient_sqlite_error(error),
            _ => false,
        }
    }
}

impl Display for TaskRepositoryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Thread(error) => write!(formatter, "{error}"),
            Self::Sql(error) => write!(formatter, "SQLite task repository failed: {error}"),
            Self::Json(error) => write!(formatter, "task JSON failed: {error}"),
            Self::NotFound { entity, key } => write!(formatter, "{entity} not found: {key}"),
            Self::ScopeMismatch { entity, key } => write!(formatter, "{entity} scope mismatch: {key}"),
            Self::InvalidRecord(message) => formatter.write_str(message),
        }
    }
}

impl Error for TaskRepositoryError {}

impl From<ThreadRepositoryError> for TaskRepositoryError {
    fn from(error: ThreadRepositoryError) -> Self {
        Self::Thread(error)
    }
}

impl From<sqlx::Error> for TaskRepositoryError {
    fn from(error: sqlx::Error) -> Self {
        Self::Sql(error)
    }
}

impl From<serde_json::Error> for TaskRepositoryError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

fn is_transient_sqlite_error(error: &sqlx::Error) -> bool {
    let Some(code) = error.as_database_error().and_then(|database_error| database_error.code()) else {
        return false;
    };
    let Ok(code) = code.parse::<i64>() else {
        return false;
    };

    matches!(code & 0xff, 5 | 6)
}

pub(crate) trait AgentTaskRepository: Send + Sync {
    /// 原子创建 Turn、用户消息、TaskRun 与初始事件，并返回创建后的任务运行记录。
    fn create<'a>(&'a self, input: NewAgentTaskRun) -> TaskRepositoryFuture<'a, AgentTaskRun>;

    /// 基于中断任务的检查点创建一次恢复尝试，并返回新的可执行任务与恢复结果。
    fn create_resume_attempt<'a>(&'a self, input: CreateResumeAttemptInput) -> TaskRepositoryFuture<'a, AgentTaskResumeResult>;

    /// 按 Run ID 查找任务运行记录；不存在时返回 `None`。
    fn find_by_run_id<'a>(&'a self, run_id: String) -> TaskRepositoryFuture<'a, Option<AgentTaskRun>>;

    /// 按请求 ID 查找任务运行记录，用于提交幂等性判断；不存在时返回 `None`。
    fn find_by_request_id<'a>(&'a self, request_id: String) -> TaskRepositoryFuture<'a, Option<AgentTaskRun>>;

    /// 列出项目的工作区历史记录，供恢复前检查工作区是否发生变化。
    fn list_workspace_history<'a>(&'a self, project_id: i64) -> TaskRepositoryFuture<'a, Vec<AgentWorkspaceHistoryEntry>>;

    /// 按预期版本保存任务终态检查点，并返回任务状态、事件与写入结果。
    fn save_checkpoint<'a>(&'a self, checkpoint: SaveAgentTaskCheckpoint) -> TaskRepositoryFuture<'a, AgentTaskCheckpointResult>;

    /// 读取指定 Run 的可恢复工作检查点；不存在时返回 `None`。
    fn find_working_checkpoint<'a>(&'a self, run_id: String) -> TaskRepositoryFuture<'a, Option<AgentWorkingCheckpoint>>;

    /// 按乐观锁版本保存运行中的工作检查点，并返回保存后的最新检查点。
    fn save_working_checkpoint<'a>(&'a self, checkpoint: SaveAgentWorkingCheckpoint) -> TaskRepositoryFuture<'a, AgentWorkingCheckpoint>;

    /// 创建一条 Agent 时间线项目，并返回包含序号的写入结果。
    fn start_item<'a>(&'a self, input: StartItemInput) -> TaskRepositoryFuture<'a, TimelineWriteResult>;

    /// 将事件应用到既有时间线项目，并返回更新后的时间线写入结果。
    fn apply_item_event<'a>(&'a self, input: ApplyItemEventInput) -> TaskRepositoryFuture<'a, TimelineWriteResult>;

    /// 在项目、Thread 与 Turn 范围内查找指定时间线项目；不存在时返回 `None`。
    fn find_item<'a>(&'a self, project_id: i64, thread_id: String, turn_id: String, item_id: String) -> TaskRepositoryFuture<'a, Option<AgentItem>>;

    /// 读取尚未消费的已接收用户追加输入，按 `limit` 限制返回数量。
    fn list_accepted_turn_inputs<'a>(
        &'a self,
        project_id: i64,
        thread_id: String,
        turn_id: String,
        limit: u32,
    ) -> TaskRepositoryFuture<'a, Vec<AgentTurnInput>>;

    /// 原子标记一条追加输入已被消费，并返回实际消费的输入内容。
    fn consume_turn_input<'a>(&'a self, input: ConsumeTurnInput) -> TaskRepositoryFuture<'a, ConsumedTurnInput>;

    /// 追加一条任务事件，并返回带持久化序号的事件记录。
    fn append_event<'a>(&'a self, event: NewAgentTaskEvent) -> TaskRepositoryFuture<'a, AgentTaskEvent>;

    /// 按任务范围和游标分页读取事件，用于前端回放与断线补偿。
    fn list_events<'a>(&'a self, query: ListTaskEvents) -> TaskRepositoryFuture<'a, Vec<AgentTaskEvent>>;

    /// 将所有未终态任务标记为中断，并返回本次更新的任务数量。
    fn mark_non_terminal_interrupted<'a>(&'a self, interrupted_at: i64) -> TaskRepositoryFuture<'a, u64>;
}

#[derive(Clone)]
pub(crate) struct SqliteAgentTaskRepository {
    pool: SqlitePool,
    threads: SqliteThreadRepository,
    timeline: TimelineWriter,
}

impl SqliteAgentTaskRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self {
            pool: pool.clone(),
            threads: SqliteThreadRepository::new(pool.clone()),
            timeline: TimelineWriter::new(pool),
        }
    }

    async fn create_inner(&self, input: NewAgentTaskRun) -> Result<AgentTaskRun, TaskRepositoryError> {
        validate_new_task(&input)?;
        let result = self
            .threads
            .create_turn(CreateTurnInput {
                project_id: input.project_id,
                thread_id: input.thread_id,
                request_id: input.request_id,
                request: input.request,
                input_summary: input.input_summary,
                model_config_id: input.model_config_id,
                runtime_model_id: input.runtime_model_id,
                provider: input.provider,
                entry_digest: input.entry_digest,
                client_session_id: input.client_session_id,
                task_type: input.task_type,
                limits: serde_json::to_value(input.limits)?,
                project_snapshot_ref: input.project_snapshot_ref,
                rig_version: input.rig_version,
                rig_snapshot_ref: input.rig_snapshot_ref,
                user_item_payload: input.user_item_payload,
                attachments: input.attachments,
                created_at: input.created_at,
            })
            .await?;
        task_from_ref(result.run)
    }

    async fn create_resume_attempt_inner(&self, input: CreateResumeAttemptInput) -> Result<AgentTaskResumeResult, TaskRepositoryError> {
        let result = self.threads.create_resume_attempt(input).await?;
        Ok(AgentTaskResumeResult { task: task_from_ref(result.run)?, events: result.events, replayed: result.replayed })
    }

    async fn find_by_run_id_inner(&self, run_id: &str) -> Result<Option<AgentTaskRun>, TaskRepositoryError> {
        self.threads
            .find_task_run_by_run_id(run_id)
            .await?
            .map(task_from_ref)
            .transpose()
    }

    async fn find_by_request_id_inner(&self, request_id: &str) -> Result<Option<AgentTaskRun>, TaskRepositoryError> {
        self.threads
            .find_task_run_by_request_id(request_id)
            .await?
            .map(task_from_ref)
            .transpose()
    }

    async fn list_workspace_history_inner(&self, project_id: i64) -> Result<Vec<AgentWorkspaceHistoryEntry>, TaskRepositoryError> {
        if project_id <= 0 {
            return Err(TaskRepositoryError::InvalidRecord("projectId 必须为正整数".to_string()));
        }
        let rows = sqlx::query_as::<_, (String, String, String, Option<i64>)>(
            "SELECT turn.turn_id, run.run_id, run.status, \
               CASE WHEN run.last_artifact_id = 'result' THEN run.last_artifact_revision ELSE NULL END \
             FROM ai_agent_task_run run JOIN ai_agent_turn turn ON turn.turn_id = run.turn_id \
             WHERE turn.project_id = ?1 \
             ORDER BY COALESCE(run.ended_at, run.updated_at) DESC, run.attempt_no DESC, run.run_id DESC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(turn_id, run_id, status, revision)| {
                let status =
                    AgentTaskStatus::from_str(&status).ok_or_else(|| TaskRepositoryError::InvalidRecord(format!("unknown task status: {status}")))?;
                let result_revision = revision
                    .map(|revision| {
                        u64::try_from(revision).map_err(|_| TaskRepositoryError::InvalidRecord(format!("negative result revision: runId={run_id}")))
                    })
                    .transpose()?;
                Ok(AgentWorkspaceHistoryEntry { turn_id, run_id, status, result_revision })
            })
            .collect()
    }

    async fn save_checkpoint_inner(&self, checkpoint: SaveAgentTaskCheckpoint) -> Result<AgentTaskCheckpointResult, TaskRepositoryError> {
        validate_checkpoint(&checkpoint)?;
        let current = self
            .find_by_run_id_inner(&checkpoint.run_id)
            .await?
            .ok_or_else(|| TaskRepositoryError::NotFound { entity: "task run", key: checkpoint.run_id.clone() })?;
        ensure_checkpoint_scope(&current, &checkpoint)?;
        ensure_immutable_refs(&current, &checkpoint)?;

        let result = match checkpoint.status {
            AgentTaskStatus::Created if current.status == AgentTaskStatus::Created => {
                AgentTaskCheckpointResult { task: current, events: Vec::new(), closed_items: Vec::new() }
            }
            AgentTaskStatus::Running if current.status == AgentTaskStatus::Created => {
                let (_, run, event) = self
                    .threads
                    .claim_turn_running(ClaimTurnRunningInput {
                        project_id: checkpoint.project_id,
                        thread_id: checkpoint.thread_id,
                        turn_id: checkpoint.turn_id,
                        run_id: checkpoint.run_id,
                        started_at: checkpoint.updated_at,
                    })
                    .await?;
                AgentTaskCheckpointResult { task: task_from_ref(run)?, events: vec![event], closed_items: Vec::new() }
            }
            AgentTaskStatus::Running if current.status.is_waiting() => {
                let (_, run, event) = self
                    .threads
                    .resume_turn(ResumeTurnInput {
                        project_id: checkpoint.project_id,
                        thread_id: checkpoint.thread_id,
                        turn_id: checkpoint.turn_id,
                        run_id: checkpoint.run_id,
                        updated_at: checkpoint.updated_at,
                    })
                    .await?;
                AgentTaskCheckpointResult { task: task_from_ref(run)?, events: vec![event], closed_items: Vec::new() }
            }
            AgentTaskStatus::Running if current.status == AgentTaskStatus::Running => {
                AgentTaskCheckpointResult { task: current, events: Vec::new(), closed_items: Vec::new() }
            }
            AgentTaskStatus::Verifying if current.status == AgentTaskStatus::Running => {
                let (_, run, event) = self
                    .threads
                    .transition_turn_execution_status(TransitionTurnExecutionStatusInput {
                        project_id: checkpoint.project_id,
                        thread_id: checkpoint.thread_id,
                        turn_id: checkpoint.turn_id,
                        run_id: checkpoint.run_id,
                        status: AgentTaskStatus::Verifying.turn_status(),
                        updated_at: checkpoint.updated_at,
                    })
                    .await?;
                AgentTaskCheckpointResult { task: task_from_ref(run)?, events: vec![event], closed_items: Vec::new() }
            }
            AgentTaskStatus::Verifying if current.status == AgentTaskStatus::Verifying => {
                AgentTaskCheckpointResult { task: current, events: Vec::new(), closed_items: Vec::new() }
            }
            AgentTaskStatus::Running if current.status == AgentTaskStatus::Verifying => {
                let (_, run, event) = self
                    .threads
                    .transition_turn_execution_status(TransitionTurnExecutionStatusInput {
                        project_id: checkpoint.project_id,
                        thread_id: checkpoint.thread_id,
                        turn_id: checkpoint.turn_id,
                        run_id: checkpoint.run_id,
                        status: AgentTaskStatus::Running.turn_status(),
                        updated_at: checkpoint.updated_at,
                    })
                    .await?;
                AgentTaskCheckpointResult { task: task_from_ref(run)?, events: vec![event], closed_items: Vec::new() }
            }
            status if status.is_waiting() && current.status == AgentTaskStatus::Running => {
                let pending = checkpoint
                    .pending
                    .as_ref()
                    .ok_or_else(|| TaskRepositoryError::InvalidRecord("waiting checkpoint requires pending".to_string()))?;
                let (_, run, event) = self
                    .threads
                    .mark_turn_waiting(MarkTurnWaitingInput {
                        project_id: checkpoint.project_id,
                        thread_id: checkpoint.thread_id,
                        turn_id: checkpoint.turn_id,
                        run_id: checkpoint.run_id,
                        status: status.turn_status(),
                        waiting_item_id: pending.waiting_item_id.clone(),
                        pending: serde_json::to_value(pending)?,
                        updated_at: checkpoint.updated_at,
                    })
                    .await?;
                AgentTaskCheckpointResult { task: task_from_ref(run)?, events: vec![event], closed_items: Vec::new() }
            }
            status if status.is_waiting() && current.status == status && current.pending == checkpoint.pending => {
                AgentTaskCheckpointResult { task: current, events: Vec::new(), closed_items: Vec::new() }
            }
            status if status.is_terminal() => {
                let ended_at = checkpoint
                    .ended_at
                    .ok_or_else(|| TaskRepositoryError::InvalidRecord("terminal checkpoint requires ended_at".to_string()))?;
                let result = self
                    .threads
                    .complete_turn(CompleteTurnInput {
                        project_id: checkpoint.project_id,
                        thread_id: checkpoint.thread_id,
                        turn_id: checkpoint.turn_id,
                        run_id: checkpoint.run_id,
                        status: status.turn_status(),
                        usage: checkpoint.usage,
                        last_artifact_id: checkpoint.last_artifact.as_ref().map(|artifact| artifact.artifact_id.clone()),
                        last_artifact_revision: checkpoint.last_artifact.as_ref().map(|artifact| artifact.revision),
                        error_code: checkpoint.error.as_ref().map(|error| error.code.clone()),
                        error_message: checkpoint.error.as_ref().map(|error| error.message.clone()),
                        thread_preview: checkpoint.thread_preview,
                        ended_at,
                    })
                    .await?;
                AgentTaskCheckpointResult { task: task_from_ref(result.run)?, events: result.events, closed_items: result.closed_items }
            }
            _ => {
                return Err(TaskRepositoryError::InvalidRecord(format!(
                    "task {} cannot transition from {} to {}",
                    checkpoint.run_id,
                    current.status.as_str(),
                    checkpoint.status.as_str()
                )));
            }
        };
        Ok(result)
    }

    async fn append_event_inner(&self, event: NewAgentTaskEvent) -> Result<AgentTaskEvent, TaskRepositoryError> {
        validate_event(&event)?;
        Ok(self
            .timeline
            .append_event(AppendTimelineEventInput {
                project_id: event.project_id,
                thread_id: event.thread_id,
                turn_id: event.turn_id,
                run_id: event.run_id,
                item_id: event.item_id,
                kind: event.kind,
                payload: event.payload,
                dedupe_key: event.dedupe_key,
                schema_version: event.schema_version,
                created_at: event.created_at,
            })
            .await?
            .event)
    }

    async fn start_item_inner(&self, input: StartItemInput) -> Result<TimelineWriteResult, TaskRepositoryError> {
        Ok(self.timeline.start_item(input).await?)
    }

    async fn apply_item_event_inner(&self, input: ApplyItemEventInput) -> Result<TimelineWriteResult, TaskRepositoryError> {
        Ok(self.timeline.apply_item_event(input).await?)
    }

    async fn list_events_inner(&self, query: ListTaskEvents) -> Result<Vec<AgentTaskEvent>, TaskRepositoryError> {
        validate_persisted_identity(query.project_id, &query.thread_id, &query.turn_id, &query.run_id)?;
        Ok(self
            .threads
            .list_run_events(ListRunEventsQuery {
                project_id: query.project_id,
                thread_id: query.thread_id,
                turn_id: query.turn_id,
                run_id: query.run_id,
                after_sequence: query.after_sequence,
                limit: query.limit,
            })
            .await?
            .data)
    }

    async fn mark_non_terminal_interrupted_inner(&self, interrupted_at: i64) -> Result<u64, TaskRepositoryError> {
        Ok(self.threads.interrupt_active_turns(interrupted_at).await?)
    }

    async fn find_working_checkpoint_inner(&self, run_id: &str) -> Result<Option<AgentWorkingCheckpoint>, TaskRepositoryError> {
        let row = sqlx::query(
            "SELECT checkpoint.task_id, checkpoint.run_id, checkpoint.turn_id, checkpoint.checkpoint_version, \
                    checkpoint.state_json, checkpoint.updated_at \
             FROM ai_agent_working_checkpoint checkpoint \
             JOIN ai_agent_task_run run ON run.run_id = checkpoint.run_id AND run.task_id = checkpoint.task_id \
             WHERE checkpoint.run_id = ?1",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(working_checkpoint_from_row).transpose()
    }

    async fn save_working_checkpoint_inner(&self, checkpoint: SaveAgentWorkingCheckpoint) -> Result<AgentWorkingCheckpoint, TaskRepositoryError> {
        checkpoint.state.validate().map_err(TaskRepositoryError::InvalidRecord)?;
        validate_persisted_identity(checkpoint.project_id, &checkpoint.thread_id, &checkpoint.turn_id, &checkpoint.run_id)?;
        require_text("task_id", &checkpoint.task_id)?;
        let expected_version = i64::try_from(checkpoint.expected_version)
            .map_err(|_| TaskRepositoryError::InvalidRecord("checkpoint version exceeds SQLite INTEGER".to_string()))?;
        let state_json = serde_json::to_string(&checkpoint.state)?;
        let mut transaction = self.pool.begin().await?;
        let scope = sqlx::query(
            "SELECT run.task_id, run.turn_id, turn.project_id, turn.thread_id \
             FROM ai_agent_task_run run JOIN ai_agent_turn turn ON turn.turn_id = run.turn_id \
             WHERE run.run_id = ?1",
        )
        .bind(&checkpoint.run_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| TaskRepositoryError::NotFound { entity: "task run", key: checkpoint.run_id.clone() })?;
        let stored_task_id: String = scope.try_get("task_id")?;
        let stored_turn_id: String = scope.try_get("turn_id")?;
        let stored_project_id: i64 = scope.try_get("project_id")?;
        let stored_thread_id: String = scope.try_get("thread_id")?;
        if stored_task_id != checkpoint.task_id
            || stored_turn_id != checkpoint.turn_id
            || stored_project_id != checkpoint.project_id
            || stored_thread_id != checkpoint.thread_id
        {
            return Err(TaskRepositoryError::ScopeMismatch { entity: "working checkpoint", key: checkpoint.run_id });
        }

        let existing = sqlx::query("SELECT checkpoint_version, state_json FROM ai_agent_working_checkpoint WHERE run_id = ?1")
            .bind(&checkpoint.run_id)
            .fetch_optional(&mut *transaction)
            .await?;
        let version = match existing {
            None if expected_version == 0 => {
                sqlx::query(
                    "INSERT INTO ai_agent_working_checkpoint \
                     (run_id, task_id, turn_id, checkpoint_version, state_json, updated_at) \
                     VALUES (?1, ?2, ?3, 1, ?4, ?5)",
                )
                .bind(&checkpoint.run_id)
                .bind(&checkpoint.task_id)
                .bind(&checkpoint.turn_id)
                .bind(&state_json)
                .bind(checkpoint.state.updated_at)
                .execute(&mut *transaction)
                .await?;
                1_i64
            }
            Some(row) => {
                let current_version: i64 = row.try_get("checkpoint_version")?;
                let current_state: String = row.try_get("state_json")?;
                if current_version == expected_version.saturating_add(1) && current_state == state_json {
                    current_version
                } else if current_version == expected_version {
                    let updated = sqlx::query(
                        "UPDATE ai_agent_working_checkpoint \
                         SET checkpoint_version = checkpoint_version + 1, state_json = ?1, updated_at = ?2 \
                         WHERE run_id = ?3 AND checkpoint_version = ?4",
                    )
                    .bind(&state_json)
                    .bind(checkpoint.state.updated_at)
                    .bind(&checkpoint.run_id)
                    .bind(expected_version)
                    .execute(&mut *transaction)
                    .await?;
                    if updated.rows_affected() != 1 {
                        return Err(TaskRepositoryError::InvalidRecord("工作检查点发生并发冲突".to_string()));
                    }
                    current_version.saturating_add(1)
                } else {
                    return Err(TaskRepositoryError::InvalidRecord(format!(
                        "工作检查点版本冲突：expected={expected_version}, actual={current_version}"
                    )));
                }
            }
            None => return Err(TaskRepositoryError::InvalidRecord("首次工作检查点 expectedVersion 必须为 0".to_string())),
        };
        transaction.commit().await?;
        Ok(AgentWorkingCheckpoint {
            task_id: checkpoint.task_id,
            run_id: checkpoint.run_id,
            turn_id: checkpoint.turn_id,
            checkpoint_version: u64::try_from(version)
                .map_err(|_| TaskRepositoryError::InvalidRecord("checkpoint version is negative".to_string()))?,
            state: checkpoint.state.clone(),
            updated_at: checkpoint.state.updated_at,
        })
    }
}

impl AgentTaskRepository for SqliteAgentTaskRepository {
    fn create<'a>(&'a self, input: NewAgentTaskRun) -> TaskRepositoryFuture<'a, AgentTaskRun> {
        Box::pin(async move { self.create_inner(input).await })
    }

    fn create_resume_attempt<'a>(&'a self, input: CreateResumeAttemptInput) -> TaskRepositoryFuture<'a, AgentTaskResumeResult> {
        Box::pin(async move { self.create_resume_attempt_inner(input).await })
    }

    fn find_by_run_id<'a>(&'a self, run_id: String) -> TaskRepositoryFuture<'a, Option<AgentTaskRun>> {
        Box::pin(async move { self.find_by_run_id_inner(&run_id).await })
    }

    fn find_by_request_id<'a>(&'a self, request_id: String) -> TaskRepositoryFuture<'a, Option<AgentTaskRun>> {
        Box::pin(async move { self.find_by_request_id_inner(&request_id).await })
    }

    fn list_workspace_history<'a>(&'a self, project_id: i64) -> TaskRepositoryFuture<'a, Vec<AgentWorkspaceHistoryEntry>> {
        Box::pin(async move { self.list_workspace_history_inner(project_id).await })
    }

    fn save_checkpoint<'a>(&'a self, checkpoint: SaveAgentTaskCheckpoint) -> TaskRepositoryFuture<'a, AgentTaskCheckpointResult> {
        Box::pin(async move { self.save_checkpoint_inner(checkpoint).await })
    }

    fn find_working_checkpoint<'a>(&'a self, run_id: String) -> TaskRepositoryFuture<'a, Option<AgentWorkingCheckpoint>> {
        Box::pin(async move { self.find_working_checkpoint_inner(&run_id).await })
    }

    fn save_working_checkpoint<'a>(&'a self, checkpoint: SaveAgentWorkingCheckpoint) -> TaskRepositoryFuture<'a, AgentWorkingCheckpoint> {
        Box::pin(async move { self.save_working_checkpoint_inner(checkpoint).await })
    }

    fn start_item<'a>(&'a self, input: StartItemInput) -> TaskRepositoryFuture<'a, TimelineWriteResult> {
        Box::pin(async move { self.start_item_inner(input).await })
    }

    fn apply_item_event<'a>(&'a self, input: ApplyItemEventInput) -> TaskRepositoryFuture<'a, TimelineWriteResult> {
        Box::pin(async move { self.apply_item_event_inner(input).await })
    }

    fn find_item<'a>(&'a self, project_id: i64, thread_id: String, turn_id: String, item_id: String) -> TaskRepositoryFuture<'a, Option<AgentItem>> {
        Box::pin(async move { Ok(self.threads.find_item(project_id, &thread_id, &turn_id, &item_id).await?) })
    }

    fn list_accepted_turn_inputs<'a>(
        &'a self,
        project_id: i64,
        thread_id: String,
        turn_id: String,
        limit: u32,
    ) -> TaskRepositoryFuture<'a, Vec<AgentTurnInput>> {
        Box::pin(async move {
            Ok(self
                .threads
                .list_accepted_turn_inputs(project_id, &thread_id, &turn_id, limit)
                .await?)
        })
    }

    fn consume_turn_input<'a>(&'a self, input: ConsumeTurnInput) -> TaskRepositoryFuture<'a, ConsumedTurnInput> {
        Box::pin(async move { Ok(self.threads.consume_turn_input(input).await?) })
    }

    fn append_event<'a>(&'a self, event: NewAgentTaskEvent) -> TaskRepositoryFuture<'a, AgentTaskEvent> {
        Box::pin(async move { self.append_event_inner(event).await })
    }

    fn list_events<'a>(&'a self, query: ListTaskEvents) -> TaskRepositoryFuture<'a, Vec<AgentTaskEvent>> {
        Box::pin(async move { self.list_events_inner(query).await })
    }

    fn mark_non_terminal_interrupted<'a>(&'a self, interrupted_at: i64) -> TaskRepositoryFuture<'a, u64> {
        Box::pin(async move { self.mark_non_terminal_interrupted_inner(interrupted_at).await })
    }
}

fn working_checkpoint_from_row(row: sqlx::sqlite::SqliteRow) -> Result<AgentWorkingCheckpoint, TaskRepositoryError> {
    let version = row.try_get::<i64, _>("checkpoint_version")?;
    let state_json = row.try_get::<String, _>("state_json")?;
    let state = AgentWorkingCheckpointState::from_json(&state_json).map_err(TaskRepositoryError::InvalidRecord)?;
    Ok(AgentWorkingCheckpoint {
        task_id: row.try_get("task_id")?,
        run_id: row.try_get("run_id")?,
        turn_id: row.try_get("turn_id")?,
        checkpoint_version: u64::try_from(version).map_err(|_| TaskRepositoryError::InvalidRecord("checkpoint version is negative".to_string()))?,
        state,
        updated_at: row.try_get("updated_at")?,
    })
}

fn task_from_ref(run: AgentTaskRunRef) -> Result<AgentTaskRun, TaskRepositoryError> {
    let status =
        AgentTaskStatus::from_str(&run.status).ok_or_else(|| TaskRepositoryError::InvalidRecord(format!("unknown task status: {}", run.status)))?;
    let last_artifact = match (run.last_artifact_id, run.last_artifact_revision) {
        (None, None) => None,
        (Some(artifact_id), Some(revision)) => Some(AgentArtifactRef { artifact_id, revision }),
        _ => return Err(TaskRepositoryError::InvalidRecord("artifact id and revision must both be present".to_string())),
    };
    let error = match (run.error_code, run.error_message) {
        (None, None) => None,
        (Some(code), Some(message)) => Some(AgentTaskError { code, message }),
        _ => return Err(TaskRepositoryError::InvalidRecord("error code and message must both be present".to_string())),
    };
    Ok(AgentTaskRun {
        task_id: run.task_id,
        run_id: run.run_id,
        request_id: run.request_id,
        entry_digest: run.entry_digest,
        project_id: run.project_id,
        thread_id: run.thread_id,
        turn_id: run.turn_id,
        client_session_id: run.client_session_id,
        task_type: run.task_type,
        status,
        limits: serde_json::from_value(run.limits)?,
        pending: run.pending.map(serde_json::from_value).transpose()?,
        usage: run.usage,
        last_artifact,
        error,
        project_snapshot_ref: run.project_snapshot_ref,
        rig_version: run.rig_version,
        rig_snapshot_ref: run.rig_snapshot_ref,
        created_at: run.created_at,
        started_at: run.started_at,
        updated_at: run.updated_at,
        ended_at: run.ended_at,
    })
}

async fn validate_thread_scope(pool: &SqlitePool, project_id: i64, thread_id: &str) -> Result<(), TaskRepositoryError> {
    let stored = sqlx::query_scalar::<_, i64>(
        "SELECT thread.project_id FROM ai_agent_thread thread \
         JOIN project active_project ON active_project.id = thread.project_id AND active_project.deleted = 0 \
         WHERE thread.thread_id = ?1",
    )
    .bind(thread_id)
    .fetch_optional(pool)
    .await?;
    match stored {
        None => Err(TaskRepositoryError::NotFound { entity: "thread", key: thread_id.to_string() }),
        Some(value) if value == project_id => Ok(()),
        Some(_) => Err(TaskRepositoryError::ScopeMismatch { entity: "thread", key: thread_id.to_string() }),
    }
}

fn ensure_checkpoint_scope(current: &AgentTaskRun, checkpoint: &SaveAgentTaskCheckpoint) -> Result<(), TaskRepositoryError> {
    if current.project_id == checkpoint.project_id
        && current.thread_id == checkpoint.thread_id
        && current.turn_id == checkpoint.turn_id
        && current.run_id == checkpoint.run_id
    {
        Ok(())
    } else {
        Err(TaskRepositoryError::ScopeMismatch { entity: "task run", key: checkpoint.run_id.clone() })
    }
}

fn ensure_immutable_refs(current: &AgentTaskRun, checkpoint: &SaveAgentTaskCheckpoint) -> Result<(), TaskRepositoryError> {
    if current.project_snapshot_ref == checkpoint.project_snapshot_ref
        && current.rig_version == checkpoint.rig_version
        && current.rig_snapshot_ref == checkpoint.rig_snapshot_ref
    {
        Ok(())
    } else {
        Err(TaskRepositoryError::InvalidRecord(
            "project and Rig snapshot references are immutable after Turn creation".to_string(),
        ))
    }
}

fn validate_new_task(input: &NewAgentTaskRun) -> Result<(), TaskRepositoryError> {
    if input.project_id <= 0 {
        return Err(TaskRepositoryError::InvalidRecord("project_id must be positive".to_string()));
    }
    for (field, value) in [
        ("thread_id", input.thread_id.as_str()),
        ("request_id", input.request_id.as_str()),
        ("model_config_id", input.model_config_id.as_str()),
        ("runtime_model_id", input.runtime_model_id.as_str()),
        ("entry_digest", input.entry_digest.as_str()),
        ("client_session_id", input.client_session_id.as_str()),
        ("task_type", input.task_type.as_str()),
    ] {
        require_text(field, value)?;
    }
    validate_rig_snapshot(input.rig_version.as_deref(), input.rig_snapshot_ref.as_deref())
}

fn validate_checkpoint(checkpoint: &SaveAgentTaskCheckpoint) -> Result<(), TaskRepositoryError> {
    validate_persisted_identity(checkpoint.project_id, &checkpoint.thread_id, &checkpoint.turn_id, &checkpoint.run_id)?;
    if checkpoint.status.is_waiting() != checkpoint.pending.is_some() {
        return Err(TaskRepositoryError::InvalidRecord("pending must be present exactly for a waiting checkpoint".to_string()));
    }
    if checkpoint
        .pending
        .as_ref()
        .is_some_and(|pending| pending.waiting_item_id.trim().is_empty())
    {
        return Err(TaskRepositoryError::InvalidRecord("waiting checkpoint requires waiting_item_id".to_string()));
    }
    if checkpoint.status.is_terminal() != checkpoint.ended_at.is_some() {
        return Err(TaskRepositoryError::InvalidRecord("ended_at must be present exactly for a terminal checkpoint".to_string()));
    }
    if let Some(artifact) = &checkpoint.last_artifact {
        require_text("artifact_id", &artifact.artifact_id)?;
        i64::try_from(artifact.revision).map_err(|_| TaskRepositoryError::InvalidRecord("artifact revision exceeds SQLite INTEGER".to_string()))?;
    }
    if let Some(error) = &checkpoint.error {
        require_text("error code", &error.code)?;
        require_text("error message", &error.message)?;
    }
    validate_rig_snapshot(checkpoint.rig_version.as_deref(), checkpoint.rig_snapshot_ref.as_deref())
}

fn validate_event(event: &NewAgentTaskEvent) -> Result<(), TaskRepositoryError> {
    validate_persisted_identity(event.project_id, &event.thread_id, &event.turn_id, &event.run_id)?;
    require_text("event kind", &event.kind)?;
    require_text("dedupe_key", &event.dedupe_key)?;
    if event.schema_version == 0 {
        return Err(TaskRepositoryError::InvalidRecord("schema_version must be positive".to_string()));
    }
    Ok(())
}

fn validate_rig_snapshot(version: Option<&str>, snapshot_ref: Option<&str>) -> Result<(), TaskRepositoryError> {
    if let Some(version) = version {
        require_text("rig_version", version)?;
    }
    if let Some(snapshot_ref) = snapshot_ref {
        require_text("rig_snapshot_ref", snapshot_ref)?;
        if version.is_none() {
            return Err(TaskRepositoryError::InvalidRecord("rig_snapshot_ref requires rig_version".to_string()));
        }
    }
    Ok(())
}

fn require_text(field: &str, value: &str) -> Result<(), TaskRepositoryError> {
    if value.trim().is_empty() {
        Err(TaskRepositoryError::InvalidRecord(format!("{field} must not be blank")))
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
