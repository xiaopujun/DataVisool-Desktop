/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 统一 Item 事件的运行期聚合状态。

use std::time::{Duration, Instant};

pub(super) const AGENT_MESSAGE_DELTA_BATCH_CHARS: usize = 80;
pub(super) const AGENT_MESSAGE_DELTA_BATCH_INTERVAL: Duration = Duration::from_millis(100);
const MAX_AGENT_MESSAGE_CHARS: usize = 32_000;

pub(super) struct ActiveAgentMessage {
    pub(super) item_id: String,
    pub(super) row_version: u64,
    pub(super) text: String,
    pub(super) pending_chars: usize,
    pub(super) last_flush: Instant,
    pub(super) truncated: bool,
    pending_delta: String,
    text_chars: usize,
}

impl ActiveAgentMessage {
    pub(super) fn new(item_id: String) -> Self {
        Self {
            item_id,
            row_version: 1,
            text: String::new(),
            pending_delta: String::new(),
            text_chars: 0,
            pending_chars: 0,
            last_flush: Instant::now(),
            truncated: false,
        }
    }

    pub(super) fn push(&mut self, delta: &str) -> bool {
        let remaining = MAX_AGENT_MESSAGE_CHARS.saturating_sub(self.text_chars);
        let accepted = delta.chars().take(remaining).collect::<String>();
        let accepted_chars = accepted.chars().count();
        self.truncated |= accepted_chars < delta.chars().count();
        if accepted.is_empty() {
            return false;
        }
        self.text.push_str(&accepted);
        self.pending_delta.push_str(&accepted);
        self.text_chars += accepted_chars;
        self.pending_chars += accepted_chars;
        true
    }

    pub(super) fn take_pending(&mut self) -> String {
        self.pending_chars = 0;
        self.last_flush = Instant::now();
        std::mem::take(&mut self.pending_delta)
    }
}
