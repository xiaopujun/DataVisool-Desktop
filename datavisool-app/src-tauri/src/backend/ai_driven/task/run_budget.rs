/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Frozen run limits and explicit, cumulative budget extensions.
use serde::{Deserialize, Serialize};

use super::AgentExecutionLimitsSnapshot;
use crate::backend::ai_driven::agent::ExecutionUsageSnapshot;
use crate::backend::ai_driven::settings::AiDrivenRuntimeSettings;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BudgetExtension {
    pub max_total_tokens: Option<u64>,
    pub max_model_calls: Option<u32>,
    pub max_tool_calls: Option<u32>,
    pub max_write_calls: Option<u32>,
    pub max_frontend_interactions: Option<u32>,
    pub max_duration_ms: Option<u64>,
}

impl AgentExecutionLimitsSnapshot {
    /// Resuming never reads the current global preferences as a new authorization.
    pub(crate) fn runtime_settings(&self) -> AiDrivenRuntimeSettings {
        let mut settings = AiDrivenRuntimeSettings::default();
        settings.context = self.context.clone();
        settings.model_request = self.model_request.clone();
        let e = &mut settings.execution;
        e.token_budget_enabled = self.token_budget_enabled;
        e.call_budget_enabled = self.call_budget_enabled;
        e.duration_budget_enabled = self.duration_budget_enabled;
        e.max_model_calls = self.max_model_calls.into();
        e.max_tool_calls = self.max_tool_calls.into();
        e.max_write_calls = self.max_write_calls.into();
        e.max_total_tokens = self.max_total_tokens;
        e.max_duration_seconds = self.max_duration_ms.div_ceil(1000);
        e.max_tool_duration_seconds = self.max_tool_duration_ms.div_ceil(1000);
        e.max_consecutive_failures = self.max_consecutive_failures.into();
        e.max_frontend_interactions = self.max_frontend_interactions.into();
        e.max_same_read_repeats = self.max_same_read_repeats.into();
        e.max_completion_continuations = self.max_completion_continuations.into();
        settings
    }

    pub(crate) fn extend_budget(&self, extension: &BudgetExtension, usage: &ExecutionUsageSnapshot) -> Result<Self, String> {
        let mut next = self.clone();
        let mut changed = false;
        macro_rules! extend {
            ($field:ident, $used:expr, $enabled:expr, $max:expr) => {
                if let Some(value) = extension.$field {
                    if !$enabled || value <= self.$field || u64::from(value) <= $used || u64::from(value) > $max {
                        return Err(format!("{} 必须增加已启用的预算且高于累计用量，并在允许范围内", stringify!($field)));
                    }
                    next.$field = value;
                    changed = true;
                }
            };
        }
        extend!(max_total_tokens, usage.total_tokens, self.token_budget_enabled, 10_000_000);
        extend!(max_model_calls, usage.model_calls, self.call_budget_enabled, 1_000);
        extend!(max_tool_calls, usage.tool_calls, self.call_budget_enabled, 5_000);
        extend!(max_write_calls, usage.write_calls, self.call_budget_enabled, 1_000);
        extend!(max_frontend_interactions, usage.frontend_interactions, self.call_budget_enabled, 500);
        extend!(max_duration_ms, usage.elapsed_ms, self.duration_budget_enabled, 86_400_000);
        if !changed {
            return Err("继续执行需要明确增加运行预算".to_string());
        }
        if next.token_budget_enabled && next.max_total_tokens <= usage.total_tokens
            || next.duration_budget_enabled && next.max_duration_ms <= usage.elapsed_ms
            || next.call_budget_enabled
                && (u64::from(next.max_model_calls) <= usage.model_calls
                    || u64::from(next.max_tool_calls) <= usage.tool_calls
                    || u64::from(next.max_write_calls) <= usage.write_calls
                    || u64::from(next.max_frontend_interactions) <= usage.frontend_interactions)
        {
            return Err("仍有预算已耗尽，请增加所有已耗尽的预算".to_string());
        }
        next.runtime_settings().validate()?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_budget_flags_are_rejected() {
        assert!(serde_json::from_str::<AgentExecutionLimitsSnapshot>(r#"{"maxTotalTokens":120000}"#).is_err());
    }

    fn configured_limits() -> AgentExecutionLimitsSnapshot {
        serde_json::from_value(serde_json::json!({
            "tokenBudgetEnabled":true, "callBudgetEnabled":true, "durationBudgetEnabled":true,
            "maxModelCalls":120, "maxToolCalls":320, "maxWriteCalls":120,
            "maxTotalTokens":1000000, "maxDurationMs":4500000, "maxToolDurationMs":600000,
            "maxConsecutiveFailures":25, "maxFrontendInteractions":40,
            "maxSameReadRepeats":3, "maxCompletionContinuations":12
        }))
        .unwrap()
    }

    #[test]
    fn extension_must_cover_every_exhausted_dimension() {
        let limits = configured_limits();
        let usage = ExecutionUsageSnapshot { total_tokens: 1000000, model_calls: 120, elapsed_ms: 4500000, ..Default::default() };
        let mut extension = BudgetExtension { max_total_tokens: Some(2000000), ..Default::default() };
        assert!(limits.extend_budget(&extension, &usage).is_err());
        extension.max_model_calls = Some(240);
        assert!(limits.extend_budget(&extension, &usage).is_err());
        extension.max_duration_ms = Some(9000000);
        let next = limits.extend_budget(&extension, &usage).unwrap();
        assert_eq!(next.max_total_tokens, 2000000);
        assert_eq!(next.max_model_calls, 240);
        assert_eq!(next.max_duration_ms, 9000000);
        assert_eq!(next.max_tool_duration_ms, limits.max_tool_duration_ms);
        assert_eq!(next.context, limits.context);
    }

    #[test]
    fn extension_cannot_enable_disabled_budget_or_accept_empty_authorization() {
        let mut limits = configured_limits();
        limits.token_budget_enabled = false;
        assert!(limits
            .extend_budget(&BudgetExtension::default(), &ExecutionUsageSnapshot::default())
            .is_err());
        let extension = BudgetExtension { max_total_tokens: Some(2000000), ..Default::default() };
        assert!(limits.extend_budget(&extension, &ExecutionUsageSnapshot::default()).is_err());
        assert!(serde_json::from_str::<BudgetExtension>(r#"{"tokenBudgetEnabled":false}"#).is_err());
    }
}
