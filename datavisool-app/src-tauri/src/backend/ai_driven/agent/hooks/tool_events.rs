/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Tool 调用的预算、进展保护和 Timeline 记录。

use std::time::{Duration, Instant};

use rig_core::agent::{Flow, HookContext};
use rig_core::tool::{ToolFailure, ToolFailureKind, ToolOutcome};
use serde_json::{json, Value};

use crate::backend::ai_driven::task::{AgentPendingSnapshot, AgentTaskStatus, AgentWorkingPlanStep, TaskRepositoryError};
use crate::backend::ai_driven::thread::{new_item_id, ItemStatus};

use super::{
    classify_tool_call, content_summary, duration_ms, item_snapshot, now_ms, tool_kind_name, waiting_interaction, ActiveToolCall, AgentRuntimeHook,
};
use crate::backend::ai_driven::agent::argument_shape::detect_argument_shape_issue;
use crate::backend::ai_driven::agent::context_plan::{AgentRoutePhase, AgentToolRoute, AgentToolRouteSnapshot};
use crate::backend::ai_driven::agent::limits::{AgentExecutionErrorCode, ToolCallOutcome};
use crate::backend::ai_driven::agent::progress_guard::{ProgressGuardAction, ProgressViolation};
use crate::backend::ai_driven::agent::tool_concurrency::tool_concurrency_policy;
use crate::backend::ai_driven::agent::tool_failure_policy::{
    non_retryable_infrastructure_failure, repeated_failure_directive, RepeatedFailureAction, RepeatedFailureFlow,
};
use crate::backend::ai_driven::domain::operation::{CanvasOperationReceipt, CanvasOperationStatus};
use crate::backend::ai_driven::tools::canvas::{
    CREATE_CANVAS_COMPONENTS_TOOL, DELETE_CANVAS_COMPONENTS_TOOL, GET_CANVAS_OPERATION_STATUS_TOOL, PATCH_CANVAS_COMPONENTS_TOOL,
};
use crate::backend::ai_driven::tools::component_catalog::GET_COMPONENT_CATALOG_TOOL;
use crate::backend::ai_driven::tools::task_intent::UPDATE_PLAN_TOOL;
use crate::backend::utils::{debug_log_json, sanitize_debug_log_text};

/// 将成功的计划/画布回执同步到宿主驱动的 Tool 路由。
///
/// 该函数不推断用户意图，也不信任普通文本；只有结构化 Tool 回执可以推进阶段。
pub(super) fn update_tool_route(route: &AgentToolRoute, tool_name: &str, result: &str, succeeded: bool) -> bool {
    if tool_name == UPDATE_PLAN_TOOL {
        // 计划仅是展示信息，不能改变写入路由；仍拒绝无效回执，避免
        // checkpoint 保存了无法恢复的计划数据。
        return succeeded
            && serde_json::from_str::<Value>(result)
                .ok()
                .is_some_and(|plan| plan.get("plan").is_some());
    }
    if !matches!(
        tool_name,
        CREATE_CANVAS_COMPONENTS_TOOL | PATCH_CANVAS_COMPONENTS_TOOL | DELETE_CANVAS_COMPONENTS_TOOL | GET_CANVAS_OPERATION_STATUS_TOOL
    ) {
        return false;
    }
    serde_json::from_str::<CanvasOperationReceipt>(result)
        .ok()
        .filter(|receipt| !receipt.operation_id.trim().is_empty())
        .map(|receipt| {
            route.observe_operation_result(&receipt);
            true
        })
        .unwrap_or(false)
}

fn deferred_after_barrier_result(tool_name: &str, barrier_position: Option<u64>) -> String {
    json!({
        "code": "agent.tool.deferred_after_barrier",
        "message": "该调用位于用户交互或审批屏障之后，需要在下一模型步骤重新发起",
        "retryable": true,
        "toolName": tool_name,
        "barrierPosition": barrier_position,
    })
    .to_string()
}

impl AgentRuntimeHook {
    pub(super) async fn on_tool_call(
        &self,
        ctx: &HookContext,
        tool_name: &str,
        tool_call_id: Option<&str>,
        internal_call_id: &str,
        args: &str,
    ) -> Flow {
        let kind = classify_tool_call(tool_name);
        let route_snapshot = self.tool_route.as_ref().and_then(AgentToolRoute::active_snapshot);
        if self.tool_route.is_some() && route_snapshot.is_none() {
            return Flow::terminate("agent.tool.route_snapshot_missing");
        }
        let policy = tool_concurrency_policy(tool_name);
        let authorization_revision = self
            .tool_route
            .as_ref()
            .map(AgentToolRoute::authorization_revision)
            .unwrap_or(0);
        let (step_index, route_revision, admitted_phase, snapshot_generation) = route_snapshot
            .as_ref()
            .map(|snapshot| (snapshot.step_index, snapshot.route_revision, snapshot.phase, snapshot.generation()))
            .unwrap_or((ctx.turn() as u64, 0, AgentRoutePhase::Ready, 0));
        let admission_source = if route_snapshot.is_some() { "stepSnapshot" } else { "runtimeDefault" };
        let canvas_write_task = self
            .completion_requirements
            .snapshot()
            .is_some_and(|requirements| !requirements.is_empty());
        let batch = self
            .tool_batch
            .admit(snapshot_generation, tool_name, tool_call_id, internal_call_id, args, policy);
        let batch_completion = self.tool_batch.completion_guard(snapshot_generation, batch.position);
        let _batch_turn = match self.tool_batch.wait_for_turn(snapshot_generation, batch.position).await {
            Ok(guard) => guard,
            Err(_) => return Flow::terminate("agent.tool.route_generation_changed"),
        };
        self.invocation_tracker.begin(tool_name, tool_call_id, internal_call_id);
        let permission_retry_blocked = self
            .tool_failure_tracker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .blocked_attempt(tool_name, args, authorization_revision)
            .is_some();
        if permission_retry_blocked {
            self.invocation_tracker.finish(internal_call_id);
            return Flow::terminate("agent.tool.permission_retry_blocked");
        }
        if let Err(error) = self.budget.start_tool_call(kind) {
            self.invocation_tracker.finish(internal_call_id);
            return self.terminate_for_budget(ctx, error).await;
        }
        tracing::debug!(
            target: "agent.tool",
            event = "tool_call_generated",
            project_id = self.task.project_id,
            thread_id = %self.task.thread_id,
            task_id = %self.task.task_id,
            turn_id = %self.task.turn_id,
            run_id = %self.task.run_id,
            request_id = %self.task.request_id,
            step_index,
            route_revision,
            authorization_revision,
            admitted_phase = admitted_phase.as_str(),
            tool_name,
            tool_call_id,
            internal_call_id,
            category = tool_kind_name(kind),
            admission_source,
            argument_bytes = args.len(),
            "Agent tool call generated"
        );
        if let Some(snapshot) = &route_snapshot {
            if !snapshot.allows_tool_call(tool_name, args) {
                let violation = self
                    .progress_guard
                    .before_routed_call(internal_call_id, tool_name, args, kind, canvas_write_task, snapshot)
                    .expect_err("disallowed snapshot call must return a route violation");
                self.invocation_tracker.finish(internal_call_id);
                return self.terminate_for_progress(ctx, violation).await;
            }
        }
        tracing::debug!(
            target: "agent.tool",
            event = "tool_call_admitted",
            project_id = self.task.project_id,
            thread_id = %self.task.thread_id,
            task_id = %self.task.task_id,
            turn_id = %self.task.turn_id,
            run_id = %self.task.run_id,
            request_id = %self.task.request_id,
            step_index,
            route_revision,
            admitted_phase = admitted_phase.as_str(),
            live_phase = self.tool_route.as_ref().map(AgentToolRoute::phase).unwrap_or(admitted_phase).as_str(),
            tool_name,
            tool_call_id,
            internal_call_id,
            category = tool_kind_name(kind),
            admission_source,
            concurrency_policy = policy.as_str(),
            batch_position = batch.position,
            scheduler_state = if batch.deferred_after_barrier { "deferred" } else { "admitted" },
            argument_bytes = args.len(),
            arguments = %sanitize_debug_log_text(args),
            usage = %debug_log_json(&self.budget.usage_snapshot()),
            "Agent tool call admitted"
        );
        let (guard_action, deferred_progress) = if batch.deferred_after_barrier {
            (None, Some(self.progress_guard.deferred_call_metadata(tool_name, args)))
        } else {
            let guard_result = match &route_snapshot {
                Some(snapshot) => self
                    .progress_guard
                    .before_routed_call(internal_call_id, tool_name, args, kind, canvas_write_task, snapshot),
                None => self
                    .progress_guard
                    .before_call(internal_call_id, tool_name, args, kind, canvas_write_task),
            };
            match guard_result {
                Ok(action) => (Some(action), None),
                Err(violation) => {
                    self.invocation_tracker.finish(internal_call_id);
                    return self.terminate_for_progress(ctx, violation).await;
                }
            }
        };
        let metadata = deferred_progress
            .as_ref()
            .unwrap_or_else(|| match guard_action.as_ref().expect("non-deferred call has guard action") {
                ProgressGuardAction::Execute(metadata) | ProgressGuardAction::Cached { metadata, .. } => metadata,
            });
        let item_id = new_item_id();
        let snapshot = item_snapshot(
            &item_id,
            "toolCall",
            ItemStatus::InProgress,
            1,
            json!({
                "toolName": tool_name,
                "category": tool_kind_name(kind),
                "arguments": content_summary(args),
                "progress": metadata,
                "routeRevision": route_revision,
                "authorizationRevision": authorization_revision,
                "admittedPhase": admitted_phase.as_str(),
                "admissionSource": admission_source,
                "concurrencyPolicy": policy.as_str(),
                "batchPosition": batch.position,
                "schedulerState": if batch.deferred_after_barrier { "deferred" } else { "waiting" },
            }),
        );
        let item = match self
            .start_item(ctx, item_id.clone(), Some(format!("tool:{internal_call_id}")), "toolCall", ItemStatus::InProgress, snapshot)
            .await
        {
            Ok(item) => item,
            Err(_) => return Flow::terminate("agent.item_persistence_failed"),
        };
        let mut active_call = ActiveToolCall {
            started_at: Instant::now(),
            kind,
            item_id,
            row_version: item.row_version,
            step_index,
            route_revision,
            arguments: args.to_string(),
            authorization_revision,
            admitted_phase,
            admission_source,
            policy,
            batch_position: batch.position,
            deferred_progress: deferred_progress.clone(),
            batch_completion,
        };

        let payload = self.payload(
            ctx,
            json!({
                "stepIndex": step_index,
                "routeRevision": route_revision,
                "admittedPhase": admitted_phase.as_str(),
                "admissionSource": admission_source,
                "livePhase": self.tool_route.as_ref().map(AgentToolRoute::phase).unwrap_or(admitted_phase).as_str(),
                "toolName": tool_name,
                "toolCallId": tool_call_id,
                "internalCallId": internal_call_id,
                "category": tool_kind_name(kind),
                "arguments": content_summary(args),
                "progress": metadata,
                "concurrencyPolicy": policy.as_str(),
                "batchPosition": batch.position,
                "schedulerState": if batch.deferred_after_barrier { "deferred" } else { "waiting" },
                "usage": self.budget.usage_snapshot(),
            }),
        );
        let persisted = self
            .persist_or_terminate("agent.tool_call.started", Some(item.item_id), payload)
            .await;
        if !matches!(persisted, Flow::Continue) {
            return persisted;
        }
        if batch.deferred_after_barrier {
            active_call.batch_completion.activate();
            self.lock_tool_calls().insert(internal_call_id.to_string(), active_call);
            return Flow::skip(deferred_after_barrier_result(tool_name, batch.barrier_position));
        }
        let guard_action = guard_action.expect("non-deferred call has guard action");
        if let ProgressGuardAction::Cached { result, .. } = guard_action {
            active_call.batch_completion.activate();
            self.lock_tool_calls().insert(internal_call_id.to_string(), active_call);
            return Flow::skip(result);
        }
        if policy == crate::backend::ai_driven::agent::tool_concurrency::ToolConcurrencyPolicy::Barrier {
            if self
                .tool_batch
                .wait_for_prior_completion(snapshot_generation, batch.position)
                .await
                .is_err()
            {
                self.invocation_tracker.finish(internal_call_id);
                return Flow::terminate("agent.tool.prior_call_aborted");
            }
        }
        let waiting = self
            .enter_waiting_state(ctx, tool_name, tool_call_id, internal_call_id, args, route_snapshot.as_ref())
            .await;
        if !matches!(waiting, Flow::Continue) {
            self.invocation_tracker.finish(internal_call_id);
            return waiting;
        }
        self.tool_scheduler.register_call(
            tool_name,
            tool_call_id,
            internal_call_id,
            step_index,
            route_revision,
            admitted_phase.as_str(),
            admission_source,
            policy,
            batch.position,
        );
        active_call.batch_completion.activate();
        self.lock_tool_calls().insert(internal_call_id.to_string(), active_call);
        Flow::cont()
    }

    async fn enter_waiting_state(
        &self,
        ctx: &HookContext,
        tool_name: &str,
        tool_call_id: Option<&str>,
        internal_call_id: &str,
        args: &str,
        route_snapshot: Option<&AgentToolRouteSnapshot>,
    ) -> Flow {
        let Some((waiting_status, pending_kind, waiting_item_type)) = waiting_interaction(tool_name) else {
            return Flow::cont();
        };
        let suspended_at = now_ms();
        let waiting_item_id = new_item_id();
        let waiting_snapshot = item_snapshot(
            &waiting_item_id,
            waiting_item_type,
            ItemStatus::Waiting,
            1,
            json!({
                "toolName": tool_name,
                "toolCallId": tool_call_id,
                "internalCallId": internal_call_id,
                "request": content_summary(args),
            }),
        );
        let waiting_item = match self
            .start_item(
                ctx,
                waiting_item_id.clone(),
                Some(format!("waiting:{internal_call_id}")),
                waiting_item_type,
                ItemStatus::Waiting,
                waiting_snapshot,
            )
            .await
        {
            Ok(item) => item,
            Err(_) => return Flow::terminate("agent.item_persistence_failed"),
        };
        let pending = AgentPendingSnapshot {
            kind: pending_kind,
            step_index: route_snapshot.map_or(ctx.turn() as u64, |snapshot| snapshot.step_index),
            route_revision: route_snapshot.map_or(0, |snapshot| snapshot.route_revision),
            waiting_item_id: waiting_item.item_id,
            tool_name: Some(tool_name.to_string()),
            request_summary: content_summary(args),
            suspended_at,
        };
        match self.save_task_status(waiting_status, Some(pending), suspended_at).await {
            Ok(()) => Flow::cont(),
            Err(_) => Flow::terminate("agent.task_checkpoint_failed"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn on_tool_result(
        &self,
        ctx: &HookContext,
        tool_name: &str,
        tool_call_id: Option<&str>,
        internal_call_id: &str,
        result: &str,
        outcome: &ToolOutcome,
    ) -> Flow {
        self.invocation_tracker.finish(internal_call_id);
        let schedule_timing = self.tool_scheduler.finish_call(internal_call_id);
        let active = self.lock_tool_calls().remove(internal_call_id);
        let duration = active.as_ref().map(|call| call.started_at.elapsed()).unwrap_or(Duration::ZERO);
        let deferred_after_barrier = active.as_ref().is_some_and(|call| call.deferred_progress.is_some());
        let kind = active
            .as_ref()
            .map(|call| call.kind)
            .unwrap_or_else(|| classify_tool_call(tool_name));
        let semantic_outcome = semantic_tool_outcome(tool_name, result, outcome);
        let argument_shape_issue = semantic_outcome
            .failure()
            .filter(|failure| failure.kind == ToolFailureKind::InvalidArgs)
            .and_then(|_| active.as_ref())
            .and_then(|call| detect_argument_shape_issue(tool_name, &call.arguments));
        let concise_result = argument_shape_issue.map(|issue| issue.model_result());
        let result_for_model = concise_result.as_deref().unwrap_or(result);
        let semantic_canvas_failure = is_failed_canvas_receipt(tool_name, result);
        let progress = active
            .as_ref()
            .and_then(|call| call.deferred_progress.clone())
            .unwrap_or_else(|| {
                self.progress_guard
                    .finish_call(internal_call_id, tool_name, kind, result, semantic_outcome.is_success())
            });
        let effective_success = !semantic_canvas_failure && (outcome.is_success() || progress.cache_hit || deferred_after_barrier);
        let execution_duration =
            schedule_timing.map_or_else(|| if deferred_after_barrier { Duration::ZERO } else { duration }, |timing| timing.execution_duration);
        let lifecycle_outcome = if deferred_after_barrier {
            "deferredAfterBarrier"
        } else if progress.cache_hit {
            "cacheHit"
        } else if semantic_canvas_failure {
            "failedReceipt"
        } else {
            semantic_outcome.as_str()
        };
        if effective_success
            && !deferred_after_barrier
            && matches!(kind, crate::backend::ai_driven::agent::limits::ToolCallKind::WriteFrontendInteraction)
            && serde_json::from_str::<Value>(result)
                .ok()
                .and_then(|receipt| receipt.get("status").and_then(Value::as_str).map(str::to_string))
                .as_deref()
                == Some("applied")
        {
            self.completion_requirements.lock_after_canvas_write();
            if tool_name == PATCH_CANVAS_COMPONENTS_TOOL {
                if let (Some(active_call), Ok(receipt)) = (active.as_ref(), serde_json::from_str::<CanvasOperationReceipt>(result)) {
                    self.completion_requirements
                        .record_applied_canvas_fields(&receipt, &active_call.arguments);
                }
            }
        }
        let tool_outcome = if effective_success {
            ToolCallOutcome::Succeeded
        } else {
            ToolCallOutcome::Failed
        };
        let hit_tool_duration_limit = !progress.cache_hit
            && !deferred_after_barrier
            && semantic_outcome.failure().and_then(|failure| failure.code.as_deref()) == Some(AgentExecutionErrorCode::ToolDurationLimit.as_str());
        let fatal_failure_code = (!progress.cache_hit && !deferred_after_barrier)
            .then(|| non_retryable_infrastructure_failure(&semantic_outcome))
            .flatten()
            .map(|failure| {
                failure
                    .code
                    .clone()
                    .unwrap_or_else(|| "agent.tool.infrastructure_failed".to_string())
            });
        let budget_result = if hit_tool_duration_limit {
            Err(self.budget.finish_timed_out_tool_call(execution_duration))
        } else {
            self.budget.finish_tool_call(execution_duration, tool_outcome)
        };
        let step_index = active.as_ref().map_or(ctx.turn() as u64, |call| call.step_index);
        let route_revision = active.as_ref().map_or(0, |call| call.route_revision);
        let authorization_revision = active.as_ref().map_or(0, |call| call.authorization_revision);
        let admitted_phase = active.as_ref().map_or(AgentRoutePhase::Ready, |call| call.admitted_phase);
        let admission_source = active.as_ref().map_or("runtimeDefault", |call| call.admission_source);
        let policy = active
            .as_ref()
            .map_or_else(|| tool_concurrency_policy(tool_name), |call| call.policy);
        let batch_position = active.as_ref().map_or(0, |call| call.batch_position);
        tracing::debug!(
            target: "agent.tool",
            event = "tool_call_completed",
            project_id = self.task.project_id,
            thread_id = %self.task.thread_id,
            task_id = %self.task.task_id,
            turn_id = %self.task.turn_id,
            run_id = %self.task.run_id,
            request_id = %self.task.request_id,
            step_index,
            route_revision,
            authorization_revision,
            admitted_phase = admitted_phase.as_str(),
            admission_source,
            live_phase = self.tool_route.as_ref().map(AgentToolRoute::phase).unwrap_or(admitted_phase).as_str(),
            tool_name,
            tool_call_id,
            internal_call_id,
            category = tool_kind_name(kind),
            concurrency_policy = policy.as_str(),
            batch_position,
            scheduler_state = if deferred_after_barrier { "deferred" } else { "completed" },
            scheduler_wait_ms = schedule_timing.map(|timing| duration_ms(timing.wait_duration)),
            scheduler_acquired = schedule_timing.map(|timing| timing.acquired),
            outcome = lifecycle_outcome,
            failure_kind = semantic_outcome.error_kind().map(|kind| kind.as_str()),
            failure_code = semantic_outcome.failure().and_then(|failure| failure.code.as_deref()),
            duration_ms = duration_ms(execution_duration),
            total_duration_ms = duration_ms(duration),
            timing_missing = active.is_none(),
            result_bytes = result.len(),
            result = %sanitize_debug_log_text(result_for_model),
            progress = %debug_log_json(&progress),
            usage = %debug_log_json(&self.budget.usage_snapshot()),
            "Agent tool result received"
        );
        let repeated_failure_action = if progress.cache_hit || deferred_after_barrier {
            RepeatedFailureAction::Continue
        } else {
            self.tool_failure_tracker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .record_attempt(
                    tool_name,
                    &semantic_outcome,
                    result_for_model,
                    active.as_ref().map_or("", |call| call.arguments.as_str()),
                    authorization_revision,
                )
        };
        let payload = self.payload(
            ctx,
            json!({
                "stepIndex": step_index,
                "routeRevision": route_revision,
                "authorizationRevision": authorization_revision,
                "admittedPhase": admitted_phase.as_str(),
                "admissionSource": admission_source,
                "livePhase": self.tool_route.as_ref().map(AgentToolRoute::phase).unwrap_or(admitted_phase).as_str(),
                "toolName": tool_name,
                "toolCallId": tool_call_id,
                "internalCallId": internal_call_id,
                "category": tool_kind_name(kind),
                "outcome": lifecycle_outcome,
                "failureKind": semantic_outcome.error_kind().map(|kind| kind.as_str()),
                "failureCode": semantic_outcome.failure().and_then(|failure| failure.code.as_deref()),
                "failureRetryable": semantic_outcome.failure().and_then(|failure| failure.retryable),
                "durationMs": duration_ms(execution_duration),
                "totalDurationMs": duration_ms(duration),
                "schedulerWaitMs": schedule_timing.map(|timing| duration_ms(timing.wait_duration)),
                "schedulerAcquired": schedule_timing.map(|timing| timing.acquired),
                "concurrencyPolicy": policy.as_str(),
                "batchPosition": batch_position,
                "schedulerState": if deferred_after_barrier { "deferred" } else { "completed" },
                "timingMissing": active.is_none(),
                "result": content_summary(result_for_model),
                "progress": progress,
                "usage": self.budget.usage_snapshot(),
            }),
        );
        let item_id = active.as_ref().map(|call| call.item_id.clone());
        let persisted = self
            .persist_or_terminate("agent.tool_call.completed", item_id.clone(), payload)
            .await;
        if !matches!(persisted, Flow::Continue) {
            return persisted;
        }
        let Some(active) = active else {
            return Flow::terminate("agent.item_state_missing");
        };
        let item_status = if effective_success { ItemStatus::Completed } else { ItemStatus::Failed };
        let snapshot = item_snapshot(
            &active.item_id,
            "toolCall",
            item_status,
            active.row_version + 1,
            json!({
                "toolName": tool_name,
                "category": tool_kind_name(kind),
                "outcome": lifecycle_outcome,
                "durationMs": duration_ms(execution_duration),
                "totalDurationMs": duration_ms(duration),
                "routeRevision": route_revision,
                "authorizationRevision": authorization_revision,
                "admittedPhase": admitted_phase.as_str(),
                "admissionSource": admission_source,
                "concurrencyPolicy": policy.as_str(),
                "batchPosition": batch_position,
                "progress": progress,
            }),
        );
        let event_kind = if item_status == ItemStatus::Completed {
            "item.completed"
        } else {
            "item.failed"
        };
        if self
            .apply_item(ctx, &active.item_id, active.row_version, item_status, snapshot, event_kind, now_ms())
            .await
            .is_err()
        {
            return Flow::terminate("agent.item_persistence_failed");
        }
        if !deferred_after_barrier
            && waiting_interaction(tool_name).is_some()
            && self.save_task_status(AgentTaskStatus::Running, None, now_ms()).await.is_err()
        {
            return Flow::terminate("agent.task_checkpoint_failed");
        }
        let has_canvas_receipt = is_canvas_operation_tool(tool_name)
            && serde_json::from_str::<CanvasOperationReceipt>(result)
                .ok()
                .is_some_and(|receipt| !receipt.operation_id.trim().is_empty());
        if !deferred_after_barrier
            && !progress.cache_hit
            && self.tool_route.is_some()
            && ((is_checkpoint_tool(tool_name) && (effective_success || has_canvas_receipt))
                || (tool_name == GET_COMPONENT_CATALOG_TOOL && effective_success))
        {
            if is_checkpoint_tool(tool_name)
                && self
                    .tool_route
                    .as_ref()
                    .is_some_and(|route| !update_tool_route(route, tool_name, result, effective_success))
            {
                return Flow::terminate("agent.task_checkpoint_invalid_result");
            }
            if self.checkpoint_tool_result(tool_name, result, now_ms()).await.is_err() {
                return Flow::terminate("agent.task_checkpoint_failed");
            }
        }
        if let Some(code) = fatal_failure_code {
            let payload = self.payload(
                ctx,
                json!({ "code": &code, "toolName": tool_name, "toolCallId": tool_call_id, "internalCallId": internal_call_id }),
            );
            let persisted = self.persist_or_terminate("agent.failed", None, payload).await;
            return if matches!(persisted, Flow::Continue) {
                Flow::terminate(code)
            } else {
                persisted
            };
        }
        if let Err(error) = budget_result {
            return self.terminate_for_budget(ctx, error).await;
        }
        if let Some(directive) = repeated_failure_directive(repeated_failure_action, tool_name, &semantic_outcome) {
            let persisted = self
                .persist_or_terminate(directive.event_kind, None, self.payload(ctx, directive.payload))
                .await;
            if !matches!(persisted, Flow::Continue) {
                return persisted;
            }
            return match directive.flow {
                RepeatedFailureFlow::RewriteResult(prompt) => {
                    Flow::rewrite_result(format!("{result_for_model}\n\n[Agent strategy correction]\n{prompt}"))
                }
                RepeatedFailureFlow::Terminate(code) => Flow::terminate(code),
            };
        }
        if tool_name == UPDATE_PLAN_TOOL && semantic_outcome.is_success() && !deferred_after_barrier {
            let plan = match serde_json::from_str::<Value>(result) {
                Ok(plan) => plan,
                Err(_) => return Flow::terminate("agent.plan.invalid_result"),
            };
            if self.persist_plan_item(ctx, internal_call_id, &plan).await.is_err() {
                return Flow::terminate("agent.item_persistence_failed");
            }
        }
        concise_result.map_or_else(Flow::cont, Flow::rewrite_result)
    }

    async fn checkpoint_tool_result(&self, tool_name: &str, result: &str, updated_at: i64) -> Result<(), TaskRepositoryError> {
        if tool_name == GET_COMPONENT_CATALOG_TOOL {
            let value: Value =
                serde_json::from_str(result).map_err(|error| TaskRepositoryError::InvalidRecord(format!("组件目录回执无效：{error}")))?;
            if value.get("catalogVersion").and_then(Value::as_str).is_none() || value.get("components").and_then(Value::as_array).is_none() {
                return Err(TaskRepositoryError::InvalidRecord("组件目录回执缺少 catalogVersion 或 components".to_string()));
            }
            return self
                .update_working_checkpoint(updated_at, move |state| {
                    state.component_catalog_result = Some(result.to_string());
                })
                .await;
        }

        if tool_name == UPDATE_PLAN_TOOL {
            let value: Value =
                serde_json::from_str(result).map_err(|error| TaskRepositoryError::InvalidRecord(format!("工作计划回执无效：{error}")))?;
            let plan = serde_json::from_value::<Vec<AgentWorkingPlanStep>>(value.get("plan").cloned().unwrap_or(Value::Null))
                .map_err(|error| TaskRepositoryError::InvalidRecord(format!("工作计划步骤无效：{error}")))?;
            let next_step = plan
                .iter()
                .find(|step| step.status == "in_progress")
                .or_else(|| plan.iter().find(|step| step.status == "pending"))
                .map(|step| step.step.clone());
            return self
                .update_working_checkpoint(updated_at, move |state| {
                    state.plan = plan;
                    state.next_step = next_step;
                })
                .await;
        }

        if matches!(
            tool_name,
            crate::backend::ai_driven::tools::file::APPLY_WORKSPACE_PATCH_TOOL
                | crate::backend::ai_driven::tools::file::MOVE_WORKSPACE_PATH_TOOL
                | crate::backend::ai_driven::tools::file::DELETE_WORKSPACE_PATH_TOOL
        ) {
            let value: Value =
                serde_json::from_str(result).map_err(|error| TaskRepositoryError::InvalidRecord(format!("Workspace 写入回执无效：{error}")))?;
            let revision = value
                .get("workspaceRevision")
                .and_then(Value::as_u64)
                .ok_or_else(|| TaskRepositoryError::InvalidRecord("Workspace 写入回执缺少 workspaceRevision".to_string()))?;
            return self
                .update_working_checkpoint(updated_at, move |state| {
                    state.workspace_write_started = true;
                    state.workspace_revision = Some(revision);
                    state.next_step = state
                        .plan
                        .iter()
                        .find(|step| step.status == "in_progress" || step.status == "pending")
                        .map(|step| step.step.clone());
                })
                .await;
        }

        let receipt: CanvasOperationReceipt =
            serde_json::from_str(result).map_err(|error| TaskRepositoryError::InvalidRecord(format!("画布动作回执无效：{error}")))?;
        if receipt.status == CanvasOperationStatus::Applied {
            // 首个真实 Applied 回执到达后记录写入事实，不能等待最终完成门禁。
            self.completion_requirements.lock_after_canvas_write();
        }
        self.update_working_checkpoint(updated_at, move |state| {
            if !state
                .operation_ids
                .iter()
                .any(|operation_id| operation_id == &receipt.operation_id)
            {
                state.operation_ids.push(receipt.operation_id.clone());
            }
            if let Some(revision) = receipt.result_revision.clone() {
                state.canvas_revision = Some(revision);
            }
            match receipt.status {
                CanvasOperationStatus::Prepared | CanvasOperationStatus::Applying => {
                    state.next_step = Some(format!("等待画布操作 {} 完成", receipt.operation_id));
                }
                CanvasOperationStatus::Applied => {
                    state.canvas_write_started = true;
                    state.next_step = state
                        .plan
                        .iter()
                        .find(|step| step.status == "in_progress" || step.status == "pending")
                        .map(|step| step.step.clone());
                }
                CanvasOperationStatus::Rejected | CanvasOperationStatus::Failed => {
                    state.next_step = Some(format!("处理画布操作 {} 的失败回执", receipt.operation_id));
                }
            }
        })
        .await
    }

    async fn terminate_for_progress(&self, ctx: &HookContext, violation: ProgressViolation) -> Flow {
        let code = violation.code.as_str();
        let payload = self.payload(
            ctx,
            json!({
                "code": code,
                "message": violation.message,
                "progress": violation,
                "usage": self.budget.usage_snapshot(),
            }),
        );
        let persisted = self.persist_or_terminate("agent.failed", None, payload).await;
        if matches!(persisted, Flow::Continue) {
            Flow::terminate(code)
        } else {
            persisted
        }
    }
}

fn is_checkpoint_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        UPDATE_PLAN_TOOL
            | CREATE_CANVAS_COMPONENTS_TOOL
            | PATCH_CANVAS_COMPONENTS_TOOL
            | DELETE_CANVAS_COMPONENTS_TOOL
            | GET_CANVAS_OPERATION_STATUS_TOOL
            | crate::backend::ai_driven::tools::file::APPLY_WORKSPACE_PATCH_TOOL
            | crate::backend::ai_driven::tools::file::MOVE_WORKSPACE_PATH_TOOL
            | crate::backend::ai_driven::tools::file::DELETE_WORKSPACE_PATH_TOOL
    )
}

fn is_canvas_operation_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        CREATE_CANVAS_COMPONENTS_TOOL | PATCH_CANVAS_COMPONENTS_TOOL | DELETE_CANVAS_COMPONENTS_TOOL | GET_CANVAS_OPERATION_STATUS_TOOL
    )
}

fn is_failed_canvas_receipt(tool_name: &str, result: &str) -> bool {
    if !is_canvas_operation_tool(tool_name) {
        return false;
    }
    serde_json::from_str::<CanvasOperationReceipt>(result)
        .ok()
        .is_some_and(|receipt| matches!(receipt.status, CanvasOperationStatus::Rejected | CanvasOperationStatus::Failed))
}

/// Rig reports a successful tool return when the tool produced a structured
/// rejected/failed canvas receipt. Normalize that receipt before budgets,
/// failure policy, logs, and completion handling consume the result.
fn semantic_tool_outcome(tool_name: &str, result: &str, outcome: &ToolOutcome) -> ToolOutcome {
    let Some(receipt) = serde_json::from_str::<CanvasOperationReceipt>(result)
        .ok()
        .filter(|_| is_canvas_operation_tool(tool_name))
    else {
        return outcome.clone();
    };
    // Prepared/Applying are normal in-flight states.  The status query can
    // legitimately return either one while the operation is still pending;
    // do not turn that receipt into a synthetic failure or consume the
    // repeated-failure budget.  Preserve any actual Rig error unchanged.
    if matches!(receipt.status, CanvasOperationStatus::Prepared | CanvasOperationStatus::Applying) {
        return outcome.clone();
    }
    let Some(code) = receipt
        .error
        .as_ref()
        .map(|error| error.code.trim())
        .filter(|code| !code.is_empty())
    else {
        if receipt.status == CanvasOperationStatus::Applied {
            return outcome.clone();
        }
        return ToolOutcome::Error(
            ToolFailure::other("画布动作返回了失败终态回执")
                .with_retryable(true)
                .with_code(receipt.status.as_str()),
        );
    };
    match receipt.status {
        CanvasOperationStatus::Rejected => {
            let normalized = code.to_ascii_lowercase();
            if normalized.contains("permission") || normalized.contains("forbidden") || normalized.contains("scope") {
                ToolOutcome::Error(ToolFailure::permission_denied("画布动作被当前任务能力拒绝").with_code(code))
            } else if normalized.contains("cancel") || normalized.contains("reject") || normalized.contains("user") {
                ToolOutcome::Error(ToolFailure::cancelled("用户拒绝了画布动作").with_code(code))
            } else {
                ToolOutcome::Error(ToolFailure::other("画布动作被拒绝").with_retryable(true).with_code(code))
            }
        }
        CanvasOperationStatus::Failed => ToolOutcome::Error(ToolFailure::other("画布动作执行失败").with_retryable(true).with_code(code)),
        _ => outcome.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::agent_context::input::AgentRunInput;
    use crate::backend::ai_driven::domain::operation::CanvasOperationKind;

    #[test]
    fn legacy_execution_scope_receipts_do_not_change_the_route() {
        let route = AgentToolRoute::new(AgentRunInput::new("创建组件"), false);
        let plan = r#"{"plan":[{"step":"创建组件","status":"in_progress"}]}"#;

        assert!(!update_tool_route(&route, UPDATE_PLAN_TOOL, "not-json", true));
        assert!(update_tool_route(&route, UPDATE_PLAN_TOOL, plan, true));
        assert!(route
            .current_plan()
            .active_tools()
            .iter()
            .any(|tool| tool == CREATE_CANVAS_COMPONENTS_TOOL));
        assert!(!update_tool_route(&route, "declare_execution_scope", "not-json", true));
        assert!(!update_tool_route(&route, "declare_execution_scope", "{}", true));
        assert!(route
            .current_plan()
            .active_tools()
            .iter()
            .any(|tool| tool == CREATE_CANVAS_COMPONENTS_TOOL));
        assert!(!update_tool_route(&route, UPDATE_PLAN_TOOL, plan, false));
    }

    #[test]
    fn rejected_canvas_receipt_is_a_failed_tool_result_even_when_rig_succeeds() {
        let mut receipt = CanvasOperationReceipt::prepared(
            "op-1".to_string(),
            "call-1".to_string(),
            CanvasOperationKind::CreateComponents,
            "1".to_string(),
            None,
            vec!["draft-1".to_string()],
        );
        receipt.status = CanvasOperationStatus::Rejected;
        let result = serde_json::to_string(&receipt).unwrap();

        assert!(is_failed_canvas_receipt(CREATE_CANVAS_COMPONENTS_TOOL, &result));
        assert!(!is_failed_canvas_receipt("inspect_current_page", &result));
        assert!(matches!(
            semantic_tool_outcome(CREATE_CANVAS_COMPONENTS_TOOL, &result, &ToolOutcome::Success),
            ToolOutcome::Error(_)
        ));
    }

    #[test]
    fn failed_canvas_receipt_is_a_failed_tool_result_without_error_details() {
        let mut receipt = CanvasOperationReceipt::prepared(
            "op-1".to_string(),
            "call-1".to_string(),
            CanvasOperationKind::PatchComponents,
            "1".to_string(),
            None,
            vec!["component-1".to_string()],
        );
        receipt.status = CanvasOperationStatus::Failed;
        let result = serde_json::to_string(&receipt).unwrap();

        assert!(is_failed_canvas_receipt(PATCH_CANVAS_COMPONENTS_TOOL, &result));
        assert!(matches!(
            semantic_tool_outcome(PATCH_CANVAS_COMPONENTS_TOOL, &result, &ToolOutcome::Success),
            ToolOutcome::Error(_)
        ));
    }

    #[test]
    fn prepared_and_applying_canvas_receipts_remain_pending_successes() {
        let mut failure_tracker = crate::backend::ai_driven::agent::tool_failure_policy::ToolFailureTracker::default();
        for status in [CanvasOperationStatus::Prepared, CanvasOperationStatus::Applying] {
            let mut receipt = CanvasOperationReceipt::prepared(
                format!("op-{}", status.as_str()),
                "call-1".to_string(),
                CanvasOperationKind::CreateComponents,
                "1".to_string(),
                None,
                vec!["draft-1".to_string()],
            );
            receipt.status = status;
            let result = serde_json::to_string(&receipt).unwrap();

            let semantic_outcome = semantic_tool_outcome(CREATE_CANVAS_COMPONENTS_TOOL, &result, &ToolOutcome::Success);
            assert!(matches!(semantic_outcome, ToolOutcome::Success));
            assert_eq!(
                failure_tracker.record_attempt(CREATE_CANVAS_COMPONENTS_TOOL, &semantic_outcome, &result, "{}", 0),
                RepeatedFailureAction::Continue
            );
        }
        assert_eq!(failure_tracker.snapshot()["counts"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn pending_canvas_receipts_preserve_actual_tool_errors() {
        let mut receipt = CanvasOperationReceipt::prepared(
            "op-1".to_string(),
            "call-1".to_string(),
            CanvasOperationKind::CreateComponents,
            "1".to_string(),
            None,
            vec!["draft-1".to_string()],
        );
        receipt.status = CanvasOperationStatus::Applying;
        let result = serde_json::to_string(&receipt).unwrap();
        let actual_error = ToolOutcome::Error(ToolFailure::invalid_args("invalid canvas arguments").with_code("invalid_args"));

        assert_eq!(semantic_tool_outcome(CREATE_CANVAS_COMPONENTS_TOOL, &result, &actual_error), actual_error);
    }
}
