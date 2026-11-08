/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 后端分配的不可预测领域标识。

use rand::rngs::OsRng;
use rand::RngCore;

pub(crate) fn new_thread_id() -> String {
    opaque_id("ai-agent-thread")
}

pub(crate) fn new_turn_id() -> String {
    opaque_id("ai-agent-turn")
}

pub(crate) fn new_item_id() -> String {
    opaque_id("ai-agent-item")
}

pub(crate) fn new_task_id() -> String {
    opaque_id("ai-agent-task")
}

pub(crate) fn new_run_id() -> String {
    opaque_id("ai-agent-run")
}

pub(super) fn new_event_id() -> String {
    opaque_id("ai-agent-event")
}

fn opaque_id(prefix: &str) -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    let suffix = bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    format!("{prefix}-{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_prefixed_opaque_and_unique() {
        let first = new_thread_id();
        let second = new_thread_id();
        assert!(first.starts_with("ai-agent-thread-"));
        assert_eq!(first.len(), "ai-agent-thread-".len() + 32);
        assert_ne!(first, second);
    }
}
