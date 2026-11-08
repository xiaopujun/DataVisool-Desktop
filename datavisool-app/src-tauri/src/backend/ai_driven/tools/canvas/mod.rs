/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 直接作用于真实画布的 Agent 动作工具。

mod create;
mod delete;
mod patch;
mod selection;
mod status;

use serde::Serialize;
use serde_json::{json, Value};

use crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceTerminalValidationIssue;
use crate::backend::ai_driven::domain::operation::{CanvasOperationError, CanvasOperationKind, CanvasOperationReceipt, CanvasOperationStatus};
use crate::backend::ai_driven::task::CanvasOperationRecord;
use crate::backend::ai_driven::tools::error::AgentToolError;
use crate::backend::ai_driven::tools::runtime::AgentToolRuntime;
use crate::backend::utils::sha256_hex;

pub(crate) use create::{CreateCanvasComponentsTool, CREATE_CANVAS_COMPONENTS_TOOL};
pub(crate) use delete::{DeleteCanvasComponentsTool, DELETE_CANVAS_COMPONENTS_TOOL};
pub(crate) use patch::{PatchCanvasComponentsTool, PATCH_CANVAS_COMPONENTS_TOOL};
pub(crate) use selection::{GetCurrentCanvasSelectionTool, GET_CURRENT_CANVAS_SELECTION_TOOL};
pub(crate) use status::{GetCanvasOperationStatusTool, GET_CANVAS_OPERATION_STATUS_TOOL};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FrontendOperationStatus {
    pub(super) status: String,
    #[serde(default)]
    pub(super) result: Option<Value>,
    #[serde(default)]
    pub(super) error: Option<CanvasOperationError>,
}

pub(super) async fn query_frontend_operation(runtime: &AgentToolRuntime, operation_id: &str) -> Result<FrontendOperationStatus, AgentToolError> {
    runtime
        .frontend()?
        .call(
            "getCanvasOperationStatus",
            json!({ "operationId": operation_id }),
            Some(std::time::Duration::from_secs(10)),
            runtime.context(),
        )
        .await
}

fn request_fingerprint<T: Serialize>(value: &T) -> Result<String, AgentToolError> {
    let bytes = serde_json::to_vec(value).map_err(|error| AgentToolError::internal("agent.operation.request_serialize_failed", error.to_string()))?;
    Ok(format!("sha256:{}", sha256_hex(&bytes)))
}

fn require_manager(runtime: &AgentToolRuntime) -> Result<String, AgentToolError> {
    runtime
        .context()
        .scope()
        .manager_id
        .clone()
        .ok_or_else(|| AgentToolError::validation("agent.operation.manager_required", "当前任务缺少画布 Manager 作用域"))
}

fn validation_error(issues: Vec<WorkspaceTerminalValidationIssue>) -> AgentToolError {
    let message = serde_json::to_string(&json!({ "status": "failed", "code": "validation_failed", "retryable": true, "issues": issues }))
        .unwrap_or_else(|_| "动作参数校验失败".to_string());
    AgentToolError::validation("agent.operation.validation_failed", message)
}

fn failed_receipt(mut receipt: CanvasOperationReceipt, error: &AgentToolError) -> CanvasOperationReceipt {
    receipt.status = CanvasOperationStatus::Failed;
    receipt.error = Some(CanvasOperationError { code: error.code.clone(), message: error.message.clone(), retryable: error.retryable });
    receipt
}

fn frontend_failure(status: &FrontendOperationStatus) -> AgentToolError {
    status
        .error
        .as_ref()
        .map(|error| AgentToolError::validation(&error.code, &error.message))
        .unwrap_or_else(|| AgentToolError::validation("agent.operation.frontend_failed", "前端动作执行失败"))
}

fn existing_terminal(record: &CanvasOperationRecord) -> Option<CanvasOperationReceipt> {
    record.receipt.status.is_terminal().then(|| record.receipt.clone())
}

/// 将已持久化的终态回执同步到当前任务的动态资源授权。
/// 只有严格校验的创建回执才能扩权；删除回执只撤销当前任务登记。
pub(super) fn sync_security_resource(runtime: &AgentToolRuntime, receipt: &CanvasOperationReceipt) -> Result<(), AgentToolError> {
    match receipt.kind {
        CanvasOperationKind::CreateComponents if receipt.status == CanvasOperationStatus::Applied => runtime
            .context()
            .security()
            .register_created_resource(runtime.context().scope(), receipt)?,
        CanvasOperationKind::DeleteComponents if receipt.status == CanvasOperationStatus::Applied => runtime
            .context()
            .security()
            .revoke_deleted_resources(runtime.context().scope(), receipt)?,
        _ => {}
    }
    Ok(())
}
