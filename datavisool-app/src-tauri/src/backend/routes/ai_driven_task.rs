/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI Agent v3 执行入口。Thread/Turn/Run 身份全部来自持久化层。

use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::app_state::AppState;
use crate::backend::ai_driven::agent_context::input::{
    AgentCanvasSnapshot, AgentClientContext, AgentInputAttachment, AgentRunInput, AgentSelectionContext, AgentShortcutContext,
};
use crate::backend::ai_driven::model::find_ai_driven_provider_model;
use crate::backend::ai_driven::provider_model_catalog::ProviderCatalog;
use crate::backend::ai_driven::settings::{AiDrivenRuntimeSettings, AiDrivenRuntimeSettingsStore};
use crate::backend::ai_driven::task::{
    ai_driven_task_registry, scoped_task_status, AgentExecutionLimitsSnapshot, AgentSteerInput, AgentTaskEventPublisher, AgentTaskRepository,
    AgentTaskRequest, AgentTaskService, AgentTaskStatus, AgentWorkingCheckpointState, AiDrivenTaskCancelError, AiDrivenTaskEventEnvelope,
    AiDrivenTaskIdentity, AiDrivenTaskRegistrationError, AiDrivenTaskRunSnapshot, AiDrivenTaskSteerError, CanvasOperationJournal, ListTaskEvents,
    NewAgentTaskRun, SaveAgentWorkingCheckpoint, ScopedTaskStatusError, SqliteAgentTaskRepository, TaskRepositoryError,
};
use crate::backend::ai_driven::thread::{
    thread_usage_snapshot, AcceptTurnInput, AttachmentClaim, CreateResumeAttemptInput, ThreadService, TurnInputStatus,
};
use crate::backend::types::{err, err_with_detail, ok, HttpResponse};
use crate::backend::FrontendEffectRegistry;
use crate::db::DbState;

use super::ai_driven_history_common::{parse_project_id, repository_error, required_trimmed};

const TASK_SCHEMA_VERSION: &str = "ai-driven-task/v3";
const RIG_VERSION: &str = "rig-core/0.40.0";
const TASK_TYPE: &str = "general";
const MAX_STEER_INPUT_CHARS: usize = 8_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitRequest {
    schema_version: String,
    request_id: String,
    session_id: String,
    project_id: String,
    thread_id: String,
    model: ModelSelection,
    requirement: String,
    shortcut: Option<AgentShortcutContext>,
    #[serde(default)]
    attachments: Vec<AgentInputAttachment>,
    canvas_snapshot: Option<AgentCanvasSnapshot>,
    selection: Option<AgentSelectionContext>,
    client_context: Option<AgentClientContext>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelSelection {
    model_config_id: String,
    runtime_model_id: String,
    provider: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecutionScopeRequest {
    project_id: String,
    thread_id: String,
    turn_id: String,
    run_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SteerRequest {
    project_id: String,
    thread_id: String,
    expected_turn_id: String,
    run_id: String,
    input_id: String,
    input: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsRequest {
    project_id: String,
    thread_id: String,
    turn_id: String,
    run_id: String,
    #[serde(default)]
    after_sequence: u64,
    #[serde(default = "default_event_page_size")]
    limit: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResumeRequest {
    budget_extension: Option<crate::backend::ai_driven::task::BudgetExtension>,
    project_id: String,
    thread_id: String,
    turn_id: String,
    previous_run_id: Option<String>,
    expected_checkpoint_version: Option<u64>,
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedTaskRequestSnapshot {
    model: ModelSelection,
    input: AgentRunInput,
}

struct PreparedSubmission {
    request: AgentTaskRequest,
    new_task: NewAgentTaskRun,
}

/// 先原子创建 Turn/User Item/Run/初始事件，再注册并启动后台执行器。
pub(crate) async fn submit(app_handle: &AppHandle, body: Value) -> Result<HttpResponse, String> {
    // 1. 解析前端提交的任务载荷；格式不合法时直接返回 400。
    let request: SubmitRequest = match serde_json::from_value(body) {
        Ok(value) => value,
        Err(error) => return Ok(err_with_detail(400, "任务请求格式错误", error.to_string())),
    };

    // 2. 取得数据库状态，并读取本次执行所需的运行时限制配置。
    let db = app_handle.state::<DbState>();
    let runtime_settings = match AiDrivenRuntimeSettingsStore::new(db.pool.clone()).load().await {
        Ok(settings) => settings,
        Err(error) => return Ok(err_with_detail(500, "读取 AI 运行设置失败", error)),
    };

    // 3. 校验、标准化输入并生成稳定的 Thread / Turn / TaskRun 创建参数。
    let prepared = match prepare_submission(request, now_ms(), runtime_settings) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };

    // 4. 在创建任务前确认所选模型具备处理当前图片附件的能力。
    if let Err(response) = validate_image_input_capability(&prepared.request) {
        return Ok(response);
    }

    // 5. 原子写入 Turn、用户消息、TaskRun 和初始事件，获得持久化任务实体。
    let repository = SqliteAgentTaskRepository::new(db.pool.clone());
    let task = match repository.create(prepared.new_task).await {
        Ok(task) => task,
        Err(error) => return Ok(task_repository_error(error)),
    };

    // 6. 为新任务保存初始工作检查点，供中断恢复和执行状态追踪使用。
    if let Err(error) = ensure_initial_working_checkpoint(&repository, &task, prepared.request.input.original_requirement_text()).await {
        return Ok(task_repository_error(error));
    }

    // 7. 读取当前任务快照及已记录的画布操作，作为接口的统一返回内容。
    let snapshot = match snapshot_with_operations(db.inner(), AiDrivenTaskRunSnapshot::from_persisted(&task)).await {
        Ok(snapshot) => snapshot,
        Err(response) => return Ok(response),
    };

    // 8. 幂等重试命中终态任务时只返回已有结果，不再次注册或启动执行器。
    if task.status.is_terminal() {
        return Ok(snapshot_response(snapshot));
    }

    // 9. 注册运行期控制通道并异步启动执行器；接口立即返回已创建的任务快照。
    Ok(launch_persisted_task(app_handle, prepared.request, task, snapshot))
}

/// 为已持久化的非终态 Run 建立进程内控制通道，并异步启动执行器。
///
/// 调用方已经完成 Turn、Item、Run 和初始检查点的写入；本方法不再创建
/// 持久化实体。注册必须先于 `spawn`，这样前端在收到 `queued` 快照后即可
/// 使用取消、追加输入和状态查询能力。函数本身不会等待 Agent 结束。
fn launch_persisted_task(
    app_handle: &AppHandle,
    request: AgentTaskRequest,
    task: crate::backend::ai_driven::task::AgentTaskRun,
    snapshot: AiDrivenTaskRunSnapshot,
) -> HttpResponse {
    // 运行期注册表以完整身份区分任务。entryDigest 将内存执行请求绑定到
    // 已持久化的入口事实，防止同一 requestId 被不同内容复用。
    let identity = AiDrivenTaskIdentity {
        request_id: task.request_id.clone(),
        project_id: task.project_id,
        thread_id: task.thread_id.clone(),
        turn_id: task.turn_id.clone(),
        task_id: task.task_id.clone(),
        run_id: task.run_id.clone(),
        entry_digest: request.entry_digest.clone(),
    };
    let tasks = ai_driven_task_registry();
    // 注册表不是任务队列：它只保存当前 Run 的取消通道、追加输入邮箱和
    // 完成通知通道，同时限制同一项目 / Thread 只能有一个活动执行器。
    let runtime = match tasks.register(identity.clone()) {
        // 相同身份说明幂等重试命中正在执行的 Run；直接返回已有快照，不能
        // 再次 spawn，否则两个执行器会同时写同一个 Turn。
        Ok(runtime) => runtime,
        Err(AiDrivenTaskRegistrationError::AlreadyRunning) => return snapshot_response(snapshot),
        // 其余冲突不启动后台任务。这里返回的不是执行结果，而是注册失败。
        Err(AiDrivenTaskRegistrationError::ProjectBusy) => {
            return err_with_detail(409, "当前项目已有活动任务", "agent.project.active_turn");
        }
        Err(AiDrivenTaskRegistrationError::ThreadBusy) => {
            return err_with_detail(409, "当前 Thread 已有活动任务", "agent.thread.active_turn");
        }
        Err(AiDrivenTaskRegistrationError::IdentityMismatch) => {
            return err_with_detail(409, "requestId 已绑定到另一任务入口", "agent.task.idempotency_conflict");
        }
    };

    // 复制 AppHandle 交给独立异步任务；HTTP 处理在 spawn 后立即结束。
    let background_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let frontend_effects = background_handle.state::<FrontendEffectRegistry>();
        let db = background_handle.state::<DbState>();
        let app_state = background_handle.state::<AppState>();
        let service = AgentTaskService::new(&background_handle, frontend_effects.inner(), db.inner(), app_state.inner());
        let steering = runtime.steering.clone();
        let outcome = service
            .execute_persisted(request, task, runtime.cancel_rx, steering.clone())
            .await;
        // 先关闭追加输入入口，再通知等待者最终快照，最后移除注册记录；
        // 这一顺序保证取消等待者能读取终态，且后续任务可以重新注册。
        steering.close();
        tasks.complete(outcome.snapshot);
        tasks.unregister(&identity);
    });

    // 返回的是创建 / 排队时快照，不是后台 Agent 的最终执行结果。
    snapshot_response(snapshot)
}

/// 仅恢复应用重启中断的 Turn；普通 submit 不猜测“继续”语义。
pub(crate) async fn resume(app_handle: &AppHandle, body: Value) -> Result<HttpResponse, String> {
    let request: ResumeRequest = match serde_json::from_value(body) {
        Ok(value) => value,
        Err(error) => return Ok(err_with_detail(400, "恢复请求格式错误", error.to_string())),
    };
    let project_id = match parse_project_id(&request.project_id) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let thread_id = match required_trimmed(&request.thread_id, "threadId") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let turn_id = match required_trimmed(&request.turn_id, "turnId") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let client_session_id = match required_trimmed(&request.session_id, "sessionId") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    if request.expected_checkpoint_version == Some(0) {
        return Ok(err_with_detail(400, "checkpointVersion 必须大于 0", "agent.resume.checkpoint_version_invalid"));
    }

    let db = app_handle.state::<DbState>();
    let threads = ThreadService::new(db.pool.clone());
    let turn = match threads.find_turn(project_id, &thread_id, &turn_id).await {
        Ok(Some(turn)) => turn,
        Ok(None) => return Ok(err(404, "Turn 不存在")),
        Err(error) => return Ok(repository_error(error)),
    };
    let latest_run = match threads.find_task_run_by_request_id(&turn.request_id).await {
        Ok(Some(run)) if run.project_id == project_id && run.thread_id == thread_id && run.turn_id == turn_id => run,
        Ok(Some(_)) => return Ok(err(403, "无权恢复此任务")),
        Ok(None) => return Ok(err(404, "TaskRun 不存在")),
        Err(error) => return Ok(repository_error(error)),
    };
    let previous_run_id = match request.previous_run_id {
        Some(value) => match required_trimmed(&value, "previousRunId") {
            Ok(value) if value == latest_run.run_id => value,
            Ok(value) => {
                let replay: Option<String> = sqlx::query_scalar("SELECT run_id FROM ai_agent_run_resume WHERE previous_run_id = ?1 AND turn_id = ?2")
                    .bind(&value)
                    .bind(&turn_id)
                    .fetch_optional(&db.pool)
                    .await
                    .map_err(|e| e.to_string())?;
                if replay.as_deref() != Some(&latest_run.run_id) {
                    return Ok(err_with_detail(409, "恢复目标已不是最新 Run", "agent.resume.stale_run"));
                }
                value
            }
            Err(response) => return Ok(response),
        },
        None => latest_run.run_id.clone(),
    };
    let persisted: PersistedTaskRequestSnapshot = match serde_json::from_value(turn.request.clone()) {
        Ok(value) => value,
        Err(error) => return Ok(err_with_detail(409, "原任务入口无法恢复", error.to_string())),
    };
    let provider = persisted
        .model
        .provider
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let supports_image_input = match resolve_catalog_image_support(provider.as_deref(), &persisted.model.runtime_model_id) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let saved_limits: AgentExecutionLimitsSnapshot = match serde_json::from_value(latest_run.limits.clone()) {
        Ok(value) => value,
        Err(error) => return Ok(err_with_detail(409, "原任务预算无法恢复", error.to_string())),
    };
    let runtime_settings = saved_limits.runtime_settings();
    let task_request = AgentTaskRequest {
        request_id: turn.request_id.clone(),
        project_id,
        thread_id: thread_id.clone(),
        client_session_id: client_session_id.clone(),
        model_config_id: persisted.model.model_config_id,
        runtime_model_id: persisted.model.runtime_model_id,
        provider_hint: provider,
        supports_image_input,
        input: persisted.input,
        entry_digest: turn.entry_digest,
        created_at: now_ms(),
        runtime_settings,
    };
    if let Err(response) = validate_image_input_capability(&task_request) {
        return Ok(response);
    }
    let repository = SqliteAgentTaskRepository::new(db.pool.clone());
    if AgentTaskStatus::from_str(&latest_run.status).is_some_and(|status| !status.is_terminal()) {
        let task = match repository.find_by_run_id(latest_run.run_id.clone()).await {
            Ok(Some(task)) => task,
            Ok(None) => return Ok(err(404, "TaskRun 不存在")),
            Err(error) => return Ok(task_repository_error(error)),
        };
        let snapshot = match snapshot_with_operations(db.inner(), AiDrivenTaskRunSnapshot::from_persisted(&task)).await {
            Ok(snapshot) => snapshot,
            Err(response) => return Ok(response),
        };
        return Ok(launch_persisted_task(app_handle, task_request, task, snapshot));
    }
    let checkpoint = match repository.find_working_checkpoint(previous_run_id.clone()).await {
        Ok(Some(checkpoint)) => checkpoint,
        Ok(None) => return Ok(err_with_detail(409, "任务没有可恢复的工作检查点", "agent.resume.checkpoint_missing")),
        Err(error) => return Ok(task_repository_error(error)),
    };
    let expected_checkpoint_version = request.expected_checkpoint_version.unwrap_or(checkpoint.checkpoint_version);
    let resumed = match repository
        .create_resume_attempt(CreateResumeAttemptInput {
            budget_extension: request.budget_extension,
            project_id,
            thread_id,
            turn_id,
            previous_run_id,
            expected_checkpoint_version,
            client_session_id,
            created_at: now_ms(),
        })
        .await
    {
        Ok(value) => value,
        Err(error) => return Ok(task_repository_error(error)),
    };
    if !resumed.replayed {
        let publisher = AgentTaskEventPublisher::new(app_handle.clone(), resumed.task.request_id.clone(), project_id);
        for event in resumed.events {
            publisher.publish(event);
        }
    }
    let snapshot = match snapshot_with_operations(db.inner(), AiDrivenTaskRunSnapshot::from_persisted(&resumed.task)).await {
        Ok(snapshot) => snapshot,
        Err(response) => return Ok(response),
    };
    Ok(launch_persisted_task(app_handle, task_request, resumed.task, snapshot))
}

/// 追加要求先写入 User Item + Inbox，再尽力唤醒进程内执行器。
pub(crate) async fn steer(app_handle: &AppHandle, db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let request: SteerRequest = match serde_json::from_value(body) {
        Ok(value) => value,
        Err(error) => return Ok(err_with_detail(400, "追加要求格式错误", error.to_string())),
    };
    let project_id = match parse_project_id(&request.project_id) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let thread_id = match required_trimmed(&request.thread_id, "threadId") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let turn_id = match required_trimmed(&request.expected_turn_id, "expectedTurnId") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let run_id = match required_trimmed(&request.run_id, "runId") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let input_id = match required_trimmed(&request.input_id, "inputId") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let input = match required_trimmed(&request.input, "input") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    if input.chars().count() > MAX_STEER_INPUT_CHARS {
        return Ok(err(400, format!("追加要求不能超过 {MAX_STEER_INPUT_CHARS} 个字符")));
    }

    let threads = ThreadService::new(db.pool.clone());
    let run = match threads.find_task_run_by_run_id(&run_id).await {
        Ok(Some(run)) if run.project_id == project_id && run.thread_id == thread_id && run.turn_id == turn_id => run,
        Ok(Some(_)) => return Ok(err(403, "无权向此 Turn 追加输入")),
        Ok(None) => return Ok(err(404, "TaskRun 不存在")),
        Err(error) => return Ok(repository_error(error)),
    };
    let content_digest = format!("sha256:{}", sha256_hex(input.as_bytes()));
    match threads.find_turn_input(project_id, &thread_id, &turn_id, &input_id).await {
        Ok(Some(existing)) if existing.content_digest == content_digest => {
            return Ok(ok(json!({ "threadId": thread_id, "turnId": turn_id, "acceptedInputId": input_id })));
        }
        Ok(Some(_)) => {
            return Ok(err_with_detail(409, "inputId 已绑定到不同内容", "agent.input.idempotency_conflict"));
        }
        Ok(None) => {}
        Err(error) => return Ok(repository_error(error)),
    }
    let reservation = match ai_driven_task_registry().reserve_steer(project_id, &thread_id, &turn_id, &run_id) {
        Ok(reservation) => reservation,
        Err(error) => return Ok(steer_reservation_error(error)),
    };
    let accepted_at = now_ms();
    let accepted = match threads
        .accept_turn_input(AcceptTurnInput {
            project_id,
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            run_id: run_id.clone(),
            input_id: input_id.clone(),
            content_digest,
            user_item_payload: json!({ "text": input, "source": "steer" }),
            schema_version: 1,
            accepted_at,
        })
        .await
    {
        Ok(value) => value,
        Err(error) => return Ok(repository_error(error)),
    };
    if !accepted.replayed {
        let publisher = AgentTaskEventPublisher::new(app_handle.clone(), run.request_id, project_id);
        for event in accepted.events {
            publisher.publish(event);
        }
    }
    if accepted.input.status == TurnInputStatus::Accepted && !accepted.replayed {
        let wake = AgentSteerInput { input_id: input_id.clone(), text: input, accepted_at };
        reservation.commit(wake);
    }
    Ok(ok(json!({ "threadId": thread_id, "turnId": turn_id, "acceptedInputId": input_id })))
}

fn steer_reservation_error(error: AiDrivenTaskSteerError) -> HttpResponse {
    match error {
        AiDrivenTaskSteerError::ScopeMismatch => err(403, "无权向此 Turn 追加输入"),
        AiDrivenTaskSteerError::TurnMismatch => err_with_detail(409, "活动 Turn 已变化", "agent.steer.turn_mismatch"),
        AiDrivenTaskSteerError::MailboxFull => err_with_detail(409, "追加要求过多，请稍后重试", "agent.steer.mailbox_full"),
        AiDrivenTaskSteerError::NotFound | AiDrivenTaskSteerError::NotAccepting => {
            err_with_detail(409, "任务已进入收尾阶段，不能再追加要求", "agent.steer.not_accepting")
        }
    }
}

pub(crate) async fn cancel(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let request: ExecutionScopeRequest = match serde_json::from_value(body) {
        Ok(value) => value,
        Err(error) => return Ok(err_with_detail(400, "取消请求格式错误", error.to_string())),
    };
    let (project_id, thread_id, turn_id, run_id) = match execution_scope(request) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let repository = SqliteAgentTaskRepository::new(db.pool.clone());
    let task = match scoped_task(&repository, project_id, &thread_id, &turn_id, &run_id).await {
        Ok(task) => task,
        Err(response) => return Ok(response),
    };
    let waiter = match ai_driven_task_registry().cancel(project_id, &thread_id, &turn_id, &run_id) {
        Ok(waiter) => waiter,
        Err(AiDrivenTaskCancelError::ScopeMismatch) => return Ok(err(403, "无权取消此任务")),
        Err(AiDrivenTaskCancelError::NotFound) if task.status.is_terminal() => {
            let snapshot = match snapshot_with_operations(db, AiDrivenTaskRunSnapshot::from_persisted(&task)).await {
                Ok(snapshot) => snapshot,
                Err(response) => return Ok(response),
            };
            return Ok(snapshot_response(snapshot));
        }
        Err(AiDrivenTaskCancelError::NotFound) => {
            return status_for_scope(db, &repository, project_id, &thread_id, &turn_id, &run_id).await;
        }
    };
    match waiter.wait().await {
        Some(snapshot) => match snapshot_with_operations(db, snapshot).await {
            Ok(snapshot) => Ok(snapshot_response(snapshot)),
            Err(response) => Ok(response),
        },
        None => status_for_scope(db, &repository, project_id, &thread_id, &turn_id, &run_id).await,
    }
}

pub(crate) async fn status(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let request: ExecutionScopeRequest = match serde_json::from_value(body) {
        Ok(value) => value,
        Err(error) => return Ok(err_with_detail(400, "状态请求格式错误", error.to_string())),
    };
    let (project_id, thread_id, turn_id, run_id) = match execution_scope(request) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let repository = SqliteAgentTaskRepository::new(db.pool.clone());
    status_for_scope(db, &repository, project_id, &thread_id, &turn_id, &run_id).await
}

pub(crate) async fn events(db: &DbState, body: Value) -> Result<HttpResponse, String> {
    let request: EventsRequest = match serde_json::from_value(body) {
        Ok(value) => value,
        Err(error) => return Ok(err_with_detail(400, "事件请求格式错误", error.to_string())),
    };
    let project_id = match parse_project_id(&request.project_id) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let thread_id = match required_trimmed(&request.thread_id, "threadId") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let turn_id = match required_trimmed(&request.turn_id, "turnId") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let run_id = match required_trimmed(&request.run_id, "runId") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let repository = SqliteAgentTaskRepository::new(db.pool.clone());
    let task = match scoped_task(&repository, project_id, &thread_id, &turn_id, &run_id).await {
        Ok(task) => task,
        Err(response) => return Ok(response),
    };
    let persisted = match repository
        .list_events(ListTaskEvents {
            project_id,
            thread_id,
            turn_id,
            run_id,
            after_sequence: request.after_sequence,
            limit: request.limit.clamp(1, 200),
        })
        .await
    {
        Ok(events) => events,
        Err(error) => return Ok(task_repository_error(error)),
    };
    let next_cursor = persisted.last().map_or(request.after_sequence, |event| event.sequence);
    let events = persisted
        .into_iter()
        .map(|event| AiDrivenTaskEventEnvelope::from_event(event, task.request_id.clone(), project_id.to_string()))
        .collect::<Vec<_>>();
    Ok(ok(json!({ "events": events, "nextCursor": next_cursor })))
}

/// 校验并标准化任务提交输入，同时生成创建 TaskRun 的持久化参数与后台执行参数。
///
/// 该方法不写入数据库；所有副作用由调用方在取得 `PreparedSubmission` 后统一执行。
fn prepare_submission(
    request: SubmitRequest,
    created_at: i64,
    runtime_settings: AiDrivenRuntimeSettings,
) -> Result<PreparedSubmission, HttpResponse> {
    // 1. 仅接受当前版本的提交协议，避免按错误结构创建持久化任务。
    if request.schema_version.trim() != TASK_SCHEMA_VERSION {
        return Err(err_with_detail(400, "仅支持 ai-driven-task/v3 协议", "agent.task.unsupported_schema"));
    }

    // 2. 提取并校验任务身份、会话、模型和用户需求等必填字段。
    let request_id = required_trimmed(&request.request_id, "requestId")?;
    let project_id = parse_project_id(&request.project_id)?;
    let thread_id = required_trimmed(&request.thread_id, "threadId")?;
    let client_session_id = required_trimmed(&request.session_id, "sessionId")?;
    let model_config_id = required_trimmed(&request.model.model_config_id, "model.modelConfigId")?;
    let runtime_model_id = required_trimmed(&request.model.runtime_model_id, "model.runtimeModelId")?;
    let requirement = required_trimmed(&request.requirement, "requirement")?;

    // 3. 规范化可选 Provider；空字符串按未指定处理，后续由保存的模型配置确定。
    let provider = request
        .model
        .provider
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    // 4. 从模型目录确认图片输入能力，并校验可选快捷指令上下文。
    let supports_image_input = resolve_catalog_image_support(provider.as_deref(), &runtime_model_id)?;
    let shortcut = request.shortcut.map(validate_shortcut).transpose()?;

    // 5. 将用户提交的画布、选区、附件和客户端事实整理为统一执行输入。
    let input = AgentRunInput {
        source_schema_version: TASK_SCHEMA_VERSION.to_string(),
        requirement: requirement.clone(),
        attachments: request.attachments,
        canvas_snapshot: request.canvas_snapshot,
        selection: request.selection,
        client_context: request.client_context,
        shortcut,
        ..AgentRunInput::default()
    };

    // 6. 计算不可变入口摘要，并冻结本次任务的执行限制。
    let entry_digest = immutable_entry_digest(project_id, &thread_id, &model_config_id, &runtime_model_id, provider.as_deref(), &input)?;
    let limits = limits_snapshot(&runtime_settings);

    // 7. 将附件转换为持久化声明，供后续创建任务时原子认领和校验。
    let attachments = input
        .attachments
        .iter()
        .map(|attachment| AttachmentClaim {
            attachment_id: attachment.attachment_id.trim().to_string(),
            expected_sha256: attachment.sha256.clone(),
            expected_byte_size: None,
        })
        .collect::<Vec<_>>();

    // 8. 构造写入数据库的请求快照，保留本次入口事实供审计和恢复使用。
    let request_snapshot = json!({
        "schemaVersion": TASK_SCHEMA_VERSION,
        "requestId": request_id,
        "projectId": project_id.to_string(),
        "threadId": thread_id,
        "model": {
            "modelConfigId": model_config_id,
            "runtimeModelId": runtime_model_id,
            "provider": provider,
        },
        "input": input,
    });

    // 9. 组装原子创建 Turn、用户消息、TaskRun 和初始事件所需的持久化参数。
    let new_task = NewAgentTaskRun {
        project_id,
        thread_id: thread_id.clone(),
        request_id: request_id.clone(),
        request: request_snapshot,
        input_summary: requirement,
        model_config_id: model_config_id.clone(),
        runtime_model_id: runtime_model_id.clone(),
        provider: provider.clone(),
        entry_digest: entry_digest.clone(),
        client_session_id: client_session_id.clone(),
        task_type: TASK_TYPE.to_string(),
        limits,
        project_snapshot_ref: None,
        rig_version: Some(RIG_VERSION.to_string()),
        rig_snapshot_ref: None,
        user_item_payload: json!({
            "text": input.requirement,
            "attachments": input.attachment_ids(),
            "source": "initial"
        }),
        attachments,
        created_at,
    };

    // 10. 组装后台执行器所需的运行时请求；它与持久化参数共享同一份入口事实。
    let task_request = AgentTaskRequest {
        request_id,
        project_id,
        thread_id,
        client_session_id,
        model_config_id,
        runtime_model_id,
        provider_hint: provider,
        supports_image_input,
        input,
        entry_digest,
        created_at,
        runtime_settings,
    };

    // 11. 同时返回持久化创建参数和运行时执行参数，交由 submit 的后续步骤使用。
    Ok(PreparedSubmission { request: task_request, new_task })
}

async fn scoped_task(
    repository: &SqliteAgentTaskRepository,
    project_id: i64,
    thread_id: &str,
    turn_id: &str,
    run_id: &str,
) -> Result<crate::backend::ai_driven::task::AgentTaskRun, HttpResponse> {
    let task = repository
        .find_by_run_id(run_id.to_string())
        .await
        .map_err(task_repository_error)?
        .ok_or_else(|| err(404, "TaskRun 不存在"))?;
    if task.project_id != project_id || task.thread_id != thread_id || task.turn_id != turn_id {
        return Err(err(403, "无权访问此任务"));
    }
    Ok(task)
}

async fn status_for_scope(
    db: &DbState,
    repository: &SqliteAgentTaskRepository,
    project_id: i64,
    thread_id: &str,
    turn_id: &str,
    run_id: &str,
) -> Result<HttpResponse, String> {
    match scoped_task_status(repository, ai_driven_task_registry(), project_id, thread_id, turn_id, run_id).await {
        Ok(snapshot) => match snapshot_with_operations(db, snapshot).await {
            Ok(snapshot) => Ok(snapshot_response(snapshot)),
            Err(response) => Ok(response),
        },
        Err(ScopedTaskStatusError::NotFound) => Ok(err(404, "AI 任务不存在")),
        Err(ScopedTaskStatusError::ScopeMismatch) => Ok(err(403, "无权查看此任务")),
        Err(ScopedTaskStatusError::Persistence(error)) => Ok(err_with_detail(500, "AI 任务状态读取失败", error)),
    }
}

async fn snapshot_with_operations(db: &DbState, snapshot: AiDrivenTaskRunSnapshot) -> Result<AiDrivenTaskRunSnapshot, HttpResponse> {
    let operations = CanvasOperationJournal::list_for_scope(
        &db.pool,
        &snapshot.task_id,
        &snapshot.run_id,
        &snapshot.thread_id,
        &snapshot.turn_id,
        snapshot.project_id,
    )
    .await
    .map_err(|error| err_with_detail(500, "AI 任务动作日志读取失败", error.to_string()))?;
    let repository = SqliteAgentTaskRepository::new(db.pool.clone());
    let checkpoint_version = repository
        .find_working_checkpoint(snapshot.run_id.clone())
        .await
        .map_err(task_repository_error)?
        .map(|checkpoint| checkpoint.checkpoint_version);
    let (thread_usage, latest_usage) = thread_usage_snapshot(&db.pool, snapshot.project_id, &snapshot.thread_id, &snapshot.turn_id, &snapshot.run_id)
        .await
        .map_err(|error| err_with_detail(500, "会话 Token 用量读取失败", error.to_string()))?;
    Ok(snapshot
        .with_operations(operations)
        .with_checkpoint_version(checkpoint_version)
        .with_optional_latest_usage_snapshot(latest_usage)
        .with_thread_usage(thread_usage))
}

async fn ensure_initial_working_checkpoint(
    repository: &SqliteAgentTaskRepository,
    task: &crate::backend::ai_driven::task::AgentTaskRun,
    objective: String,
) -> Result<(), TaskRepositoryError> {
    if repository.find_working_checkpoint(task.run_id.clone()).await?.is_some() {
        return Ok(());
    }
    repository
        .save_working_checkpoint(SaveAgentWorkingCheckpoint {
            project_id: task.project_id,
            thread_id: task.thread_id.clone(),
            task_id: task.task_id.clone(),
            run_id: task.run_id.clone(),
            turn_id: task.turn_id.clone(),
            expected_version: 0,
            state: AgentWorkingCheckpointState::initial(objective, task.created_at),
        })
        .await?;
    Ok(())
}

fn execution_scope(request: ExecutionScopeRequest) -> Result<(i64, String, String, String), HttpResponse> {
    Ok((
        parse_project_id(&request.project_id)?,
        required_trimmed(&request.thread_id, "threadId")?,
        required_trimmed(&request.turn_id, "turnId")?,
        required_trimmed(&request.run_id, "runId")?,
    ))
}

fn task_repository_error(error: TaskRepositoryError) -> HttpResponse {
    match error {
        TaskRepositoryError::Thread(error) => repository_error(error),
        TaskRepositoryError::NotFound { .. } => err(404, "记录不存在"),
        TaskRepositoryError::ScopeMismatch { .. } => err(403, "无权访问此记录"),
        TaskRepositoryError::InvalidRecord(message) => err_with_detail(409, message, "agent.task.invalid_state"),
        TaskRepositoryError::Sql(_) | TaskRepositoryError::Json(_) => err(500, "AI 任务持久化失败"),
    }
}

fn snapshot_response(snapshot: AiDrivenTaskRunSnapshot) -> HttpResponse {
    let mut value = serde_json::to_value(snapshot).unwrap_or(Value::Null);
    if let Value::Object(object) = &mut value {
        if let Some(project_id) = object.get("projectId").and_then(Value::as_i64) {
            object.insert("projectId".to_string(), Value::String(project_id.to_string()));
        }
    }
    ok(value)
}

fn resolve_catalog_image_support(provider: Option<&str>, runtime_model_id: &str) -> Result<Option<bool>, HttpResponse> {
    let Some(provider) = provider else {
        return Ok(None);
    };
    if let Some(model) =
        find_ai_driven_provider_model(provider, runtime_model_id).map_err(|error| err_with_detail(500, "模型能力配置加载失败", error))?
    {
        return Ok(Some(model.supports_image_input));
    }
    Ok(ProviderCatalog::parse_provider(provider).map(|kind| ProviderCatalog::infer_image_support(kind, runtime_model_id)))
}

fn validate_image_input_capability(request: &AgentTaskRequest) -> Result<(), HttpResponse> {
    if request.input.attachments.is_empty() || request.supports_image_input != Some(false) {
        Ok(())
    } else {
        Err(err(400, "当前模型不支持图片输入，请选择支持图片输入的模型后重试"))
    }
}

fn validate_shortcut(shortcut: AgentShortcutContext) -> Result<AgentShortcutContext, HttpResponse> {
    if shortcut.raw_text.trim().is_empty() {
        Err(err(400, "请求参数错误：shortcut.rawText 不能为空"))
    } else {
        Ok(shortcut)
    }
}

/// 将影响任务身份的入口事实规范化并计算 SHA-256 摘要，用于幂等校验和身份一致性验证。
fn immutable_entry_digest(
    project_id: i64,
    thread_id: &str,
    model_config_id: &str,
    runtime_model_id: &str,
    provider: Option<&str>,
    input: &AgentRunInput,
) -> Result<String, HttpResponse> {
    // 1. 按固定字段与固定结构序列化任务入口事实，确保同一输入得到相同字节序列。
    let canonical = serde_json::to_vec(&json!({
        "projectId": project_id,
        "threadId": thread_id,
        "modelConfigId": model_config_id,
        "runtimeModelId": runtime_model_id,
        "provider": provider,
        "input": input,
    }))
    .map_err(|error| err_with_detail(400, "任务入口无法序列化", error.to_string()))?;

    // 2. 对规范化字节计算 SHA-256，并加上算法前缀作为可存储、可比较的摘要值。
    Ok(format!("sha256:{}", sha256_hex(&canonical)))
}

/// 将当前运行设置冻结为任务级限制快照，确保任务运行期间不受后续设置修改影响。
///
/// 窄化整数和时长单位转换前均会截断到目标类型上限，避免转换溢出。
fn limits_snapshot(settings: &AiDrivenRuntimeSettings) -> AgentExecutionLimitsSnapshot {
    // 1. 将可持久化设置转换为执行器实际使用的限制类型，例如秒数转换为 Duration。
    let limits = settings.execution.to_limits();

    // 2. 复制并规范化所有限制；u32 字段和毫秒字段在转换前均限制在目标类型范围内。
    AgentExecutionLimitsSnapshot {
        token_budget_enabled: limits.token_budget_enabled,
        call_budget_enabled: limits.call_budget_enabled,
        duration_budget_enabled: limits.duration_budget_enabled,
        // 模型、工具和写入调用次数在快照中使用 u32 存储。
        max_model_calls: limits.max_model_calls.min(u64::from(u32::MAX)) as u32,
        max_tool_calls: limits.max_tool_calls.min(u64::from(u32::MAX)) as u32,
        max_write_calls: limits.max_write_calls.min(u64::from(u32::MAX)) as u32,

        // Token 总预算保持 u64，时长统一换算为毫秒后持久化。
        max_total_tokens: limits.max_total_tokens,
        max_duration_ms: limits.max_duration.as_millis().min(u128::from(u64::MAX)) as u64,
        max_tool_duration_ms: limits.max_tool_duration.as_millis().min(u128::from(u64::MAX)) as u64,

        // 其余计数限制同样安全窄化为 u32。
        max_consecutive_failures: limits.max_consecutive_failures.min(u64::from(u32::MAX)) as u32,
        max_frontend_interactions: limits.max_frontend_interactions.min(u64::from(u32::MAX)) as u32,
        max_same_read_repeats: limits.max_same_read_repeats.min(u64::from(u32::MAX)) as u32,
        max_completion_continuations: limits.max_completion_continuations.min(u64::from(u32::MAX)) as u32,

        // 克隆复合限制，保证快照独立于后续设置对象的变更。
        workspace_files: limits.workspace_files.clone(),
        context: settings.context.clone(),
        model_request: settings.model_request.clone(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn default_event_page_size() -> u32 {
    100
}

fn now_ms() -> i64 {
    crate::backend::ai_driven::time::now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v3_entry_digest_uses_real_thread_and_ignores_client_session_identity() {
        let input = AgentRunInput::new("生成统计图");
        let first = immutable_entry_digest(1, "thread-a", "7", "gpt", Some("openai"), &input).unwrap();
        let second = immutable_entry_digest(1, "thread-b", "7", "gpt", Some("openai"), &input).unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with("sha256:"));
    }
}
