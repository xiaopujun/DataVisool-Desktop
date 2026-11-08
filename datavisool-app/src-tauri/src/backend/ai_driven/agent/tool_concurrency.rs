/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Agent Tool 的显式并发策略和任务级调度闸门。
//!
//! Tool 默认按独占方式执行。只有明确列入 `parallel_safe_tool` 的只读 Tool
//! 使用共享读锁；用户交互、前端查询和所有写入 Tool 均保持独占执行。

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rig_core::tool::{ToolCallExtensions, ToolDyn, ToolError, ToolExecutionResult};
use rig_core::wasm_compat::WasmBoxedFuture;
use serde_json::Value;

use crate::backend::ai_driven::tools::canvas::{CREATE_CANVAS_COMPONENTS_TOOL, DELETE_CANVAS_COMPONENTS_TOOL, PATCH_CANVAS_COMPONENTS_TOOL};
use crate::backend::ai_driven::tools::file::{LIST_WORKSPACE_FILES_TOOL, READ_WORKSPACE_FILE_TOOL, SEARCH_WORKSPACE_FILES_TOOL};
use crate::backend::ai_driven::tools::interaction::REQUEST_USER_INPUT_TOOL;
use crate::backend::ai_driven::tools::selection::REQUEST_COMPONENT_SELECTION_TOOL;

/// Tool 是否可以和同一模型步骤中的其它 Tool 重叠执行。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolConcurrencyPolicy {
    /// 明确没有共享写入，可以持有共享读锁。
    ParallelSafe,
    /// 独占执行，不能和任何其它 Tool 重叠。
    Exclusive,
    /// 独占执行，并要求调用方在结果后结束当前步骤。
    Barrier,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ToolBatchAdmission {
    pub position: u64,
    pub deferred_after_barrier: bool,
    pub barrier_position: Option<u64>,
}

#[derive(Clone, Default)]
pub(crate) struct ToolBatchTracker {
    state: Arc<Mutex<ToolBatchState>>,
    turn_notify: Arc<tokio::sync::Notify>,
    completion_notify: Arc<tokio::sync::Notify>,
}

#[derive(Default)]
struct ToolBatchState {
    snapshot_generation: u64,
    next_position: u64,
    next_hook_position: u64,
    next_incomplete_position: u64,
    barrier_position: Option<u64>,
    observed_internal_ids: HashMap<String, VecDeque<String>>,
    prepared_calls: Vec<PreparedToolBatchCall>,
    finished_hook_positions: BTreeSet<u64>,
    completed_positions: BTreeSet<u64>,
    aborted_positions: BTreeSet<u64>,
}

struct PreparedToolBatchCall {
    internal_call_id: Option<String>,
    tool_call_id: Option<String>,
    tool_name: String,
    arguments: String,
    admission: ToolBatchAdmission,
    consumed: bool,
}

impl ToolBatchTracker {
    pub(crate) fn reset(&self, snapshot_generation: u64) {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *state = ToolBatchState { snapshot_generation, ..ToolBatchState::default() };
        drop(state);
        self.turn_notify.notify_waiters();
        self.completion_notify.notify_waiters();
    }

    /// 记录流式响应为当前 Tool Call 分配的 Rig 内部身份。
    pub(crate) fn record_stream_call(&self, snapshot_generation: u64, content_id: &str, internal_call_id: &str) {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.snapshot_generation != snapshot_generation {
            *state = ToolBatchState { snapshot_generation, ..ToolBatchState::default() };
        }
        let queue = state.observed_internal_ids.entry(content_id.to_string()).or_default();
        if !queue.iter().any(|queued| queued == internal_call_id) {
            queue.push_back(internal_call_id.to_string());
        }
    }

    /// 在并发 Tool Hook 启动前，按模型响应中的原始顺序预分配批次位置。
    pub(crate) fn prepare(&self, snapshot_generation: u64, calls: &[(String, Option<String>, String, String)]) {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut observed_internal_ids = if state.snapshot_generation == snapshot_generation {
            std::mem::take(&mut state.observed_internal_ids)
        } else {
            HashMap::new()
        };
        *state = ToolBatchState { snapshot_generation, ..ToolBatchState::default() };
        for (position, (content_id, tool_call_id, tool_name, arguments)) in calls.iter().enumerate() {
            let position = position as u64;
            let deferred_after_barrier = state.barrier_position.is_some();
            if !deferred_after_barrier && tool_concurrency_policy(tool_name) == ToolConcurrencyPolicy::Barrier {
                state.barrier_position = Some(position);
            }
            let admission = ToolBatchAdmission { position, deferred_after_barrier, barrier_position: state.barrier_position };
            let internal_call_id = observed_internal_ids.get_mut(content_id).and_then(VecDeque::pop_front);
            state.prepared_calls.push(PreparedToolBatchCall {
                internal_call_id,
                tool_call_id: tool_call_id.clone(),
                tool_name: tool_name.clone(),
                arguments: arguments.clone(),
                admission,
                consumed: false,
            });
            state.next_position = position.saturating_add(1);
        }
        drop(state);
        self.turn_notify.notify_waiters();
        self.completion_notify.notify_waiters();
    }

    pub(crate) fn admit(
        &self,
        snapshot_generation: u64,
        tool_name: &str,
        tool_call_id: Option<&str>,
        internal_call_id: &str,
        args: &str,
        policy: ToolConcurrencyPolicy,
    ) -> ToolBatchAdmission {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.snapshot_generation != snapshot_generation {
            *state = ToolBatchState { snapshot_generation, ..ToolBatchState::default() };
        }
        let arguments = canonical_arguments(args);
        let exact_internal = state
            .prepared_calls
            .iter()
            .position(|call| !call.consumed && call.internal_call_id.as_deref() == Some(internal_call_id));
        let exact_provider = tool_call_id.and_then(|tool_call_id| {
            state
                .prepared_calls
                .iter()
                .position(|call| !call.consumed && call.tool_name == tool_name && call.tool_call_id.as_deref() == Some(tool_call_id))
        });
        let exact_arguments = state
            .prepared_calls
            .iter()
            .position(|call| !call.consumed && call.tool_name == tool_name && call.arguments == arguments);
        let same_tool = state
            .prepared_calls
            .iter()
            .position(|call| !call.consumed && call.tool_name == tool_name);
        if let Some(index) = exact_internal.or(exact_provider).or(exact_arguments).or(same_tool) {
            state.prepared_calls[index].consumed = true;
            return state.prepared_calls[index].admission;
        }
        let position = state.next_position;
        state.next_position = state.next_position.saturating_add(1);
        let deferred_after_barrier = state.barrier_position.is_some();
        if !deferred_after_barrier && policy == ToolConcurrencyPolicy::Barrier {
            state.barrier_position = Some(position);
        }
        ToolBatchAdmission { position, deferred_after_barrier, barrier_position: state.barrier_position }
    }

    /// 并发 Hook 可以乱序抵达，但进入持久化与 Tool body 前必须按模型顺序放行。
    pub(crate) async fn wait_for_turn(&self, snapshot_generation: u64, position: u64) -> Result<ToolBatchTurnGuard, ToolBatchWaitError> {
        let guard = ToolBatchTurnGuard { tracker: self.clone(), snapshot_generation, position };
        loop {
            let notified = self.turn_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let decision = {
                let state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if state.snapshot_generation != snapshot_generation {
                    Some(Err(ToolBatchWaitError::GenerationChanged))
                } else if state.next_hook_position == position {
                    Some(Ok(()))
                } else {
                    None
                }
            };
            if let Some(decision) = decision {
                decision?;
                return Ok(guard);
            }
            notified.await;
        }
    }

    pub(crate) async fn wait_for_prior_completion(&self, snapshot_generation: u64, position: u64) -> Result<(), ToolBatchWaitError> {
        loop {
            let notified = self.completion_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let decision = {
                let state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if state.snapshot_generation != snapshot_generation {
                    Some(Err(ToolBatchWaitError::GenerationChanged))
                } else if state.next_incomplete_position >= position {
                    let aborted = state.aborted_positions.range(..position).next().is_some();
                    Some(if aborted { Err(ToolBatchWaitError::PriorCallAborted) } else { Ok(()) })
                } else {
                    None
                }
            };
            if let Some(decision) = decision {
                return decision;
            }
            notified.await;
        }
    }

    pub(crate) fn completion_guard(&self, snapshot_generation: u64, position: u64) -> ToolBatchCompletionGuard {
        ToolBatchCompletionGuard { tracker: self.clone(), snapshot_generation, position, aborted: true }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolBatchWaitError {
    PriorCallAborted,
    GenerationChanged,
}

pub(crate) struct ToolBatchTurnGuard {
    tracker: ToolBatchTracker,
    snapshot_generation: u64,
    position: u64,
}

impl Drop for ToolBatchTurnGuard {
    fn drop(&mut self) {
        let advanced = {
            let mut state = self.tracker.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.snapshot_generation != self.snapshot_generation {
                return;
            }
            state.finished_hook_positions.insert(self.position);
            let mut advanced = false;
            loop {
                let next = state.next_hook_position;
                if !state.finished_hook_positions.remove(&next) {
                    break;
                }
                state.next_hook_position = state.next_hook_position.saturating_add(1);
                advanced = true;
            }
            advanced
        };
        if advanced {
            self.tracker.turn_notify.notify_waiters();
        }
    }
}

pub(crate) struct ToolBatchCompletionGuard {
    tracker: ToolBatchTracker,
    snapshot_generation: u64,
    position: u64,
    aborted: bool,
}

impl ToolBatchCompletionGuard {
    pub(crate) fn activate(&mut self) {
        self.aborted = false;
    }
}

impl Drop for ToolBatchCompletionGuard {
    fn drop(&mut self) {
        let advanced = {
            let mut state = self.tracker.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.snapshot_generation != self.snapshot_generation {
                return;
            }
            if self.aborted {
                state.aborted_positions.insert(self.position);
            }
            state.completed_positions.insert(self.position);
            let mut advanced = false;
            loop {
                let next = state.next_incomplete_position;
                if !state.completed_positions.remove(&next) {
                    break;
                }
                state.next_incomplete_position = state.next_incomplete_position.saturating_add(1);
                advanced = true;
            }
            advanced
        };
        if advanced {
            self.tracker.completion_notify.notify_waiters();
        }
    }
}

pub(crate) fn canonical_arguments(args: &str) -> String {
    serde_json::from_str::<Value>(args)
        .ok()
        .and_then(|value| serde_json::to_string(&value).ok())
        .unwrap_or_else(|| args.to_string())
}

impl Default for ToolConcurrencyPolicy {
    fn default() -> Self {
        Self::Exclusive
    }
}

impl ToolConcurrencyPolicy {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ParallelSafe => "parallelSafe",
            Self::Exclusive => "exclusive",
            Self::Barrier => "barrier",
        }
    }
}

/// 返回 Tool 的静态并发策略。
///
/// 只把静态文档读取和 Workspace 读取列入并发白名单。前端读取也可能更新
/// 快照基线或依赖前端状态，因此默认保持独占；未知 Tool 同样默认独占。
pub(crate) fn tool_concurrency_policy(tool_name: &str) -> ToolConcurrencyPolicy {
    if barrier_tool(tool_name) {
        ToolConcurrencyPolicy::Barrier
    } else if parallel_safe_tool(tool_name) {
        ToolConcurrencyPolicy::ParallelSafe
    } else {
        ToolConcurrencyPolicy::Exclusive
    }
}

fn barrier_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        REQUEST_USER_INPUT_TOOL
            | CREATE_CANVAS_COMPONENTS_TOOL
            | PATCH_CANVAS_COMPONENTS_TOOL
            | DELETE_CANVAS_COMPONENTS_TOOL
            | REQUEST_COMPONENT_SELECTION_TOOL
    )
}

fn parallel_safe_tool(tool_name: &str) -> bool {
    matches!(tool_name, LIST_WORKSPACE_FILES_TOOL | SEARCH_WORKSPACE_FILES_TOOL | READ_WORKSPACE_FILE_TOOL)
}

/// 同一 Agent Run 共享的调度闸门。
///
/// Tokio 的 `RwLock` 是写者优先的公平锁：等待中的独占调用会阻止新的读者
/// 插队。读/写 guard 都在被包装的 Future 结束、失败或取消时自动释放。
#[derive(Clone, Default)]
pub(crate) struct ToolCallScheduler {
    gate: Arc<tokio::sync::RwLock<()>>,
    calls: Arc<Mutex<ToolScheduleState>>,
}

#[derive(Default)]
struct ToolScheduleState {
    pending_by_tool: HashMap<String, VecDeque<String>>,
    observations: HashMap<String, ToolScheduleObservation>,
}

struct ToolScheduleObservation {
    tool_name: String,
    tool_call_id: Option<String>,
    step_index: u64,
    route_revision: u64,
    admitted_phase: String,
    admission_source: String,
    policy: ToolConcurrencyPolicy,
    batch_position: u64,
    admitted_at: Instant,
    acquired_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ToolScheduleTiming {
    pub wait_duration: Duration,
    pub execution_duration: Duration,
    pub acquired: bool,
}

impl ToolCallScheduler {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn register_call(
        &self,
        tool_name: &str,
        tool_call_id: Option<&str>,
        internal_call_id: &str,
        step_index: u64,
        route_revision: u64,
        admitted_phase: &str,
        admission_source: &str,
        policy: ToolConcurrencyPolicy,
        batch_position: u64,
    ) {
        let mut state = self.lock_calls();
        state
            .pending_by_tool
            .entry(tool_name.to_string())
            .or_default()
            .push_back(internal_call_id.to_string());
        state.observations.insert(
            internal_call_id.to_string(),
            ToolScheduleObservation {
                tool_name: tool_name.to_string(),
                tool_call_id: tool_call_id.map(str::to_string),
                step_index,
                route_revision,
                admitted_phase: admitted_phase.to_string(),
                admission_source: admission_source.to_string(),
                policy,
                batch_position,
                admitted_at: Instant::now(),
                acquired_at: None,
            },
        );
    }

    pub(crate) fn finish_call(&self, internal_call_id: &str) -> Option<ToolScheduleTiming> {
        let now = Instant::now();
        let mut state = self.lock_calls();
        let observation = state.observations.remove(internal_call_id)?;
        if observation.acquired_at.is_none() {
            if let Some(queue) = state.pending_by_tool.get_mut(&observation.tool_name) {
                if let Some(index) = queue.iter().position(|queued| queued == internal_call_id) {
                    queue.remove(index);
                }
            }
        }
        let acquired_at = observation.acquired_at;
        Some(ToolScheduleTiming {
            wait_duration: acquired_at
                .map(|acquired_at| acquired_at.saturating_duration_since(observation.admitted_at))
                .unwrap_or_else(|| now.saturating_duration_since(observation.admitted_at)),
            execution_duration: acquired_at
                .map(|acquired_at| now.saturating_duration_since(acquired_at))
                .unwrap_or(Duration::ZERO),
            acquired: acquired_at.is_some(),
        })
    }

    fn mark_acquired(&self, tool_name: &str) {
        let acquired_at = Instant::now();
        let observation = {
            let mut state = self.lock_calls();
            let internal_call_id = state.pending_by_tool.get_mut(tool_name).and_then(VecDeque::pop_front);
            let Some(internal_call_id) = internal_call_id else {
                return;
            };
            if state.pending_by_tool.get(tool_name).is_some_and(VecDeque::is_empty) {
                state.pending_by_tool.remove(tool_name);
            }
            state.observations.get_mut(&internal_call_id).map(|observation| {
                observation.acquired_at = Some(acquired_at);
                (
                    internal_call_id,
                    observation.tool_call_id.clone(),
                    observation.step_index,
                    observation.route_revision,
                    observation.admitted_phase.clone(),
                    observation.admission_source.clone(),
                    observation.policy,
                    observation.batch_position,
                    acquired_at.saturating_duration_since(observation.admitted_at),
                )
            })
        };
        if let Some((internal_call_id, tool_call_id, step_index, route_revision, admitted_phase, admission_source, policy, batch_position, wait)) =
            observation
        {
            tracing::debug!(
                target: "agent.tool",
                event = "tool_call_scheduler_acquired",
                step_index,
                route_revision,
                admitted_phase,
                admission_source,
                tool_name,
                tool_call_id,
                internal_call_id,
                concurrency_policy = policy.as_str(),
                batch_position,
                scheduler_state = "acquired",
                scheduler_wait_ms = duration_ms(wait),
                "Agent tool call acquired scheduler gate"
            );
        }
    }

    fn lock_calls(&self) -> MutexGuard<'_, ToolScheduleState> {
        self.calls.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 使用一个任务级 Scheduler 包装 Tool 列表。
pub(crate) fn with_tool_concurrency(tools: Vec<Box<dyn ToolDyn>>, scheduler: Arc<ToolCallScheduler>) -> Vec<Box<dyn ToolDyn>> {
    tools
        .into_iter()
        .map(|tool| {
            let policy = tool_concurrency_policy(&tool.name());
            Box::new(ScheduledTool { inner: tool, policy, scheduler: scheduler.clone() }) as Box<dyn ToolDyn>
        })
        .collect()
}

struct ScheduledTool {
    inner: Box<dyn ToolDyn>,
    policy: ToolConcurrencyPolicy,
    scheduler: Arc<ToolCallScheduler>,
}

impl ToolDyn for ScheduledTool {
    fn name(&self) -> String {
        self.inner.name()
    }

    fn description(&self) -> String {
        self.inner.description()
    }

    fn parameters(&self) -> Value {
        self.inner.parameters()
    }

    fn call<'a>(&'a self, args: String) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
        Box::pin(async move {
            match self.policy {
                ToolConcurrencyPolicy::ParallelSafe => {
                    let _guard = self.scheduler.gate.read().await;
                    self.scheduler.mark_acquired(&self.inner.name());
                    self.inner.call(args).await
                }
                ToolConcurrencyPolicy::Exclusive | ToolConcurrencyPolicy::Barrier => {
                    let _guard = self.scheduler.gate.write().await;
                    self.scheduler.mark_acquired(&self.inner.name());
                    self.inner.call(args).await
                }
            }
        })
    }

    fn call_with_extensions<'a>(&'a self, args: String, extensions: &'a ToolCallExtensions) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
        Box::pin(async move {
            match self.policy {
                ToolConcurrencyPolicy::ParallelSafe => {
                    let _guard = self.scheduler.gate.read().await;
                    self.scheduler.mark_acquired(&self.inner.name());
                    self.inner.call_with_extensions(args, extensions).await
                }
                ToolConcurrencyPolicy::Exclusive | ToolConcurrencyPolicy::Barrier => {
                    let _guard = self.scheduler.gate.write().await;
                    self.scheduler.mark_acquired(&self.inner.name());
                    self.inner.call_with_extensions(args, extensions).await
                }
            }
        })
    }

    fn call_structured<'a>(&'a self, args: String, extensions: &'a ToolCallExtensions) -> WasmBoxedFuture<'a, ToolExecutionResult> {
        Box::pin(async move {
            match self.policy {
                ToolConcurrencyPolicy::ParallelSafe => {
                    let _guard = self.scheduler.gate.read().await;
                    self.scheduler.mark_acquired(&self.inner.name());
                    self.inner.call_structured(args, extensions).await
                }
                ToolConcurrencyPolicy::Exclusive | ToolConcurrencyPolicy::Barrier => {
                    let _guard = self.scheduler.gate.write().await;
                    self.scheduler.mark_acquired(&self.inner.name());
                    self.inner.call_structured(args, extensions).await
                }
            }
        })
    }
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;
    use crate::backend::ai_driven::tools::interaction::{INSPECT_CURRENT_PAGE_TOOL, REQUEST_USER_INPUT_TOOL};
    use rig_core::tool::{ToolCallExtensions, ToolError, ToolExecutionResult};
    use serde_json::json;

    #[derive(Default)]
    struct ProbeState {
        active: AtomicUsize,
        max_active: AtomicUsize,
        started: Mutex<Vec<String>>,
        started_count: AtomicUsize,
    }

    struct ProbeTool {
        name: String,
        state: Arc<ProbeState>,
        delay: Option<Duration>,
        block: bool,
    }

    impl ProbeTool {
        fn new(name: &str, state: Arc<ProbeState>, delay: Option<Duration>) -> Self {
            Self { name: name.to_string(), state, delay, block: false }
        }

        fn blocking(name: &str, state: Arc<ProbeState>) -> Self {
            Self { name: name.to_string(), state, delay: None, block: true }
        }
    }

    impl ToolDyn for ProbeTool {
        fn name(&self) -> String {
            self.name.clone()
        }

        fn description(&self) -> String {
            "test probe".to_string()
        }

        fn parameters(&self) -> Value {
            json!({"type":"object"})
        }

        fn call<'a>(&'a self, args: String) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
            let state = self.state.clone();
            let delay = self.delay;
            let block = self.block;
            Box::pin(async move {
                let marker = args;
                let active = state.active.fetch_add(1, Ordering::AcqRel) + 1;
                state.max_active.fetch_max(active, Ordering::AcqRel);
                state.started.lock().unwrap().push(marker);
                state.started_count.fetch_add(1, Ordering::AcqRel);
                if block {
                    pending::<()>().await;
                } else if let Some(delay) = delay {
                    tokio::time::sleep(delay).await;
                }
                state.active.fetch_sub(1, Ordering::AcqRel);
                Ok("ok".to_string())
            })
        }

        fn call_structured<'a>(&'a self, args: String, extensions: &'a ToolCallExtensions) -> WasmBoxedFuture<'a, ToolExecutionResult> {
            Box::pin(async move {
                match self.call_with_extensions(args, extensions).await {
                    Ok(value) => ToolExecutionResult::success(value),
                    Err(error) => ToolExecutionResult::failed(error.to_string(), rig_core::tool::ToolFailure::other(error.to_string())),
                }
            })
        }
    }

    async fn run(tool: &dyn ToolDyn, marker: &str) {
        let result = tool.call_structured(marker.to_string(), &ToolCallExtensions::new()).await;
        assert!(result.outcome().is_success());
    }

    async fn wait_for_started(state: &ProbeState, expected: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while state.started_count.load(Ordering::Acquire) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("probe tool did not start in time");
    }

    #[test]
    fn policy_defaults_to_exclusive_and_only_whitelists_reads() {
        assert_eq!(ToolConcurrencyPolicy::default(), ToolConcurrencyPolicy::Exclusive);
        assert_eq!(tool_concurrency_policy("future_tool"), ToolConcurrencyPolicy::Exclusive);
        assert_eq!(tool_concurrency_policy(INSPECT_CURRENT_PAGE_TOOL), ToolConcurrencyPolicy::Exclusive);
        assert_eq!(tool_concurrency_policy(REQUEST_USER_INPUT_TOOL), ToolConcurrencyPolicy::Barrier);
        assert_eq!(tool_concurrency_policy(CREATE_CANVAS_COMPONENTS_TOOL), ToolConcurrencyPolicy::Barrier);
        assert_eq!(tool_concurrency_policy(PATCH_CANVAS_COMPONENTS_TOOL), ToolConcurrencyPolicy::Barrier);
        assert_eq!(tool_concurrency_policy("get_component_document"), ToolConcurrencyPolicy::Exclusive);
        assert_eq!(tool_concurrency_policy("search_component_documents"), ToolConcurrencyPolicy::Exclusive);
        assert_eq!(tool_concurrency_policy(LIST_WORKSPACE_FILES_TOOL), ToolConcurrencyPolicy::ParallelSafe);
        assert_eq!(tool_concurrency_policy(SEARCH_WORKSPACE_FILES_TOOL), ToolConcurrencyPolicy::ParallelSafe);
        assert_eq!(tool_concurrency_policy(READ_WORKSPACE_FILE_TOOL), ToolConcurrencyPolicy::ParallelSafe);
    }

    #[test]
    fn prepared_batch_keeps_model_order_when_hooks_arrive_out_of_order() {
        let tracker = ToolBatchTracker::default();
        tracker.prepare(
            1,
            &[
                ("call-1".to_string(), None, REQUEST_USER_INPUT_TOOL.to_string(), "{}".to_string()),
                ("call-2".to_string(), None, READ_WORKSPACE_FILE_TOOL.to_string(), r#"{"path":"after.md"}"#.to_string()),
            ],
        );

        let later = tracker.admit(
            1,
            READ_WORKSPACE_FILE_TOOL,
            None,
            "internal-2",
            r#"{ "path": "after.md" }"#,
            ToolConcurrencyPolicy::ParallelSafe,
        );
        assert_eq!(later.position, 1);
        assert!(later.deferred_after_barrier);
        assert_eq!(later.barrier_position, Some(0));

        let barrier = tracker.admit(1, REQUEST_USER_INPUT_TOOL, None, "internal-1", "{}", ToolConcurrencyPolicy::Barrier);
        assert_eq!(barrier.position, 0);
        assert!(!barrier.deferred_after_barrier);
    }

    #[test]
    fn duplicate_calls_bind_to_stream_internal_identity() {
        let tracker = ToolBatchTracker::default();
        tracker.reset(4);
        tracker.record_stream_call(4, "content-1", "internal-1");
        tracker.record_stream_call(4, "content-2", "internal-2");
        tracker.prepare(
            4,
            &[
                ("content-1".to_string(), None, READ_WORKSPACE_FILE_TOOL.to_string(), r#"{"path":"same.md"}"#.to_string()),
                ("content-2".to_string(), None, READ_WORKSPACE_FILE_TOOL.to_string(), r#"{"path":"same.md"}"#.to_string()),
            ],
        );

        let second = tracker.admit(4, READ_WORKSPACE_FILE_TOOL, None, "internal-2", r#"{"path":"same.md"}"#, ToolConcurrencyPolicy::ParallelSafe);
        let first = tracker.admit(4, READ_WORKSPACE_FILE_TOOL, None, "internal-1", r#"{"path":"same.md"}"#, ToolConcurrencyPolicy::ParallelSafe);

        assert_eq!(second.position, 1);
        assert_eq!(first.position, 0);
    }

    #[tokio::test]
    async fn hook_turns_are_released_in_model_order() {
        let tracker = ToolBatchTracker::default();
        tracker.prepare(
            2,
            &[
                ("call-1".to_string(), None, LIST_WORKSPACE_FILES_TOOL.to_string(), "{}".to_string()),
                ("call-2".to_string(), None, READ_WORKSPACE_FILE_TOOL.to_string(), r#"{"path":"later.md"}"#.to_string()),
            ],
        );
        let later = tracker.admit(2, READ_WORKSPACE_FILE_TOOL, None, "internal-2", r#"{"path":"later.md"}"#, ToolConcurrencyPolicy::ParallelSafe);
        let earlier = tracker.admit(2, LIST_WORKSPACE_FILES_TOOL, None, "internal-1", "{}", ToolConcurrencyPolicy::ParallelSafe);
        let later_tracker = tracker.clone();
        let later_task = tokio::spawn(async move {
            let _turn = later_tracker.wait_for_turn(2, later.position).await.unwrap();
        });
        tokio::task::yield_now().await;
        assert!(!later_task.is_finished());

        drop(tracker.wait_for_turn(2, earlier.position).await.unwrap());
        tokio::time::timeout(Duration::from_secs(1), later_task)
            .await
            .expect("later hook should be released")
            .unwrap();
    }

    #[tokio::test]
    async fn generation_change_wakes_hook_and_completion_waiters() {
        let tracker = ToolBatchTracker::default();
        tracker.prepare(
            6,
            &[
                ("call-1".to_string(), None, "update_plan".to_string(), "{}".to_string()),
                ("call-2".to_string(), None, REQUEST_USER_INPUT_TOOL.to_string(), "{}".to_string()),
            ],
        );
        let hook_tracker = tracker.clone();
        let hook_waiter = tokio::spawn(async move { hook_tracker.wait_for_turn(6, 1).await });
        let completion_tracker = tracker.clone();
        let completion_waiter = tokio::spawn(async move { completion_tracker.wait_for_prior_completion(6, 1).await });
        tokio::task::yield_now().await;

        tracker.reset(7);

        assert!(matches!(hook_waiter.await.unwrap(), Err(ToolBatchWaitError::GenerationChanged)));
        assert!(matches!(completion_waiter.await.unwrap(), Err(ToolBatchWaitError::GenerationChanged)));
    }

    #[tokio::test]
    async fn barrier_waits_until_every_prior_result_is_finished() {
        let tracker = ToolBatchTracker::default();
        tracker.prepare(
            3,
            &[
                ("call-1".to_string(), None, "update_plan".to_string(), "{}".to_string()),
                ("call-2".to_string(), None, REQUEST_USER_INPUT_TOOL.to_string(), "{}".to_string()),
            ],
        );
        let earlier = tracker.admit(3, "update_plan", None, "internal-1", "{}", ToolConcurrencyPolicy::Exclusive);
        let barrier = tracker.admit(3, REQUEST_USER_INPUT_TOOL, None, "internal-2", "{}", ToolConcurrencyPolicy::Barrier);
        let barrier_tracker = tracker.clone();
        let barrier_task = tokio::spawn(async move { barrier_tracker.wait_for_prior_completion(3, barrier.position).await });
        tokio::task::yield_now().await;
        assert!(!barrier_task.is_finished());

        let mut earlier_completion = tracker.completion_guard(3, earlier.position);
        earlier_completion.activate();
        drop(earlier_completion);
        tokio::time::timeout(Duration::from_secs(1), barrier_task)
            .await
            .expect("barrier should start after the earlier result hook")
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn barrier_stops_when_a_prior_hook_aborts() {
        let tracker = ToolBatchTracker::default();
        tracker.prepare(
            5,
            &[
                ("call-1".to_string(), None, "update_plan".to_string(), "{}".to_string()),
                ("call-2".to_string(), None, REQUEST_USER_INPUT_TOOL.to_string(), "{}".to_string()),
            ],
        );
        let earlier = tracker.admit(5, "update_plan", None, "internal-1", "{}", ToolConcurrencyPolicy::Exclusive);
        let barrier = tracker.admit(5, REQUEST_USER_INPUT_TOOL, None, "internal-2", "{}", ToolConcurrencyPolicy::Barrier);
        let barrier_tracker = tracker.clone();
        let barrier_task = tokio::spawn(async move { barrier_tracker.wait_for_prior_completion(5, barrier.position).await });
        tokio::task::yield_now().await;

        drop(tracker.completion_guard(5, earlier.position));

        let error = tokio::time::timeout(Duration::from_secs(1), barrier_task)
            .await
            .expect("aborted predecessor must release the barrier")
            .unwrap()
            .unwrap_err();
        assert_eq!(error, ToolBatchWaitError::PriorCallAborted);
    }

    #[tokio::test]
    async fn parallel_safe_calls_overlap() {
        let state = Arc::new(ProbeState::default());
        let scheduler = Arc::new(ToolCallScheduler::new());
        let first = ScheduledTool {
            inner: Box::new(ProbeTool::new(LIST_WORKSPACE_FILES_TOOL, state.clone(), Some(Duration::from_millis(40)))),
            policy: ToolConcurrencyPolicy::ParallelSafe,
            scheduler: scheduler.clone(),
        };
        let second = ScheduledTool {
            inner: Box::new(ProbeTool::new(SEARCH_WORKSPACE_FILES_TOOL, state.clone(), Some(Duration::from_millis(40)))),
            policy: ToolConcurrencyPolicy::ParallelSafe,
            scheduler,
        };

        tokio::join!(run(&first, "first"), run(&second, "second"));

        assert_eq!(state.max_active.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn exclusive_calls_do_not_overlap_and_keep_arrival_order() {
        let state = Arc::new(ProbeState::default());
        let scheduler = Arc::new(ToolCallScheduler::new());
        let first = Arc::new(ScheduledTool {
            inner: Box::new(ProbeTool::new("first_exclusive", state.clone(), Some(Duration::from_millis(30)))),
            policy: ToolConcurrencyPolicy::Exclusive,
            scheduler: scheduler.clone(),
        });
        let second = Arc::new(ScheduledTool {
            inner: Box::new(ProbeTool::new("second_exclusive", state.clone(), Some(Duration::from_millis(1)))),
            policy: ToolConcurrencyPolicy::Exclusive,
            scheduler,
        });

        let first_task = tokio::spawn({
            let first = first.clone();
            async move { run(first.as_ref(), "first").await }
        });
        wait_for_started(&state, 1).await;
        let second_task = tokio::spawn({
            let second = second.clone();
            async move { run(second.as_ref(), "second").await }
        });
        first_task.await.unwrap();
        second_task.await.unwrap();

        assert_eq!(state.max_active.load(Ordering::Acquire), 1);
        assert_eq!(&*state.started.lock().unwrap(), &["first", "second"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mixed_calls_wait_for_exclusive_boundary() {
        let state = Arc::new(ProbeState::default());
        let scheduler = Arc::new(ToolCallScheduler::new());
        let read_a = Arc::new(ScheduledTool {
            inner: Box::new(ProbeTool::new(LIST_WORKSPACE_FILES_TOOL, state.clone(), Some(Duration::from_millis(30)))),
            policy: ToolConcurrencyPolicy::ParallelSafe,
            scheduler: scheduler.clone(),
        });
        let read_b = Arc::new(ScheduledTool {
            inner: Box::new(ProbeTool::new(READ_WORKSPACE_FILE_TOOL, state.clone(), Some(Duration::from_millis(30)))),
            policy: ToolConcurrencyPolicy::ParallelSafe,
            scheduler: scheduler.clone(),
        });
        let exclusive = Arc::new(ScheduledTool {
            inner: Box::new(ProbeTool::new("exclusive", state.clone(), Some(Duration::from_millis(5)))),
            policy: ToolConcurrencyPolicy::Exclusive,
            scheduler: scheduler.clone(),
        });
        let read_d = Arc::new(ScheduledTool {
            inner: Box::new(ProbeTool::new(SEARCH_WORKSPACE_FILES_TOOL, state.clone(), Some(Duration::from_millis(1)))),
            policy: ToolConcurrencyPolicy::ParallelSafe,
            scheduler,
        });

        let read_a_task = tokio::spawn({
            let read_a = read_a.clone();
            async move { run(read_a.as_ref(), "a").await }
        });
        let read_b_task = tokio::spawn({
            let read_b = read_b.clone();
            async move { run(read_b.as_ref(), "b").await }
        });
        wait_for_started(&state, 2).await;
        let exclusive_task = tokio::spawn({
            let exclusive = exclusive.clone();
            async move { run(exclusive.as_ref(), "c").await }
        });
        tokio::task::yield_now().await;
        let read_d_task = tokio::spawn({
            let read_d = read_d.clone();
            async move { run(read_d.as_ref(), "d").await }
        });
        read_a_task.await.unwrap();
        read_b_task.await.unwrap();
        exclusive_task.await.unwrap();
        read_d_task.await.unwrap();

        let started = state.started.lock().unwrap().clone();
        assert_eq!(state.max_active.load(Ordering::Acquire), 2);
        assert_eq!(&started[started.len() - 2..], &["c", "d"]);
    }

    #[tokio::test]
    async fn cancelling_a_call_releases_the_scheduler_lock() {
        let state = Arc::new(ProbeState::default());
        let started_state = state.clone();
        let scheduler = Arc::new(ToolCallScheduler::new());
        let blocked = Arc::new(ScheduledTool {
            inner: Box::new(ProbeTool::blocking("blocked", state.clone())),
            policy: ToolConcurrencyPolicy::Exclusive,
            scheduler: scheduler.clone(),
        });
        let next = ScheduledTool {
            inner: Box::new(ProbeTool::new("next", state, Some(Duration::from_millis(1)))),
            policy: ToolConcurrencyPolicy::Exclusive,
            scheduler,
        };

        let task = tokio::spawn({
            let blocked = blocked.clone();
            async move { run(blocked.as_ref(), "blocked").await }
        });
        wait_for_started(&started_state, 1).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        tokio::time::timeout(Duration::from_millis(50), run(&next, "next"))
            .await
            .expect("cancelled tool must release its write lock");
    }
}
