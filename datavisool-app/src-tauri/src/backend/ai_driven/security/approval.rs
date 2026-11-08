/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rand::RngCore;
use serde::Serialize;

use crate::backend::ai_driven::time::now_ms;
use crate::backend::ai_driven::tools::context::AgentToolScope;
use crate::backend::utils::sha256_hex;

use super::SecurityDecisionError;

const APPROVAL_TTL_MS: i64 = 5 * 60 * 1000;

/// 只由 Rust 签发的一次性审批票据。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApprovalTicket {
    pub(crate) approval_id: String,
    pub(crate) task_id: String,
    pub(crate) run_id: String,
    pub(crate) tool_name: String,
    pub(crate) argument_hash: String,
    pub(crate) resource_scope_hash: String,
    pub(crate) issued_at_ms: i64,
    pub(crate) expires_at_ms: i64,
    pub(crate) nonce: String,
}

#[derive(Clone, Default)]
pub(crate) struct ApprovalTicketStore {
    pending: Arc<Mutex<HashMap<String, ApprovalTicket>>>,
}

impl ApprovalTicketStore {
    pub(crate) fn issue(&self, scope: &AgentToolScope, tool_name: &str, argument_hash: &str, resource_scope: &[String]) -> ApprovalTicket {
        let issued_at_ms = now_ms();
        let ticket = ApprovalTicket {
            approval_id: random_id(),
            task_id: scope.task_id.clone(),
            run_id: scope.run_id.clone(),
            tool_name: tool_name.to_string(),
            argument_hash: argument_hash.to_string(),
            resource_scope_hash: hash_scope(resource_scope),
            issued_at_ms,
            expires_at_ms: issued_at_ms.saturating_add(APPROVAL_TTL_MS),
            nonce: random_id(),
        };
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(ticket.approval_id.clone(), ticket.clone());
        ticket
    }

    pub(crate) fn consume(
        &self,
        ticket: &ApprovalTicket,
        scope: &AgentToolScope,
        tool_name: &str,
        argument_hash: &str,
        resource_scope: &[String],
    ) -> Result<(), SecurityDecisionError> {
        let stored = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&ticket.approval_id)
            .ok_or_else(|| SecurityDecisionError::deny("agent.security.approval_missing", "审批票据不存在、已过期或已被使用"))?;
        if stored.expires_at_ms <= now_ms() {
            return Err(SecurityDecisionError::deny("agent.security.approval_expired", "审批票据已过期"));
        }
        if stored.task_id != scope.task_id
            || stored.run_id != scope.run_id
            || stored.tool_name != tool_name
            || stored.argument_hash != argument_hash
            || stored.resource_scope_hash != hash_scope(resource_scope)
            || stored.nonce != ticket.nonce
        {
            return Err(SecurityDecisionError::deny("agent.security.approval_mismatch", "审批票据与当前动作参数或作用域不匹配"));
        }
        Ok(())
    }
}

fn hash_scope(resource_scope: &[String]) -> String {
    let mut values = resource_scope.iter().map(|value| value.trim()).collect::<Vec<_>>();
    values.sort_unstable();
    format!("sha256:{}", sha256_hex(values.join("\n").as_bytes()))
}

fn random_id() -> String {
    let mut bytes = [0_u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> AgentToolScope {
        AgentToolScope::new("task", "run", "thread", "project").unwrap()
    }

    #[test]
    fn ticket_is_single_use_and_parameter_bound() {
        let store = ApprovalTicketStore::default();
        let scope = scope();
        let ticket = store.issue(&scope, "patch", "args-1", &["component-1".to_string()]);
        assert!(store
            .consume(&ticket, &scope, "patch", "args-1", &["component-1".to_string()])
            .is_ok());
        assert_eq!(
            store
                .consume(&ticket, &scope, "patch", "args-1", &["component-1".to_string()])
                .unwrap_err()
                .code(),
            "agent.security.approval_missing"
        );
    }

    #[test]
    fn ticket_rejects_changed_arguments() {
        let store = ApprovalTicketStore::default();
        let scope = scope();
        let ticket = store.issue(&scope, "patch", "args-1", &["component-1".to_string()]);
        assert_eq!(
            store
                .consume(&ticket, &scope, "patch", "args-2", &["component-1".to_string()])
                .unwrap_err()
                .code(),
            "agent.security.approval_mismatch"
        );
    }
}
