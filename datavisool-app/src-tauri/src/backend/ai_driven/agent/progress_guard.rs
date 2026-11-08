/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 任务级重复读取缓存和业务无进展熔断。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::backend::ai_driven::tools::component_catalog::GET_COMPONENT_CATALOG_TOOL;
use crate::backend::ai_driven::tools::component_resolution::RESOLVE_COMPONENT_SPECS_TOOL;
use crate::backend::ai_driven::tools::interaction::INSPECT_CURRENT_PAGE_TOOL;
use crate::backend::ai_driven::tools::task_intent::UPDATE_PLAN_TOOL;
use crate::backend::utils::sha256_hex;

use super::limits::{AgentExecutionErrorCode, AgentExecutionLimits, ToolCallKind};
use super::tool_route::AgentToolRouteSnapshot;

const GET_CANVAS_OPERATION_STATUS_TOOL: &str = "get_canvas_operation_status";
const MAX_EMPTY_COMPONENT_RESOLUTIONS: u64 = 3;
const MAX_IDENTICAL_PLAN_REPEATS: u64 = 3;

#[derive(Clone, Debug)]
pub(crate) struct ProgressGuard {
    max_same_read_repeats: u64,
    state: Arc<Mutex<ProgressState>>,
}

#[derive(Debug, Default)]
struct ProgressState {
    entries: HashMap<String, ReadEntry>,
    pending: HashMap<String, PendingCall>,
    last_read_key: Option<String>,
    same_read_repeats: u64,
    reads_without_business_progress: u64,
    progress_sequence: u64,
    last_progress: Option<String>,
    last_plan_key: Option<String>,
    identical_plan_repeats: u64,
    facts: progress_facts::ProgressFacts,
    consecutive_empty_component_resolutions: u64,
    component_catalog_call_in_flight: bool,
    applied_write_facts: HashSet<String>,
}

#[derive(Clone, Debug, Default)]
struct ReadEntry {
    actual_executions: u64,
    source_version: Option<String>,
    cached_result: Option<String>,
}

#[derive(Clone, Debug)]
struct PendingCall {
    key: Option<String>,
    arguments: String,
    fingerprint: String,
    repeat_count: u64,
    cache_hit: bool,
    plan_progress: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum ProgressGuardAction {
    Execute(ProgressCallMetadata),
    Cached { metadata: ProgressCallMetadata, result: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProgressCallMetadata {
    pub fingerprint: String,
    pub repeat_count: u64,
    pub actual_execution_count: u64,
    pub source_version: Option<String>,
    pub cache_hit: bool,
    pub produced_new_fact: bool,
    pub business_write: bool,
    pub reads_without_business_progress: u64,
    pub last_progress_sequence: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProgressViolation {
    pub code: AgentExecutionErrorCode,
    pub message: String,
    pub tool_name: String,
    pub fingerprint: String,
    pub repeat_count: u64,
    pub reads_without_business_progress: u64,
    pub last_progress_sequence: u64,
    pub last_progress: Option<String>,
}

impl ProgressGuard {
    pub(crate) fn new(limits: &AgentExecutionLimits) -> Self {
        Self {
            max_same_read_repeats: limits.max_same_read_repeats,
            state: Arc::new(Mutex::new(ProgressState::default())),
        }
    }

    /// 恢复同一 Task 先前成功读取的组件目录，避免重启后再次请求前端。
    pub(crate) fn restore_component_catalog(&self, result: String) {
        let Some(source_version) = extract_source_version(&result) else {
            return;
        };
        let Some(value) = serde_json::from_str::<Value>(&result).ok() else {
            return;
        };
        if value.get("components").and_then(Value::as_array).is_none() {
            return;
        }

        let mut state = self.lock_state();
        let key = canonical_call_key(GET_COMPONENT_CATALOG_TOOL, "{}");
        state.facts.record(GET_COMPONENT_CATALOG_TOOL, &result);
        state
            .entries
            .insert(key, ReadEntry { actual_executions: 1, source_version: Some(source_version), cached_result: Some(result) });
    }

    // TODO(legacy-compat): 迁移固定 Tool 测试及内部调用后，移除旧调用入口。
    pub(crate) fn before_call(
        &self,
        internal_call_id: &str,
        tool_name: &str,
        args: &str,
        kind: ToolCallKind,
        canvas_write_task: bool,
    ) -> Result<ProgressGuardAction, ProgressViolation> {
        self.before_call_in_phase(internal_call_id, tool_name, args, kind, canvas_write_task)
    }

    fn before_call_in_phase(
        &self,
        internal_call_id: &str,
        tool_name: &str,
        args: &str,
        kind: ToolCallKind,
        _canvas_write_task: bool,
    ) -> Result<ProgressGuardAction, ProgressViolation> {
        let mut state = self.lock_state();
        if tool_name == UPDATE_PLAN_TOOL {
            let plan_key = canonical_call_key(tool_name, args);
            let plan_progress = state.last_plan_key.as_deref() != Some(&plan_key);
            if !plan_progress {
                state.identical_plan_repeats = state.identical_plan_repeats.saturating_add(1);
                if state.identical_plan_repeats >= MAX_IDENTICAL_PLAN_REPEATS {
                    return Err(violation(
                        &state,
                        AgentExecutionErrorCode::NoProgressLimit,
                        "画布写任务反复提交未推进的计划",
                        tool_name,
                        fingerprint(tool_name, args, None),
                        state.identical_plan_repeats,
                    ));
                }
                state.reads_without_business_progress = state.reads_without_business_progress.saturating_add(1);
            } else {
                state.identical_plan_repeats = 1;
            }
            state.last_plan_key = Some(plan_key);
            let plan_repeat_count = state.identical_plan_repeats;
            let metadata = pending_metadata(&state, fingerprint(tool_name, args, None), plan_repeat_count, false, false, false, None, 0);
            state.pending.insert(
                internal_call_id.to_string(),
                PendingCall {
                    key: None,
                    arguments: args.to_string(),
                    fingerprint: metadata.fingerprint.clone(),
                    repeat_count: plan_repeat_count,
                    cache_hit: false,
                    plan_progress,
                },
            );
            return Ok(ProgressGuardAction::Execute(metadata));
        }
        if tool_name == RESOLVE_COMPONENT_SPECS_TOOL && state.consecutive_empty_component_resolutions >= MAX_EMPTY_COMPONENT_RESOLUTIONS {
            return Err(violation(
                &state,
                AgentExecutionErrorCode::NoProgressLimit,
                "已连续三次未返回完整组件规格；请核对明确的 componentType，并按完整组件拆批；类型不明确时可调用 get_component_catalog",
                tool_name,
                fingerprint(tool_name, args, None),
                state.consecutive_empty_component_resolutions.saturating_add(1),
            ));
        }
        if is_business_write_tool(kind) {
            let metadata = pending_metadata(&state, fingerprint(tool_name, args, None), 1, false, false, true, None, 0);
            state.pending.insert(
                internal_call_id.to_string(),
                PendingCall {
                    key: None,
                    arguments: args.to_string(),
                    fingerprint: metadata.fingerprint.clone(),
                    repeat_count: 1,
                    cache_hit: false,
                    plan_progress: false,
                },
            );
            return Ok(ProgressGuardAction::Execute(metadata));
        }
        if !is_guarded_read(tool_name, kind) {
            let metadata = pending_metadata(&state, fingerprint(tool_name, args, None), 1, false, false, false, None, 0);
            state.pending.insert(
                internal_call_id.to_string(),
                PendingCall {
                    key: None,
                    arguments: args.to_string(),
                    fingerprint: metadata.fingerprint.clone(),
                    repeat_count: 1,
                    cache_hit: false,
                    plan_progress: false,
                },
            );
            return Ok(ProgressGuardAction::Execute(metadata));
        }

        let key = canonical_call_key(tool_name, args);
        let repeat_count = if state.last_read_key.as_deref() == Some(&key) {
            state.same_read_repeats.saturating_add(1)
        } else {
            1
        };
        let entry = state.entries.get(&key).cloned().unwrap_or_default();
        let call_fingerprint = fingerprint(tool_name, args, entry.source_version.as_deref());
        if tool_name == GET_COMPONENT_CATALOG_TOOL && state.component_catalog_call_in_flight {
            return Err(violation(
                &state,
                AgentExecutionErrorCode::RepeatedReadLimit,
                "组件目录正在读取；请等待首次结果，不得在同一任务中重复调用",
                tool_name,
                call_fingerprint,
                repeat_count,
            ));
        }
        if cacheable_read(tool_name) && tool_name != GET_COMPONENT_CATALOG_TOOL && repeat_count >= self.max_same_read_repeats {
            return Err(violation(
                &state,
                AgentExecutionErrorCode::RepeatedReadLimit,
                "同一只读查询已重复执行，结果和来源版本均未变化",
                tool_name,
                call_fingerprint,
                repeat_count,
            ));
        }
        if state.reads_without_business_progress >= self.max_same_read_repeats {
            return Err(violation(
                &state,
                AgentExecutionErrorCode::NoProgressLimit,
                "连续只读调用未产生新的业务事实；请复用已有结果、读取新版本或推进业务动作",
                tool_name,
                call_fingerprint,
                repeat_count,
            ));
        }
        state.last_read_key = Some(key.clone());
        state.same_read_repeats = repeat_count;
        state.reads_without_business_progress = state.reads_without_business_progress.saturating_add(1);
        let cache_hit = cacheable_read(tool_name) && entry.cached_result.is_some();
        let metadata = pending_metadata(
            &state,
            call_fingerprint.clone(),
            repeat_count,
            cache_hit,
            false,
            false,
            entry.source_version.clone(),
            entry.actual_executions,
        );
        state.pending.insert(
            internal_call_id.to_string(),
            PendingCall {
                key: Some(key),
                arguments: args.to_string(),
                fingerprint: call_fingerprint,
                repeat_count,
                cache_hit,
                plan_progress: false,
            },
        );
        if tool_name == GET_COMPONENT_CATALOG_TOOL && !cache_hit {
            state.component_catalog_call_in_flight = true;
        }
        if let Some(result) = entry.cached_result.filter(|_| cache_hit) {
            let result = cached_result(result, metadata.source_version.as_deref(), repeat_count, &metadata.fingerprint);
            Ok(ProgressGuardAction::Cached { metadata, result })
        } else {
            Ok(ProgressGuardAction::Execute(metadata))
        }
    }

    /// 在预算/重复读取检查前执行阶段路由检查。
    ///
    /// 路由拒绝不会执行 Tool；调用方应将该违规作为受控 Agent 失败处理。保留
    /// `before_call` 兼容旧的固定 Tool 测试和内部调用。
    pub(crate) fn before_routed_call(
        &self,
        internal_call_id: &str,
        tool_name: &str,
        args: &str,
        kind: ToolCallKind,
        canvas_write_task: bool,
        route_snapshot: &AgentToolRouteSnapshot,
    ) -> Result<ProgressGuardAction, ProgressViolation> {
        if !route_snapshot.allows_tool_call(tool_name, args) {
            let state = self.lock_state();
            return Err(violation(
                &state,
                AgentExecutionErrorCode::ToolNotAdvertisedInStep,
                "该 Tool 未在生成调用时的模型步骤中暴露",
                tool_name,
                fingerprint(tool_name, args, None),
                1,
            ));
        }
        self.before_call_in_phase(internal_call_id, tool_name, args, kind, canvas_write_task)
    }

    /// Barrier 后的兄弟调用由运行时返回延迟结果，不进入重复读取或业务进展统计。
    pub(crate) fn deferred_call_metadata(&self, tool_name: &str, args: &str) -> ProgressCallMetadata {
        let state = self.lock_state();
        pending_metadata(&state, fingerprint(tool_name, args, None), 1, false, false, false, None, 0)
    }

    pub(crate) fn finish_call(
        &self,
        internal_call_id: &str,
        tool_name: &str,
        kind: ToolCallKind,
        result: &str,
        succeeded: bool,
    ) -> ProgressCallMetadata {
        let mut state = self.lock_state();
        let pending = state.pending.remove(internal_call_id).unwrap_or_else(|| PendingCall {
            key: None,
            arguments: "{}".to_string(),
            fingerprint: fingerprint(tool_name, "{}", None),
            repeat_count: 1,
            cache_hit: false,
            plan_progress: false,
        });
        let mut source_version = None;
        let mut actual_executions = 0;
        let mut produced_new_fact = false;
        if tool_name == GET_COMPONENT_CATALOG_TOOL {
            state.component_catalog_call_in_flight = false;
        }
        if tool_name == RESOLVE_COMPONENT_SPECS_TOOL {
            if succeeded && component_resolution_has_matches(result) {
                state.consecutive_empty_component_resolutions = 0;
            } else {
                state.consecutive_empty_component_resolutions = state.consecutive_empty_component_resolutions.saturating_add(1);
            }
        }
        if let Some(key) = pending.key.as_ref() {
            let observed_version = (succeeded && !pending.cache_hit)
                .then(|| extract_source_version(result))
                .flatten();
            let unseen_result = succeeded && !pending.cache_hit && state.facts.record_with_args(tool_name, Some(&pending.arguments), result);
            let entry = state.entries.entry(key.clone()).or_default();
            if !pending.cache_hit {
                entry.actual_executions = entry.actual_executions.saturating_add(1);
            }
            if succeeded && !pending.cache_hit {
                source_version = observed_version;
                produced_new_fact = unseen_result;
                entry.source_version = source_version.clone();
                if cacheable_read(tool_name) && source_version.is_some() {
                    entry.cached_result = Some(result.to_string());
                }
            } else {
                source_version = entry.source_version.clone();
            }
            actual_executions = entry.actual_executions;
        }
        if pending.plan_progress && !succeeded {
            state.last_plan_key = None;
            state.identical_plan_repeats = 0;
        }
        let business_write = succeeded
            && is_business_write_tool(kind)
            && semantic_fact_signature(tool_name, result).is_some_and(|signature| state.applied_write_facts.insert(signature));
        if produced_new_fact {
            state.reads_without_business_progress = 0;
            state.progress_sequence = state.progress_sequence.saturating_add(1);
            state.last_progress = Some(if business_write {
                tool_name.to_string()
            } else {
                format!("new_fact:{tool_name}")
            });
            state.last_read_key = None;
            state.same_read_repeats = 0;
        }
        if business_write {
            state.reads_without_business_progress = 0;
            state.consecutive_empty_component_resolutions = 0;
            if business_write && !matches!(kind, ToolCallKind::Read | ToolCallKind::FrontendInteraction) {
                state
                    .entries
                    .retain(|key, _| key.starts_with(&format!("{GET_COMPONENT_CATALOG_TOOL}:")));
            }
        }
        pending_metadata(
            &state,
            pending.fingerprint,
            pending.repeat_count,
            pending.cache_hit,
            produced_new_fact,
            business_write,
            source_version,
            actual_executions,
        )
    }

    fn lock_state(&self) -> MutexGuard<'_, ProgressState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn pending_metadata(
    state: &ProgressState,
    fingerprint: String,
    repeat_count: u64,
    cache_hit: bool,
    produced_new_fact: bool,
    business_write: bool,
    source_version: Option<String>,
    actual_execution_count: u64,
) -> ProgressCallMetadata {
    ProgressCallMetadata {
        fingerprint,
        repeat_count,
        actual_execution_count,
        source_version,
        cache_hit,
        produced_new_fact,
        business_write,
        reads_without_business_progress: state.reads_without_business_progress,
        last_progress_sequence: state.progress_sequence,
    }
}

fn violation(
    state: &ProgressState,
    code: AgentExecutionErrorCode,
    message: &str,
    tool_name: &str,
    fingerprint: String,
    repeat_count: u64,
) -> ProgressViolation {
    ProgressViolation {
        code,
        message: message.to_string(),
        tool_name: tool_name.to_string(),
        fingerprint,
        repeat_count,
        reads_without_business_progress: state.reads_without_business_progress,
        last_progress_sequence: state.progress_sequence,
        last_progress: state.last_progress.clone(),
    }
}

fn canonical_call_key(tool_name: &str, args: &str) -> String {
    let mut value = serde_json::from_str::<Value>(args).unwrap_or_else(|_| Value::String(args.trim().to_string()));
    if tool_name == RESOLVE_COMPONENT_SPECS_TOOL {
        canonicalize_component_spec_args(&mut value);
    }
    let canonical = canonical_json(&value);
    format!("{tool_name}:{}", serde_json::to_string(&canonical).unwrap_or_default())
}

fn canonicalize_component_spec_args(value: &mut Value) {
    let Some(object) = value.as_object_mut() else { return };
    if let Some(items) = object.get_mut("componentTypes").and_then(Value::as_array_mut) {
        let mut values = items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        values.sort();
        values.dedup();
        *items = values.into_iter().map(Value::String).collect();
    }
    // Persisted calls from the old field/page protocol must share the new
    // identity.  The current schema rejects these keys before execution.
    for key in ["fields", "page", "corpusVersion"] {
        object.remove(key);
    }
}

fn fingerprint(tool_name: &str, args: &str, source_version: Option<&str>) -> String {
    let key = canonical_call_key(tool_name, args);
    let bytes = format!("{key}:{}", source_version.unwrap_or("unknown"));
    format!("sha256:{}", sha256_hex(bytes.as_bytes()))
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
            let mut canonical = Map::new();
            for (key, value) in entries {
                canonical.insert(key.clone(), canonical_json(value));
            }
            Value::Object(canonical)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical_json).collect()),
        value => value.clone(),
    }
}

/// Extract a stable, tool-specific fact set for progress decisions. A raw JSON
/// hash treats response ordering and transport metadata as progress; the model
/// should advance only when the tool's meaningful state changes.
fn semantic_fact_signature(tool_name: &str, result: &str) -> Option<String> {
    let mut value = serde_json::from_str::<Value>(result).ok()?;
    strip_volatile_fact_fields(&mut value);
    let value = semantic_fact_value(tool_name, value);
    if tool_name == INSPECT_CURRENT_PAGE_TOOL && value.is_null() {
        return None;
    }
    let canonical = canonical_json(&value);
    let bytes = serde_json::to_vec(&canonical).ok()?;
    let scoped = if tool_name.is_empty() {
        bytes
    } else {
        [tool_name.as_bytes(), b":", &bytes].concat()
    };
    Some(format!("sha256:{}", sha256_hex(&scoped)))
}

fn semantic_fact_value(tool_name: &str, value: Value) -> Value {
    match tool_name {
        RESOLVE_COMPONENT_SPECS_TOOL => semantic_component_spec_facts(value),
        INSPECT_CURRENT_PAGE_TOOL => semantic_canvas_snapshot_facts(value),
        GET_CANVAS_OPERATION_STATUS_TOOL => semantic_operation_status_facts(value),
        GET_COMPONENT_CATALOG_TOOL => project_object_fields(value, &["catalogVersion", "components", "status", "code", "message"]),
        _ => value,
    }
}

fn semantic_component_spec_facts(value: Value) -> Value {
    let Some(object) = value.as_object() else {
        return value;
    };
    let Some(specs) = object.get("specs").and_then(Value::as_array) else {
        return project_object_fields_strict(value, &["corpusVersion", "catalogVersion", "totalMatches", "status", "code", "message"]);
    };
    let mut facts = Vec::new();
    for spec in specs {
        let Some(spec_object) = spec.as_object() else {
            continue;
        };
        let Some(component_type) = spec_object
            .get("componentType")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|component_type| !component_type.is_empty())
        else {
            continue;
        };
        if progress_facts::complete_component_type(spec).is_none() {
            continue;
        }
        let version = object.get("corpusVersion").cloned();
        let mut fact = Map::new();
        fact.insert("componentType".to_string(), Value::String(component_type.to_string()));
        if let Some(version) = version {
            fact.insert("version".to_string(), version);
        }
        facts.push(Value::Object(fact));
    }
    if facts.is_empty() {
        return project_object_fields_strict(value, &["corpusVersion", "catalogVersion", "totalMatches", "status", "code", "message"]);
    }
    facts.sort_by_key(|fact| serde_json::to_string(fact).unwrap_or_default());
    let mut result = Map::new();
    result.insert("specFacts".to_string(), Value::Array(facts));
    Value::Object(result)
}

fn semantic_canvas_snapshot_facts(value: Value) -> Value {
    let Some(object) = value.as_object() else {
        return value;
    };
    let mut component_ids = object
        .get("components")
        .and_then(Value::as_array)
        .map(|components| {
            components
                .iter()
                .filter_map(|component| component.get("id").and_then(Value::as_str))
                .map(str::trim)
                .filter(|component_id| !component_id.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    component_ids.sort();
    component_ids.dedup();
    let mut missing_ids = text_values(object.get("missingComponentIds"));
    missing_ids.sort();
    missing_ids.dedup();
    if component_ids.is_empty() && missing_ids.is_empty() {
        return Value::Null;
    }
    let mut facts = Map::new();
    facts.insert("componentIds".to_string(), json!(component_ids));
    if !missing_ids.is_empty() {
        facts.insert("missingComponentIds".to_string(), json!(missing_ids));
    }
    for key in ["revision", "workspaceRevision"] {
        if let Some(version) = object.get(key) {
            facts.insert(key.to_string(), version.clone());
        }
    }
    Value::Object(facts)
}

fn semantic_operation_status_facts(value: Value) -> Value {
    let mut projected = project_object_fields(
        value,
        &[
            "operationId",
            "status",
            "resultRevision",
            "workspaceRevision",
            "componentIds",
            "componentBindings",
            "componentFields",
            "configuredComponentCount",
            "defaultConfigComponentCount",
            "error",
            "code",
            "message",
        ],
    );
    if let Some(object) = projected.as_object_mut() {
        if let Some(component_ids) = object.get_mut("componentIds").and_then(Value::as_array_mut) {
            let mut ids = component_ids
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            ids.sort();
            ids.dedup();
            *component_ids = ids.into_iter().map(Value::String).collect();
        }
    }
    projected
}

fn text_values(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn project_object_fields(value: Value, fields: &[&str]) -> Value {
    let Some(object) = value.as_object() else {
        return value;
    };
    let mut projected = Map::new();
    for key in fields {
        if let Some(value) = object.get(*key) {
            projected.insert((*key).to_string(), value.clone());
        }
    }
    if projected.is_empty() {
        Value::Object(object.clone())
    } else {
        Value::Object(projected)
    }
}

fn project_object_fields_strict(value: Value, fields: &[&str]) -> Value {
    let Some(object) = value.as_object() else {
        return Value::Null;
    };
    let mut projected = Map::new();
    for key in fields {
        if let Some(value) = object.get(*key) {
            projected.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(projected)
}

fn strip_volatile_fact_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for key in [
                "elapsedMs",
                "durationMs",
                "timestamp",
                "updatedAt",
                "createdAt",
                "observedAt",
                "fetchedAt",
                "checkedAt",
                "cacheHit",
                "repeatCount",
                "fingerprint",
            ] {
                object.remove(key);
            }
            for child in object.values_mut() {
                strip_volatile_fact_fields(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                strip_volatile_fact_fields(child);
            }
        }
        _ => {}
    }
}

fn extract_source_version(result: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(result).ok()?;
    ["sourceVersion", "corpusVersion", "workspaceRevision", "catalogVersion", "resultRevision", "revision"]
        .into_iter()
        .find_map(|key| find_field(&value, key))
        .and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
}

fn component_resolution_has_matches(result: &str) -> bool {
    serde_json::from_str::<Value>(result)
        .ok()
        .and_then(|value| value.get("specs").and_then(Value::as_array).cloned())
        .is_some_and(|specs| specs.iter().any(|spec| progress_facts::complete_component_type(spec).is_some()))
}

fn find_field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(object) => object
            .get(key)
            .or_else(|| object.values().find_map(|child| find_field(child, key))),
        Value::Array(items) => items.iter().find_map(|child| find_field(child, key)),
        _ => None,
    }
}

fn cached_result(result: String, source_version: Option<&str>, repeat_count: u64, fingerprint: &str) -> String {
    let mut value = serde_json::from_str::<Value>(&result).unwrap_or_else(|_| json!({ "content": result }));
    let metadata = json!({
        "cacheHit": true,
        "sourceVersion": source_version,
        "repeatCount": repeat_count,
        "fingerprint": fingerprint,
    });
    if let (Some(object), Some(metadata)) = (value.as_object_mut(), metadata.as_object()) {
        object.extend(metadata.clone());
    }
    value.to_string()
}

fn cacheable_read(tool_name: &str) -> bool {
    matches!(
        tool_name,
        GET_COMPONENT_CATALOG_TOOL | "list_workspace_files" | "search_workspace_files" | "read_workspace_file"
    )
}

fn is_guarded_read(tool_name: &str, kind: ToolCallKind) -> bool {
    if tool_name == UPDATE_PLAN_TOOL {
        return false;
    }
    matches!(kind, ToolCallKind::Read | ToolCallKind::FrontendInteraction)
        && !matches!(tool_name, "request_user_input" | "request_component_selection")
}

fn is_business_write_tool(kind: ToolCallKind) -> bool {
    matches!(kind, ToolCallKind::Write | ToolCallKind::WriteFrontendInteraction)
}

#[cfg(test)]
#[path = "progress_guard_tests.rs"]
mod tests;

#[path = "progress_facts.rs"]
mod progress_facts;
