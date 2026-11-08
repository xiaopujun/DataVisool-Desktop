/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 会话窗口裁剪的项目策略。

use serde::{Deserialize, Serialize};

use super::conversation_store::{ConversationMessage, ConversationSummary};

/// 加载最近消息时使用的双重窗口限制。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversationWindowPolicy {
    pub max_messages: u32,
    pub max_estimated_tokens: u64,
}

impl Default for ConversationWindowPolicy {
    fn default() -> Self {
        Self { max_messages: 40, max_estimated_tokens: 12_000 }
    }
}

/// 可直接交给上层 Agent 适配器的有序会话窗口。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversationWindow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ConversationSummary>,
    pub messages: Vec<ConversationMessage>,
    pub estimated_tokens: u64,
    pub was_truncated: bool,
    /// `was_truncated` 时，早期消息可安全压缩到的序号（含）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated_through_sequence: Option<u64>,
}
