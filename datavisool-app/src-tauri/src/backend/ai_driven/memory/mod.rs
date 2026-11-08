/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 项目自有 ConversationMemory 后端与 Rig 隔离适配层。

mod adapter;
mod conversation_store;
mod policy;
mod run_scoped;
mod task_adapter;
mod task_store;

pub(crate) use adapter::ConversationMemoryAdapter;
pub(crate) use conversation_store::{ConversationMemoryBackend, SqliteConversationMemoryBackend};
pub(crate) use policy::ConversationWindowPolicy;
pub(crate) use run_scoped::RunScopedConversationMemory;
pub(crate) use task_adapter::{new_task_memory, task_conversation_key};
