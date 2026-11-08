/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 结构化入口快捷动作的宿主完成约束。

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::backend::ai_driven::agent_context::input::{AgentRunInput, AgentShortcutCommand};
use crate::backend::ai_driven::domain::operation::{CanvasOperationKind, CanvasOperationReceipt, CanvasOperationStatus};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RequiredCanvasEffect {
    CreateComponents,
    PatchComponents,
    DeleteComponents,
}

impl RequiredCanvasEffect {
    pub(crate) fn matches(self, kind: CanvasOperationKind) -> bool {
        matches!(
            (self, kind),
            (Self::CreateComponents, CanvasOperationKind::CreateComponents)
                | (Self::PatchComponents, CanvasOperationKind::PatchComponents)
                | (Self::DeleteComponents, CanvasOperationKind::DeleteComponents)
        )
    }

    pub(crate) fn tool_name(self) -> &'static str {
        match self {
            Self::CreateComponents => "create_canvas_components",
            Self::PatchComponents => "patch_canvas_components",
            Self::DeleteComponents => "delete_canvas_components",
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct CompletionRequirementsState {
    state: Arc<Mutex<RequirementsState>>,
}

#[derive(Default)]
struct RequirementsState {
    /// Only host-selected shortcut requirements are stored here. Model output
    /// and plan text can never add entries to this set.
    required_effects: Option<BTreeSet<RequiredCanvasEffect>>,
    host_effects: BTreeSet<RequiredCanvasEffect>,
    required_component_ids: Vec<String>,
    required_fields: Vec<String>,
    applied_component_fields: std::collections::BTreeMap<String, BTreeSet<String>>,
    applied_fields_by_operation: std::collections::BTreeMap<String, std::collections::BTreeMap<String, BTreeSet<String>>>,
    canvas_write_started: bool,
    workspace_write_started: bool,
    completion_attempt: u32,
    missing_requirement_attempt: u32,
    last_missing_signature: Option<String>,
    continuation_count: u32,
}

/// One coherent snapshot for the deterministic completion gate.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CompletionRequirementsSnapshot {
    pub required_canvas_effects: Option<Vec<RequiredCanvasEffect>>,
    pub required_component_ids: Vec<String>,
    pub required_fields: Vec<String>,
    pub applied_component_fields: std::collections::BTreeMap<String, Vec<String>>,
    pub canvas_write_started: bool,
    pub workspace_write_started: bool,
}

impl CompletionRequirementsState {
    pub(crate) fn for_input(input: &AgentRunInput) -> Self {
        let state = Self::default();
        let mut facts = state.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        facts.required_component_ids = input
            .selection
            .as_ref()
            .map(|selection| selection.target_ids.iter().cloned())
            .into_iter()
            .flatten()
            .chain(input.selected_components().iter().map(|component| component.id.clone()))
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if let Some(shortcut) = &input.shortcut {
            if matches!(shortcut.command, AgentShortcutCommand::Style | AgentShortcutCommand::Data) {
                facts.host_effects.insert(RequiredCanvasEffect::PatchComponents);
                facts.required_effects = Some(facts.host_effects.clone());
                facts.required_fields = match shortcut.command {
                    AgentShortcutCommand::Style => vec!["style".to_string()],
                    AgentShortcutCommand::Data => vec!["data".to_string()],
                };
            }
        }
        drop(facts);
        state
    }

    pub(crate) fn lock_after_canvas_write(&self) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .canvas_write_started = true;
    }

    pub(crate) fn mark_workspace_write_started(&self) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .workspace_write_started = true;
    }

    pub(crate) fn record_workspace_write(&self) {
        self.mark_workspace_write_started();
    }

    pub(crate) fn snapshot_facts(&self) -> CompletionRequirementsSnapshot {
        let state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        CompletionRequirementsSnapshot {
            required_canvas_effects: state.required_effects.as_ref().map(|effects| effects.iter().copied().collect()),
            required_component_ids: state.required_component_ids.clone(),
            required_fields: state.required_fields.clone(),
            applied_component_fields: state
                .applied_component_fields
                .iter()
                .map(|(component_id, fields)| (component_id.clone(), fields.iter().cloned().collect()))
                .collect(),
            canvas_write_started: state.canvas_write_started,
            workspace_write_started: state.workspace_write_started,
        }
    }

    pub(crate) fn snapshot(&self) -> Option<Vec<RequiredCanvasEffect>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .required_effects
            .as_ref()
            .map(|effects| effects.iter().copied().collect())
    }

    /// Record field facts from the structured patch request that produced an
    /// Applied receipt. The receipt remains authoritative for status and IDs;
    /// arguments only provide the explicit fields that were requested.
    pub(crate) fn record_applied_canvas_fields(&self, receipt: &CanvasOperationReceipt, args: &str) {
        if receipt.status != CanvasOperationStatus::Applied || receipt.operation_id.trim().is_empty() {
            return;
        }
        let receipt_component_ids = receipt
            .component_ids
            .iter()
            .cloned()
            .chain(receipt.component_bindings.keys().cloned())
            .chain(receipt.component_bindings.values().cloned())
            .collect::<BTreeSet<_>>();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(args) else {
            return;
        };
        let Some(patches) = value.get("patches").and_then(serde_json::Value::as_array) else {
            return;
        };
        let mut parsed_fields = std::collections::BTreeMap::<String, BTreeSet<String>>::new();
        for patch in patches {
            let Some(component_id) = patch
                .get("componentId")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            if !receipt_component_ids.contains(component_id) {
                continue;
            }
            let fields = parsed_fields.entry(component_id.to_string()).or_default();
            for (input_key, field) in [("stylePatch", "style"), ("dataPatch", "data"), ("layerPatch", "layer"), ("filterPatch", "filter")] {
                if patch.get(input_key).is_some_and(|value| !value.is_null()) {
                    fields.insert(field.to_string());
                }
            }
        }
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        {
            let operation_fields = state
                .applied_fields_by_operation
                .entry(receipt.operation_id.clone())
                .or_default();
            for (component_id, fields) in &parsed_fields {
                operation_fields
                    .entry(component_id.clone())
                    .or_default()
                    .extend(fields.iter().cloned());
            }
        }
        for (component_id, fields) in parsed_fields {
            state.applied_component_fields.entry(component_id).or_default().extend(fields);
        }
    }

    /// Rebuild the flattened field facts from operation-bound facts and the
    /// authoritative journal. Checkpoint data alone cannot make a field
    /// complete after a task or canvas identity changes.
    pub(crate) fn reconcile_applied_component_fields(&self, operations: &[CanvasOperationReceipt]) {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.applied_component_fields.clear();
        for operation in operations
            .iter()
            .filter(|operation| operation.status == CanvasOperationStatus::Applied)
        {
            // New patch receipts retain the validated request fields before dispatch,
            // so a timeout followed by status recovery does not depend on a hook
            // having seen the original successful tool result. Legacy receipts
            // can still use operation-bound checkpoint evidence.
            let operation_fields = if operation.component_fields.is_empty() {
                state
                    .applied_fields_by_operation
                    .get(&operation.operation_id)
                    .cloned()
                    .unwrap_or_default()
            } else if operation.kind == CanvasOperationKind::PatchComponents {
                operation
                    .component_fields
                    .iter()
                    .map(|(id, fields)| {
                        (
                            id.clone(),
                            fields
                                .iter()
                                .filter(|field| matches!(field.as_str(), "style" | "data" | "layer" | "filter"))
                                .cloned()
                                .collect::<BTreeSet<_>>(),
                        )
                    })
                    .collect()
            } else {
                continue;
            };
            let receipt_component_ids = operation
                .component_ids
                .iter()
                .cloned()
                .chain(operation.component_bindings.keys().cloned())
                .chain(operation.component_bindings.values().cloned())
                .collect::<BTreeSet<_>>();
            for (component_id, fields) in operation_fields {
                if receipt_component_ids.contains(&component_id) {
                    state
                        .applied_component_fields
                        .entry(component_id.clone())
                        .or_default()
                        .extend(fields.iter().cloned());
                }
            }
        }
    }

    /// Snapshot field facts with their source operation, for checkpoint
    /// recovery. The flattened view must be rebuilt from the operation log.
    pub(crate) fn applied_fields_by_operation_snapshot(&self) -> serde_json::Value {
        let state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        operation_fields_value(&state)
    }

    pub(crate) fn restore_applied_component_fields(&self, value: &serde_json::Value) -> Result<(), String> {
        let restored: std::collections::BTreeMap<String, std::collections::BTreeMap<String, Vec<String>>> =
            serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for (operation_id, components) in restored {
            let operation_fields = state.applied_fields_by_operation.entry(operation_id).or_default();
            for (component_id, fields) in components {
                let existing = operation_fields.entry(component_id).or_default();
                for field in fields {
                    existing.insert(field);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn next_completion_attempt(&self) -> u32 {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.completion_attempt = state.completion_attempt.saturating_add(1);
        state.completion_attempt
    }

    pub(crate) fn missing_requirement_attempt(&self, signature: Option<&str>) -> u32 {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(signature) = signature else {
            state.last_missing_signature = None;
            state.missing_requirement_attempt = 0;
            return 0;
        };
        if state.last_missing_signature.as_deref() == Some(signature) {
            state.missing_requirement_attempt = state.missing_requirement_attempt.saturating_add(1);
        } else {
            state.last_missing_signature = Some(signature.to_string());
            state.missing_requirement_attempt = 1;
        }
        state.missing_requirement_attempt
    }

    pub(crate) fn continuation_state(&self, needs_continuation: bool, max_continuations: u32) -> (u32, bool) {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if needs_continuation {
            state.continuation_count = state.continuation_count.saturating_add(1);
        }
        (state.continuation_count, needs_continuation && state.continuation_count > max_continuations)
    }

    pub(crate) fn completion_recovery_snapshot(&self) -> serde_json::Value {
        let state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        serde_json::json!({
            "appliedFieldsByOperation": operation_fields_value(&state),
            "attempt": state.completion_attempt,
            "missingRequirementAttempt": state.missing_requirement_attempt,
            "lastMissingSignature": state.last_missing_signature,
            "continuationCount": state.continuation_count,
        })
    }

    pub(crate) fn restore_completion_recovery(&self, value: &serde_json::Value) -> Result<(), String> {
        if let Some(applied_fields) = value.get("appliedFieldsByOperation") {
            self.restore_applied_component_fields(applied_fields)?;
        }
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(attempt) = value.get("attempt").and_then(serde_json::Value::as_u64) {
            state.completion_attempt = u32::try_from(attempt).map_err(|_| "completion attempt exceeds u32".to_string())?;
        }
        if let Some(attempt) = value.get("missingRequirementAttempt").and_then(serde_json::Value::as_u64) {
            state.missing_requirement_attempt = u32::try_from(attempt).map_err(|_| "missing requirement attempt exceeds u32".to_string())?;
        }
        if let Some(signature) = value.get("lastMissingSignature") {
            state.last_missing_signature = serde_json::from_value(signature.clone()).map_err(|error| error.to_string())?;
        }
        if let Some(count) = value.get("continuationCount").and_then(serde_json::Value::as_u64) {
            state.continuation_count = u32::try_from(count).map_err(|_| "completion continuation count exceeds u32".to_string())?;
        }
        Ok(())
    }
}

fn operation_fields_value(state: &RequirementsState) -> serde_json::Value {
    serde_json::to_value(
        state
            .applied_fields_by_operation
            .iter()
            .map(|(operation_id, components)| {
                (
                    operation_id,
                    components
                        .iter()
                        .map(|(component_id, fields)| (component_id, fields.iter().cloned().collect::<Vec<_>>()))
                        .collect::<std::collections::BTreeMap<_, _>>(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>(),
    )
    .unwrap_or_else(|_| serde_json::json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::agent_context::input::AgentShortcutContext;

    #[test]
    fn shortcut_keeps_host_effect_and_structured_targets() {
        let mut input = AgentRunInput::new("优化选中组件");
        input.shortcut = Some(AgentShortcutContext { command: AgentShortcutCommand::Style, raw_text: String::new() });
        input.selection =
            Some(crate::backend::ai_driven::agent_context::input::AgentSelectionContext { target_ids: vec!["c1".to_string()], ..Default::default() });
        let state = CompletionRequirementsState::for_input(&input);
        let facts = state.snapshot_facts();

        assert_eq!(facts.required_canvas_effects, Some(vec![RequiredCanvasEffect::PatchComponents]));
        assert_eq!(facts.required_component_ids, vec!["c1"]);
        assert_eq!(facts.required_fields, vec!["style"]);
    }

    #[test]
    fn applied_field_facts_round_trip_without_duplicates() {
        let state = CompletionRequirementsState::default();
        let mut receipt = CanvasOperationReceipt::prepared(
            "op-1".to_string(),
            "call-1".to_string(),
            CanvasOperationKind::PatchComponents,
            "1".to_string(),
            None,
            vec!["c1".to_string()],
        );
        receipt.status = CanvasOperationStatus::Applied;
        state.record_applied_canvas_fields(&receipt, r#"{"patches":[{"componentId":"c1","stylePatch":{"color":"red"},"dataPatch":{"value":1}}]}"#);
        let saved = state.applied_fields_by_operation_snapshot();

        let restored = CompletionRequirementsState::default();
        restored.restore_applied_component_fields(&saved).unwrap();
        restored.restore_applied_component_fields(&saved).unwrap();
        restored.reconcile_applied_component_fields(&[receipt]);

        assert_eq!(
            restored.snapshot_facts().applied_component_fields.get("c1"),
            Some(&vec!["data".to_string(), "style".to_string()])
        );
    }

    #[test]
    fn completion_progress_round_trips_and_counts_only_same_signature() {
        let state = CompletionRequirementsState::default();
        assert_eq!(state.next_completion_attempt(), 1);
        assert_eq!(state.missing_requirement_attempt(Some("same")), 1);
        assert_eq!(state.missing_requirement_attempt(Some("same")), 2);
        assert_eq!(state.continuation_state(true, 2), (1, false));
        let saved = state.completion_recovery_snapshot();

        let restored = CompletionRequirementsState::default();
        restored.restore_completion_recovery(&saved).unwrap();
        assert_eq!(restored.next_completion_attempt(), 2);
        assert_eq!(restored.missing_requirement_attempt(Some("same")), 3);
        assert_eq!(restored.missing_requirement_attempt(Some("changed")), 1);
        assert_eq!(restored.continuation_state(true, 2), (2, false));
    }
}
