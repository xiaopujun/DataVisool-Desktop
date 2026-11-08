/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Tauri 任务外壳到 Rig Agent 的统一编排服务。

use std::sync::Arc;

use rig_core::completion::Message;
use rig_core::message::{ImageMediaType, UserContent};
use rig_core::OneOrMany;
use serde_json::json;
use tokio::sync::watch;

use crate::app_state::AppState;
#[cfg(test)]
use crate::backend::ai_driven::agent::AgentExecutionLimits;
use crate::backend::ai_driven::agent::{
    run_built_agent, AgentBuildRequest, AgentContextPlan, AgentRunFailure, AgentRuntimeHook, AgentSteeringHook, AgentToolRoute, ExecutionBudget,
    ExecutionCancellationToken, ExecutionUsageSnapshot, RigAgentFactory,
};
use crate::backend::ai_driven::agent_context::input::AgentRunInput;
use crate::backend::ai_driven::ai_task_execution::workspace::{
    TurnSnapshotIdentity, TurnSnapshotOutcomeStatus, TurnSnapshotStore, WorkspaceFileService, WorkspaceFileTrace, WorkspaceSnapshotReason,
    WorkspaceSnapshotStore,
};
use crate::backend::ai_driven::domain::reference_image::prepare_reference_image;
use crate::backend::ai_driven::domain::validation::AgentArtifactValidationContext;
use crate::backend::ai_driven::domain::validation_evidence::AgentValidationEvidence;
use crate::backend::ai_driven::memory::{
    new_task_memory, task_conversation_key, ConversationMemoryBackend, ConversationWindowPolicy, SqliteConversationMemoryBackend,
};
use crate::backend::ai_driven::model_invocation::{
    DbAttachmentStore, DbModelConfigStore, ModelAttachmentResolver, ModelConfigResolver, ModelRuntimeCapabilities,
};
use crate::backend::ai_driven::provider::RigModelFactory;
use crate::backend::ai_driven::security::{AgentCapabilityGrant, AgentSecurity};
use crate::backend::ai_driven::settings::AiDrivenRuntimeSettings;
use crate::backend::ai_driven::tools::context::{AgentToolContext, AgentToolScope};
use crate::backend::ai_driven::tools::policy::{AgentToolPolicy, WriteApprovalPolicy};
use crate::backend::ai_driven::tools::runtime::{AgentFrontendRuntime, AgentToolInvocationTracker, AgentToolRuntime};
use crate::backend::utils::{debug_log_json, sanitize_debug_log_text};
use crate::backend::FrontendEffectRegistry;
use crate::db::DbState;

use super::consistency::{append_event_record, save_status};
use super::validation_preflight::preload_validation_facts;
use super::workspace::{open_project_workspace, turn_input_snapshot_name};
#[cfg(test)]
use super::AgentExecutionLimitsSnapshot;
use super::{
    AgentArtifactRef, AgentSteeringMailbox, AgentTaskError, AgentTaskEvent, AgentTaskEventPublisher, AgentTaskRepository, AgentTaskRun,
    AgentTaskStatus, AiDrivenTaskRunSnapshot, CanvasOperationJournal, CompletionRequirementsState, ObjectiveSpec, SaveAgentWorkingCheckpoint,
    SqliteAgentTaskRepository, TaskCompletionGate,
};

pub(super) const RIG_VERSION: &str = "rig-core/0.40.0";

/// 已由 HTTP 边界校验和标准化的任务入口事实。
pub(crate) struct AgentTaskRequest {
    pub request_id: String,
    pub project_id: i64,
    pub thread_id: String,
    pub client_session_id: String,
    pub model_config_id: String,
    pub runtime_model_id: String,
    pub provider_hint: Option<String>,
    pub supports_image_input: Option<bool>,
    pub input: AgentRunInput,
    pub entry_digest: String,
    pub created_at: i64,
    pub runtime_settings: AiDrivenRuntimeSettings,
}

/// submit、状态查询和取消共用的稳定任务快照。
pub(crate) struct AgentTaskServiceOutcome {
    pub snapshot: AiDrivenTaskRunSnapshot,
}

pub(crate) struct AgentTaskService<'a> {
    app_handle: &'a tauri::AppHandle,
    frontend_effects: &'a FrontendEffectRegistry,
    db: &'a DbState,
    app_state: &'a AppState,
}

impl<'a> AgentTaskService<'a> {
    pub(crate) fn new(
        app_handle: &'a tauri::AppHandle,
        frontend_effects: &'a FrontendEffectRegistry,
        db: &'a DbState,
        app_state: &'a AppState,
    ) -> Self {
        Self { app_handle, frontend_effects, db, app_state }
    }

    /// 路由已同步创建 Turn/TaskRun 并注册进程内通道；此处只领取和执行真实持久化 Run。
    pub(crate) async fn execute_persisted(
        &self,
        mut request: AgentTaskRequest,
        task: AgentTaskRun,
        cancel_rx: watch::Receiver<bool>,
        steering: AgentSteeringMailbox,
    ) -> AgentTaskServiceOutcome {
        let repository: Arc<dyn AgentTaskRepository> = Arc::new(SqliteAgentTaskRepository::new(self.db.pool.clone()));
        if let Err(error) = validate_execution_identity(&request, &task) {
            tracing::debug!(
                target: "agent.validation",
                event = "execution_identity_validation_failed",
                project_id = task.project_id,
                thread_id = %task.thread_id,
                task_id = %task.task_id,
                turn_id = %task.turn_id,
                run_id = %task.run_id,
                request_id = %task.request_id,
                code = %error.code.as_str(),
                message = %sanitize_debug_log_text(&error.message),
                "Agent execution identity validation failed"
            );
            return failure_outcome(&task, error, ExecutionUsageSnapshot::default());
        }
        let publisher = AgentTaskEventPublisher::new(self.app_handle.clone(), task.request_id.clone(), task.project_id);
        if task.status.is_terminal() {
            let operations = CanvasOperationJournal::new(self.db.pool.clone(), repository.clone(), publisher.clone(), task.clone())
                .list()
                .await
                .unwrap_or_default();
            return AgentTaskServiceOutcome { snapshot: AiDrivenTaskRunSnapshot::from_persisted(&task).with_operations(operations) };
        }
        request.runtime_settings = task.limits.runtime_settings();
        let mut limits = request.runtime_settings.execution.to_limits();
        limits.max_duration = std::time::Duration::from_millis(task.limits.max_duration_ms);
        limits.workspace_files = task.limits.workspace_files.clone();
        let claimed = match save_status(
            repository.as_ref(),
            &task,
            AgentTaskStatus::Running,
            None,
            task.usage.clone(),
            task.last_artifact.clone(),
            task.error.clone(),
            None,
            None,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                return failure_outcome(
                    &task,
                    AgentTaskError { code: "agent.task.running_checkpoint_failed".to_string(), message: bounded_message(error) },
                    ExecutionBudget::new(limits).usage_snapshot(),
                );
            }
        };
        publish_events(&publisher, claimed.events);
        let task = claimed.task;

        let cancellation = ExecutionCancellationToken::default();
        let budget = ExecutionBudget::with_cancellation(limits, cancellation.clone());
        if let Some(usage) = task.usage.clone().and_then(|value| serde_json::from_value(value).ok()) {
            budget.restore_usage(&usage);
        }
        let result = self
            .run_created_task(&request, &task, repository.clone(), cancel_rx.clone(), steering, publisher.clone(), budget.clone())
            .await;

        match result {
            Ok(success) => {
                let artifact = success.artifact.clone();
                let completed_summary_event = append_run_event(
                    repository.as_ref(),
                    &publisher,
                    &task,
                    "agent.turn.completed_summary",
                    json!({
                        "artifact": artifact,
                        "operations": success.operations,
                        "usage": success.output.usage,
                        "provider": success.provider,
                        "model": success.model,
                    }),
                )
                .await;
                let checkpoint = save_status(
                    repository.as_ref(),
                    &task,
                    AgentTaskStatus::Completed,
                    None,
                    serde_json::to_value(&success.output.usage).ok(),
                    artifact.clone(),
                    None,
                    (!success.output.final_text.trim().is_empty()).then(|| success.output.final_text.clone()),
                    Some(request.created_at.max(now_ms())),
                )
                .await;
                let completed = match checkpoint {
                    Ok(result) => result,
                    Err(error) => {
                        tracing::debug!(
                            target: "agent.run",
                            event = "run_checkpoint_failed",
                            project_id = task.project_id,
                            thread_id = %task.thread_id,
                            task_id = %task.task_id,
                            turn_id = %task.turn_id,
                            run_id = %task.run_id,
                            request_id = %task.request_id,
                            intended_status = AgentTaskStatus::Completed.as_str(),
                            error = %sanitize_debug_log_text(&error),
                            "Agent completed but terminal checkpoint failed"
                        );
                        let persistence_error = AgentTaskError {
                            code: "agent.task.completed_checkpoint_failed".to_string(),
                            message: bounded_message(format!("Agent 已结束，但 Turn 完成事务失败：{error}")),
                        };
                        let snapshot = AiDrivenTaskRunSnapshot::from_persisted(&task)
                            .with_operations(success.operations.clone())
                            .with_persistence_error(persistence_error);
                        return AgentTaskServiceOutcome { snapshot };
                    }
                };
                let completed_event_sequence = completed.events.iter().map(|event| event.sequence).max();
                publish_events(&publisher, completed.events);
                let events_recorded = completed_event_sequence
                    .or_else(|| completed_summary_event.as_ref().ok().map(|event| event.sequence))
                    .map(|sequence| sequence.saturating_add(1))
                    .unwrap_or(success.events_recorded);
                tracing::debug!(
                    target: "agent.run",
                    event = "run_finished",
                    project_id = task.project_id,
                    thread_id = %task.thread_id,
                    task_id = %task.task_id,
                    turn_id = %task.turn_id,
                    run_id = %task.run_id,
                    request_id = %task.request_id,
                    status = AgentTaskStatus::Completed.as_str(),
                    provider = %success.provider,
                    model = %success.model,
                    usage = %debug_log_json(&success.output.usage),
                    operations = %debug_log_json(&success.operations),
                    artifact = %debug_log_json(&artifact),
                    final_text = %sanitize_debug_log_text(&success.output.final_text),
                    "Agent run finished"
                );
                let snapshot =
                    AiDrivenTaskRunSnapshot::completed(&completed.task, success.provider, success.model, success.output, artifact, events_recorded);
                AgentTaskServiceOutcome { snapshot }
            }
            Err(failure) => {
                if failure.cancelled || *cancel_rx.borrow() {
                    cancellation.cancel();
                }
                let usage = budget.usage_snapshot();
                let status = if failure.cancelled || *cancel_rx.borrow() {
                    AgentTaskStatus::Cancelled
                } else if is_budget_exhausted(&failure.error.code) {
                    AgentTaskStatus::BudgetExhausted
                } else {
                    AgentTaskStatus::Failed
                };
                let error = AgentTaskError { code: failure.error.code, message: bounded_message(failure.error.message) };
                let kind = if status == AgentTaskStatus::Cancelled {
                    "agent.cancelled"
                } else if status == AgentTaskStatus::BudgetExhausted {
                    "agent.budget_exhausted"
                } else {
                    "agent.failed"
                };
                let terminal_event = append_run_event(
                    repository.as_ref(),
                    &publisher,
                    &task,
                    kind,
                    json!({
                        "intendedStatus": status.as_str(),
                        "artifact": failure.last_artifact,
                        "operations": &failure.operations,
                        "error": error,
                        "usage": usage,
                    }),
                )
                .await;
                let checkpoint_result = save_status(
                    repository.as_ref(),
                    &task,
                    status,
                    None,
                    serde_json::to_value(&usage).ok(),
                    failure.last_artifact.clone(),
                    Some(error.clone()),
                    None,
                    Some(now_ms()),
                )
                .await;
                let checkpoint = match checkpoint_result {
                    Ok(result) => {
                        publish_events(&publisher, result.events);
                        result.task
                    }
                    Err(checkpoint_error) => {
                        tracing::debug!(
                            target: "agent.run",
                            event = "run_checkpoint_failed",
                            project_id = task.project_id,
                            thread_id = %task.thread_id,
                            task_id = %task.task_id,
                            turn_id = %task.turn_id,
                            run_id = %task.run_id,
                            request_id = %task.request_id,
                            intended_status = status.as_str(),
                            error = %sanitize_debug_log_text(&checkpoint_error),
                            "Agent terminal checkpoint failed"
                        );
                        return AgentTaskServiceOutcome {
                            snapshot: AiDrivenTaskRunSnapshot::from_persisted(&task)
                                .with_operations(failure.operations.clone())
                                .with_persistence_error(AgentTaskError {
                                    code: "agent.task.terminal_checkpoint_failed".to_string(),
                                    message: bounded_message(checkpoint_error),
                                }),
                        };
                    }
                };
                let events_recorded = terminal_event
                    .as_ref()
                    .map(|event| event.sequence + 1)
                    .unwrap_or(failure.events_recorded);
                tracing::debug!(
                    target: "agent.run",
                    event = "run_finished",
                    project_id = task.project_id,
                    thread_id = %task.thread_id,
                    task_id = %task.task_id,
                    turn_id = %task.turn_id,
                    run_id = %task.run_id,
                    request_id = %task.request_id,
                    status = status.as_str(),
                    provider = ?failure.provider,
                    model = ?failure.model,
                    error_code = %error.code,
                    error_message = %sanitize_debug_log_text(&error.message),
                    usage = %debug_log_json(&usage),
                    operations = %debug_log_json(&failure.operations),
                    "Agent run finished"
                );
                let snapshot = AiDrivenTaskRunSnapshot::terminal_error(
                    &checkpoint,
                    status,
                    failure.provider,
                    failure.model,
                    usage,
                    error.clone(),
                    events_recorded,
                )
                .with_operations(failure.operations.clone());
                let snapshot = match terminal_event {
                    Ok(_) => snapshot,
                    Err(event_error) => snapshot.with_persistence_error(AgentTaskError {
                        code: "agent.task.event_persistence_failed".to_string(),
                        message: bounded_message(event_error),
                    }),
                };
                AgentTaskServiceOutcome { snapshot }
            }
        }
    }

    async fn run_created_task(
        &self,
        request: &AgentTaskRequest,
        task: &AgentTaskRun,
        repository: Arc<dyn AgentTaskRepository>,
        mut cancel_rx: watch::Receiver<bool>,
        steering: AgentSteeringMailbox,
        publisher: AgentTaskEventPublisher,
        budget: ExecutionBudget,
    ) -> Result<SuccessfulRun, FailedRun> {
        if *cancel_rx.borrow() {
            budget.cancellation_token().cancel();
        }
        append_run_event(repository.as_ref(), &publisher, task, "agent.started", json!({ "rigVersion": RIG_VERSION }))
            .await
            .map_err(|error| failed("agent.task.event_failed", error, None, None, false, 0))?;
        tracing::debug!(
            target: "agent.run",
            event = "run_started",
            project_id = task.project_id,
            thread_id = %task.thread_id,
            task_id = %task.task_id,
            turn_id = %task.turn_id,
            run_id = %task.run_id,
            request_id = %task.request_id,
            model_config_id = %request.model_config_id,
            runtime_model_id = %request.runtime_model_id,
            provider_hint = ?request.provider_hint,
            input = %debug_log_json(&request.input),
            "Agent run started"
        );
        let objective = ObjectiveSpec::build(&request.input);
        append_run_event(
            repository.as_ref(),
            &publisher,
            task,
            "agent.task.entry_scope",
            json!({
                "source": "structuredEntry",
                "completionAuthority": "model",
                "hostFactChecks": ["canvasOperationTerminal", "hostEntryConstraints", "verifiedOperationReceipts", "workspaceWriteAuthorized"],
                "objective": objective
            }),
        )
        .await
        .map_err(|error| failed("agent.task.entry_scope_persistence_failed", error, None, None, false, 1))?;

        budget.check().map_err(|error| {
            let cancelled = error.code.as_str() == "agent_execution.cancelled";
            failed(error.code.as_str(), error.to_string(), None, None, cancelled, 1)
        })?;
        let storage_dir = self.app_state.ai_driven_storage_dir();
        let workspace = open_project_workspace(&storage_dir, task.project_id)
            .map_err(|error| failed("agent.workspace.create_failed", error, None, None, false, 1))?;
        let request_snapshot = json!({
            "projectId": request.project_id,
            "threadId": task.thread_id,
            "turnId": task.turn_id,
            "clientSessionId": task.client_session_id,
            "input": request.input,
        });
        let start_revision = WorkspaceFileService::new(workspace.layout.clone(), budget.limits().workspace_files.clone())
            .workspace_revision()
            .map_err(|error| failed("agent.workspace.snapshot_revision_failed", error.to_string(), None, None, false, 1))?;
        WorkspaceSnapshotStore::new(&workspace, budget.limits().workspace_files.clone())
            .capture(&task.run_id, WorkspaceSnapshotReason::TurnStart, start_revision)
            .map_err(|error| failed("agent.workspace.snapshot_begin_failed", error.to_string(), None, None, false, 1))?;
        let turn_journal = TurnSnapshotStore::new(&workspace)
            .begin_turn_without_artifact(
                TurnSnapshotIdentity::new(task.project_id, &task.thread_id, &task.turn_id, &task.task_id, &task.run_id)
                    .map_err(|error| failed("agent.workspace.snapshot_identity_invalid", error.to_string(), None, None, false, 1))?,
                request_snapshot.clone(),
            )
            .map_err(|error| failed("agent.workspace.snapshot_begin_failed", error.to_string(), None, None, false, 1))?;
        let workspace_file_trace = WorkspaceFileTrace::default();
        let operation_journal = CanvasOperationJournal::new(self.db.pool.clone(), repository.clone(), publisher.clone(), task.clone());
        let completion_requirements = CompletionRequirementsState::for_input(&request.input);
        let execution: Result<SuccessfulRun, FailedRun> = async {
            workspace
                .write_input_json(&turn_input_snapshot_name(&task.turn_id), &request_snapshot)
                .map_err(|error| failed("agent.workspace.input_failed", error.to_string(), None, None, false, 1))?;

            let model_runtime = ModelConfigResolver::new()
                .resolve(
                    &DbModelConfigStore::new(self.db, self.app_state),
                    &request.model_config_id,
                    &request.runtime_model_id,
                    Some(&ModelRuntimeCapabilities { supports_image_input: request.supports_image_input }),
                )
                .await
                .map_err(|error| failed("agent.model.config_invalid", error.to_string(), None, None, false, 1))?;
            if let Some(provider_hint) = request.provider_hint.as_deref() {
                if !provider_hint.trim().eq_ignore_ascii_case(&model_runtime.provider) {
                    Err(failed(
                        "agent.model.provider_mismatch",
                        "请求模型厂商与已保存配置不一致",
                        Some(model_runtime.provider.clone()),
                        Some(model_runtime.model_name.clone()),
                        false,
                        1,
                    ))?;
                }
            }
            if !request.input.attachments.is_empty() && !model_runtime.supports_image_input {
                return Err(failed(
                    "agent.model.image_not_supported",
                    "当前模型不支持图片输入",
                    Some(model_runtime.provider),
                    Some(model_runtime.model_name),
                    false,
                    1,
                ));
            }

            let built_model = RigModelFactory::with_request_timeout(request.runtime_settings.model_request_timeout())
                .and_then(|factory| {
                    factory.build_from_runtime_parts(
                        &model_runtime.provider,
                        model_runtime.api_key.as_deref(),
                        &model_runtime.base_url,
                        &model_runtime.model_name,
                    )
                })
                .map_err(|error| {
                    failed(
                        "agent.model.factory_failed",
                        error.to_string(),
                        Some(model_runtime.provider.clone()),
                        Some(model_runtime.model_name.clone()),
                        false,
                        1,
                    )
                })?;
            let provider = built_model.provider().as_str().to_string();
            let model = built_model.model_name().to_string();
            let enable_workspace_file_tools = workspace_file_tools_enabled();
            let tool_route = AgentToolRoute::new(request.input.clone(), enable_workspace_file_tools);
            let mut cached_component_catalog = None;
            let mut run_recovery_state = None;
            let mut recovered_checkpoint = None;
            if let Some(mut checkpoint) = repository.find_working_checkpoint(task.run_id.clone()).await.map_err(|error| {
                failed("agent.task_checkpoint_load_failed", error.to_string(), Some(provider.clone()), Some(model.clone()), false, 1)
            })? {
                if checkpoint
                    .state
                    .workspace_revision
                    .is_some_and(|revision| revision != start_revision)
                {
                    return Err(failed(
                        "agent.resume.workspace_changed",
                        "工作区已在任务中断后发生变化，不能直接恢复",
                        Some(provider.clone()),
                        Some(model.clone()),
                        false,
                        1,
                    ));
                }
                if checkpoint.state.workspace_revision.is_none() {
                    checkpoint.state.workspace_revision = Some(start_revision);
                    checkpoint.state.updated_at = crate::backend::ai_driven::time::now_ms();
                    checkpoint = repository
                        .save_working_checkpoint(SaveAgentWorkingCheckpoint {
                            project_id: task.project_id,
                            thread_id: task.thread_id.clone(),
                            task_id: task.task_id.clone(),
                            run_id: task.run_id.clone(),
                            turn_id: task.turn_id.clone(),
                            expected_version: checkpoint.checkpoint_version,
                            state: checkpoint.state,
                        })
                        .await
                        .map_err(|error| {
                            failed("agent.task_checkpoint_save_failed", error.to_string(), Some(provider.clone()), Some(model.clone()), false, 1)
                        })?;
                }
                let state = &checkpoint.state;
                cached_component_catalog = state.component_catalog_result.clone();
                // 旧模型声明不参与当前权限或完成约束；仅恢复执行事实与累计预算。
                run_recovery_state = Some(state.run_recovery_state.clone());
                if !state.run_recovery_state.is_null() || !state.plan.is_empty() || !state.operation_ids.is_empty() {
                    recovered_checkpoint = Some(state.clone());
                }
            }

            let factory = RigAgentFactory;
            let allowed_tools = factory.allowed_tool_names(enable_workspace_file_tools);
            let scope = AgentToolScope::new(&task.task_id, &task.run_id, &task.thread_id, task.project_id.to_string())
                .map_err(|error| failed("agent.tool.invalid_scope", error.to_string(), Some(provider.clone()), Some(model.clone()), false, 1))?
                .with_turn_id(&task.turn_id)
                .with_manager_id(
                    request
                        .input
                        .canvas_snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.manager_id.as_deref()),
                );
            let policy = Arc::new(AgentToolPolicy::new(allowed_tools, enable_workspace_file_tools, true, WriteApprovalPolicy::SecurityDecision));
            let grant = AgentCapabilityGrant::for_task(&scope, objective.target_component_ids.clone(), workspace.manifest.workspace_id.clone())
                .with_workspace_write_enabled(enable_workspace_file_tools);
            let security = Arc::new(AgentSecurity::new(grant));
            let operations = operation_journal.list().await.map_err(|error| {
                failed("agent.operation.restore_failed", error.to_string(), Some(provider.clone()), Some(model.clone()), false, 1)
            })?;
            security.restore_created_resources(&scope, &operations);
            tool_route.bind_security(security.clone());
            for receipt in &operations {
                tool_route.observe_operation_result(receipt);
            }
            let mut prompt = self
                .build_prompt(request, task, &tool_route.current_plan(), &objective, start_revision)
                .await
                .map_err(|error| failed("agent.attachment.invalid", error, Some(provider.clone()), Some(model.clone()), false, 1))?;
            super::recovery_context::append_recovery_context(&mut prompt, recovered_checkpoint.as_ref(), &operations);
            let tool_context = AgentToolContext::new(scope, policy, security, cancel_rx.clone());
            let frontend = AgentFrontendRuntime::new(self.app_handle.clone(), self.frontend_effects.clone());
            let validation_evidence = AgentValidationEvidence::default();
            preload_validation_facts(&frontend, &tool_context, &validation_evidence, &request.input)
                .await
                .map_err(|error| failed(&error.code, error.message, Some(provider.clone()), Some(model.clone()), false, 1))?;
            if let Some(cached_catalog) = cached_component_catalog.as_deref() {
                let cached_catalog = serde_json::from_str(cached_catalog).map_err(|error| {
                    failed(
                        "agent.evidence.component_catalog_invalid",
                        format!("恢复的组件目录无效：{error}"),
                        Some(provider.clone()),
                        Some(model.clone()),
                        false,
                        1,
                    )
                })?;
                AgentValidationEvidence::validate_restored_component_catalog(&cached_catalog)
                    .map_err(|error| failed(error.code, error.message, Some(provider.clone()), Some(model.clone()), false, 1))?;
            }
            let mut validation_context = validation_context();
            validation_context.validation_evidence = validation_evidence.clone();
            let invocation_tracker = AgentToolInvocationTracker::default();
            let tool_runtime = AgentToolRuntime::new(
                self.app_handle.clone(),
                tool_context,
                workspace.clone(),
                validation_context,
                validation_evidence,
                Some(frontend),
                budget.limits().workspace_files.clone(),
                workspace_file_trace.clone(),
                operation_journal.clone(),
                invocation_tracker.clone(),
                completion_requirements.clone(),
            );
            let memory_backend: Arc<dyn ConversationMemoryBackend> = Arc::new(SqliteConversationMemoryBackend::new(self.db.pool.clone()));
            let memory = new_task_memory(
                request.project_id,
                memory_backend,
                ConversationWindowPolicy::default(),
                task.thread_id.clone(),
                task.turn_id.clone(),
            );
            let agent = factory.build(AgentBuildRequest {
                model: built_model,
                memory,
                tool_runtime: tool_runtime.clone(),
                tool_route: tool_route.clone(),
                enable_workspace_file_tools,
                max_model_calls: if budget.limits().call_budget_enabled {
                    usize::try_from(budget.limits().max_model_calls).unwrap_or(usize::MAX)
                } else {
                    usize::MAX
                },
                temperature: model_runtime.temperature.map(f64::from),
                max_tokens: model_runtime.max_tokens.map(u64::from),
                context_window_tokens: model_runtime.context_window_tokens,
                context_limits: request.runtime_settings.context.to_limits(),
            });
            let hook = AgentRuntimeHook::new(budget.clone(), repository.clone(), task.clone())
                .with_invalid_tool_retry_limit(1)
                .with_event_publisher(publisher.clone())
                .with_invocation_tracker(invocation_tracker)
                .with_tool_route(tool_route.clone())
                .with_cached_component_catalog(cached_component_catalog)
                .with_completion_requirements(completion_requirements.clone())
                .with_recovery_state(run_recovery_state.as_ref())
                .map_err(|error| failed("agent.resume.recovery_state_invalid", error, Some(provider.clone()), Some(model.clone()), false, 1))?;
            let steering_hook = AgentSteeringHook::new(steering, repository.clone(), publisher.clone(), task.clone());
            let completion_gate = Arc::new(TaskCompletionGate::new(
                operation_journal.clone(),
                completion_requirements,
                repository.clone(),
                publisher.clone(),
                task.clone(),
                hook.clone(),
            ));
            let max_duration = std::time::Duration::from_millis(budget.usage_snapshot().remaining_duration_ms.unwrap_or(86_400_000));
            let run = run_built_agent(agent, prompt, task_conversation_key(&task.turn_id), hook, steering_hook, budget.clone(), completion_gate);
            tokio::pin!(run);
            let deadline = tokio::time::sleep(max_duration);
            tokio::pin!(deadline);
            let run_result = tokio::select! {
                result = &mut run => result,
                changed = cancel_rx.changed() => {
                    if changed.is_ok() && *cancel_rx.borrow() {
                        budget.cancellation_token().cancel();
                        if operation_journal.has_applying().await.unwrap_or(false) {
                            // 真实写入已发送，必须等待动作工具得到明确回执。
                            (&mut run).await
                        } else {
                            Err(AgentRunFailure {
                                code: "agent_execution.cancelled".to_string(),
                                message: "Agent 任务已取消".to_string(),
                            })
                        }
                    } else {
                        (&mut run).await
                    }
                }
                _ = &mut deadline, if budget.limits().duration_budget_enabled => {
                    if operation_journal.has_applying().await.unwrap_or(false) {
                        let _ = (&mut run).await;
                    }
                    Err(AgentRunFailure {
                        code: "agent_execution.duration_limit".to_string(),
                        message: "Agent 任务执行时间超过上限".to_string(),
                    })
                }
            };
            let run_result = match run_result {
                Ok(output) => output,
                Err(error) => {
                    let cancelled = error.code == "agent_execution.cancelled" || *cancel_rx.borrow();
                    Err(failed(&error.code, error.message, Some(provider.clone()), Some(model.clone()), cancelled, 1))?
                }
            };

            Ok(SuccessfulRun { provider, model, output: run_result, artifact: None, operations: Vec::new(), events_recorded: 1 })
        }
        .await;
        let mut execution = execution;
        let operations = match operation_journal.list().await {
            Ok(operations) => operations,
            Err(error) => {
                let mut failure = failed("agent.operation.final_read_failed", error.to_string(), None, None, false, 1);
                // Retain any receipts already returned by the runner while reporting the storage failure.
                failure.operations = match &execution {
                    Ok(success) => success.output.operations.clone(),
                    Err(failure) => failure.operations.clone(),
                };
                execution = Err(failure);
                Vec::new()
            }
        };
        match &mut execution {
            Ok(success) => {
                success.output.operations = operations.clone();
                success.operations = operations;
            }
            Err(failure) if !operations.is_empty() => failure.operations = operations,
            Err(_) => {}
        }
        let outcome_status = match &execution {
            Ok(_) => TurnSnapshotOutcomeStatus::Completed,
            Err(failure) if failure.cancelled || *cancel_rx.borrow() => TurnSnapshotOutcomeStatus::Cancelled,
            Err(_) => TurnSnapshotOutcomeStatus::Failed,
        };
        if let Err(error) = turn_journal.finish_with_workspace_trace(outcome_status, None, workspace_file_trace.snapshot()) {
            tracing::error!(error = %error, turn_id = %task.turn_id, run_id = %task.run_id, "failed to persist Turn workspace outcome");
        }
        execution
    }

    async fn build_prompt(
        &self,
        request: &AgentTaskRequest,
        task: &AgentTaskRun,
        context_plan: &AgentContextPlan,
        objective: &ObjectiveSpec,
        workspace_revision: u64,
    ) -> Result<Message, String> {
        let task_context = serde_json::to_string(&context_plan.entry_context(&request.input, objective, Some(workspace_revision)))
            .map_err(|error| error.to_string())?;
        let mut content = vec![UserContent::text(format!(
            "[Agent task context]\n执行以下桌面设计任务。入口事实不可扩权；先按需读取上下文，需要改变真实画布时调用直接动作工具。\n{task_context}"
        ))];
        let attachment_store = DbAttachmentStore::new(self.db, self.app_state);
        let resolver = ModelAttachmentResolver::new();
        for attachment in &request.input.attachments {
            let resolved = resolver
                .resolve_image_attachment(
                    &attachment_store,
                    &request.project_id.to_string(),
                    &request.thread_id,
                    &task.turn_id,
                    &attachment.attachment_id,
                )
                .await
                .map_err(|error| error.to_string())?;
            let prepared = prepare_reference_image(resolved.attachment_id, &resolved.mime_type, &resolved.bytes, resolved.width, resolved.height)
                .map_err(|error| error.to_string())?;
            let instruction = format!(
                "输入图片 sourceAttachmentId={}，可信实际尺寸={}×{}。以下依次为栅格化原图、像素坐标卡尺图和对比度增强图。",
                prepared.attachment_id, prepared.width, prepared.height
            );
            content.push(UserContent::text(instruction));
            content.push(UserContent::image_raw(prepared.source_png, Some(ImageMediaType::PNG), None));
            content.push(UserContent::image_raw(prepared.coordinate_png, Some(ImageMediaType::PNG), None));
            content.push(UserContent::image_raw(prepared.contrast_png, Some(ImageMediaType::PNG), None));
        }
        let content = OneOrMany::many(content).map_err(|error| error.to_string())?;
        Ok(Message::User { content })
    }
}

async fn append_run_event(
    repository: &dyn AgentTaskRepository,
    publisher: &AgentTaskEventPublisher,
    task: &AgentTaskRun,
    kind: &str,
    payload: serde_json::Value,
) -> Result<AgentTaskEvent, String> {
    let event = append_event_record(repository, task, None, kind, payload, format!("run:{}:{kind}", task.run_id)).await?;
    publisher.publish(event.clone());
    Ok(event)
}

fn publish_events(publisher: &AgentTaskEventPublisher, events: Vec<crate::backend::ai_driven::thread::AgentTimelineEvent>) {
    for event in events {
        publisher.publish(event);
    }
}

struct SuccessfulRun {
    provider: String,
    model: String,
    output: crate::backend::ai_driven::agent::AgentRunOutput,
    artifact: Option<AgentArtifactRef>,
    operations: Vec<crate::backend::ai_driven::domain::operation::CanvasOperationReceipt>,
    events_recorded: u64,
}

struct FailedRun {
    error: AgentTaskError,
    provider: Option<String>,
    model: Option<String>,
    last_artifact: Option<AgentArtifactRef>,
    operations: Vec<crate::backend::ai_driven::domain::operation::CanvasOperationReceipt>,
    cancelled: bool,
    events_recorded: u64,
}

fn failed(
    code: impl Into<String>,
    message: impl Into<String>,
    provider: Option<String>,
    model: Option<String>,
    cancelled: bool,
    events_recorded: u64,
) -> FailedRun {
    FailedRun {
        error: AgentTaskError { code: code.into(), message: bounded_message(message.into()) },
        provider,
        model,
        last_artifact: None,
        operations: Vec::new(),
        cancelled,
        events_recorded,
    }
}

fn validate_execution_identity(request: &AgentTaskRequest, task: &AgentTaskRun) -> Result<(), AgentTaskError> {
    let scope_matches = request.request_id == task.request_id && request.project_id == task.project_id && request.thread_id == task.thread_id;
    let entry_matches = request.entry_digest == task.entry_digest;
    if scope_matches && entry_matches {
        Ok(())
    } else {
        Err(AgentTaskError {
            code: "agent.task.identity_mismatch".to_string(),
            message: "持久化 TaskRun 与执行请求的不可变身份不一致".to_string(),
        })
    }
}

fn failure_outcome(task: &AgentTaskRun, error: AgentTaskError, usage: ExecutionUsageSnapshot) -> AgentTaskServiceOutcome {
    AgentTaskServiceOutcome {
        snapshot: AiDrivenTaskRunSnapshot::terminal_error(task, AgentTaskStatus::Failed, None, None, usage, error, 0),
    }
}

#[cfg(test)]
fn limits_snapshot(limits: &AgentExecutionLimits) -> AgentExecutionLimitsSnapshot {
    AgentExecutionLimitsSnapshot {
        token_budget_enabled: limits.token_budget_enabled,
        call_budget_enabled: limits.call_budget_enabled,
        duration_budget_enabled: limits.duration_budget_enabled,
        max_model_calls: limits.max_model_calls.min(u64::from(u32::MAX)) as u32,
        max_tool_calls: limits.max_tool_calls.min(u64::from(u32::MAX)) as u32,
        max_write_calls: limits.max_write_calls.min(u64::from(u32::MAX)) as u32,
        max_total_tokens: limits.max_total_tokens,
        max_duration_ms: limits.max_duration.as_millis().min(u128::from(u64::MAX)) as u64,
        max_tool_duration_ms: limits.max_tool_duration.as_millis().min(u128::from(u64::MAX)) as u64,
        max_consecutive_failures: limits.max_consecutive_failures.min(u64::from(u32::MAX)) as u32,
        max_frontend_interactions: limits.max_frontend_interactions.min(u64::from(u32::MAX)) as u32,
        max_same_read_repeats: limits.max_same_read_repeats.min(u64::from(u32::MAX)) as u32,
        max_completion_continuations: limits.max_completion_continuations.min(u64::from(u32::MAX)) as u32,
        workspace_files: limits.workspace_files.clone(),
        context: Default::default(),
        model_request: Default::default(),
    }
}

fn validation_context() -> AgentArtifactValidationContext {
    AgentArtifactValidationContext { validation_evidence: AgentValidationEvidence::default() }
}

fn workspace_file_tools_enabled() -> bool {
    std::env::var("DATAVISOOL_AGENT_WORKSPACE_FILE_TOOLS")
        .map(|value| !matches!(value.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off"))
        .unwrap_or(true)
}

fn bounded_message(message: String) -> String {
    const LIMIT: usize = 2_000;
    let message = message.trim();
    if message.chars().count() <= LIMIT {
        message.to_string()
    } else {
        message.chars().take(LIMIT).collect::<String>() + "…"
    }
}

fn now_ms() -> i64 {
    crate::backend::ai_driven::time::now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_limits_are_persisted_without_unit_loss() {
        let limits = AgentExecutionLimits::default();
        let snapshot = limits_snapshot(&limits);
        assert_eq!(snapshot.max_model_calls, limits.max_model_calls as u32);
        assert_eq!(snapshot.max_duration_ms, limits.max_duration.as_millis() as u64);
        assert_eq!(snapshot.workspace_files, limits.workspace_files);
    }
}

fn is_budget_exhausted(code: &str) -> bool {
    matches!(
        code,
        "agent_execution.token_limit"
            | "agent_execution.model_call_limit"
            | "agent_execution.tool_call_limit"
            | "agent_execution.write_call_limit"
            | "agent_execution.frontend_interaction_limit"
            | "agent_execution.duration_limit"
    )
}
