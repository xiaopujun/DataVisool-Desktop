/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 工具失败是否应立即终止 Agent 的项目级策略。

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

use rig_core::tool::{ToolFailure, ToolFailureKind, ToolOutcome};
use serde_json::{json, Value};

use super::argument_shape::detect_argument_shape_issue;

const STRATEGY_SWITCH_THRESHOLD: u8 = 2;
const REPEATED_FAILURE_LIMIT: u8 = 3;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
struct FailureSignature {
    tool_name: String,
    code: String,
    field_path: String,
    targets: Vec<String>,
    authorization_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RepeatedFailureAction {
    Continue,
    PermissionDenied,
    Cancelled,
    SwitchStrategy { attempt: u8, field_path: String },
    Terminate { attempts: u8, field_path: String },
}

pub(super) enum RepeatedFailureFlow {
    RewriteResult(&'static str),
    Terminate(&'static str),
}

pub(super) struct RepeatedFailureDirective {
    pub event_kind: &'static str,
    pub payload: Value,
    pub flow: RepeatedFailureFlow,
}

#[derive(Default)]
pub(super) struct ToolFailureTracker {
    counts: HashMap<FailureSignature, u8>,
    denied: Vec<DeniedAttempt>,
}

#[derive(Clone, Deserialize, Serialize)]
struct DeniedAttempt {
    tool_name: String,
    targets: Vec<String>,
    authorization_revision: u64,
}

#[derive(Default, Deserialize, Serialize)]
struct FailureState {
    counts: Vec<(FailureSignature, u8)>,
    denied: Vec<DeniedAttempt>,
}

pub(super) fn repeated_failure_directive(action: RepeatedFailureAction, tool_name: &str, outcome: &ToolOutcome) -> Option<RepeatedFailureDirective> {
    let failure_code = outcome.failure().and_then(|failure| failure.code.as_deref());
    match action {
        RepeatedFailureAction::Continue => None,
        RepeatedFailureAction::PermissionDenied => Some(RepeatedFailureDirective {
            event_kind: "agent.tool_call.permission_denied",
            payload: json!({"toolName": tool_name, "failureCode": failure_code, "retryable": false}),
            flow: RepeatedFailureFlow::RewriteResult(
                "当前授权拒绝了此动作，不得换参数或重复读取来重试。保留已成功结果，选择已获授权的替代方案、说明未应用动作或请求用户决策。",
            ),
        }),
        RepeatedFailureAction::Cancelled => Some(RepeatedFailureDirective {
            event_kind: "agent.cancelled",
            payload: json!({"toolName": tool_name, "failureCode": failure_code}),
            flow: RepeatedFailureFlow::Terminate("agent_execution.cancelled"),
        }),
        RepeatedFailureAction::SwitchStrategy { attempt, field_path } => Some(RepeatedFailureDirective {
            event_kind: "agent.tool_call.strategy_required",
            payload: json!({
                "toolName": tool_name,
                "failureCode": failure_code,
                "fieldPath": field_path,
                "attempt": attempt,
            }),
            flow: RepeatedFailureFlow::RewriteResult(if tool_name == "create_canvas_components" && field_path == "components" {
                "同一参数形状再次失败。必须重新生成工具参数：components 直接传 JSON 数组，正确形状是 {\"components\":[{...}]}，错误形状是 {\"components\":\"[{...}]\"}。禁止原样重放，禁止 JSON.stringify；下一次相同失败将耗尽修正预算。"
            } else {
                "同一原因再次失败。按照错误的 fieldPath 和 recovery 修正参数；版本冲突须先读取当前状态再计算补丁；不要盲目重放原请求。下一次相同失败将耗尽修正预算。"
            }),
        }),
        RepeatedFailureAction::Terminate { attempts, field_path } => Some(RepeatedFailureDirective {
            event_kind: "agent.failed",
            payload: json!({
                "code": "agent.tool.repeated_failure",
                "toolName": tool_name,
                "failureCode": failure_code,
                "fieldPath": field_path,
                "attempts": attempts,
            }),
            flow: RepeatedFailureFlow::Terminate("agent.tool.repeated_failure"),
        }),
    }
}

impl ToolFailureTracker {
    pub(super) fn record(&mut self, tool_name: &str, outcome: &ToolOutcome, result: &str) -> RepeatedFailureAction {
        self.record_attempt(tool_name, outcome, result, "{}", 0)
    }

    pub(super) fn snapshot(&self) -> Value {
        serde_json::to_value(FailureState {
            counts: self.counts.iter().map(|(key, count)| (key.clone(), *count)).collect(),
            denied: self.denied.clone(),
        })
        .expect("failure state is serializable")
    }

    pub(super) fn restore(&mut self, value: &Value) -> Result<(), String> {
        let state: FailureState = serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
        self.counts = state.counts.into_iter().collect();
        self.denied = state.denied;
        Ok(())
    }

    pub(super) fn blocked_attempt(&self, tool_name: &str, args: &str, authorization_revision: u64) -> Option<String> {
        let targets = argument_targets(args);
        self.denied
            .iter()
            .find(|denied| {
                denied.tool_name == tool_name
                    && denied.authorization_revision == authorization_revision
                    && (denied.targets.is_empty() || targets.iter().any(|target| denied.targets.contains(target)))
            })
            .map(|_| "agent.tool.permission_retry_blocked".to_string())
    }

    pub(super) fn record_attempt(
        &mut self,
        tool_name: &str,
        outcome: &ToolOutcome,
        result: &str,
        args: &str,
        authorization_revision: u64,
    ) -> RepeatedFailureAction {
        let Some(failure) = outcome.failure() else {
            return RepeatedFailureAction::Continue;
        };
        if failure.kind == ToolFailureKind::Cancelled {
            return RepeatedFailureAction::Cancelled;
        }
        let permission_denied = failure.kind == ToolFailureKind::PermissionDenied;
        let targets = argument_targets(args);
        if permission_denied {
            self.denied
                .push(DeniedAttempt { tool_name: tool_name.to_string(), targets: targets.clone(), authorization_revision });
        }
        let signature = FailureSignature {
            tool_name: tool_name.to_string(),
            code: failure.code.clone().unwrap_or_else(|| failure.kind.as_str().to_string()),
            field_path: failure_field_path(result, tool_name, args),
            // A different forbidden ID must not reset the permission recovery allowance.
            targets: if permission_denied { Vec::new() } else { targets },
            authorization_revision: if permission_denied { authorization_revision } else { 0 },
        };
        let attempts = self.counts.entry(signature.clone()).or_default();
        *attempts = attempts.saturating_add(1);
        if permission_denied && *attempts == 1 {
            return RepeatedFailureAction::PermissionDenied;
        }
        if permission_denied || *attempts >= REPEATED_FAILURE_LIMIT {
            return RepeatedFailureAction::Terminate { attempts: *attempts, field_path: signature.field_path };
        }
        if *attempts >= STRATEGY_SWITCH_THRESHOLD {
            RepeatedFailureAction::SwitchStrategy { attempt: *attempts, field_path: signature.field_path }
        } else {
            RepeatedFailureAction::Continue
        }
    }
}

/// 存储或内部基础设施错误无法由模型修复，不应继续消耗调用预算。
pub(super) fn non_retryable_infrastructure_failure(outcome: &ToolOutcome) -> Option<&ToolFailure> {
    outcome
        .failure()
        .filter(|failure| failure.kind == ToolFailureKind::Other && failure.retryable == Some(false))
}

fn failure_field_path(result: &str, tool_name: &str, args: &str) -> String {
    let value = serde_json::from_str::<Value>(result).ok().or_else(|| {
        result
            .rsplit_once(": ")
            .and_then(|(_, payload)| serde_json::from_str::<Value>(payload).ok())
    });
    value
        .and_then(|value| find_field_path(&value))
        .map(|path| normalize_field_path(&path))
        .filter(|path| !path.is_empty())
        .or_else(|| detect_argument_shape_issue(tool_name, args).map(|issue| issue.field_path.to_string()))
        .unwrap_or_default()
}

fn find_field_path(value: &Value) -> Option<String> {
    match value {
        Value::Object(object) => {
            for key in ["fieldPath", "instancePath", "path", "field"] {
                if let Some(path) = object
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                {
                    return Some(path.to_string());
                }
            }
            object.values().find_map(find_field_path)
        }
        Value::Array(values) => values.iter().find_map(find_field_path),
        _ => None,
    }
}

fn normalize_field_path(path: &str) -> String {
    path.trim()
        .trim_start_matches('/')
        .replace('/', ".")
        .replace('[', ".")
        .replace(']', "")
        .split('.')
        .filter(|part| !part.is_empty())
        .map(|part| if part.chars().all(|c| c.is_ascii_digit()) { "*" } else { part })
        .collect::<Vec<_>>()
        .join(".")
}

fn argument_targets(args: &str) -> Vec<String> {
    fn collect(value: &Value, targets: &mut BTreeSet<String>) {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    if matches!(key.as_str(), "componentId" | "targetId" | "operationId" | "path") {
                        if let Some(id) = value.as_str().map(str::trim).filter(|id| !id.is_empty()) {
                            targets.insert(id.to_string());
                        }
                    } else if matches!(key.as_str(), "componentIds" | "targetIds") {
                        if let Some(values) = value.as_array() {
                            targets.extend(
                                values
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .map(str::trim)
                                    .filter(|id| !id.is_empty())
                                    .map(str::to_string),
                            );
                        }
                    } else if key != "config" {
                        collect(value, targets);
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    collect(value, targets);
                }
            }
            _ => {}
        }
    }
    let mut targets = BTreeSet::new();
    if let Ok(value) = serde_json::from_str(args) {
        collect(&value, &mut targets);
    }
    targets.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_only_non_retryable_infrastructure_failures() {
        let fatal = ToolOutcome::Error(
            ToolFailure::other("storage failed")
                .with_code("agent.workspace.storage_failed")
                .with_retryable(false),
        );
        assert_eq!(
            non_retryable_infrastructure_failure(&fatal).and_then(|failure| failure.code.as_deref()),
            Some("agent.workspace.storage_failed")
        );

        let repairable = ToolOutcome::Error(ToolFailure::invalid_args("fix arguments").with_retryable(true));
        assert!(non_retryable_infrastructure_failure(&repairable).is_none());
    }

    #[test]
    fn repeated_same_failure_requires_strategy_switch_then_terminates() {
        let outcome = ToolOutcome::Error(
            ToolFailure::invalid_args("invalid field")
                .with_code("agent.patch.invalid")
                .with_retryable(true),
        );
        let mut tracker = ToolFailureTracker::default();
        assert_eq!(tracker.record("patch", &outcome, r#"{"fieldPath":"style.color"}"#), RepeatedFailureAction::Continue);
        assert!(matches!(
            tracker.record("patch", &outcome, r#"{"fieldPath":"style.color"}"#),
            RepeatedFailureAction::SwitchStrategy { attempt: 2, .. }
        ));
        assert!(matches!(
            tracker.record("patch", &outcome, r#"{"fieldPath":"style.color"}"#),
            RepeatedFailureAction::Terminate { attempts: 3, .. }
        ));
        assert!(matches!(
            tracker.record("patch", &outcome, r#"{"fieldPath":"style.color"}"#),
            RepeatedFailureAction::Terminate { attempts: 4, .. }
        ));
    }

    #[test]
    fn reads_component_spec_path_from_a_prefixed_structured_error() {
        assert_eq!(
            failure_field_path(
                r#"agent.component_specs.component_types_limit_exceeded: {"fieldPath":"componentTypes","maxItems":10,"actualItems":11}"#,
                "resolve_component_specs",
                "{}",
            ),
            "componentTypes"
        );
    }

    #[test]
    fn strategy_switch_rewrites_the_tool_result_instead_of_retrying_the_hook_event() {
        let outcome = ToolOutcome::Error(
            ToolFailure::invalid_args("too many component types")
                .with_code("agent.component_specs.component_types_limit_exceeded")
                .with_retryable(false),
        );
        let directive = repeated_failure_directive(
            RepeatedFailureAction::SwitchStrategy { attempt: 2, field_path: "componentTypes".to_string() },
            "resolve_component_specs",
            &outcome,
        )
        .unwrap();

        assert!(matches!(directive.flow, RepeatedFailureFlow::RewriteResult(_)));
    }

    #[test]
    fn stringified_components_share_a_precise_failure_signature_and_correction() {
        let outcome = ToolOutcome::Error(ToolFailure::invalid_args("expected a sequence"));
        let args = r#"{"components":"[{\"id\":\"component-1\"}]"}"#;
        let mut tracker = ToolFailureTracker::default();

        assert_eq!(
            tracker.record_attempt("create_canvas_components", &outcome, "expected a sequence", args, 0),
            RepeatedFailureAction::Continue
        );
        let action = tracker.record_attempt("create_canvas_components", &outcome, "expected a sequence", args, 0);
        assert_eq!(action, RepeatedFailureAction::SwitchStrategy { attempt: 2, field_path: "components".to_string() });
        let directive = repeated_failure_directive(action, "create_canvas_components", &outcome).unwrap();
        let RepeatedFailureFlow::RewriteResult(prompt) = directive.flow else {
            panic!("expected a correction prompt");
        };
        assert!(prompt.contains(r#"{"components":[{...}]}"#));
        assert!(prompt.contains("禁止 JSON.stringify"));
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn text_changes_and_unrelated_success_do_not_reset_failure_budget() {
        let failure = ToolOutcome::Error(ToolFailure::invalid_args("invalid").with_code("invalid"));
        let mut tracker = ToolFailureTracker::default();
        assert_eq!(tracker.record("patch", &failure, "first message"), RepeatedFailureAction::Continue);
        assert_eq!(tracker.record("read_spec", &ToolOutcome::Success, "valid spec"), RepeatedFailureAction::Continue);
        assert!(matches!(tracker.record("patch", &failure, "second message"), RepeatedFailureAction::SwitchStrategy { .. }));
        let mut restored = ToolFailureTracker::default();
        restored.restore(&tracker.snapshot()).unwrap();
        assert!(matches!(restored.record("patch", &failure, "third message"), RepeatedFailureAction::Terminate { .. }));
        assert_eq!(failure_field_path("plain text", "patch", "{}"), "");
        assert_eq!(normalize_field_path("/components/2/style/color"), "components.*.style.color");
        assert_eq!(normalize_field_path("components[0].style.color"), "components.*.style.color");
    }
    #[test]
    fn denied_target_is_frozen_but_an_authorized_alternative_remains_possible() {
        let outcome = ToolOutcome::Error(ToolFailure::permission_denied("denied").with_code("out_of_scope"));
        let mut tracker = ToolFailureTracker::default();
        let args = r#"{"componentIds":["old"]}"#;
        assert_eq!(tracker.record_attempt("patch", &outcome, "denied", args, 1), RepeatedFailureAction::PermissionDenied);
        assert!(tracker.blocked_attempt("patch", args, 1).is_some());
        assert!(tracker.blocked_attempt("patch", r#"{"componentIds":["new"]}"#, 1).is_none());
        assert!(tracker.blocked_attempt("patch", args, 2).is_none());
        assert!(matches!(
            tracker.record_attempt("patch", &outcome, "changed", r#"{"componentIds":["other"]}"#, 1),
            RepeatedFailureAction::Terminate { .. }
        ));
    }
}
