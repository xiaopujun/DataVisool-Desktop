/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 画布动作的持久化幂等日志与 Timeline 投影。

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::json;
use sqlx::{Row, SqlitePool};

use crate::backend::ai_driven::domain::operation::{CanvasOperationKind, CanvasOperationReceipt, CanvasOperationStatus};
use crate::backend::utils::debug_log_json;

use super::{AgentTaskEventPublisher, AgentTaskRepository, AgentTaskRun, NewAgentTaskEvent};

#[derive(Clone, Debug)]
pub(crate) struct CanvasOperationRecord {
    pub(crate) tool_name: String,
    pub(crate) request_fingerprint: String,
    pub(crate) receipt: CanvasOperationReceipt,
    pub(crate) updated_at: i64,
}

#[derive(Debug)]
pub(crate) struct CanvasOperationJournalError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl std::fmt::Display for CanvasOperationJournalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CanvasOperationJournalError {}

#[derive(Clone)]
pub(crate) struct CanvasOperationJournal {
    pool: SqlitePool,
    repository: Arc<dyn AgentTaskRepository>,
    publisher: Option<AgentTaskEventPublisher>,
    task: AgentTaskRun,
}

impl CanvasOperationJournal {
    pub(crate) fn new(pool: SqlitePool, repository: Arc<dyn AgentTaskRepository>, publisher: AgentTaskEventPublisher, task: AgentTaskRun) -> Self {
        Self { pool, repository, publisher: Some(publisher), task }
    }

    pub(crate) async fn prepare(
        &self,
        tool_name: &str,
        tool_call_id: &str,
        kind: CanvasOperationKind,
        request_fingerprint: &str,
        component_ids: Vec<String>,
    ) -> Result<CanvasOperationRecord, CanvasOperationJournalError> {
        let operation_id = operation_id(&self.task.run_id, tool_call_id);
        let now = crate::backend::ai_driven::time::now_ms();
        let receipt = CanvasOperationReceipt::prepared(
            operation_id.clone(),
            tool_call_id.to_string(),
            kind,
            self.task.project_id.to_string(),
            None,
            component_ids,
        );
        let receipt_json = serde_json::to_string(&receipt).map_err(json_error)?;
        sqlx::query(
            "INSERT OR IGNORE INTO ai_agent_canvas_operation (
                operation_id, tool_call_id, tool_name, kind, status, task_id, run_id, thread_id, turn_id,
                project_id, manager_id, request_fingerprint, receipt_json, prepared_at, updated_at, applied_at
             ) SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14, NULL
             WHERE ?4 != 'createComponents' OR NOT EXISTS (
                 SELECT 1 FROM ai_agent_canvas_operation prior
                 WHERE prior.project_id = ?10 AND prior.thread_id = ?8 AND prior.turn_id = ?9
                 AND prior.kind = 'createComponents' AND prior.request_fingerprint = ?12
                 AND prior.status IN ('prepared','applying','applied') AND prior.run_id IN (
                     WITH RECURSIVE lineage(run_id) AS (
                         SELECT ?7 UNION ALL SELECT resume.previous_run_id FROM ai_agent_run_resume resume JOIN lineage ON resume.run_id = lineage.run_id
                     ) SELECT run_id FROM lineage
                 )
             )",
        )
        .bind(&operation_id)
        .bind(tool_call_id)
        .bind(tool_name)
        .bind(kind.as_str())
        .bind(CanvasOperationStatus::Prepared.as_str())
        .bind(&self.task.task_id)
        .bind(&self.task.run_id)
        .bind(&self.task.thread_id)
        .bind(&self.task.turn_id)
        .bind(self.task.project_id.to_string())
        .bind(receipt.manager_id.as_deref())
        .bind(request_fingerprint)
        .bind(receipt_json)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        let record = if kind == CanvasOperationKind::CreateComponents {
            self.find_equivalent_create(request_fingerprint)
                .await?
                .ok_or_else(|| CanvasOperationJournalError {
                    code: "agent.operation.not_found", message: "创建批次日志未保存".to_string()
                })?
        } else {
            self.require(&operation_id).await?
        };
        self.validate_identity(&record, tool_name, kind, request_fingerprint)?;
        if record.receipt.status == CanvasOperationStatus::Prepared {
            self.publish_event(&record).await?;
        }
        Ok(record)
    }

    pub(crate) async fn find_existing(
        &self,
        tool_name: &str,
        tool_call_id: &str,
        kind: CanvasOperationKind,
        request_fingerprint: &str,
    ) -> Result<Option<CanvasOperationRecord>, CanvasOperationJournalError> {
        let operation_id = operation_id(&self.task.run_id, tool_call_id);
        let row = sqlx::query(
            "SELECT tool_name, request_fingerprint, receipt_json, updated_at
             FROM ai_agent_canvas_operation
             WHERE operation_id = ?1 AND task_id = ?2 AND run_id = ?3 AND thread_id = ?4 AND turn_id = ?5 AND project_id = ?6",
        )
        .bind(operation_id)
        .bind(&self.task.task_id)
        .bind(&self.task.run_id)
        .bind(&self.task.thread_id)
        .bind(&self.task.turn_id)
        .bind(self.task.project_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let record = record_from_row(&row)?;
        self.validate_identity(&record, tool_name, kind, request_fingerprint)?;
        Ok(Some(record))
    }

    /// A create batch keeps its identity when a model retries with a new call ID
    /// or the same logical Turn resumes in another Run. Rejected/failed requests
    /// remain retryable, but uncertain writes must recover their original ID.
    pub(crate) async fn find_equivalent_create(
        &self,
        request_fingerprint: &str,
    ) -> Result<Option<CanvasOperationRecord>, CanvasOperationJournalError> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT operation.operation_id FROM ai_agent_canvas_operation operation
             WHERE operation.project_id = ?1 AND operation.thread_id = ?2 AND operation.turn_id = ?3
             AND operation.kind = 'createComponents' AND operation.request_fingerprint = ?4
             AND operation.status IN ('prepared','applying','applied') AND operation.run_id IN (
                 WITH RECURSIVE lineage(run_id) AS (
                     SELECT ?5 UNION ALL SELECT resume.previous_run_id FROM ai_agent_run_resume resume JOIN lineage ON resume.run_id = lineage.run_id
                 ) SELECT run_id FROM lineage
             ) ORDER BY operation.prepared_at, operation.operation_id LIMIT 1",
        )
        .bind(self.task.project_id.to_string())
        .bind(&self.task.thread_id)
        .bind(&self.task.turn_id)
        .bind(request_fingerprint)
        .bind(&self.task.run_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        match id {
            Some(id) => self.require(&id).await.map(Some),
            None => Ok(None),
        }
    }

    pub(crate) async fn transition(&self, receipt: CanvasOperationReceipt) -> Result<CanvasOperationRecord, CanvasOperationJournalError> {
        let current = self.require(&receipt.operation_id).await?;
        if receipt.tool_call_id != current.receipt.tool_call_id
            || receipt.kind != current.receipt.kind
            || receipt.project_id != current.receipt.project_id
        {
            return Err(conflict("动作回执身份与已准备记录不一致"));
        }
        if current.receipt.status.is_terminal() {
            if current.receipt == receipt {
                return Ok(current);
            }
            return Err(conflict("已完成动作不能改写为不同回执"));
        }
        if !valid_transition(current.receipt.status, receipt.status) {
            return Err(conflict(format!("非法动作状态迁移：{} -> {}", current.receipt.status.as_str(), receipt.status.as_str())));
        }
        validate_component_fields(&current.receipt, &receipt)?;
        let now = crate::backend::ai_driven::time::now_ms();
        let receipt_json = serde_json::to_string(&receipt).map_err(json_error)?;
        let updated = sqlx::query(
            "UPDATE ai_agent_canvas_operation
             SET status = ?1, manager_id = ?2, receipt_json = ?3, updated_at = ?4, applied_at = ?5
             WHERE operation_id = ?6 AND status = ?7",
        )
        .bind(receipt.status.as_str())
        .bind(receipt.manager_id.as_deref())
        .bind(receipt_json)
        .bind(now)
        .bind(receipt.applied_at)
        .bind(&receipt.operation_id)
        .bind(current.receipt.status.as_str())
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        if updated.rows_affected() != 1 {
            return Err(conflict("动作状态被并发修改，请重新读取"));
        }
        let record = self.require(&receipt.operation_id).await?;
        if let Err(error) = self.publish_event(&record).await {
            tracing::error!(operation_id = %record.receipt.operation_id, error = %error, "failed to project canvas operation event");
        }
        Ok(record)
    }

    pub(crate) async fn require(&self, operation_id: &str) -> Result<CanvasOperationRecord, CanvasOperationJournalError> {
        let row = sqlx::query(
            "SELECT operation.tool_name, operation.request_fingerprint, operation.receipt_json, operation.updated_at
             FROM ai_agent_canvas_operation operation
             JOIN ai_agent_task_run run ON run.run_id = operation.run_id AND run.task_id = operation.task_id AND run.turn_id = operation.turn_id
             WHERE operation.operation_id = ?1 AND operation.project_id = ?2 AND operation.thread_id = ?3 AND operation.turn_id = ?4 AND operation.run_id IN (
                 WITH RECURSIVE lineage(run_id) AS (
                     SELECT ?5 UNION ALL
                     SELECT resume.previous_run_id FROM ai_agent_run_resume resume JOIN lineage ON resume.run_id = lineage.run_id
                 ) SELECT run_id FROM lineage
             )",
        )
        .bind(operation_id)
        .bind(self.task.project_id.to_string())
        .bind(&self.task.thread_id)
        .bind(&self.task.turn_id)
        .bind(&self.task.run_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .ok_or_else(|| CanvasOperationJournalError {
            code: "agent.operation.not_found", message: format!("未找到动作日志：{operation_id}")
        })?;
        record_from_row(&row)
    }

    pub(crate) async fn list(&self) -> Result<Vec<CanvasOperationReceipt>, CanvasOperationJournalError> {
        Self::list_for_scope(
            &self.pool,
            &self.task.task_id,
            &self.task.run_id,
            &self.task.thread_id,
            &self.task.turn_id,
            self.task.project_id,
        )
        .await
    }

    pub(crate) async fn list_for_scope(
        pool: &SqlitePool,
        _task_id: &str,
        run_id: &str,
        thread_id: &str,
        turn_id: &str,
        project_id: i64,
    ) -> Result<Vec<CanvasOperationReceipt>, CanvasOperationJournalError> {
        let rows = sqlx::query(
            "SELECT operation.receipt_json FROM ai_agent_canvas_operation operation
             JOIN ai_agent_task_run run ON run.run_id = operation.run_id AND run.task_id = operation.task_id AND run.turn_id = operation.turn_id
             WHERE operation.project_id = ?4 AND operation.thread_id = ?2 AND operation.turn_id = ?3 AND operation.run_id IN (
                 WITH RECURSIVE lineage(run_id) AS (
                     SELECT ?1 UNION ALL
                     SELECT resume.previous_run_id FROM ai_agent_run_resume resume JOIN lineage ON resume.run_id = lineage.run_id
                 ) SELECT run_id FROM lineage
             )
             ORDER BY operation.prepared_at ASC, operation.operation_id ASC",
        )
        .bind(run_id)
        .bind(thread_id)
        .bind(turn_id)
        .bind(project_id.to_string())
        .fetch_all(pool)
        .await
        .map_err(storage_error)?;
        rows.into_iter()
            .map(|row| {
                let receipt_json = row.try_get::<String, _>("receipt_json").map_err(storage_error)?;
                serde_json::from_str(&receipt_json).map_err(json_error)
            })
            .collect()
    }

    pub(crate) async fn has_applying(&self) -> Result<bool, CanvasOperationJournalError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(1) FROM ai_agent_canvas_operation operation
             JOIN ai_agent_task_run run ON run.run_id = operation.run_id AND run.task_id = operation.task_id AND run.turn_id = operation.turn_id
             WHERE operation.project_id = ?4 AND operation.thread_id = ?2 AND operation.turn_id = ?3 AND operation.status = 'applying' AND operation.run_id IN (
                 WITH RECURSIVE lineage(run_id) AS (
                     SELECT ?1 UNION ALL
                     SELECT resume.previous_run_id FROM ai_agent_run_resume resume JOIN lineage ON resume.run_id = lineage.run_id
                 ) SELECT run_id FROM lineage
             )",
        )
        .bind(&self.task.run_id)
        .bind(&self.task.thread_id)
        .bind(&self.task.turn_id)
        .bind(self.task.project_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(count > 0)
    }

    fn validate_identity(
        &self,
        record: &CanvasOperationRecord,
        tool_name: &str,
        kind: CanvasOperationKind,
        request_fingerprint: &str,
    ) -> Result<(), CanvasOperationJournalError> {
        if record.tool_name != tool_name || record.receipt.kind != kind || record.request_fingerprint != request_fingerprint {
            return Err(conflict("相同 toolCallId 被用于不同动作参数"));
        }
        Ok(())
    }

    async fn publish_event(&self, record: &CanvasOperationRecord) -> Result<(), CanvasOperationJournalError> {
        let status = record.receipt.status.as_str();
        tracing::debug!(
            target: "agent.canvas",
            event = "canvas_operation_state_changed",
            project_id = self.task.project_id,
            thread_id = %self.task.thread_id,
            task_id = %self.task.task_id,
            turn_id = %self.task.turn_id,
            run_id = %self.task.run_id,
            request_id = %self.task.request_id,
            tool_name = %record.tool_name,
            operation_id = %record.receipt.operation_id,
            tool_call_id = %record.receipt.tool_call_id,
            operation_kind = record.receipt.kind.as_str(),
            operation_status = status,
            updated_at = record.updated_at,
            receipt = %debug_log_json(&record.receipt),
            "Agent canvas operation state changed"
        );
        let event = self
            .repository
            .append_event(NewAgentTaskEvent {
                project_id: self.task.project_id,
                thread_id: self.task.thread_id.clone(),
                turn_id: self.task.turn_id.clone(),
                run_id: self.task.run_id.clone(),
                item_id: None,
                kind: format!("agent.canvas_operation.{status}"),
                payload: json!({ "operation": record.receipt }),
                dedupe_key: format!("operation:{}:{status}", record.receipt.operation_id),
                schema_version: 1,
                created_at: record.updated_at,
            })
            .await
            .map_err(|error| CanvasOperationJournalError { code: "agent.operation.event_persistence_failed", message: error.to_string() })?;
        if let Some(publisher) = &self.publisher {
            publisher.publish(event);
        }
        Ok(())
    }
}

fn record_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<CanvasOperationRecord, CanvasOperationJournalError> {
    let receipt_json = row.try_get::<String, _>("receipt_json").map_err(storage_error)?;
    Ok(CanvasOperationRecord {
        tool_name: row.try_get("tool_name").map_err(storage_error)?,
        request_fingerprint: row.try_get("request_fingerprint").map_err(storage_error)?,
        receipt: serde_json::from_str(&receipt_json).map_err(json_error)?,
        updated_at: row.try_get("updated_at").map_err(storage_error)?,
    })
}

fn valid_transition(from: CanvasOperationStatus, to: CanvasOperationStatus) -> bool {
    matches!(
        (from, to),
        (CanvasOperationStatus::Prepared, CanvasOperationStatus::Applying)
            | (CanvasOperationStatus::Prepared, CanvasOperationStatus::Rejected)
            | (CanvasOperationStatus::Prepared, CanvasOperationStatus::Failed)
            | (CanvasOperationStatus::Applying, CanvasOperationStatus::Applied)
            | (CanvasOperationStatus::Applying, CanvasOperationStatus::Rejected)
            | (CanvasOperationStatus::Applying, CanvasOperationStatus::Failed)
    )
}

fn validate_component_fields(current: &CanvasOperationReceipt, next: &CanvasOperationReceipt) -> Result<(), CanvasOperationJournalError> {
    if next.kind != CanvasOperationKind::PatchComponents && !next.component_fields.is_empty() {
        return Err(conflict("非 Patch 动作不能携带组件字段证据"));
    }
    if current.status != CanvasOperationStatus::Prepared && current.component_fields != next.component_fields {
        return Err(conflict("已发送动作的组件字段证据不能被改写"));
    }
    if current.status == CanvasOperationStatus::Prepared && !current.component_fields.is_empty() && current.component_fields != next.component_fields
    {
        return Err(conflict("已准备动作的组件字段证据不能被改写"));
    }
    let target_ids = current.component_ids.iter().map(|id| id.trim()).collect::<BTreeSet<_>>();
    for (component_id, fields) in &next.component_fields {
        if component_id.trim().is_empty() || !target_ids.contains(component_id.trim()) || fields.is_empty() {
            return Err(conflict("组件字段证据的目标不属于已准备动作或字段为空"));
        }
        let mut seen = BTreeSet::new();
        if fields
            .iter()
            .any(|field| !matches!(field.as_str(), "style" | "data" | "layer" | "filter") || !seen.insert(field.as_str()))
        {
            return Err(conflict("组件字段证据包含未知或重复字段"));
        }
    }
    if current.kind == CanvasOperationKind::PatchComponents
        && current.status == CanvasOperationStatus::Prepared
        && next.status == CanvasOperationStatus::Applying
        && next.component_fields.is_empty()
    {
        return Err(conflict("Patch 动作进入 Applying 前必须持久化字段证据"));
    }
    Ok(())
}

fn operation_id(run_id: &str, tool_call_id: &str) -> String {
    format!("{run_id}:{tool_call_id}")
}

fn storage_error(error: impl std::fmt::Display) -> CanvasOperationJournalError {
    CanvasOperationJournalError { code: "agent.operation.storage_failed", message: error.to_string() }
}

fn json_error(error: impl std::fmt::Display) -> CanvasOperationJournalError {
    CanvasOperationJournalError { code: "agent.operation.invalid_receipt", message: error.to_string() }
}

fn conflict(message: impl Into<String>) -> CanvasOperationJournalError {
    CanvasOperationJournalError { code: "agent.operation.conflict", message: message.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::agent_context::input::{AgentRunInput, AgentSelectionContext, AgentShortcutCommand, AgentShortcutContext};
    use crate::backend::ai_driven::task::completion::{evaluate_completion_facts, CompletionFactCheckStatus};
    use crate::backend::ai_driven::task::completion_requirements::CompletionRequirementsState;
    use crate::backend::ai_driven::task::{AgentExecutionLimitsSnapshot, NewAgentTaskRun, SqliteAgentTaskRepository};
    use crate::backend::ai_driven::thread::{CreateThreadInput, ThreadService};
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    const TASK_MIGRATION: &str = include_str!("../../../../migrations_desktop/1014_desktop_ai_thread_turn_item.sql");
    const OPERATION_MIGRATION: &str = include_str!("../../../../migrations_desktop/1015_desktop_ai_canvas_operation.sql");
    const WORKING_CHECKPOINT_MIGRATION: &str = include_str!("../../../../migrations_desktop/1017_desktop_ai_working_checkpoint.sql");
    const DELETE_MIGRATION: &str = include_str!("../../../../migrations_desktop/1018_desktop_ai_canvas_operation_delete.sql");

    async fn journal() -> CanvasOperationJournal {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql("CREATE TABLE project (id INTEGER PRIMARY KEY, name TEXT NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::raw_sql(TASK_MIGRATION).execute(&pool).await.unwrap();
        sqlx::raw_sql(OPERATION_MIGRATION).execute(&pool).await.unwrap();
        sqlx::raw_sql(DELETE_MIGRATION).execute(&pool).await.unwrap();
        sqlx::raw_sql(WORKING_CHECKPOINT_MIGRATION).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO project (id, name, deleted) VALUES (1, 'project', 0)")
            .execute(&pool)
            .await
            .unwrap();
        let thread = ThreadService::new(pool.clone())
            .create_thread(CreateThreadInput { project_id: 1, create_request_id: "create-operation-test".to_string(), title: None, created_at: 1 })
            .await
            .unwrap();
        let repository: Arc<dyn AgentTaskRepository> = Arc::new(SqliteAgentTaskRepository::new(pool.clone()));
        let task = repository
            .create(NewAgentTaskRun {
                project_id: 1,
                thread_id: thread.thread_id,
                request_id: "request-operation-test".to_string(),
                request: json!({"prompt": "test"}),
                input_summary: "test".to_string(),
                model_config_id: "config".to_string(),
                runtime_model_id: "model".to_string(),
                provider: None,
                entry_digest: "sha256:test".to_string(),
                client_session_id: "session".to_string(),
                task_type: "test".to_string(),
                limits: AgentExecutionLimitsSnapshot::default(),
                project_snapshot_ref: None,
                rig_version: None,
                rig_snapshot_ref: None,
                user_item_payload: json!({"text": "test"}),
                attachments: Vec::new(),
                created_at: 2,
            })
            .await
            .unwrap();
        CanvasOperationJournal { pool, repository, publisher: None, task }
    }

    #[tokio::test]
    async fn prepare_and_terminal_transition_are_idempotent() {
        let journal = journal().await;
        let first = journal
            .prepare(
                "patch_canvas_components",
                "call-1",
                CanvasOperationKind::PatchComponents,
                "sha256:request",
                vec!["c1".to_string()],
            )
            .await
            .unwrap();
        let repeated = journal
            .prepare(
                "patch_canvas_components",
                "call-1",
                CanvasOperationKind::PatchComponents,
                "sha256:request",
                vec!["c1".to_string()],
            )
            .await
            .unwrap();
        assert_eq!(first.receipt, repeated.receipt);

        let mut applying = first.receipt;
        applying.manager_id = Some("manager-1".to_string());
        applying.component_fields = std::collections::BTreeMap::from([(String::from("c1"), vec![String::from("style")])]);
        applying.status = CanvasOperationStatus::Applying;
        let applying = journal.transition(applying).await.unwrap();
        assert_eq!(applying.receipt.component_fields.get("c1"), Some(&vec![String::from("style")]));
        assert!(journal.has_applying().await.unwrap());
        let mut applied = applying.receipt;
        applied.status = CanvasOperationStatus::Applied;
        applied.applied_at = Some(10);
        applied.component_fields = std::collections::BTreeMap::from([(String::from("c1"), vec![String::from("data")])]);
        let tamper = journal.transition(applied.clone()).await.unwrap_err();
        assert_eq!(tamper.code, "agent.operation.conflict");
        applied.component_fields = std::collections::BTreeMap::from([(String::from("c1"), vec![String::from("style")])]);
        let applied = journal.transition(applied).await.unwrap();
        assert_eq!(applied.receipt.status, CanvasOperationStatus::Applied);
        assert!(!journal.has_applying().await.unwrap());
        assert_eq!(journal.list().await.unwrap(), vec![applied.receipt.clone()]);
        let recovered = journal
            .find_existing("patch_canvas_components", "call-1", CanvasOperationKind::PatchComponents, "sha256:request")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.receipt, applied.receipt);
        let manager_id = sqlx::query_scalar::<_, Option<String>>("SELECT manager_id FROM ai_agent_canvas_operation WHERE operation_id = ?1")
            .bind(&applied.receipt.operation_id)
            .fetch_one(&journal.pool)
            .await
            .unwrap();
        assert_eq!(manager_id.as_deref(), Some("manager-1"));

        let conflict = journal
            .prepare(
                "patch_canvas_components",
                "call-1",
                CanvasOperationKind::PatchComponents,
                "sha256:different",
                vec!["c1".to_string()],
            )
            .await
            .unwrap_err();
        assert_eq!(conflict.code, "agent.operation.conflict");
    }

    #[tokio::test]
    async fn legacy_applying_empty_fields_cannot_gain_evidence_after_dispatch() {
        let journal = journal().await;
        let prepared = journal
            .prepare(
                "patch_canvas_components",
                "call-legacy-fields",
                CanvasOperationKind::PatchComponents,
                "sha256:legacy-fields",
                vec!["c1".to_string()],
            )
            .await
            .unwrap();
        let mut legacy_applying = prepared.receipt.clone();
        legacy_applying.status = CanvasOperationStatus::Applying;
        let receipt_json = serde_json::to_string(&legacy_applying).unwrap();
        sqlx::query("UPDATE ai_agent_canvas_operation SET status = 'applying', receipt_json = ?1 WHERE operation_id = ?2")
            .bind(receipt_json)
            .bind(&legacy_applying.operation_id)
            .execute(&journal.pool)
            .await
            .unwrap();

        let mut injected = legacy_applying.clone();
        injected.status = CanvasOperationStatus::Applied;
        injected.applied_at = Some(5);
        injected.component_fields = std::collections::BTreeMap::from([(String::from("c1"), vec![String::from("data")])]);
        let error = journal.transition(injected).await.unwrap_err();
        assert_eq!(error.code, "agent.operation.conflict");

        let mut legacy_applied = legacy_applying;
        legacy_applied.status = CanvasOperationStatus::Applied;
        legacy_applied.applied_at = Some(5);
        let record = journal.transition(legacy_applied).await.unwrap();
        assert!(record.receipt.component_fields.is_empty());
    }

    #[tokio::test]
    async fn restores_operations_across_resume_attempt_task_ids() {
        let journal = journal().await;
        let prepared = journal
            .prepare(
                "patch_canvas_components",
                "call-resume",
                CanvasOperationKind::PatchComponents,
                "sha256:resume",
                vec!["c1".to_string()],
            )
            .await
            .unwrap();
        let mut applying = prepared.receipt;
        applying.manager_id = Some("manager-1".to_string());
        applying.component_fields = std::collections::BTreeMap::from([(String::from("c1"), vec![String::from("data")])]);
        applying.status = CanvasOperationStatus::Applying;
        journal.transition(applying).await.unwrap();

        let resumed_task_id = "resumed-task-id";
        let resumed_run_id = "resumed-run-id";
        sqlx::query(
            "INSERT INTO ai_agent_task_run (
                task_id, run_id, request_id, turn_id, attempt_no, client_session_id, status, task_type, limits_json,
                pending_json, usage_json, last_artifact_id, last_artifact_revision, error_code, error_message,
                rig_version, rig_snapshot_ref, project_snapshot_ref, next_event_sequence, created_at, started_at,
                updated_at, ended_at
             ) VALUES (?1, ?2, ?3, ?4, 2, ?5, 'running', 'test', '{}', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 1, 3, 3, 3, NULL)",
        )
        .bind(resumed_task_id)
        .bind(resumed_run_id)
        .bind("resumed-request")
        .bind(&journal.task.turn_id)
        .bind("resumed-session")
        .execute(&journal.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO ai_agent_run_resume (run_id, previous_run_id, turn_id, created_at) VALUES (?1, ?2, ?3, ?4)")
            .bind(resumed_run_id)
            .bind(&journal.task.run_id)
            .bind(&journal.task.turn_id)
            .bind(3)
            .execute(&journal.pool)
            .await
            .unwrap();

        let mut resumed = journal.clone();
        resumed.task.task_id = resumed_task_id.to_string();
        resumed.task.run_id = resumed_run_id.to_string();
        let restored = resumed.list().await.unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].operation_id, operation_id(&journal.task.run_id, "call-resume"));
        assert_eq!(resumed.require(&restored[0].operation_id).await.unwrap().receipt.status, CanvasOperationStatus::Applying);
        assert!(resumed.has_applying().await.unwrap());

        let mut applied = restored[0].clone();
        applied.status = CanvasOperationStatus::Applied;
        applied.applied_at = Some(4);
        let applied = resumed.transition(applied).await.unwrap().receipt;
        assert_eq!(applied.component_fields.get("c1"), Some(&vec![String::from("data")]));
        assert_eq!(resumed.list().await.unwrap(), vec![applied]);
        assert!(!resumed.has_applying().await.unwrap());

        let mut input = AgentRunInput::new("修改选中组件数据");
        input.shortcut = Some(AgentShortcutContext { command: AgentShortcutCommand::Data, raw_text: String::new() });
        input.selection = Some(AgentSelectionContext { target_ids: vec!["c1".to_string()], ..Default::default() });
        let requirements = CompletionRequirementsState::for_input(&input);
        let operations = resumed.list().await.unwrap();
        requirements.reconcile_applied_component_fields(&operations);
        let check = evaluate_completion_facts(&requirements.snapshot_facts(), &operations);
        assert_eq!(check.status, CompletionFactCheckStatus::Passed);
    }
    #[tokio::test]
    async fn three_create_batches_preserve_sixty_ids_and_retries_reuse_receipts() {
        let journal = journal().await;
        for batch in 0..3 {
            let fingerprint = format!("sha256:batch-{batch}");
            let original = journal
                .prepare(
                    "create_canvas_components",
                    &format!("call-{batch}"),
                    CanvasOperationKind::CreateComponents,
                    &fingerprint,
                    (0..20).map(|index| format!("component-{}", batch * 20 + index)).collect(),
                )
                .await
                .unwrap();
            let mut applying = original.receipt.clone();
            applying.status = CanvasOperationStatus::Applying;
            applying.manager_id = Some("manager".into());
            let mut applied = journal.transition(applying).await.unwrap().receipt;
            applied.status = CanvasOperationStatus::Applied;
            applied.applied_at = Some(5);
            journal.transition(applied).await.unwrap();
            let retry = journal
                .prepare(
                    "create_canvas_components",
                    &format!("retry-{batch}"),
                    CanvasOperationKind::CreateComponents,
                    &fingerprint,
                    original.receipt.component_ids.clone(),
                )
                .await
                .unwrap();
            assert_eq!(retry.receipt.operation_id, original.receipt.operation_id);
            assert_eq!(retry.receipt.status, CanvasOperationStatus::Applied);
        }
        let receipts = journal.list().await.unwrap();
        assert_eq!(receipts.len(), 3);
        assert_eq!(
            receipts
                .iter()
                .flat_map(|r| r.component_ids.iter())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            60
        );
    }

    #[tokio::test]
    async fn parallel_create_preparation_reuses_unknown_operation_id() {
        let journal = journal().await;
        let (left, right) = tokio::join!(
            journal.prepare("create_canvas_components", "left", CanvasOperationKind::CreateComponents, "same-batch", vec!["c1".into()]),
            journal.prepare("create_canvas_components", "right", CanvasOperationKind::CreateComponents, "same-batch", vec!["c1".into()])
        );
        assert_eq!(left.unwrap().receipt.operation_id, right.unwrap().receipt.operation_id);
        assert_eq!(journal.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn resumed_create_reuses_original_applying_batch() {
        let journal = journal().await;
        let original = journal
            .prepare(
                "create_canvas_components",
                "original",
                CanvasOperationKind::CreateComponents,
                "same-create",
                vec!["c1".into()],
            )
            .await
            .unwrap();
        let mut applying = original.receipt.clone();
        applying.status = CanvasOperationStatus::Applying;
        applying.manager_id = Some("manager".into());
        journal.transition(applying).await.unwrap();
        sqlx::query("INSERT INTO ai_agent_task_run (
            task_id,run_id,request_id,turn_id,attempt_no,client_session_id,status,task_type,limits_json,next_event_sequence,created_at,started_at,updated_at
            ) VALUES ('resume-create-task','resume-create-run','resumed-request',?1,2,'client','running','test','{}',1,3,3,3)")
            .bind(&journal.task.turn_id).execute(&journal.pool).await.unwrap();
        sqlx::query("INSERT INTO ai_agent_run_resume VALUES ('resume-create-run',?1,?2,3)")
            .bind(&journal.task.run_id)
            .bind(&journal.task.turn_id)
            .execute(&journal.pool)
            .await
            .unwrap();
        let mut resumed = journal.clone();
        resumed.task.task_id = "resume-create-task".into();
        resumed.task.run_id = "resume-create-run".into();
        let replay = resumed
            .prepare(
                "create_canvas_components",
                "new-call",
                CanvasOperationKind::CreateComponents,
                "same-create",
                vec!["c1".into()],
            )
            .await
            .unwrap();
        assert_eq!(replay.receipt.operation_id, original.receipt.operation_id);
        assert_eq!(replay.receipt.status, CanvasOperationStatus::Applying);
        let different = resumed
            .prepare(
                "create_canvas_components",
                "second-batch",
                CanvasOperationKind::CreateComponents,
                "different-create",
                vec!["c2".into()],
            )
            .await
            .unwrap();
        assert_ne!(different.receipt.operation_id, original.receipt.operation_id);
        assert_eq!(resumed.list().await.unwrap().len(), 2);
    }
}
