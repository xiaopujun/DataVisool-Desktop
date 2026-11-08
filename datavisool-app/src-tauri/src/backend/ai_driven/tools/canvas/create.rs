/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use rig_core::tool::{Tool, ToolFailure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::backend::ai_driven::component_docs::component_data_requirements;
use crate::backend::ai_driven::component_docs::component_documents_response;
use crate::backend::ai_driven::domain::artifact::PageDraftComponent;
use crate::backend::ai_driven::domain::operation::{CanvasOperationKind, CanvasOperationReceipt, CanvasOperationStatus};
use crate::backend::ai_driven::domain::validation::validate_page_operation;
use crate::backend::ai_driven::domain::validation_data_contract::is_component_data_field_mapping_key;
use crate::backend::ai_driven::security::{
    authorize_canvas_create, reserved_component_id, ApprovalTicket, AuthorizationDecision, SecurityAuditEvent, SecurityRisk, MAX_TARGETS_PER_ACTION,
};
use crate::backend::ai_driven::tools::error::AgentToolError;
use crate::backend::ai_driven::tools::policy::AgentToolEffect;
use crate::backend::ai_driven::tools::runtime::AgentToolRuntime;

use super::{
    existing_terminal, failed_receipt, query_frontend_operation, request_fingerprint, require_manager, sync_security_resource, validation_error,
};

pub(crate) const CREATE_CANVAS_COMPONENTS_TOOL: &str = "create_canvas_components";
const FRONTEND_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub(crate) struct CreateCanvasComponentsTool(pub(crate) AgentToolRuntime);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateCanvasComponentsArgs {
    components: Vec<CreateCanvasComponentInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCanvasComponentInput {
    id: String,
    #[serde(rename = "type")]
    component_type: String,
    #[serde(default)]
    name: Option<String>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    #[serde(default)]
    order: Option<i64>,
    #[serde(default)]
    config: Option<Map<String, Value>>,
    #[serde(default)]
    use_default_config: bool,
}

#[derive(Clone, Copy, Debug)]
struct CreateConfigStats {
    configured: usize,
    defaults: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderRequest<'a> {
    action: &'static str,
    incremental: bool,
    interactive: bool,
    components: &'a [PageDraftComponent],
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenderResponse {
    status: String,
    draft_id: String,
    layer_count: usize,
    #[serde(default)]
    component_bindings: BTreeMap<String, String>,
    #[serde(default)]
    applied_layer_ids: Vec<String>,
    #[serde(default)]
    result_revision: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmRequest<'a> {
    draft_id: &'a str,
    message: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmResponse {
    confirmed: bool,
    draft_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AcceptRequest<'a> {
    action: &'static str,
    draft_id: &'a str,
    operation_id: &'a str,
    operation_fingerprint: &'a str,
    requested_component_ids: &'a [String],
    reserved_component_ids: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_ticket: Option<&'a ApprovalTicket>,
}

impl Tool for CreateCanvasComponentsTool {
    const NAME: &'static str = CREATE_CANVAS_COMPONENTS_TOOL;
    type Error = AgentToolError;
    type Args = CreateCanvasComponentsArgs;
    type Output = CanvasOperationReceipt;

    fn description(&self) -> String {
        format!(
            "创建真实画布组件：复杂配置默认每批 8–10 个，单次硬上限为 {MAX_TARGETS_PER_ACTION} 个。预计更多时保留完整组件方案并连续拆批；components 必须直接传 JSON 数组，禁止传序列化字符串。先生成隔离预览并等待用户确认，确认后原子写入；Applied 回执就是运行事实。"
        )
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "components": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_TARGETS_PER_ACTION,
                    "description": format!("必须直接传 JSON 数组，禁止传序列化字符串；复杂配置默认每批 8–10 个，单次硬上限为 {MAX_TARGETS_PER_ACTION} 个；超过时保留完整组件方案并连续拆批"),
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "type": { "type": "string" },
                            "name": { "type": "string" },
                            "x": { "type": "number" },
                            "y": { "type": "number" },
                            "width": { "type": "number", "exclusiveMinimum": 0 },
                            "height": { "type": "number", "exclusiveMinimum": 0 },
                            "order": { "type": "integer" },
                            "config": { "type": "object", "minProperties": 1 },
                            "useDefaultConfig": { "type": "boolean" }
                        },
                        "required": ["id", "type", "x", "y", "width", "height"],
                        "oneOf": [
                            { "required": ["config"] },
                            {
                                "required": ["useDefaultConfig"],
                                "properties": { "useDefaultConfig": { "const": true } }
                            }
                        ],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["components"],
            "additionalProperties": false
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0.context().authorize(Self::NAME, AgentToolEffect::Commit, false)?;
        self.0.context().security().grant().validate_scope(self.0.context().scope())?;
        let (components, config_stats) = normalize_create_components(args.components)?;
        let manager_id = require_manager(&self.0)?;
        let invocation = self.0.invocation(Self::NAME)?;
        let fingerprint = request_fingerprint(&components)?;
        let component_ids = components.iter().map(|component| component.id.clone()).collect();
        let journal = self.0.operation_journal()?;
        let existing = match journal
            .find_existing(Self::NAME, &invocation.tool_call_id, CanvasOperationKind::CreateComponents, &fingerprint)
            .await?
        {
            Some(record) => Some(record),
            None => journal.find_equivalent_create(&fingerprint).await?,
        };
        if let Some(record) = &existing {
            if let Some(receipt) = existing_terminal(record) {
                sync_security_resource(&self.0, &receipt)?;
                return Ok(receipt);
            }
            if record.receipt.status == CanvasOperationStatus::Applying {
                return self.recover_applying(record.receipt.clone()).await;
            }
        }

        self.refresh_validation_facts(&components).await?;
        let issues = validate_page_operation(&components, self.0.operation_validation());
        if !issues.is_empty() {
            return Err(validation_error(issues));
        }
        let draft_targets = components.iter().map(|component| component.id.clone()).collect::<Vec<_>>();
        let decision = match authorize_canvas_create(self.0.context().security().grant(), &components) {
            Ok(decision) => decision,
            Err(error) => {
                self.audit_security(
                    &invocation.tool_call_id,
                    &fingerprint,
                    &draft_targets,
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
        let found_existing = existing.is_some();
        let mut receipt = match existing {
            Some(record) => record.receipt,
            None => {
                journal
                    .prepare(Self::NAME, &invocation.tool_call_id, CanvasOperationKind::CreateComponents, &fingerprint, component_ids)
                    .await?
                    .receipt
            }
        };
        if !found_existing {
            if receipt.status.is_terminal() {
                sync_security_resource(&self.0, &receipt)?;
                return Ok(receipt);
            }
            if receipt.status == CanvasOperationStatus::Applying {
                return self.recover_applying(receipt).await;
            }
        }
        receipt.manager_id = Some(manager_id);
        receipt.configured_component_count = config_stats.configured;
        receipt.default_config_component_count = config_stats.defaults;
        self.audit_security(
            &invocation.tool_call_id,
            &fingerprint,
            &receipt.component_ids,
            risk,
            "requireApproval",
            "agent.security.approval_required",
            None,
            "pendingApproval",
        );

        let rendered = self.render_preview(&components).await?;
        let confirmation = self.confirm_preview(&rendered.draft_id).await;
        let confirmation = match confirmation {
            Ok(confirmation) => confirmation,
            Err(error) => {
                self.clear_preview().await;
                let receipt = failed_receipt(receipt, &error);
                return Ok(self.0.operation_journal()?.transition(receipt).await?.receipt);
            }
        };
        if confirmation.draft_id != rendered.draft_id {
            self.clear_preview().await;
            let error = AgentToolError::validation("agent.preview.draft_changed", "等待确认期间临时预览已发生变化");
            let receipt = failed_receipt(receipt, &error);
            return Ok(self.0.operation_journal()?.transition(receipt).await?.receipt);
        }
        if !confirmation.confirmed {
            self.clear_preview().await;
            receipt.status = CanvasOperationStatus::Rejected;
            self.audit_security(
                &invocation.tool_call_id,
                &fingerprint,
                &receipt.component_ids,
                risk,
                "deny",
                "agent.security.user_rejected",
                None,
                "rejected",
            );
            return Ok(self.0.operation_journal()?.transition(receipt).await?.receipt);
        }

        let approval_ticket = match decision {
            AuthorizationDecision::RequireApproval(_) => {
                let ticket =
                    self.0
                        .context()
                        .security()
                        .approvals()
                        .issue(self.0.context().scope(), Self::NAME, &fingerprint, &receipt.component_ids);
                self.0.context().security().approvals().consume(
                    &ticket,
                    self.0.context().scope(),
                    Self::NAME,
                    &fingerprint,
                    &receipt.component_ids,
                )?;
                self.audit_security(
                    &invocation.tool_call_id,
                    &fingerprint,
                    &receipt.component_ids,
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

        if let Err(error) = self.0.context().ensure_active() {
            self.clear_preview().await;
            return Err(error.into());
        }
        receipt.status = CanvasOperationStatus::Applying;
        receipt = self.0.operation_journal()?.transition(receipt).await?.receipt;
        let accepted = self
            .0
            .frontend()?
            .call_irreversible::<_, RenderResponse>(
                "renderDraft",
                AcceptRequest {
                    action: "accept",
                    draft_id: &rendered.draft_id,
                    operation_id: &receipt.operation_id,
                    operation_fingerprint: &fingerprint,
                    requested_component_ids: &receipt.component_ids,
                    reserved_component_ids: receipt
                        .component_ids
                        .iter()
                        .map(|component_id| (component_id.clone(), reserved_component_id(&receipt.operation_id, component_id)))
                        .collect(),
                    approval_ticket: approval_ticket.as_ref(),
                },
                Some(FRONTEND_TIMEOUT),
                self.0.context(),
            )
            .await;
        match accepted {
            Ok(accepted) if accepted.draft_id == rendered.draft_id => self.finish_accept(receipt, accepted, components.len()).await,
            Ok(_) => {
                let error = AgentToolError::validation("agent.operation.draft_changed", "前端接受的草稿与已确认草稿不一致");
                let receipt = failed_receipt(receipt, &error);
                Ok(self.0.operation_journal()?.transition(receipt).await?.receipt)
            }
            Err(error) => match self.recover_applying(receipt.clone()).await {
                Ok(recovered) => Ok(recovered),
                Err(_) => {
                    tracing::warn!(operation_id = %receipt.operation_id, error = %error, "canvas create outcome remains unresolved");
                    Ok(receipt)
                }
            },
        }
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

impl CreateCanvasComponentsTool {
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

    async fn refresh_validation_facts(&self, components: &[PageDraftComponent]) -> Result<(), AgentToolError> {
        let catalog: Value = self
            .0
            .frontend()?
            .call("getComponentCatalog", json!({}), Some(FRONTEND_TIMEOUT), self.0.context())
            .await?;
        self.0
            .validation_evidence()
            .record_component_catalog(&catalog)
            .map_err(|error| AgentToolError::validation(error.code, error.message))?;

        let data_component_types = components
            .iter()
            .filter(|component| component.config.contains_key("data"))
            .map(|component| component.component_type.trim())
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>();
        if !data_component_types.is_empty() {
            let docs = component_documents_response(data_component_types);
            self.0
                .validation_evidence()
                .record_component_documents(&docs)
                .map_err(|error| AgentToolError::validation(error.code, error.message))?;
        }
        for component in components.iter().filter(|component| component.config.contains_key("data")) {
            self.0
                .validation_evidence()
                .validate_component_spec_for_create(component.component_type.trim())
                .map_err(|error| AgentToolError::validation(error.code, error.message))?;
        }
        Ok(())
    }

    async fn render_preview(&self, components: &[PageDraftComponent]) -> Result<RenderResponse, AgentToolError> {
        let rendered: RenderResponse = self
            .0
            .frontend()?
            .call(
                "renderDraft",
                RenderRequest { action: "render", incremental: false, interactive: false, components },
                Some(FRONTEND_TIMEOUT),
                self.0.context(),
            )
            .await?;
        if rendered.status != "draft-rendered" || rendered.draft_id.trim().is_empty() || rendered.layer_count != components.len() {
            self.clear_preview().await;
            return Err(AgentToolError::validation("agent.preview.render_result_invalid", "前端返回的临时预览与动作参数不一致"));
        }
        Ok(rendered)
    }

    async fn confirm_preview(&self, draft_id: &str) -> Result<ConfirmResponse, AgentToolError> {
        self.0
            .frontend()?
            .call(
                "showPreview",
                ConfirmRequest { draft_id, message: "请检查页面预览；确认后将原子写入正式画布。" },
                None,
                self.0.context(),
            )
            .await
    }

    async fn finish_accept(
        &self,
        mut receipt: CanvasOperationReceipt,
        accepted: RenderResponse,
        expected_count: usize,
    ) -> Result<CanvasOperationReceipt, AgentToolError> {
        let expected_sources = receipt.component_ids.iter().map(|id| id.trim()).collect::<BTreeSet<_>>();
        let binding_keys = accepted.component_bindings.keys().map(|id| id.trim()).collect::<BTreeSet<_>>();
        let binding_values = accepted
            .component_bindings
            .values()
            .map(|id| id.trim().to_string())
            .collect::<Vec<_>>();
        let applied_ids = accepted
            .applied_layer_ids
            .iter()
            .map(|id| id.trim().to_string())
            .collect::<BTreeSet<_>>();
        let reserved_ids = receipt
            .component_ids
            .iter()
            .map(|component_id| reserved_component_id(&receipt.operation_id, component_id))
            .collect::<BTreeSet<_>>();
        let valid_binding = expected_sources.len() == expected_count
            && expected_sources.len() == receipt.component_ids.len()
            && expected_sources == binding_keys
            && binding_values.len() == binding_values.iter().filter(|id| !id.is_empty()).count()
            && binding_values.len() == applied_ids.len()
            && applied_ids.len() == expected_count
            && applied_ids == binding_values.iter().cloned().collect::<BTreeSet<_>>();
        let valid_reserved_binding = accepted
            .component_bindings
            .iter()
            .all(|(source_id, actual_id)| reserved_component_id(&receipt.operation_id, source_id) == actual_id.trim());
        if accepted.status != "draft-accepted"
            || accepted.layer_count != expected_count
            || accepted.component_bindings.len() != expected_count
            || !valid_binding
            || !valid_reserved_binding
            || applied_ids != reserved_ids
        {
            let error = AgentToolError::validation("agent.operation.draft_accept_invalid", "前端未完整写入已确认的组件");
            let receipt = failed_receipt(receipt, &error);
            return Ok(self.0.operation_journal()?.transition(receipt).await?.receipt);
        }
        receipt.status = CanvasOperationStatus::Applied;
        receipt.component_ids = accepted.component_bindings.values().cloned().collect();
        receipt.component_bindings = accepted.component_bindings;
        receipt.result_revision = accepted.result_revision;
        receipt.applied_at = Some(crate::backend::ai_driven::time::now_ms());
        let record = self.0.operation_journal()?.transition(receipt).await?;
        sync_security_resource(&self.0, &record.receipt)?;
        Ok(record.receipt)
    }

    pub(super) async fn recover_applying(&self, receipt: CanvasOperationReceipt) -> Result<CanvasOperationReceipt, AgentToolError> {
        let expected_count = receipt.component_ids.len();
        let status = match query_frontend_operation(&self.0, &receipt.operation_id).await {
            Ok(status) => status,
            Err(_) => return Ok(receipt),
        };
        match status.status.as_str() {
            "applied" => {
                let accepted = serde_json::from_value::<RenderResponse>(status.result.unwrap_or(Value::Null))
                    .map_err(|error| AgentToolError::validation("agent.operation.frontend_receipt_invalid", error.to_string()))?;
                self.finish_accept(receipt, accepted, expected_count).await
            }
            "failed" => {
                let error = status
                    .error
                    .map(|error| AgentToolError::validation(error.code, error.message))
                    .unwrap_or_else(|| AgentToolError::validation("agent.operation.frontend_failed", "前端动作执行失败"));
                let receipt = failed_receipt(receipt, &error);
                Ok(self.0.operation_journal()?.transition(receipt).await?.receipt)
            }
            "notFound" | "applying" => Ok(receipt),
            _ => Err(AgentToolError::validation("agent.operation.frontend_status_invalid", "前端返回了未知动作状态")),
        }
    }

    async fn clear_preview(&self) {
        let Ok(frontend) = self.0.frontend() else {
            return;
        };
        let _: Result<Value, AgentToolError> = frontend
            .call("renderDraft", json!({ "action": "clear" }), Some(FRONTEND_TIMEOUT), self.0.context())
            .await;
    }
}

fn normalize_create_components(components: Vec<CreateCanvasComponentInput>) -> Result<(Vec<PageDraftComponent>, CreateConfigStats), AgentToolError> {
    if components.is_empty() {
        return Err(AgentToolError::invalid("agent.operation.empty_page", "创建组件列表不能为空"));
    }
    let mut drafts = Vec::with_capacity(components.len());
    let mut stats = CreateConfigStats { configured: 0, defaults: 0 };
    for (index, component) in components.into_iter().enumerate() {
        let config = match (component.config, component.use_default_config) {
            (Some(_), true) => {
                return Err(AgentToolError::invalid(
                    "agent.operation.create_config_intent_conflict",
                    format!("components[{index}] 不能同时提供 config 和 useDefaultConfig=true"),
                ));
            }
            (Some(config), false) if config.is_empty() => {
                return Err(AgentToolError::invalid(
                    "agent.operation.empty_create_config",
                    format!("components[{index}].config 不能为空；需要默认配置时显式使用 useDefaultConfig=true"),
                ));
            }
            (Some(config), false) => {
                validate_business_config(index, &component.component_type, &config)?;
                stats.configured += 1;
                config
            }
            (None, true) => {
                if component_data_requirements(&component.component_type).is_some_and(|(_, supports)| supports) {
                    return Err(AgentToolError::invalid(
                        "agent.operation.data_component_default_config_forbidden",
                        format!("components[{index}] 是数据组件，创建时必须提供实际 config.data 和字段映射"),
                    ));
                }
                stats.defaults += 1;
                Map::new()
            }
            (None, false) => {
                return Err(AgentToolError::invalid(
                    "agent.operation.create_config_intent_required",
                    format!("components[{index}] 必须提供 config 或 useDefaultConfig=true"),
                ));
            }
        };
        drafts.push(PageDraftComponent {
            id: component.id.trim().to_string(),
            component_type: component.component_type.trim().to_string(),
            name: component.name.map(|name| name.trim().to_string()),
            x: component.x,
            y: component.y,
            width: component.width,
            height: component.height,
            order: component.order,
            config,
        });
    }
    Ok((drafts, stats))
}

fn validate_business_config(index: usize, component_type: &str, config: &Map<String, Value>) -> Result<(), AgentToolError> {
    let supports_data = component_data_requirements(component_type).is_some_and(|(_, supports)| supports);
    if !supports_data {
        return Ok(());
    }
    if !config.get("data").is_some_and(|value| !value.is_null()) {
        return Err(AgentToolError::invalid(
            "agent.operation.create_data_required",
            format!("components[{index}].config.data 必须包含实际业务数据"),
        ));
    }
    validate_business_data(index, component_type, &config["data"])?;
    if component_type.starts_with("Antd") {
        let style = config.get("style").and_then(Value::as_object).ok_or_else(|| {
            AgentToolError::invalid(
                "agent.operation.create_style_required",
                format!("components[{index}].config.style 必须包含图表字段映射和必要样式"),
            )
        })?;
        if !style.keys().any(|key| is_component_data_field_mapping_key(key)) {
            return Err(AgentToolError::invalid(
                "agent.operation.create_field_mapping_required",
                format!("components[{index}].config.style 缺少图表字段映射"),
            ));
        }
    }
    Ok(())
}

fn validate_business_data(index: usize, component_type: &str, data: &Value) -> Result<(), AgentToolError> {
    let source_type = data
        .get("sourceType")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            AgentToolError::invalid("agent.operation.create_data_source_required", format!("components[{index}].config.data.sourceType 不能为空"))
        })?;
    if source_type != "static" {
        return Ok(());
    }
    let value = data.get("staticData").and_then(|value| value.get("value"));
    let has_content = value.is_some_and(|value| match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    });
    if !has_content {
        return Err(AgentToolError::invalid(
            "agent.operation.create_static_data_empty",
            format!("components[{index}]（{component_type}）必须包含非空 staticData.value"),
        ));
    }
    if component_type == "BaseText" && !value.is_some_and(|value| value.as_str().is_some_and(|text| !text.trim().is_empty())) {
        return Err(AgentToolError::invalid(
            "agent.operation.create_text_required",
            format!("components[{index}] 创建文本组件时必须提供非空文本"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod config_tests {
    use super::*;

    fn component(config: Value) -> CreateCanvasComponentInput {
        serde_json::from_value(json!({
            "id": "area",
            "type": "AntdBaseArea",
            "x": 0,
            "y": 0,
            "width": 100,
            "height": 100,
            "config": config
        }))
        .unwrap()
    }

    #[test]
    fn create_requires_explicit_non_empty_business_config() {
        let missing = serde_json::from_value::<CreateCanvasComponentInput>(json!({
            "id": "x", "type": "BaseText", "x": 0, "y": 0, "width": 10, "height": 10
        }))
        .unwrap();
        assert_eq!(normalize_create_components(vec![missing]).unwrap_err().code, "agent.operation.create_config_intent_required");
        assert_eq!(
            normalize_create_components(vec![component(json!({}))]).unwrap_err().code,
            "agent.operation.empty_create_config"
        );
    }

    #[test]
    fn chart_create_requires_data_and_field_mapping() {
        let no_mapping = component(json!({
            "data": { "sourceType": "static", "staticData": { "value": [{"time":"10:00","ph":7.1}] } },
            "style": {}
        }));
        assert_eq!(
            normalize_create_components(vec![no_mapping]).unwrap_err().code,
            "agent.operation.create_field_mapping_required"
        );
        let valid = component(json!({
            "data": { "sourceType": "static", "staticData": { "value": [{"time":"10:00","ph":7.1}] } },
            "style": { "xField": "time", "yField": "ph", "color": "#00ffff" }
        }));
        let (_, stats) = normalize_create_components(vec![valid]).unwrap();
        assert_eq!((stats.configured, stats.defaults), (1, 0));
    }

    #[test]
    fn static_data_must_contain_business_content() {
        let empty = component(json!({
            "data": { "sourceType": "static", "staticData": { "value": [] } },
            "style": { "xField": "time", "yField": "ph" }
        }));
        assert_eq!(normalize_create_components(vec![empty]).unwrap_err().code, "agent.operation.create_static_data_empty");
    }

    #[test]
    fn text_create_requires_non_empty_text_value() {
        let text = serde_json::from_value::<CreateCanvasComponentInput>(json!({
            "id": "title",
            "type": "BaseText",
            "x": 0,
            "y": 0,
            "width": 100,
            "height": 30,
            "config": {
                "data": { "sourceType": "static", "staticData": { "value": [{"text":""}] } },
                "style": { "color": "#FFFFFF" }
            }
        }))
        .unwrap();

        assert_eq!(normalize_create_components(vec![text]).unwrap_err().code, "agent.operation.create_text_required");
    }
}
