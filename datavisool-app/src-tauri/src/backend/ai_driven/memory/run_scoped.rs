/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 活动任务内保留原始 Rig 消息，同时继续写入稳定持久化摘要。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use rig_core::completion::Message;
use rig_core::memory::{ConversationMemory, MemoryError};
use rig_core::wasm_compat::WasmBoxedFuture;

use super::ConversationMemoryAdapter;

#[derive(Clone)]
pub(crate) struct RunScopedConversationMemory {
    persistent: ConversationMemoryAdapter,
    conversations: Arc<Mutex<HashMap<String, ActiveConversation>>>,
}

#[derive(Clone, Debug, Default)]
struct ActiveConversation {
    loaded: bool,
    messages: Vec<Message>,
}

impl RunScopedConversationMemory {
    pub(crate) fn new(persistent: ConversationMemoryAdapter) -> Self {
        Self { persistent, conversations: Arc::new(Mutex::new(HashMap::new())) }
    }

    fn lock_conversations(&self) -> MutexGuard<'_, HashMap<String, ActiveConversation>> {
        self.conversations.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl ConversationMemory for RunScopedConversationMemory {
    fn load<'a>(&'a self, conversation_id: &'a str) -> WasmBoxedFuture<'a, Result<Vec<Message>, MemoryError>> {
        Box::pin(async move {
            if let Some(messages) = self
                .lock_conversations()
                .get(conversation_id)
                .filter(|conversation| conversation.loaded)
                .map(|conversation| conversation.messages.clone())
            {
                return Ok(messages);
            }
            let loaded = self.persistent.load(conversation_id).await?;
            let mut conversations = self.lock_conversations();
            let conversation = conversations.entry(conversation_id.to_string()).or_default();
            if !conversation.loaded {
                conversation.messages = loaded;
                conversation.loaded = true;
            }
            Ok(conversation.messages.clone())
        })
    }

    fn append<'a>(&'a self, conversation_id: &'a str, messages: Vec<Message>) -> WasmBoxedFuture<'a, Result<(), MemoryError>> {
        Box::pin(async move {
            if conversation_id.trim().is_empty() {
                return Err(MemoryError::Policy("conversation_id must not be blank".to_string()));
            }
            self.load(conversation_id).await?;
            {
                let mut conversations = self.lock_conversations();
                let conversation = conversations.entry(conversation_id.to_string()).or_default();
                conversation.loaded = true;
                conversation.messages.extend(messages.iter().cloned());
            }
            self.persistent.append(conversation_id, messages).await
        })
    }

    fn clear<'a>(&'a self, conversation_id: &'a str) -> WasmBoxedFuture<'a, Result<(), MemoryError>> {
        Box::pin(async move {
            self.lock_conversations().remove(conversation_id);
            self.persistent.clear(conversation_id).await
        })
    }
}
