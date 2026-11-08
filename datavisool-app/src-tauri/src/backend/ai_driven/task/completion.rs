/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 模型结束决策后的确定性动作事实检查。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::Serialize;
use serde_json::json;

use crate::backend::ai_driven::agent::{AgentCompletionDecision, AgentCompletionGate, AgentRunFailure, AgentRuntimeHook};
use crate::backend::ai_driven::domain::operation::{CanvasOperationReceipt, CanvasOperationStatus};

use super::consistency::append_event_record;
use super::{
    AgentTaskEventPublisher, AgentTaskRepository, AgentTaskRun, CanvasOperationJournal, CompletionRequirementsSnapshot, CompletionRequirementsState,
    RequiredCanvasEffect,
};

const MAX_MISSING_REQUIREMENT_ATTEMPTS: u32 = 2;
const DEFAULT_MAX_COMPLETION_CONTINUATIONS: u32 = 12;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompletionFactCheck {
    pub status: CompletionFactCheckStatus,
    pub operation_ids: Vec<String>,
    pub unsettled_operation_ids: Vec<String>,
    pub unresolved_failure_ids: Vec<String>,
    pub requirements_declared: bool,
    pub canvas_write_started: bool,
    pub workspace_write_started: bool,
    pub workspace_write_permitted: bool,
    pub required_canvas_effects: Vec<RequiredCanvasEffect>,
    pub applied_canvas_effects: Vec<RequiredCanvasEffect>,
    pub missing_required_canvas_effects: Vec<RequiredCanvasEffect>,
    pub required_component_ids: Vec<String>,
    pub missing_required_component_ids: Vec<String>,
    pub required_fields: Vec<String>,
    pub missing_required_fields: Vec<String>,
    pub configured_component_count: usize,
    pub default_config_component_count: usize,
}

impl CompletionFactCheck {
    pub(crate) const fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CompletionFactCheckStatus {
    Passed,
    Waiting,
    Failed,
}

impl CompletionFactCheckStatus {
    pub(crate) const fn is_terminal(self) -> bool {
        !matches!(self, Self::Waiting)
    }
}

pub(crate) struct TaskCompletionGate {
    journal: CanvasOperationJournal,
    completion_requirements: CompletionRequirementsState,
    repository: Arc<dyn AgentTaskRepository>,
    publisher: AgentTaskEventPublisher,
    task: AgentTaskRun,
    max_continuations: u32,
    runtime_hook: AgentRuntimeHook,
}

impl TaskCompletionGate {
    pub(crate) fn new(
        journal: CanvasOperationJournal,
        completion_requirements: CompletionRequirementsState,
        repository: Arc<dyn AgentTaskRepository>,
        publisher: AgentTaskEventPublisher,
        task: AgentTaskRun,
        runtime_hook: AgentRuntimeHook,
    ) -> Self {
        let max_continuations = if task.limits.max_completion_continuations == 0 {
            DEFAULT_MAX_COMPLETION_CONTINUATIONS
        } else {
            task.limits.max_completion_continuations
        };
        Self { journal, completion_requirements, repository, publisher, task, max_continuations, runtime_hook }
    }

    async fn evaluate_candidate(&self) -> Result<AgentCompletionDecision, AgentRunFailure> {
        let operations = self
            .journal
            .list()
            .await
            .map_err(|error| AgentRunFailure { code: "agent.completion.operation_read_failed".to_string(), message: error.to_string() })?;
        if operations
            .iter()
            .any(|operation| operation.status == CanvasOperationStatus::Applied)
        {
            self.completion_requirements.lock_after_canvas_write();
        }
        self.completion_requirements.reconcile_applied_component_fields(&operations);
        let requirements = self.completion_requirements.snapshot_facts();
        let check = evaluate_completion_facts(&requirements, &operations);
        let attempt = self.completion_requirements.next_completion_attempt();
        let missing_requirement_attempt = if check.unsettled_operation_ids.is_empty() && check.has_missing_completion_requirement() {
            let signature = completion_requirement_signature(&requirements, &check);
            self.completion_requirements.missing_requirement_attempt(Some(&signature))
        } else {
            self.completion_requirements.missing_requirement_attempt(None)
        };
        let needs_continuation = !check.status.is_terminal();
        let (continuation_count, continuation_limit_reached) = self
            .completion_requirements
            .continuation_state(needs_continuation, self.max_continuations);
        self.runtime_hook
            .persist_run_recovery()
            .await
            .map_err(|error| AgentRunFailure { code: "agent.completion.checkpoint_failed".into(), message: error.to_string() })?;
        let event = append_event_record(
            self.repository.as_ref(),
            &self.task,
            None,
            "agent.completion.fact_check",
            json!({
                "attempt": attempt,
                "missingRequirementAttempt": missing_requirement_attempt,
                "continuationCount": continuation_count,
                "maxContinuations": self.max_continuations,
                "check": check
            }),
            format!("run:{}:completion:fact-check:{attempt}", self.task.run_id),
        )
        .await
        .map_err(|message| AgentRunFailure { code: "agent.completion.fact_check_persistence_failed".to_string(), message })?;
        self.publisher.publish(event);

        if continuation_limit_reached {
            return Ok(AgentCompletionDecision::Fail {
                code: "agent_execution.completion_continuation_limit".to_string(),
                message: format!("完成门禁续跑超过 {} 次，任务已在保留既有动作事实后终止", self.max_continuations),
            });
        }

        match check.status {
            CompletionFactCheckStatus::Passed => Ok(AgentCompletionDecision::Complete),
            CompletionFactCheckStatus::Waiting if !check.unsettled_operation_ids.is_empty() => Ok(AgentCompletionDecision::Continue {
                prompt: format!(
                    "仍有画布动作尚未得到终态回执。请仅查询原 operationId，等待明确结果后再决定是否结束：{}",
                    check.unsettled_operation_ids.join(", ")
                ),
            }),
            CompletionFactCheckStatus::Waiting if missing_requirement_attempt >= MAX_MISSING_REQUIREMENT_ATTEMPTS => {
                Ok(AgentCompletionDecision::Fail {
                    code: "agent.completion.required_effect_missing".to_string(),
                    message: missing_requirement_message(&check),
                })
            }
            CompletionFactCheckStatus::Waiting => Ok(AgentCompletionDecision::Continue { prompt: missing_requirement_prompt(&check) }),
            CompletionFactCheckStatus::Failed => Ok(AgentCompletionDecision::Fail {
                code: "agent.completion.operation_failed".to_string(),
                message: format!("存在未解决的执行失败，已保留成功回执：{}", check.unresolved_failure_ids.join(", ")),
            }),
        }
    }
}

impl AgentCompletionGate for TaskCompletionGate {
    fn evaluate<'a>(&'a self, _final_text: &'a str) -> crate::backend::ai_driven::agent::CompletionGateFuture<'a> {
        Box::pin(async move { self.evaluate_candidate().await })
    }
}

pub(crate) fn evaluate_operation_facts(
    required_effects: Option<&[RequiredCanvasEffect]>,
    operations: &[CanvasOperationReceipt],
) -> CompletionFactCheck {
    let requirements = CompletionRequirementsSnapshot {
        required_canvas_effects: required_effects.map(<[RequiredCanvasEffect]>::to_vec),
        required_component_ids: Vec::new(),
        required_fields: Vec::new(),
        applied_component_fields: BTreeMap::new(),
        canvas_write_started: !operations.is_empty(),
        workspace_write_started: false,
    };
    evaluate_completion_facts(&requirements, operations)
}

pub(crate) fn evaluate_completion_facts(requirements: &CompletionRequirementsSnapshot, operations: &[CanvasOperationReceipt]) -> CompletionFactCheck {
    let unsettled_operation_ids = operations
        .iter()
        .filter(|operation| !operation.status.is_terminal())
        .map(|operation| operation.operation_id.clone())
        .collect::<Vec<_>>();
    let unresolved_failure_ids = unresolved_operation_failures(operations);
    // A journal entry is a fact that a canvas write was attempted, including
    // Prepared/Applying/Failed entries.  The state flag also covers a restored
    // run whose journal has not yet been reloaded by a caller.
    let canvas_write_started = requirements.canvas_write_started || !operations.is_empty();
    let workspace_write_started = requirements.workspace_write_started;
    let required_canvas_effects = requirements.required_canvas_effects.clone().unwrap_or_default();
    let applied_canvas_effects = required_canvas_effects
        .iter()
        .copied()
        .filter(|effect| {
            operations
                .iter()
                .any(|operation| operation_satisfies_effect(operation, *effect))
        })
        .collect::<Vec<_>>();
    let missing_required_canvas_effects = required_canvas_effects
        .iter()
        .copied()
        .filter(|effect| !applied_canvas_effects.contains(effect))
        .collect::<Vec<_>>();
    let requirements_declared = !required_canvas_effects.is_empty();
    let missing_required_effects = !missing_required_canvas_effects.is_empty();
    let missing_required_targets = required_target_ids_missing(requirements, operations);
    let missing_required_fields = required_fields_missing(requirements);
    let requirements_satisfied = missing_required_effects == false && missing_required_targets.is_empty() && missing_required_fields.is_empty();
    // Optional failed/rejected adjustments remain visible in the fact check,
    // but do not force a retry once host-required facts are complete.
    let blocking_failure = !requirements_satisfied && !required_canvas_effects.is_empty() && !unresolved_failure_ids.is_empty();
    let status = if !unsettled_operation_ids.is_empty() {
        CompletionFactCheckStatus::Waiting
    } else if blocking_failure {
        CompletionFactCheckStatus::Failed
    } else if missing_required_effects || !missing_required_targets.is_empty() || !missing_required_fields.is_empty() {
        CompletionFactCheckStatus::Waiting
    } else {
        CompletionFactCheckStatus::Passed
    };
    CompletionFactCheck {
        status,
        operation_ids: operations.iter().map(|operation| operation.operation_id.clone()).collect(),
        unsettled_operation_ids,
        unresolved_failure_ids,
        requirements_declared,
        canvas_write_started,
        workspace_write_started,
        // Workspace authorization is checked by the write Tool against the
        // bound security grant. Completion never infers it from model facts.
        workspace_write_permitted: true,
        required_canvas_effects,
        applied_canvas_effects,
        missing_required_canvas_effects,
        required_component_ids: requirements.required_component_ids.clone(),
        missing_required_component_ids: missing_required_targets,
        required_fields: requirements.required_fields.clone(),
        missing_required_fields,
        configured_component_count: operations.iter().map(|operation| operation.configured_component_count).sum(),
        default_config_component_count: operations
            .iter()
            .map(|operation| operation.default_config_component_count)
            .sum(),
    }
}

impl CompletionFactCheck {
    fn has_missing_completion_requirement(&self) -> bool {
        !self.missing_required_canvas_effects.is_empty()
            || !self.missing_required_component_ids.is_empty()
            || !self.missing_required_fields.is_empty()
    }
}

fn operation_satisfies_effect(operation: &CanvasOperationReceipt, effect: RequiredCanvasEffect) -> bool {
    if operation.status != CanvasOperationStatus::Applied || !effect.matches(operation.kind) {
        return false;
    }
    if effect != RequiredCanvasEffect::CreateComponents {
        return true;
    }
    let created_count = operation.component_ids.len().max(operation.component_bindings.len());
    created_count > 0
        && operation
            .configured_component_count
            .saturating_add(operation.default_config_component_count)
            >= created_count
}

fn missing_requirement_prompt(check: &CompletionFactCheck) -> String {
    if !check.missing_required_component_ids.is_empty() {
        return format!(
            "快捷入口要求对指定目标完成结构化修改，请使用 patch_canvas_components 覆盖这些目标后再结束：{}。",
            check.missing_required_component_ids.join(", ")
        );
    }
    if !check.missing_required_fields.is_empty() {
        return format!(
            "快捷入口要求修改指定字段，请使用 patch_canvas_components 完成这些字段后再结束：{}。",
            check.missing_required_fields.join(", ")
        );
    }
    format!(
        "任务完成门禁尚未满足。请调用对应动作工具并取得 Applied 回执后再结束：{}。",
        required_effect_tool_names(&check.missing_required_canvas_effects)
    )
}

fn missing_requirement_message(check: &CompletionFactCheck) -> String {
    if !check.missing_required_component_ids.is_empty() {
        return format!("连续结束候选均未完成快捷入口指定目标：{}", check.missing_required_component_ids.join(", "));
    }
    if !check.missing_required_fields.is_empty() {
        return format!("连续结束候选均未完成快捷入口指定字段：{}", check.missing_required_fields.join(", "));
    }
    format!(
        "连续结束候选均缺少必需的 Applied 画布动作：{}",
        required_effect_tool_names(&check.missing_required_canvas_effects)
    )
}

fn required_target_ids_missing(requirements: &CompletionRequirementsSnapshot, operations: &[CanvasOperationReceipt]) -> Vec<String> {
    if requirements.required_component_ids.is_empty() || requirements.required_canvas_effects.is_none() {
        return Vec::new();
    }
    let applied_targets = operations
        .iter()
        .filter(|operation| {
            operation.status == CanvasOperationStatus::Applied
                && operation.kind == crate::backend::ai_driven::domain::operation::CanvasOperationKind::PatchComponents
        })
        .flat_map(operation_identity_ids)
        .collect::<BTreeSet<_>>();
    requirements
        .required_component_ids
        .iter()
        .filter(|target| !applied_targets.contains(*target))
        .cloned()
        .collect()
}

fn required_fields_missing(requirements: &CompletionRequirementsSnapshot) -> Vec<String> {
    if requirements.required_component_ids.is_empty() || requirements.required_fields.is_empty() || requirements.required_canvas_effects.is_none() {
        return Vec::new();
    }
    requirements
        .required_component_ids
        .iter()
        .flat_map(|component_id| {
            requirements.required_fields.iter().filter_map(move |field| {
                let applied = requirements.applied_component_fields.get(component_id);
                (!applied.is_some_and(|fields| fields.iter().any(|value| value == field))).then(|| format!("{component_id}:{field}"))
            })
        })
        .collect()
}

fn completion_requirement_signature(requirements: &CompletionRequirementsSnapshot, check: &CompletionFactCheck) -> String {
    // 仅影响当前缺失约束的事实才构成进展；无关成功、重复状态查询不能重置预算。
    format!(
        "missing={:?};targets={:?};fields={:?};requiredTargets={:?};requiredFields={:?}",
        check.missing_required_canvas_effects,
        check.missing_required_component_ids,
        check.missing_required_fields,
        requirements.required_component_ids,
        requirements.required_fields,
    )
}

fn required_effect_tool_names(effects: &[RequiredCanvasEffect]) -> String {
    effects.iter().map(|effect| effect.tool_name()).collect::<Vec<_>>().join(", ")
}

fn unresolved_operation_failures(operations: &[CanvasOperationReceipt]) -> Vec<String> {
    operations
        .iter()
        .enumerate()
        .filter(|(_, operation)| matches!(operation.status, CanvasOperationStatus::Failed | CanvasOperationStatus::Rejected))
        .filter(|(failed_index, failed)| {
            let failed_targets = operation_identity_ids(failed);
            if failed_targets.is_empty() {
                return true;
            }
            let later_applied = operations
                .iter()
                .skip(*failed_index + 1)
                .filter(|candidate| candidate.kind == failed.kind && candidate.status == CanvasOperationStatus::Applied)
                .flat_map(operation_identity_ids)
                .collect::<BTreeSet<_>>();
            !failed_targets.is_subset(&later_applied)
        })
        .map(|(_, operation)| operation.operation_id.clone())
        .collect()
}

fn operation_identity_ids(operation: &CanvasOperationReceipt) -> BTreeSet<String> {
    operation
        .component_ids
        .iter()
        .cloned()
        .chain(operation.component_bindings.keys().cloned())
        .chain(operation.component_bindings.values().cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::backend::ai_driven::domain::operation::{CanvasOperationKind, CanvasOperationReceipt};

    fn operation_with_kind(id: &str, kind: CanvasOperationKind, status: CanvasOperationStatus, component_ids: &[&str]) -> CanvasOperationReceipt {
        let configured_component_count = if kind == CanvasOperationKind::CreateComponents {
            component_ids.len()
        } else {
            0
        };
        CanvasOperationReceipt {
            operation_id: id.to_string(),
            tool_call_id: format!("call-{id}"),
            kind,
            status,
            project_id: "1".to_string(),
            manager_id: None,
            component_ids: component_ids.iter().map(|id| (*id).to_string()).collect(),
            component_bindings: BTreeMap::new(),
            component_fields: BTreeMap::new(),
            configured_component_count,
            default_config_component_count: 0,
            base_revision: None,
            result_revision: None,
            applied_at: (status == CanvasOperationStatus::Applied).then_some(1),
            error: None,
        }
    }

    fn operation(id: &str, status: CanvasOperationStatus, component_ids: &[&str]) -> CanvasOperationReceipt {
        operation_with_kind(id, CanvasOperationKind::PatchComponents, status, component_ids)
    }

    #[test]
    fn no_write_facts_allow_completion_without_a_scope() {
        let check = evaluate_operation_facts(None, &[]);

        assert_eq!(check.status, CompletionFactCheckStatus::Passed);
        assert!(!check.requirements_declared);
    }

    #[test]
    fn ordinary_question_passes_after_empty_requirements_are_declared() {
        let check = evaluate_operation_facts(Some(&[]), &[]);

        assert_eq!(check.status, CompletionFactCheckStatus::Passed);
    }

    #[test]
    fn declared_create_scope_cannot_complete_without_an_operation() {
        let check = evaluate_completion_facts(
            &CompletionRequirementsSnapshot {
                required_canvas_effects: Some(vec![RequiredCanvasEffect::CreateComponents]),
                ..CompletionRequirementsSnapshot::default()
            },
            &[],
        );

        assert_eq!(check.status, CompletionFactCheckStatus::Waiting);
        assert_eq!(check.missing_required_canvas_effects, vec![RequiredCanvasEffect::CreateComponents]);
        assert!(check.has_missing_completion_requirement());
    }

    #[test]
    fn applied_canvas_action_completes_without_a_model_declaration() {
        let check = evaluate_operation_facts(Some(&[]), &[operation("applied", CanvasOperationStatus::Applied, &["c1"])]);

        assert_eq!(check.status, CompletionFactCheckStatus::Passed);
        assert!(check.canvas_write_started);
    }

    #[test]
    fn workspace_write_completion_uses_host_permission() {
        let check = evaluate_completion_facts(
            &CompletionRequirementsSnapshot { workspace_write_started: true, ..CompletionRequirementsSnapshot::default() },
            &[],
        );

        assert_eq!(check.status, CompletionFactCheckStatus::Passed);
        assert!(check.workspace_write_started);
        assert!(check.workspace_write_permitted);
    }

    #[test]
    fn permitted_workspace_write_can_complete_without_canvas_effects() {
        let check = evaluate_completion_facts(
            &CompletionRequirementsSnapshot {
                workspace_write_started: true,
                required_canvas_effects: Some(Vec::new()),
                ..CompletionRequirementsSnapshot::default()
            },
            &[],
        );

        assert_eq!(check.status, CompletionFactCheckStatus::Passed);
        assert!(check.workspace_write_permitted);
    }

    #[test]
    fn required_create_waits_without_an_applied_create_operation() {
        let patch = operation("patch", CanvasOperationStatus::Applied, &["c1"]);
        let check = evaluate_operation_facts(Some(&[RequiredCanvasEffect::CreateComponents]), &[patch]);

        assert_eq!(check.status, CompletionFactCheckStatus::Waiting);
        assert_eq!(check.missing_required_canvas_effects, vec![RequiredCanvasEffect::CreateComponents]);
    }

    #[test]
    fn required_create_passes_with_an_applied_create_operation() {
        let create = operation_with_kind("create", CanvasOperationKind::CreateComponents, CanvasOperationStatus::Applied, &["c1"]);
        let check = evaluate_operation_facts(Some(&[RequiredCanvasEffect::CreateComponents]), &[create]);

        assert_eq!(check.status, CompletionFactCheckStatus::Passed);
        assert_eq!(check.applied_canvas_effects, vec![RequiredCanvasEffect::CreateComponents]);
    }

    #[test]
    fn applied_empty_shell_create_does_not_satisfy_completion() {
        let mut create = operation_with_kind("create", CanvasOperationKind::CreateComponents, CanvasOperationStatus::Applied, &["c1"]);
        create.configured_component_count = 0;
        create.default_config_component_count = 0;

        let check = evaluate_operation_facts(Some(&[RequiredCanvasEffect::CreateComponents]), &[create]);

        assert_eq!(check.status, CompletionFactCheckStatus::Waiting);
        assert_eq!(check.missing_required_canvas_effects, vec![RequiredCanvasEffect::CreateComponents]);
    }

    #[test]
    fn required_patch_passes_with_an_applied_patch_operation() {
        let patch = operation("patch", CanvasOperationStatus::Applied, &["c1"]);
        let check = evaluate_operation_facts(Some(&[RequiredCanvasEffect::PatchComponents]), &[patch]);

        assert_eq!(check.status, CompletionFactCheckStatus::Passed);
    }

    #[test]
    fn shortcut_patch_requires_the_entry_targets_and_fields() {
        let patch = operation("patch", CanvasOperationStatus::Applied, &["c1"]);
        let requirements = CompletionRequirementsSnapshot {
            required_canvas_effects: Some(vec![RequiredCanvasEffect::PatchComponents]),
            required_component_ids: vec!["c1".to_string()],
            required_fields: vec!["style".to_string()],
            applied_component_fields: BTreeMap::from([("c1".to_string(), vec!["style".to_string()])]),
            ..CompletionRequirementsSnapshot::default()
        };
        assert_eq!(evaluate_completion_facts(&requirements, &[patch]).status, CompletionFactCheckStatus::Passed);

        let mut wrong_field = requirements.clone();
        wrong_field.applied_component_fields = BTreeMap::from([("c1".to_string(), vec!["layer".to_string()])]);
        let check = evaluate_completion_facts(&wrong_field, &[operation("patch-2", CanvasOperationStatus::Applied, &["c1"])]);
        assert_eq!(check.status, CompletionFactCheckStatus::Waiting);
        assert_eq!(check.missing_required_fields, vec!["c1:style"]);
    }

    #[test]
    fn unsettled_action_waits_for_the_original_operation() {
        let check = evaluate_operation_facts(Some(&[]), &[operation("applying", CanvasOperationStatus::Applying, &["c1"])]);

        assert_eq!(check.status, CompletionFactCheckStatus::Waiting);
        assert_eq!(check.unsettled_operation_ids, vec!["applying"]);
    }

    #[test]
    fn failed_required_action_fails_the_run() {
        let check =
            evaluate_operation_facts(Some(&[RequiredCanvasEffect::PatchComponents]), &[operation("failed", CanvasOperationStatus::Failed, &["c1"])]);

        assert_eq!(check.status, CompletionFactCheckStatus::Failed);
        assert_eq!(check.unresolved_failure_ids, vec!["failed"]);
    }

    #[test]
    fn optional_rejected_action_is_recorded_without_forcing_a_retry() {
        let check = evaluate_operation_facts(Some(&[]), &[operation("rejected", CanvasOperationStatus::Rejected, &["c1"])]);

        assert_eq!(check.status, CompletionFactCheckStatus::Passed);
        assert_eq!(check.unresolved_failure_ids, vec!["rejected"]);
    }

    #[test]
    fn later_applied_action_can_cover_an_earlier_failure() {
        let check = evaluate_operation_facts(
            Some(&[RequiredCanvasEffect::PatchComponents]),
            &[
                operation("failed", CanvasOperationStatus::Failed, &["c1"]),
                operation("applied", CanvasOperationStatus::Applied, &["c1"]),
            ],
        );

        assert_eq!(check.status, CompletionFactCheckStatus::Passed);
    }

    #[test]
    fn applying_retry_takes_precedence_over_an_earlier_failure() {
        let check = evaluate_operation_facts(
            Some(&[]),
            &[
                operation("failed", CanvasOperationStatus::Failed, &["c1"]),
                operation("retry", CanvasOperationStatus::Applying, &["c1"]),
            ],
        );

        assert_eq!(check.status, CompletionFactCheckStatus::Waiting);
    }

    #[test]
    fn unrelated_applied_operation_does_not_reset_missing_field_budget() {
        let requirements = CompletionRequirementsSnapshot {
            required_canvas_effects: Some(vec![RequiredCanvasEffect::PatchComponents]),
            required_component_ids: vec!["required".into()],
            required_fields: vec!["style".into()],
            ..Default::default()
        };
        let first = vec![operation("op-1", CanvasOperationStatus::Applied, &["required"])];
        let first_check = evaluate_completion_facts(&requirements, &first);
        let first_signature = completion_requirement_signature(&requirements, &first_check);
        let mut next = first;
        next.push(operation("op-unrelated", CanvasOperationStatus::Applied, &["other"]));
        let next_check = evaluate_completion_facts(&requirements, &next);
        let next_signature = completion_requirement_signature(&requirements, &next_check);
        assert_eq!(first_signature, next_signature);
        let state = CompletionRequirementsState::default();
        assert_eq!(state.missing_requirement_attempt(Some(&first_signature)), 1);
        assert_eq!(state.missing_requirement_attempt(Some(&next_signature)), 2);
    }

    #[test]
    fn completion_continuation_has_an_independent_hard_limit() {
        let counter = CompletionRequirementsState::default();
        for expected in 1..=12 {
            assert_eq!(counter.continuation_state(true, 12), (expected, false));
        }
        assert_eq!(counter.continuation_state(true, 12), (13, true));
    }

    #[test]
    fn passed_and_failed_fact_checks_are_terminal() {
        assert!(CompletionFactCheckStatus::Passed.is_terminal());
        assert!(CompletionFactCheckStatus::Failed.is_terminal());
        assert!(!CompletionFactCheckStatus::Waiting.is_terminal());
        let check = evaluate_operation_facts(Some(&[]), &[]);
        assert!(check.is_terminal());
    }
}
