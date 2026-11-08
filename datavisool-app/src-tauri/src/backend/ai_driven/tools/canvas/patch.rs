/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use rig_core::tool::{Tool, ToolFailure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::backend::ai_driven::component_docs::component_documents_response;
use crate::backend::ai_driven::domain::artifact::ComponentConfigPatch;
use crate::backend::ai_driven::domain::operation::{CanvasOperationKind, CanvasOperationReceipt, CanvasOperationStatus};
use crate::backend::ai_driven::domain::validation::validate_component_patch_operation;
use crate::backend::ai_driven::security::{authorize_canvas_patch, ApprovalTicket, AuthorizationDecision, SecurityAuditEvent, SecurityRisk};
use crate::backend::ai_driven::tools::error::AgentToolError;
use crate::backend::ai_driven::tools::policy::AgentToolEffect;
use crate::backend::ai_driven::tools::runtime::{AgentToolRuntime, ExpectedComponentSnapshot};

use super::{
    existing_terminal, failed_receipt, frontend_failure, query_frontend_operation, request_fingerprint, require_manager, validation_error,
    FrontendOperationStatus,
};

pub(crate) const PATCH_CANVAS_COMPONENTS_TOOL: &str = "patch_canvas_components";
const APPLY_TIMEOUT: Duration = Duration::from_secs(60);
const PATCH_CANVAS_COMPONENTS_DESCRIPTION: &str = "原子修改真实画布上的现有组件。调用前用 inspect_current_page 只读取本次补丁涉及的字段：stylePatch 对应 style、dataPatch 对应 data、layerPatch 对应 layer、filterPatch 对应 filter；同时修改多种字段时读取这些字段的并集，例如仅修改 stylePatch 时 fields=[\"style\"]。patches 必须直接传 JSON 数组，禁止传序列化字符串；示例：{\"patches\":[{\"componentId\":\"c1\",\"componentType\":\"BaseText\",\"layerPatch\":{\"x\":100}}]}。修改 dataPatch 或数据字段映射前还必须显式调用 get_component_data_contract。成功回执就是运行事实。";

#[derive(Clone)]
pub(crate) struct PatchCanvasComponentsTool(pub(crate) AgentToolRuntime);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PatchCanvasComponentsArgs {
    patches: Vec<ComponentConfigPatch>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplyRequest<'a> {
    operation_id: &'a str,
    operation_fingerprint: &'a str,
    patches: &'a [ComponentConfigPatch],
    expected_snapshots: &'a [ExpectedComponentSnapshot],
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_ticket: Option<&'a ApprovalTicket>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmPatchRequest<'a> {
    patches: &'a [ComponentConfigPatch],
    risk: SecurityRisk,
}

#[derive(Debug, Deserialize)]
struct ConfirmPatchResponse {
    confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplyResponse {
    #[serde(default)]
    applied: Vec<AppliedPatch>,
    #[serde(default)]
    failed: Vec<ApplyFailure>,
    #[serde(default)]
    base_revision: Option<String>,
    #[serde(default)]
    result_revision: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppliedPatch {
    component_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplyFailure {
    component_id: String,
    code: String,
    message: String,
}

impl Tool for PatchCanvasComponentsTool {
    const NAME: &'static str = PATCH_CANVAS_COMPONENTS_TOOL;
    type Error = AgentToolError;
    type Args = PatchCanvasComponentsArgs;
    type Output = CanvasOperationReceipt;

    fn description(&self) -> String {
        PATCH_CANVAS_COMPONENTS_DESCRIPTION.to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "patches": {
                    "type": "array",
                    "description": "直接传 JSON 数组，例如 {\"patches\":[{\"componentId\":\"c1\",\"componentType\":\"BaseText\",\"layerPatch\":{\"x\":100}}]}；不要传序列化后的字符串。",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "componentId": { "type": "string" },
                            "componentType": { "type": "string" },
                            "stylePatch": {
                                "type": "object",
                                "properties": { "style": { "type": "object" } },
                                "required": ["style"],
                                "additionalProperties": false
                            },
                            "dataPatch": { "type": "object" },
                            "layerPatch": {
                                "type": "object",
                                "properties": {
                                    "x": {"type": "number", "minimum": 0},
                                    "y": {"type": "number", "minimum": 0},
                                    "width": {"type": "number", "exclusiveMinimum": 0},
                                    "height": {"type": "number", "exclusiveMinimum": 0},
                                    "name": {"type": "string", "minLength": 1, "maxLength": 200}
                                },
                                "additionalProperties": false
                            },
                            "filterPatch": {
                                "type": "object",
                                "properties": {
                                    "enable": {"type": "boolean"},
                                    "blur": {"type": "number", "minimum": 0, "maximum": 100},
                                    "brightness": {"type": "number", "minimum": 0, "maximum": 10},
                                    "contrast": {"type": "number", "minimum": 0, "maximum": 10},
                                    "opacity": {"type": "number", "minimum": 0, "maximum": 1},
                                    "saturate": {"type": "number", "minimum": 0, "maximum": 10},
                                    "hueRotate": {"type": "number", "minimum": -360, "maximum": 360}
                                },
                                "additionalProperties": false
                            },
                            "reason": { "type": "string" }
                        },
                        "required": ["componentId", "componentType"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["patches"],
            "additionalProperties": false
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0.context().authorize(Self::NAME, AgentToolEffect::Commit, false)?;
        self.0.context().security().grant().validate_scope(self.0.context().scope())?;
        let manager_id = require_manager(&self.0)?;
        let invocation = self.0.invocation(Self::NAME)?;
        let fingerprint = request_fingerprint(&args)?;
        let component_ids = args.patches.iter().map(|patch| patch.component_id.clone()).collect();
        let journal = self.0.operation_journal()?;
        let existing = journal
            .find_existing(Self::NAME, &invocation.tool_call_id, CanvasOperationKind::PatchComponents, &fingerprint)
            .await?;
        if let Some(record) = &existing {
            if let Some(receipt) = existing_terminal(record) {
                return Ok(receipt);
            }
            if record.receipt.status == CanvasOperationStatus::Applying {
                return self.recover_applying(record.receipt.clone()).await;
            }
        }

        self.refresh_validation_facts(&args.patches).await?;
        let issues = validate_component_patch_operation(&args.patches, self.0.operation_validation());
        if !issues.is_empty() {
            return Err(validation_error(issues));
        }
        let patch_targets = args.patches.iter().map(|patch| patch.component_id.clone()).collect::<Vec<_>>();
        let decision = match authorize_canvas_patch(self.0.context().security().grant(), &args.patches) {
            Ok(decision) => decision,
            Err(error) => {
                self.audit_security(
                    &invocation.tool_call_id,
                    &fingerprint,
                    &patch_targets,
                    SecurityRisk::R3,
                    "deny",
                    error.code(),
                    None,
                    "rejected",
                );
                return Err(error.into());
            }
        };
        let expected_snapshots = args
            .patches
            .iter()
            .map(|patch| {
                self.0.frontend()?.expected_component_patch_snapshot(
                    &patch.component_id,
                    &patch.component_type,
                    patch.style_patch.is_some(),
                    patch.data_patch.is_some(),
                    patch.layer_patch.is_some(),
                    patch.filter_patch.is_some(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut receipt = match existing {
            Some(record) => record.receipt,
            None => {
                journal
                    .prepare(Self::NAME, &invocation.tool_call_id, CanvasOperationKind::PatchComponents, &fingerprint, component_ids)
                    .await?
                    .receipt
            }
        };
        receipt.manager_id = Some(manager_id);
        let targets = receipt.component_ids.clone();
        let approval_ticket = match decision {
            AuthorizationDecision::Allow(risk) => {
                self.audit_security(&invocation.tool_call_id, &fingerprint, &targets, risk, "allow", "agent.security.allowed", None, "authorized");
                None
            }
            AuthorizationDecision::RequireApproval(risk) => {
                self.audit_security(
                    &invocation.tool_call_id,
                    &fingerprint,
                    &targets,
                    risk,
                    "requireApproval",
                    "agent.security.approval_required",
                    None,
                    "pendingApproval",
                );
                let confirmation: ConfirmPatchResponse = self
                    .0
                    .frontend()?
                    .call("confirmCanvasPatch", ConfirmPatchRequest { patches: &args.patches, risk }, None, self.0.context())
                    .await?;
                if !confirmation.confirmed {
                    receipt.status = CanvasOperationStatus::Rejected;
                    self.audit_security(
                        &invocation.tool_call_id,
                        &fingerprint,
                        &targets,
                        risk,
                        "deny",
                        "agent.security.user_rejected",
                        None,
                        "rejected",
                    );
                    return Ok(journal.transition(receipt).await?.receipt);
                }
                let ticket = self
                    .0
                    .context()
                    .security()
                    .approvals()
                    .issue(self.0.context().scope(), Self::NAME, &fingerprint, &targets);
                self.0
                    .context()
                    .security()
                    .approvals()
                    .consume(&ticket, self.0.context().scope(), Self::NAME, &fingerprint, &targets)?;
                self.audit_security(
                    &invocation.tool_call_id,
                    &fingerprint,
                    &targets,
                    risk,
                    "allow",
                    "agent.security.approval_consumed",
                    Some(&ticket.approval_id),
                    "authorized",
                );
                Some(ticket)
            }
        };
        self.0.context().ensure_active()?;
        // Persist host-validated fields before the irreversible call. A later
        // Applying -> Applied recovery has only the operation receipt and must
        // not trust a status-query payload for completion facts.
        receipt.component_fields = component_fields_for_patches(&args.patches);
        receipt.status = CanvasOperationStatus::Applying;
        receipt = journal.transition(receipt).await?.receipt;

        let response = self
            .0
            .frontend()?
            .call_irreversible::<_, ApplyResponse>(
                "applyComponentPatch",
                ApplyRequest {
                    operation_id: &receipt.operation_id,
                    operation_fingerprint: &fingerprint,
                    patches: &args.patches,
                    expected_snapshots: &expected_snapshots,
                    approval_ticket: approval_ticket.as_ref(),
                },
                Some(APPLY_TIMEOUT),
                self.0.context(),
            )
            .await;
        match response {
            Ok(response) => self.finish_response(receipt, response).await,
            Err(error) => {
                let status = query_frontend_operation(&self.0, &receipt.operation_id).await;
                if let Ok(status) = status {
                    if matches!(status.status.as_str(), "applied" | "failed") {
                        return self.finish_from_frontend(receipt, status).await;
                    }
                }
                tracing::warn!(operation_id = %receipt.operation_id, error = %error, "canvas patch outcome remains unresolved");
                Ok(receipt)
            }
        }
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

impl PatchCanvasComponentsTool {
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
            "applied" | "failed" => self.finish_from_frontend(receipt, status).await,
            "notFound" | "applying" => Ok(receipt),
            _ => Err(frontend_failure(&status)),
        }
    }

    async fn refresh_validation_facts(&self, patches: &[ComponentConfigPatch]) -> Result<(), AgentToolError> {
        let style_types = patches
            .iter()
            .filter(|patch| patch.style_patch.is_some())
            .map(|patch| patch.component_type.trim())
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>();
        if !style_types.is_empty() {
            let docs = component_documents_response(style_types);
            self.0
                .validation_evidence()
                .record_component_documents(&docs)
                .map_err(|error| AgentToolError::validation(error.code, error.message))?;
        }

        Ok(())
    }

    async fn finish_from_frontend(
        &self,
        receipt: CanvasOperationReceipt,
        status: FrontendOperationStatus,
    ) -> Result<CanvasOperationReceipt, AgentToolError> {
        if status.status == "failed" && status.result.is_none() {
            let error = frontend_failure(&status);
            let receipt = failed_receipt(receipt, &error);
            return Ok(self.0.operation_journal()?.transition(receipt).await?.receipt);
        }
        let response = serde_json::from_value::<ApplyResponse>(status.result.unwrap_or(Value::Null))
            .map_err(|error| AgentToolError::validation("agent.operation.frontend_receipt_invalid", error.to_string()))?;
        self.finish_response(receipt, response).await
    }

    async fn finish_response(&self, mut receipt: CanvasOperationReceipt, response: ApplyResponse) -> Result<CanvasOperationReceipt, AgentToolError> {
        if !response.failed.is_empty() {
            let error = patch_failure(&response.failed);
            receipt.base_revision = response.base_revision;
            receipt.result_revision = response.result_revision;
            let receipt = failed_receipt(receipt, &error);
            return Ok(self.0.operation_journal()?.transition(receipt).await?.receipt);
        }
        let mut expected = receipt.component_ids.clone();
        let mut applied = response.applied.into_iter().map(|item| item.component_id).collect::<Vec<_>>();
        expected.sort_unstable();
        applied.sort_unstable();
        if expected != applied {
            let error = AgentToolError::validation("agent.operation.partial_apply", "前端回执与请求组件不一致");
            let receipt = failed_receipt(receipt, &error);
            return Ok(self.0.operation_journal()?.transition(receipt).await?.receipt);
        }
        receipt.status = CanvasOperationStatus::Applied;
        receipt.base_revision = response.base_revision;
        receipt.result_revision = response.result_revision;
        receipt.applied_at = Some(crate::backend::ai_driven::time::now_ms());
        Ok(self.0.operation_journal()?.transition(receipt).await?.receipt)
    }
}

fn component_fields_for_patches(patches: &[ComponentConfigPatch]) -> BTreeMap<String, Vec<String>> {
    let mut fields = BTreeMap::<String, BTreeSet<String>>::new();
    for patch in patches {
        let component_fields = fields.entry(patch.component_id.trim().to_string()).or_default();
        for (present, field) in [
            (patch.style_patch.is_some(), "style"),
            (patch.data_patch.is_some(), "data"),
            (patch.layer_patch.is_some(), "layer"),
            (patch.filter_patch.is_some(), "filter"),
        ] {
            if present {
                component_fields.insert(field.to_string());
            }
        }
    }
    fields
        .into_iter()
        .map(|(component_id, fields)| (component_id, fields.into_iter().collect()))
        .collect()
}

fn patch_failure(failures: &[ApplyFailure]) -> AgentToolError {
    let summary = failures
        .iter()
        .take(5)
        .map(|failure| format!("{}:{}:{}", failure.component_id, failure.code, failure.message))
        .collect::<Vec<_>>()
        .join("; ");
    if failures.iter().any(|failure| failure.code == "component_revision_conflict") {
        AgentToolError::validation("agent.canvas.snapshot_conflict", format!("组件已发生变化，请重新读取后再修改：{summary}"))
    } else {
        AgentToolError::validation("agent.operation.patch_failed", summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_tool_description_explains_minimal_snapshot_fields() {
        assert!(PATCH_CANVAS_COMPONENTS_DESCRIPTION.contains("仅修改 stylePatch 时 fields=[\"style\"]"));
        for mapping in ["stylePatch 对应 style", "dataPatch 对应 data", "layerPatch 对应 layer", "filterPatch 对应 filter"] {
            assert!(PATCH_CANVAS_COMPONENTS_DESCRIPTION.contains(mapping));
        }
        assert!(PATCH_CANVAS_COMPONENTS_DESCRIPTION.contains("同时修改多种字段时读取这些字段的并集"));
    }

    #[test]
    fn patch_tool_requires_native_patch_array() {
        let patches = json!([{"componentId": "c1", "componentType": "BaseText", "layerPatch": {"x": 100}}]);
        assert!(serde_json::from_value::<PatchCanvasComponentsArgs>(json!({"patches": patches})).is_ok());
        assert!(serde_json::from_value::<PatchCanvasComponentsArgs>(json!({"patches": patches.to_string()})).is_err());
    }
}
