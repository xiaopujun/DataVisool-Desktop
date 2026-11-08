/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 将 Rig Tool Call 身份传递到可并发调度的业务 Tool。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use super::AgentToolError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentToolInvocation {
    pub(crate) tool_name: String,
    pub(crate) tool_call_id: String,
    pub(crate) internal_call_id: String,
}

#[derive(Clone, Default)]
pub(crate) struct AgentToolInvocationTracker {
    state: Arc<Mutex<AgentToolInvocationState>>,
}

#[derive(Default)]
struct AgentToolInvocationState {
    /// Rig may enter several tool-call hooks before any of the concrete tools
    /// asks for its invocation. Keep one FIFO queue per tool so identities are
    /// never overwritten by a later parallel hook.
    pending_by_tool: HashMap<String, VecDeque<AgentToolInvocation>>,
    /// Preserve arrival order for the legacy mismatch diagnostic and for
    /// removing the exact entry from the per-tool queue atomically.
    pending_order: VecDeque<(String, String)>,
}

impl AgentToolInvocationTracker {
    pub(crate) fn begin(&self, tool_name: &str, tool_call_id: Option<&str>, internal_call_id: &str) {
        let tool_call_id = tool_call_id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(internal_call_id);
        let invocation = AgentToolInvocation {
            tool_name: tool_name.to_string(),
            tool_call_id: tool_call_id.to_string(),
            internal_call_id: internal_call_id.to_string(),
        };
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .pending_by_tool
            .entry(invocation.tool_name.clone())
            .or_default()
            .push_back(invocation.clone());
        state
            .pending_order
            .push_back((invocation.tool_name, invocation.internal_call_id));
    }

    pub(crate) fn finish(&self, internal_call_id: &str) {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(order_index) = state
            .pending_order
            .iter()
            .position(|(_, queued_call_id)| queued_call_id == internal_call_id)
        else {
            // A call which already passed through require() is no longer
            // pending. Its result hook must therefore be a no-op here.
            return;
        };
        let (tool_name, _) = state.pending_order[order_index].clone();
        let invocation = {
            let Some(queue) = state.pending_by_tool.get_mut(&tool_name) else {
                // Keep the tracker recoverable if its internal indexes are
                // ever inconsistent; there is no safe identity to remove in that case.
                return;
            };
            let Some(index) = queue
                .iter()
                .position(|invocation| invocation.internal_call_id == internal_call_id)
            else {
                return;
            };
            queue.remove(index).expect("invocation index was obtained from the same queue")
        };
        state.pending_order.remove(order_index);
        let should_remove_queue = state
            .pending_by_tool
            .get(&invocation.tool_name)
            .is_some_and(|queue| queue.is_empty());
        if should_remove_queue {
            state.pending_by_tool.remove(&invocation.tool_name);
        }
    }

    pub(crate) fn require(&self, tool_name: &str) -> Result<AgentToolInvocation, AgentToolError> {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let invocation = state.pending_by_tool.get_mut(tool_name).and_then(VecDeque::pop_front);
        if let Some(invocation) = invocation {
            let should_remove_queue = state.pending_by_tool.get(tool_name).is_some_and(|queue| queue.is_empty());
            if should_remove_queue {
                state.pending_by_tool.remove(tool_name);
            }
            remove_pending_order(&mut state.pending_order, &invocation.tool_name, &invocation.internal_call_id);
            return Ok(invocation);
        }

        if let Some((actual_tool_name, _)) = state.pending_order.front() {
            return Err(AgentToolError::internal(
                "agent.operation.tool_call_mismatch",
                format!("当前 Tool Call 属于 {actual_tool_name}，不能执行 {tool_name}"),
            ));
        }
        Err(AgentToolError::internal("agent.operation.tool_call_id_missing", "当前动作缺少真实 Tool Call 身份"))
    }
}

fn remove_pending_order(order: &mut VecDeque<(String, String)>, tool_name: &str, internal_call_id: &str) {
    if let Some(index) = order
        .iter()
        .position(|(queued_tool_name, queued_call_id)| queued_tool_name == tool_name && queued_call_id == internal_call_id)
    {
        order.remove(index);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::AgentToolInvocationTracker;

    #[test]
    fn keeps_interleaved_tools_isolated_and_fifo_per_tool() {
        let tracker = AgentToolInvocationTracker::default();
        tracker.begin("patch", Some("patch-1"), "internal-patch-1");
        tracker.begin("read", Some("read-1"), "internal-read-1");
        tracker.begin("patch", Some("patch-2"), "internal-patch-2");

        assert_eq!(tracker.require("patch").unwrap().tool_call_id, "patch-1");
        assert_eq!(tracker.require("read").unwrap().tool_call_id, "read-1");
        assert_eq!(tracker.require("patch").unwrap().tool_call_id, "patch-2");
    }

    #[test]
    fn finish_removes_only_an_unconsumed_call() {
        let tracker = AgentToolInvocationTracker::default();
        tracker.begin("patch", Some("patch-1"), "internal-patch-1");
        tracker.begin("patch", Some("patch-2"), "internal-patch-2");

        // Consumed entries must not affect a later result hook.
        assert_eq!(tracker.require("patch").unwrap().tool_call_id, "patch-1");
        tracker.finish("internal-patch-1");

        tracker.finish("internal-patch-2");
        let error = tracker.require("patch").unwrap_err();
        assert_eq!(error.code, "agent.operation.tool_call_id_missing");
    }

    #[test]
    fn wrong_tool_cannot_consume_another_tools_identity() {
        let tracker = AgentToolInvocationTracker::default();
        tracker.begin("patch", Some("patch-1"), "internal-patch-1");

        let error = tracker.require("delete").unwrap_err();
        assert_eq!(error.code, "agent.operation.tool_call_mismatch");
        assert_eq!(tracker.require("patch").unwrap().tool_call_id, "patch-1");
    }

    #[test]
    fn concurrent_begin_preserves_every_identity() {
        const CALL_COUNT: usize = 64;
        let tracker = Arc::new(AgentToolInvocationTracker::default());
        let handles = (0..CALL_COUNT)
            .map(|index| {
                let tracker = tracker.clone();
                thread::spawn(move || {
                    let call_id = format!("read-{index}");
                    let internal_id = format!("internal-read-{index}");
                    tracker.begin("read", Some(&call_id), &internal_id);
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().expect("begin thread should finish");
        }

        let mut call_ids = (0..CALL_COUNT)
            .map(|_| tracker.require("read").expect("every begin must be retained").tool_call_id)
            .collect::<Vec<_>>();
        call_ids.sort();
        let mut expected = (0..CALL_COUNT).map(|index| format!("read-{index}")).collect::<Vec<_>>();
        expected.sort();
        assert_eq!(call_ids, expected);
        assert_eq!(tracker.require("read").unwrap_err().code, "agent.operation.tool_call_id_missing");
    }
}
