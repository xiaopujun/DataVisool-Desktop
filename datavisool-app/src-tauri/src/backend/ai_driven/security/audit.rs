/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use serde::Serialize;

use super::SecurityRisk;

/// 安全决策审计记录。只存资源标识、字段摘要和哈希，不存明文凭证。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SecurityAuditEvent<'a> {
    pub(crate) task_id: &'a str,
    pub(crate) run_id: &'a str,
    pub(crate) tool_call_id: &'a str,
    pub(crate) tool_name: &'a str,
    pub(crate) argument_hash: &'a str,
    pub(crate) resource_scope: &'a [String],
    pub(crate) risk: SecurityRisk,
    pub(crate) decision: &'a str,
    pub(crate) decision_code: &'a str,
    pub(crate) approval_id: Option<&'a str>,
    pub(crate) network_destination: Option<&'a str>,
    pub(crate) result_status: &'a str,
}

#[derive(Clone, Default)]
pub(crate) struct AgentSecurityAudit;

impl AgentSecurityAudit {
    pub(crate) fn record(&self, event: SecurityAuditEvent<'_>) {
        tracing::info!(
            target: "agent.security.audit",
            task_id = event.task_id,
            run_id = event.run_id,
            tool_call_id = event.tool_call_id,
            tool_name = event.tool_name,
            argument_hash = event.argument_hash,
            resource_scope = ?event.resource_scope,
            risk = ?event.risk,
            decision = event.decision,
            decision_code = event.decision_code,
            approval_id = event.approval_id,
            network_destination = event.network_destination,
            result_status = event.result_status,
            "agent security decision"
        );
    }
}
