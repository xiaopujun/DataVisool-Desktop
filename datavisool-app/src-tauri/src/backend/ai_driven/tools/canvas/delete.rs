/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::collections::BTreeSet;
use std::time::Duration;

use rig_core::tool::{Tool, ToolFailure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::backend::ai_driven::domain::operation::{CanvasOperationKind, CanvasOperationReceipt, CanvasOperationStatus};
use crate::backend::ai_driven::security::{authorize_canvas_delete, ApprovalTicket, AuthorizationDecision, SecurityAuditEvent, SecurityRisk};
use crate::backend::ai_driven::tools::error::AgentToolError;
use crate::backend::ai_driven::tools::policy::AgentToolEffect;
use crate::backend::ai_driven::tools::runtime::{AgentToolRuntime, ExpectedComponentSnapshot};

use super::{
    existing_terminal, failed_receipt, query_frontend_operation, request_fingerprint, require_manager, sync_security_resource,
    FrontendOperationStatus,
};

pub(crate) const DELETE_CANVAS_COMPONENTS_TOOL: &str = "delete_canvas_components";
const DELETE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub(crate) struct DeleteCanvasComponentsTool(pub(crate) AgentToolRuntime);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeleteCanvasComponentTarget {
    component_id: String,
    component_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeleteCanvasComponentsArgs {
    targets: Vec<DeleteCanvasComponentTarget>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmDeleteRequest<'a> {
    targets: &'a [DeleteCanvasComponentTarget],
    reason: &'a Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfirmDeleteResponse {
    confirmed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteRequest<'a> {
    operation_id: &'a str,
    operation_fingerprint: &'a str,
    targets: &'a [DeleteCanvasComponentTarget],
    expected_snapshots: &'a [ExpectedComponentSnapshot],
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_ticket: Option<&'a ApprovalTicket>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteResponse {
    #[serde(default)]
    deleted_component_ids: Vec<String>,
    #[serde(default)]
    failed: Vec<DeleteFailure>,
    #[serde(default)]
    base_revision: Option<String>,
    #[serde(default)]
    result_revision: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteFailure {
    component_id: String,
    code: String,
    message: String,
}

impl Tool for DeleteCanvasComponentsTool {
    const NAME: &'static str = DELETE_CANVAS_COMPONENTS_TOOL;
    type Error = AgentToolError;
    type Args = DeleteCanvasComponentsArgs;
    type Output = CanvasOperationReceipt;

    fn description(&self) -> String {
        "删除真实画布组件。调用前必须读取目标 layer 快照；工具会再次向用户确认，并以 Applied/Rejected 回执报告结果。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "targets": {
                    "type": "array", "minItems": 1, "maxItems": 100,
                    "items": {
                        "type": "object",
                        "properties": {
                            "componentId": {"type": "string", "minLength": 1},
                            "componentType": {"type": "string", "minLength": 1}
                        },
                        "required": ["componentId", "componentType"],
                        "additionalProperties": false
                    }
                },
                "reason": {"type": "string"}
            },
            "required": ["targets"],
            "additionalProperties": false
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0.context().authorize(Self::NAME, AgentToolEffect::Commit, false)?;
        self.0.context().security().grant().validate_scope(self.0.context().scope())?;
        validate_targets(&args.targets)?;
        let manager_id = require_manager(&self.0)?;
        let invocation = self.0.invocation(Self::NAME)?;
        let fingerprint = request_fingerprint(&args)?;
        let component_ids = args
            .targets
            .iter()
            .map(|target| target.component_id.clone())
            .collect::<Vec<_>>();
        let decision = match authorize_canvas_delete(self.0.context().security().grant(), &component_ids) {
            Ok(decision) => decision,
            Err(error) => {
                self.audit_security(
                    &invocation.tool_call_id,
                    &fingerprint,
                    &component_ids,
                    SecurityRisk::R3,
                    "deny",
                    error.code(),
                    None,
                    "rejected",
                );
                return Err(error.into());
            }
        };
        let risk = decision.risk();
        let journal = self.0.operation_journal()?;
        let existing = journal
            .find_existing(Self::NAME, &invocation.tool_call_id, CanvasOperationKind::DeleteComponents, &fingerprint)
            .await?;
        if let Some(record) = &existing {
            if let Some(receipt) = existing_terminal(record) {
                sync_security_resource(&self.0, &receipt)?;
                return Ok(receipt);
            }
            if record.receipt.status == CanvasOperationStatus::Applying {
                return self.recover_applying(record.receipt.clone()).await;
            }
        }

        let expected_snapshots = args
            .targets
            .iter()
            .map(|target| {
                self.0
                    .frontend()?
                    .expected_component_layer_snapshot(&target.component_id, &target.component_type)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut receipt = match existing {
            Some(record) => record.receipt,
            None => {
                journal
                    .prepare(
                        Self::NAME,
                        &invocation.tool_call_id,
                        CanvasOperationKind::DeleteComponents,
                        &fingerprint,
                        component_ids.clone(),
                    )
                    .await?
                    .receipt
            }
        };
        receipt.manager_id = Some(manager_id);
        self.audit_security(
            &invocation.tool_call_id,
            &fingerprint,
            &component_ids,
            risk,
            "requireApproval",
            "agent.security.approval_required",
            None,
            "pendingApproval",
        );

        let confirmation: Result<ConfirmDeleteResponse, AgentToolError> = self
            .0
            .frontend()?
            .call(
                "confirmCanvasDeletion",
                ConfirmDeleteRequest { targets: &args.targets, reason: &args.reason },
                None,
                self.0.context(),
            )
            .await;
        let confirmation = match confirmation {
            Ok(confirmation) => confirmation,
            Err(error) => {
                let receipt = failed_receipt(receipt, &error);
                return Ok(journal.transition(receipt).await?.receipt);
            }
        };
        if !confirmation.confirmed {
            receipt.status = CanvasOperationStatus::Rejected;
            self.audit_security(
                &invocation.tool_call_id,
                &fingerprint,
                &component_ids,
                risk,
                "deny",
                "agent.security.user_rejected",
                None,
                "rejected",
            );
            return Ok(journal.transition(receipt).await?.receipt);
        }

        let approval_ticket = match decision {
            AuthorizationDecision::RequireApproval(_) => {
                let ticket = self
                    .0
                    .context()
                    .security()
                    .approvals()
                    .issue(self.0.context().scope(), Self::NAME, &fingerprint, &component_ids);
                self.0
                    .context()
                    .security()
                    .approvals()
                    .consume(&ticket, self.0.context().scope(), Self::NAME, &fingerprint, &component_ids)?;
                self.audit_security(
                    &invocation.tool_call_id,
                    &fingerprint,
                    &component_ids,
                    risk,
                    "allow",
                    "agent.security.approval_consumed",
                    Some(&ticket.approval_id),
                    "authorized",
                );
                Some(ticket)
            }
            AuthorizationDecision::Allow(_) => None,
        };

        self.0.context().ensure_active()?;
        receipt.status = CanvasOperationStatus::Applying;
        receipt = journal.transition(receipt).await?.receipt;
        let response = self
            .0
            .frontend()?
            .call_irreversible::<_, DeleteResponse>(
                "deleteCanvasComponents",
                DeleteRequest {
                    operation_id: &receipt.operation_id,
                    operation_fingerprint: &fingerprint,
                    targets: &args.targets,
                    expected_snapshots: &expected_snapshots,
                    approval_ticket: approval_ticket.as_ref(),
                },
                Some(DELETE_TIMEOUT),
                self.0.context(),
            )
            .await;
        match response {
            Ok(response) => self.finish_response(receipt, response).await,
            Err(error) => match self.recover_applying(receipt.clone()).await {
                Ok(recovered) => Ok(recovered),
                Err(_) => {
                    tracing::warn!(operation_id = %receipt.operation_id, error = %error, "canvas delete outcome remains unresolved");
                    Ok(receipt)
                }
            },
        }
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

impl DeleteCanvasComponentsTool {
    fn audit_security(
        &self,
        tool_call_id: &str,
        argument_hash: &str,
        targets: &[String],
        risk: SecurityRisk,
        decision: &str,
        decision_code: &str,
        approval_id: Option<&str>,
        result_status: &str,
    ) {
        let scope = self.0.context().scope();
        self.0.context().security().audit().record(SecurityAuditEvent {
            task_id: &scope.task_id,
            run_id: &scope.run_id,
            tool_call_id,
            tool_name: Self::NAME,
            argument_hash,
            resource_scope: targets,
            risk,
            decision,
            decision_code,
            approval_id,
            network_destination: None,
            result_status,
        });
    }

    pub(super) async fn recover_applying(&self, receipt: CanvasOperationReceipt) -> Result<CanvasOperationReceipt, AgentToolError> {
        let status = match query_frontend_operation(&self.0, &receipt.operation_id).await {
            Ok(status) => status,
            Err(_) => return Ok(receipt),
        };
        match status.status.as_str() {
            "applied" | "failed" if status.result.is_some() => {
                let response = serde_json::from_value::<DeleteResponse>(status.result.unwrap_or(Value::Null))
                    .map_err(|error| AgentToolError::validation("agent.operation.frontend_receipt_invalid", error.to_string()))?;
                self.finish_response(receipt, response).await
            }
            "failed" => {
                let error = frontend_status_error(&status);
                let receipt = failed_receipt(receipt, &error);
                Ok(self.0.operation_journal()?.transition(receipt).await?.receipt)
            }
            "notFound" | "applying" => Ok(receipt),
            _ => Err(AgentToolError::validation("agent.operation.frontend_status_invalid", "前端返回了未知动作状态")),
        }
    }

    async fn finish_response(&self, mut receipt: CanvasOperationReceipt, response: DeleteResponse) -> Result<CanvasOperationReceipt, AgentToolError> {
        receipt.base_revision = response.base_revision;
        receipt.result_revision = response.result_revision;
        if !response.failed.is_empty() {
            let summary = response
                .failed
                .iter()
                .take(5)
                .map(|failure| format!("{}:{}:{}", failure.component_id, failure.code, failure.message))
                .collect::<Vec<_>>()
                .join("; ");
            let receipt = failed_receipt(receipt, &AgentToolError::validation("agent.operation.delete_failed", summary));
            return Ok(self.0.operation_journal()?.transition(receipt).await?.receipt);
        }
        let mut expected = receipt.component_ids.clone();
        let mut deleted = response.deleted_component_ids;
        expected.sort_unstable();
        deleted.sort_unstable();
        if expected != deleted {
            let receipt = failed_receipt(receipt, &AgentToolError::validation("agent.operation.partial_delete", "前端删除回执与请求组件不一致"));
            return Ok(self.0.operation_journal()?.transition(receipt).await?.receipt);
        }
        receipt.status = CanvasOperationStatus::Applied;
        receipt.applied_at = Some(crate::backend::ai_driven::time::now_ms());
        let record = self.0.operation_journal()?.transition(receipt).await?;
        sync_security_resource(&self.0, &record.receipt)?;
        Ok(record.receipt)
    }
}

fn validate_targets(targets: &[DeleteCanvasComponentTarget]) -> Result<(), AgentToolError> {
    if targets.is_empty() || targets.len() > 100 {
        return Err(AgentToolError::invalid("agent.operation.delete_target_count_invalid", "targets 数量必须在 1..=100"));
    }
    let mut seen = BTreeSet::new();
    for target in targets {
        if target.component_id.trim().is_empty() || target.component_type.trim().is_empty() {
            return Err(AgentToolError::invalid("agent.operation.invalid_component_target", "componentId 和 componentType 不能为空"));
        }
        if !seen.insert(target.component_id.trim()) {
            return Err(AgentToolError::invalid("agent.operation.duplicate_component_target", "同一组件不能重复删除"));
        }
    }
    Ok(())
}

fn frontend_status_error(status: &FrontendOperationStatus) -> AgentToolError {
    status
        .error
        .as_ref()
        .map(|error| AgentToolError::validation(&error.code, &error.message))
        .unwrap_or_else(|| AgentToolError::validation("agent.operation.frontend_failed", "前端动作执行失败"))
}
