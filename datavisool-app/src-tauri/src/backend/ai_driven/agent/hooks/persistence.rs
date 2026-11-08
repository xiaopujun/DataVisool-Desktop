/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Agent Run 级仓储写入协调。
//!
//! Tool 本身可以并行执行，但同一个 Run 的 Timeline、Task checkpoint 和
//! working checkpoint 写入必须经过同一把锁。锁只覆盖仓储访问与事件发布，
//! 不覆盖 Tool 执行或前端等待。

use std::future::Future;
use std::sync::Arc;

use rig_core::agent::HookContext;
use serde_json::{json, Value};

use super::{now_ms, AgentRuntimeHook};
use crate::backend::ai_driven::task::{
    AgentPendingSnapshot, AgentTaskStatus, AgentWorkingCheckpointState, NewAgentTaskEvent, SaveAgentTaskCheckpoint, SaveAgentWorkingCheckpoint,
    TaskRepositoryError,
};
use crate::backend::ai_driven::thread::{AgentItem, ApplyItemEventInput, ItemStatus, StartItemInput};
use crate::backend::utils::sanitize_debug_log_text;

const MAX_WRITE_ATTEMPTS: usize = 3;
const RETRY_DELAYS_MS: [u64; MAX_WRITE_ATTEMPTS - 1] = [10, 20];
const ITEM_SCHEMA_VERSION: u32 = 1;

#[derive(Clone)]
struct WorkingCheckpointWrite {
    project_id: i64,
    thread_id: String,
    task_id: String,
    run_id: String,
    turn_id: String,
    expected_version: u64,
    state: AgentWorkingCheckpointState,
}

impl WorkingCheckpointWrite {
    fn input(&self) -> SaveAgentWorkingCheckpoint {
        SaveAgentWorkingCheckpoint {
            project_id: self.project_id,
            thread_id: self.thread_id.clone(),
            task_id: self.task_id.clone(),
            run_id: self.run_id.clone(),
            turn_id: self.turn_id.clone(),
            expected_version: self.expected_version,
            state: self.state.clone(),
        }
    }
}

#[derive(Clone)]
pub(super) struct PersistenceCoordinator {
    lock: Arc<tokio::sync::Mutex<()>>,
    run_id: String,
    turn_id: String,
}

impl PersistenceCoordinator {
    pub(super) fn new(run_id: impl Into<String>, turn_id: impl Into<String>) -> Self {
        Self { lock: Arc::new(tokio::sync::Mutex::new(())), run_id: run_id.into(), turn_id: turn_id.into() }
    }

    /// 执行一次写入，并在成功提交后、释放 Run 锁前发布事件。
    pub(super) async fn write_and_publish<T, F, Fut, P>(&self, operation: &str, write: F, publish: P) -> Result<T, TaskRepositoryError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, TaskRepositoryError>>,
        P: FnMut(&T),
    {
        let _guard = self.lock.lock().await;
        self.write_locked(operation, write, publish).await
    }

    /// 调用方已经持有 Run 锁时使用。用于需要把“读当前值 + 写新值”放在
    /// 同一临界区的操作，例如 working checkpoint。
    pub(super) async fn write_locked<T, F, Fut, P>(&self, operation: &str, mut write: F, mut publish: P) -> Result<T, TaskRepositoryError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, TaskRepositoryError>>,
        P: FnMut(&T),
    {
        for attempt in 1..=MAX_WRITE_ATTEMPTS {
            match write().await {
                Ok(value) => {
                    publish(&value);
                    return Ok(value);
                }
                Err(error) if error.is_transient_write_conflict() && attempt < MAX_WRITE_ATTEMPTS => {
                    let delay_ms = RETRY_DELAYS_MS[attempt - 1];
                    tracing::warn!(
                        target: "agent.persistence",
                        event = "repository_write_retry",
                        run_id = %self.run_id,
                        turn_id = %self.turn_id,
                        operation,
                        attempt,
                        max_attempts = MAX_WRITE_ATTEMPTS,
                        delay_ms,
                        error = %sanitize_debug_log_text(&error.to_string()),
                        "Transient Agent repository write conflict; retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                }
                Err(error) => {
                    tracing::error!(
                        target: "agent.persistence",
                        event = "repository_write_failed",
                        run_id = %self.run_id,
                        turn_id = %self.turn_id,
                        operation,
                        attempt,
                        max_attempts = MAX_WRITE_ATTEMPTS,
                        transient = error.is_transient_write_conflict(),
                        error = %sanitize_debug_log_text(&error.to_string()),
                        "Agent repository write failed"
                    );
                    return Err(error);
                }
            }
        }

        unreachable!("write attempts are bounded by MAX_WRITE_ATTEMPTS")
    }

    pub(super) async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.lock.lock().await
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::error::Error as StdError;
    use std::fmt::{Display, Formatter};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::backend::ai_driven::task::TaskRepositoryError;
    use crate::backend::ai_driven::thread::ThreadRepositoryError;

    #[derive(Debug)]
    struct FakeDatabaseError {
        code: String,
    }

    impl Display for FakeDatabaseError {
        fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "fake SQLite error ({})", self.code)
        }
    }

    impl StdError for FakeDatabaseError {}

    impl sqlx::error::DatabaseError for FakeDatabaseError {
        fn message(&self) -> &str {
            "fake SQLite error"
        }

        fn code(&self) -> Option<Cow<'_, str>> {
            Some(self.code.as_str().into())
        }

        fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    fn sqlite_error(code: i64) -> TaskRepositoryError {
        TaskRepositoryError::Thread(ThreadRepositoryError::Sql(sqlx::Error::database(FakeDatabaseError { code: code.to_string() })))
    }

    #[derive(Default)]
    struct WriteProbe {
        active: AtomicUsize,
        max_active: AtomicUsize,
        successful_writes: AtomicUsize,
        publishes: AtomicUsize,
        published_before_write: AtomicBool,
    }

    #[tokio::test]
    async fn serializes_concurrent_writes_and_publishes_after_success() {
        let coordinator = PersistenceCoordinator::new("run-test", "turn-test");
        let probe = Arc::new(WriteProbe::default());

        let first_probe = probe.clone();
        let first_coordinator = coordinator.clone();
        let first = first_coordinator.write_and_publish(
            "first",
            move || {
                let probe = first_probe.clone();
                async move {
                    let active = probe.active.fetch_add(1, Ordering::AcqRel) + 1;
                    probe.max_active.fetch_max(active, Ordering::AcqRel);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    probe.active.fetch_sub(1, Ordering::AcqRel);
                    probe.successful_writes.fetch_add(1, Ordering::AcqRel);
                    Ok::<_, TaskRepositoryError>(1_u8)
                }
            },
            {
                let probe = probe.clone();
                move |_| {
                    if probe.successful_writes.load(Ordering::Acquire) == 0 {
                        probe.published_before_write.store(true, Ordering::Release);
                    }
                    probe.publishes.fetch_add(1, Ordering::AcqRel);
                }
            },
        );

        let second_probe = probe.clone();
        let second_coordinator = coordinator.clone();
        let second = second_coordinator.write_and_publish(
            "second",
            move || {
                let probe = second_probe.clone();
                async move {
                    let active = probe.active.fetch_add(1, Ordering::AcqRel) + 1;
                    probe.max_active.fetch_max(active, Ordering::AcqRel);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    probe.active.fetch_sub(1, Ordering::AcqRel);
                    probe.successful_writes.fetch_add(1, Ordering::AcqRel);
                    Ok::<_, TaskRepositoryError>(2_u8)
                }
            },
            {
                let probe = probe.clone();
                move |_| {
                    if probe.successful_writes.load(Ordering::Acquire) == 0 {
                        probe.published_before_write.store(true, Ordering::Release);
                    }
                    probe.publishes.fetch_add(1, Ordering::AcqRel);
                }
            },
        );

        let (first_result, second_result) = tokio::join!(first, second);
        assert_eq!(first_result.unwrap(), 1);
        assert_eq!(second_result.unwrap(), 2);
        assert_eq!(probe.max_active.load(Ordering::Acquire), 1);
        assert_eq!(probe.successful_writes.load(Ordering::Acquire), 2);
        assert_eq!(probe.publishes.load(Ordering::Acquire), 2);
        assert!(!probe.published_before_write.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn retries_busy_twice_then_succeeds_on_third_attempt() {
        let coordinator = PersistenceCoordinator::new("run-test", "turn-test");
        let attempts = Arc::new(AtomicUsize::new(0));
        let publishes = Arc::new(AtomicUsize::new(0));
        let attempts_for_write = attempts.clone();
        let publishes_for_callback = publishes.clone();

        let result = coordinator
            .write_and_publish(
                "retry-success",
                move || {
                    let attempt = attempts_for_write.fetch_add(1, Ordering::AcqRel) + 1;
                    async move {
                        if attempt < 3 {
                            Err(sqlite_error(5))
                        } else {
                            Ok(42_u8)
                        }
                    }
                },
                move |_| {
                    publishes_for_callback.fetch_add(1, Ordering::AcqRel);
                },
            )
            .await
            .unwrap();

        assert_eq!(result, 42);
        assert_eq!(attempts.load(Ordering::Acquire), 3);
        assert_eq!(publishes.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn does_not_retry_non_transient_error() {
        let coordinator = PersistenceCoordinator::new("run-test", "turn-test");
        let attempts = Arc::new(AtomicUsize::new(0));
        let publishes = Arc::new(AtomicUsize::new(0));
        let attempts_for_write = attempts.clone();
        let publishes_for_callback = publishes.clone();

        let result = coordinator
            .write_and_publish(
                "non-transient",
                move || {
                    attempts_for_write.fetch_add(1, Ordering::AcqRel);
                    async { Err::<u8, _>(sqlite_error(19)) }
                },
                move |_| {
                    publishes_for_callback.fetch_add(1, Ordering::AcqRel);
                },
            )
            .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::Acquire), 1);
        assert_eq!(publishes.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn returns_error_after_three_busy_attempts() {
        let coordinator = PersistenceCoordinator::new("run-test", "turn-test");
        let attempts = Arc::new(AtomicUsize::new(0));
        let publishes = Arc::new(AtomicUsize::new(0));
        let attempts_for_write = attempts.clone();
        let publishes_for_callback = publishes.clone();

        let result = coordinator
            .write_and_publish(
                "retry-exhausted",
                move || {
                    attempts_for_write.fetch_add(1, Ordering::AcqRel);
                    async { Err::<u8, _>(sqlite_error(5)) }
                },
                move |_| {
                    publishes_for_callback.fetch_add(1, Ordering::AcqRel);
                },
            )
            .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::Acquire), 3);
        assert_eq!(publishes.load(Ordering::Acquire), 0);
    }
}

pub(super) async fn append_event(hook: &AgentRuntimeHook, kind: &str, item_id: Option<String>, payload: Value) -> Result<(), TaskRepositoryError> {
    let dedupe_key = super::event_dedupe_key(&hook.task.run_id, kind, item_id.as_deref(), &payload)?;
    let input = NewAgentTaskEvent {
        project_id: hook.task.project_id,
        thread_id: hook.task.thread_id.clone(),
        turn_id: hook.task.turn_id.clone(),
        run_id: hook.task.run_id.clone(),
        item_id,
        kind: kind.to_string(),
        payload,
        dedupe_key,
        schema_version: 1,
        created_at: now_ms(),
    };
    let repository = hook.repository.clone();
    hook.persistence
        .write_and_publish(
            kind,
            move || {
                let repository = repository.clone();
                let input = input.clone();
                async move { repository.append_event(input).await }
            },
            |event| hook.publish(event.clone()),
        )
        .await?;
    Ok(())
}

pub(super) async fn start_item(
    hook: &AgentRuntimeHook,
    ctx: &HookContext,
    item_id: String,
    logical_key: Option<String>,
    item_type: &str,
    status: ItemStatus,
    snapshot: Value,
) -> Result<AgentItem, TaskRepositoryError> {
    let event_payload = hook.payload(ctx, json!({ "item": snapshot }));
    let input = StartItemInput {
        project_id: hook.task.project_id,
        thread_id: hook.task.thread_id.clone(),
        turn_id: hook.task.turn_id.clone(),
        run_id: hook.task.run_id.clone(),
        item_id: Some(item_id.clone()),
        logical_key,
        item_type: item_type.to_string(),
        status,
        schema_version: ITEM_SCHEMA_VERSION,
        payload: snapshot,
        event_payload,
        dedupe_key: format!("item:{item_id}:started"),
        created_at: now_ms(),
    };
    let repository = hook.repository.clone();
    let result = hook
        .persistence
        .write_and_publish(
            "item.started",
            move || {
                let repository = repository.clone();
                let input = input.clone();
                async move { repository.start_item(input).await }
            },
            |result| hook.publish(result.event.clone()),
        )
        .await?;
    result
        .item
        .ok_or_else(|| TaskRepositoryError::InvalidRecord("start_item returned no Item snapshot".to_string()))
}

pub(super) async fn apply_item(
    hook: &AgentRuntimeHook,
    ctx: &HookContext,
    item_id: &str,
    row_version: u64,
    status: ItemStatus,
    snapshot: Value,
    event_kind: &str,
    updated_at: i64,
) -> Result<AgentItem, TaskRepositoryError> {
    let event_payload = hook.payload(ctx, json!({ "item": snapshot }));
    apply_item_with_payload(
        hook,
        item_id,
        row_version,
        status,
        snapshot,
        event_kind,
        event_payload,
        status.is_terminal().then_some(updated_at),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn apply_item_with_payload(
    hook: &AgentRuntimeHook,
    item_id: &str,
    row_version: u64,
    status: ItemStatus,
    snapshot: Value,
    event_kind: &str,
    event_payload: Value,
    completed_at: Option<i64>,
) -> Result<AgentItem, TaskRepositoryError> {
    let updated_at = completed_at.unwrap_or_else(now_ms);
    let input = ApplyItemEventInput {
        project_id: hook.task.project_id,
        thread_id: hook.task.thread_id.clone(),
        turn_id: hook.task.turn_id.clone(),
        run_id: hook.task.run_id.clone(),
        item_id: item_id.to_string(),
        expected_row_version: row_version,
        status,
        schema_version: ITEM_SCHEMA_VERSION,
        payload: snapshot,
        event_kind: event_kind.to_string(),
        event_payload,
        dedupe_key: format!("item:{item_id}:version:{}:{event_kind}", row_version + 1),
        updated_at,
        completed_at,
    };
    let repository = hook.repository.clone();
    let result = hook
        .persistence
        .write_and_publish(
            event_kind,
            move || {
                let repository = repository.clone();
                let input = input.clone();
                async move { repository.apply_item_event(input).await }
            },
            |result| hook.publish(result.event.clone()),
        )
        .await?;
    result
        .item
        .ok_or_else(|| TaskRepositoryError::InvalidRecord("apply_item_event returned no Item snapshot".to_string()))
}

pub(super) async fn save_task_status(
    hook: &AgentRuntimeHook,
    status: AgentTaskStatus,
    pending: Option<AgentPendingSnapshot>,
    updated_at: i64,
) -> Result<(), TaskRepositoryError> {
    let working_pending = pending.clone();
    let usage = serde_json::to_value(hook.budget.usage_snapshot()).ok();
    let repository = hook.repository.clone();
    let run_id = hook.task.run_id.clone();
    hook.persistence
        .write_and_publish(
            "save_checkpoint",
            move || {
                let repository = repository.clone();
                let run_id = run_id.clone();
                let pending = pending.clone();
                let usage = usage.clone();
                async move {
                    let current = repository
                        .find_by_run_id(run_id.clone())
                        .await?
                        .ok_or(TaskRepositoryError::NotFound { entity: "task run", key: run_id })?;
                    if current.status.is_terminal() {
                        return Err(TaskRepositoryError::InvalidRecord("terminal task cannot enter a waiting state".to_string()));
                    }
                    repository
                        .save_checkpoint(SaveAgentTaskCheckpoint {
                            project_id: current.project_id,
                            thread_id: current.thread_id.clone(),
                            turn_id: current.turn_id.clone(),
                            run_id: current.run_id.clone(),
                            status,
                            pending,
                            usage,
                            last_artifact: current.last_artifact,
                            error: current.error,
                            thread_preview: None,
                            project_snapshot_ref: current.project_snapshot_ref,
                            rig_version: current.rig_version,
                            rig_snapshot_ref: current.rig_snapshot_ref,
                            updated_at,
                            ended_at: None,
                        })
                        .await
                }
            },
            |saved| {
                for event in &saved.events {
                    hook.publish(event.clone());
                }
            },
        )
        .await?;
    if hook.tool_route.is_some() {
        update_working_checkpoint(hook, updated_at, |state| state.pending = working_pending).await?;
    }
    Ok(())
}

pub(super) async fn update_working_checkpoint(
    hook: &AgentRuntimeHook,
    updated_at: i64,
    update: impl FnOnce(&mut AgentWorkingCheckpointState),
) -> Result<(), TaskRepositoryError> {
    let _persistence_guard = hook.persistence.lock().await;
    let repository = hook.repository.clone();
    let run_id = hook.task.run_id.clone();
    let checkpoint = repository
        .find_working_checkpoint(run_id.clone())
        .await?
        .ok_or(TaskRepositoryError::NotFound { entity: "working checkpoint", key: run_id.clone() })?;
    let mut state = checkpoint.state;
    update(&mut state);
    state.run_recovery_state = hook.recovery_snapshot();
    state.updated_at = updated_at;
    let write = WorkingCheckpointWrite {
        project_id: hook.task.project_id,
        thread_id: hook.task.thread_id.clone(),
        task_id: hook.task.task_id.clone(),
        run_id: hook.task.run_id.clone(),
        turn_id: hook.task.turn_id.clone(),
        expected_version: checkpoint.checkpoint_version,
        state,
    };
    hook.persistence
        .write_locked(
            "save_working_checkpoint",
            move || {
                let repository = repository.clone();
                let write = write.clone();
                async move { repository.save_working_checkpoint(write.input()).await }
            },
            |_| {},
        )
        .await?;
    Ok(())
}
