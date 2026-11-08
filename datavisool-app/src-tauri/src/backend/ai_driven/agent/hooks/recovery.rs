/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 跨 runner 和应用恢复保留预算；不提供任何业务授权。
use super::AgentRuntimeHook;
use serde_json::{json, Value};

impl AgentRuntimeHook {
    /// Stop-gate budgets must be durable before publishing another continuation.
    /// Use the same Run lock and checkpoint snapshot as tool/model hooks.
    pub(crate) async fn persist_run_recovery(&self) -> Result<(), crate::backend::ai_driven::task::TaskRepositoryError> {
        self.update_working_checkpoint(super::now_ms(), |_| {}).await
    }

    pub(crate) async fn record_lightweight_compaction(&self, ctx: &rig_core::agent::HookContext, before: u64, after: u64) -> rig_core::agent::Flow {
        self.budget.record_compaction(before, after);
        let payload = self.payload(
            ctx,
            json!({"reason": "applied_receipt", "inputTokensBefore": before, "inputTokensAfter": after, "modelSummaryCalled": false,
                "usage": self.budget.usage_snapshot()}),
        );
        let flow = self
            .persist_or_terminate("agent.context.lightweight_compacted", None, payload)
            .await;
        if matches!(flow, rig_core::agent::Flow::Continue) && self.tool_route.is_some() {
            if self.update_working_checkpoint(super::now_ms(), |_| {}).await.is_err() {
                return rig_core::agent::Flow::terminate("agent.task_checkpoint_failed");
            }
        }
        flow
    }
    pub(crate) fn with_recovery_state(self, value: Option<&Value>) -> Result<Self, String> {
        let Some(value) = value.filter(|value| !value.is_null()) else {
            return Ok(self);
        };
        if let Some(usage) = value.get("usage") {
            self.budget
                .restore_usage(&serde_json::from_value(usage.clone()).map_err(|error| error.to_string())?);
        }
        if let Some(failures) = value.get("failures") {
            self.tool_failure_tracker
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .restore(failures)?;
        }
        if let Some(operations) = value.get("compactedOperations") {
            *self.compacted_operations.lock().unwrap_or_else(|e| e.into_inner()) =
                serde_json::from_value(operations.clone()).map_err(|error| error.to_string())?;
        }
        if let Some(completion) = value.get("completion") {
            self.completion_requirements.restore_completion_recovery(completion)?;
        }
        Ok(self)
    }

    pub(super) fn recovery_snapshot(&self) -> Value {
        json!({
            "usage": self.budget.usage_snapshot(),
            "failures": self.tool_failure_tracker.lock().unwrap_or_else(|e| e.into_inner()).snapshot(),
            "compactedOperations": *self.compacted_operations.lock().unwrap_or_else(|e| e.into_inner()),
            "completion": self.completion_requirements.completion_recovery_snapshot(),
        })
    }

    pub(crate) fn claim_applied_compaction(&self, operation_id: &str) -> bool {
        !operation_id.is_empty()
            && self
                .compacted_operations
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(operation_id.to_string())
    }
}
