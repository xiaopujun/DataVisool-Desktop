/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 宿主在 Agent 运行前预加载组件目录和样式文档。
//!
//! 组件数据说明书不在此处预读；必须由 Agent 显式调用
//! get_component_data_contract，才能形成数据修改凭证。

use std::time::Duration;

use serde_json::{json, Value};

use crate::backend::ai_driven::agent_context::input::AgentRunInput;
use crate::backend::ai_driven::component_docs::component_documents_response;
use crate::backend::ai_driven::domain::validation_evidence::AgentValidationEvidence;
use crate::backend::ai_driven::tools::context::AgentToolContext;
use crate::backend::ai_driven::tools::error::AgentToolError;
use crate::backend::ai_driven::tools::runtime::AgentFrontendRuntime;
use crate::backend::utils::{debug_log_json, sanitize_debug_log_text};

pub(super) async fn preload_validation_facts(
    frontend: &AgentFrontendRuntime,
    context: &AgentToolContext,
    evidence: &AgentValidationEvidence,
    input: &AgentRunInput,
) -> Result<(), AgentToolError> {
    const TIMEOUT: Duration = Duration::from_secs(60);
    let catalog: Value = frontend
        .call("getComponentCatalog", json!({}), Some(TIMEOUT), context)
        .await
        .map_err(|error| {
            log_validation_error(context, "getComponentCatalog", &error);
            error
        })?;
    log_validation_payload(context, "component catalog", &catalog);
    evidence.record_component_catalog(&catalog).map_err(|issue| {
        log_validation_issue(context, "component catalog", &issue.code, &issue.message);
        AgentToolError::validation(issue.code, issue.message)
    })?;

    let mut component_ids = input
        .selected_components()
        .iter()
        .map(|component| component.id.trim().to_string())
        .chain(
            input
                .selection
                .as_ref()
                .into_iter()
                .flat_map(|selection| selection.target_ids.iter().map(|value| value.trim().to_string())),
        )
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    component_ids.sort();
    component_ids.dedup();
    let mut component_types = input
        .selected_components()
        .iter()
        .map(|component| component.component_type_text())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();

    if !component_ids.is_empty() {
        let snapshots: Value = frontend
            .call(
                "getComponentSnapshots",
                json!({ "componentIds": component_ids, "fields": ["base", "style", "data"] }),
                Some(TIMEOUT),
                context,
            )
            .await
            .map_err(|error| {
                log_validation_error(context, "getComponentSnapshots", &error);
                error
            })?;
        log_validation_payload(context, "component snapshots", &snapshots);
        component_types.extend(snapshot_component_types(&snapshots));
    }
    component_types.sort();
    component_types.dedup();
    if !component_types.is_empty() {
        let docs = component_documents_response(component_types.iter().map(String::as_str));
        log_validation_payload(context, "component documents", &docs);
        evidence.record_component_documents(&docs).map_err(|issue| {
            log_validation_issue(context, "component documents", &issue.code, &issue.message);
            AgentToolError::validation(issue.code, issue.message)
        })?;
    }
    Ok(())
}

fn log_validation_payload(context: &AgentToolContext, stage: &str, payload: &Value) {
    let scope = context.scope();
    tracing::debug!(
        target: "agent.validation",
        task_id = %scope.task_id,
        turn_id = ?scope.turn_id.as_deref(),
        run_id = %scope.run_id,
        stage,
        payload = %debug_log_json(payload),
        "Agent validation data loaded"
    );
}

fn log_validation_error(context: &AgentToolContext, stage: &str, error: &AgentToolError) {
    log_validation_issue(context, stage, &error.code, &error.message);
}

fn log_validation_issue(context: &AgentToolContext, stage: &str, code: &str, message: &str) {
    let scope = context.scope();
    tracing::debug!(
        target: "agent.validation",
        task_id = %scope.task_id,
        turn_id = ?scope.turn_id.as_deref(),
        run_id = %scope.run_id,
        stage,
        code,
        message = %sanitize_debug_log_text(message),
        "Agent validation failed"
    );
}

fn snapshot_component_types(snapshots: &Value) -> impl Iterator<Item = String> + '_ {
    snapshots
        .get("components")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|component| {
            component
                .get("type")
                .and_then(Value::as_str)
                .or_else(|| component.get("base")?.get("type")?.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}
