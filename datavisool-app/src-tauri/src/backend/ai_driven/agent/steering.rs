/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 将持久化 Inbox 的追加输入注入 Rig 下一次模型调用。

use std::sync::{Arc, Mutex};

use rig_core::completion::Message;

use crate::backend::ai_driven::task::{AgentSteeringMailbox, AgentTaskEventPublisher, AgentTaskRepository, AgentTaskRun};
use crate::backend::ai_driven::thread::ConsumeTurnInput;

const INBOX_BATCH_LIMIT: u32 = 200;

#[derive(Clone)]
pub(crate) struct AgentSteeringHook {
    mailbox: AgentSteeringMailbox,
    injected: Arc<Mutex<Vec<Message>>>,
    repository: Arc<dyn AgentTaskRepository>,
    publisher: AgentTaskEventPublisher,
    task: AgentTaskRun,
}

impl AgentSteeringHook {
    pub(crate) fn new(
        mailbox: AgentSteeringMailbox,
        repository: Arc<dyn AgentTaskRepository>,
        publisher: AgentTaskEventPublisher,
        task: AgentTaskRun,
    ) -> Self {
        Self { mailbox, injected: Arc::new(Mutex::new(Vec::new())), repository, publisher, task }
    }

    pub(crate) fn reset_injected(&self) {
        self.injected.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
    }

    /// 结束前只检查是否有待消费输入，不关闭追加入口。
    pub(crate) async fn has_pending(&self) -> bool {
        let _ = self.mailbox.drain();
        !matches!(self.accepted_inputs().await, Ok(inputs) if inputs.is_empty())
    }

    /// 结束事实检查通过后关闭入口；并发中的预留或已持久化输入会阻止关闭。
    pub(crate) async fn close_if_idle(&self) -> bool {
        let _ = self.mailbox.drain();
        match self.accepted_inputs().await {
            Ok(inputs) if inputs.is_empty() => self.mailbox.close_if_empty(),
            Ok(_) | Err(_) => false,
        }
    }

    pub(crate) async fn history_with_pending(&self, history: &[Message]) -> Result<Option<Vec<Message>>, &'static str> {
        let _ = self.mailbox.drain();
        let pending = match self.accepted_inputs().await {
            Ok(inputs) => inputs,
            Err(_) => return Err("agent.inbox.read_failed"),
        };
        if !pending.is_empty() {
            let mut messages = Vec::with_capacity(pending.len());
            for input in pending {
                let item = match self
                    .repository
                    .find_item(self.task.project_id, self.task.thread_id.clone(), self.task.turn_id.clone(), input.user_item_id.clone())
                    .await
                {
                    Ok(Some(item)) => item,
                    _ => return Err("agent.inbox.item_missing"),
                };
                let Some(text) = item.payload.get("text").and_then(|value| value.as_str()).map(str::trim) else {
                    return Err("agent.inbox.item_invalid");
                };
                if text.is_empty() {
                    return Err("agent.inbox.item_invalid");
                }
                let consumed = match self
                    .repository
                    .consume_turn_input(ConsumeTurnInput {
                        project_id: self.task.project_id,
                        thread_id: self.task.thread_id.clone(),
                        turn_id: self.task.turn_id.clone(),
                        run_id: self.task.run_id.clone(),
                        input_id: input.input_id,
                        expected_row_version: input.row_version,
                        consumed_at: now_ms(),
                    })
                    .await
                {
                    Ok(consumed) => consumed,
                    Err(_) => return Err("agent.inbox.consume_failed"),
                };
                for event in consumed.events {
                    self.publisher.publish(event);
                }
                messages.push(Message::user(format!("用户在当前任务运行中追加要求：\n{text}")));
            }
            self.injected
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(messages);
        }

        let injected = self.injected.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
        if injected.is_empty() {
            return Ok(None);
        }
        let mut patched_history = history.to_vec();
        patched_history.extend(injected.into_iter().filter(|message| !history.contains(message)));
        Ok(Some(patched_history))
    }

    async fn accepted_inputs(
        &self,
    ) -> Result<Vec<crate::backend::ai_driven::thread::AgentTurnInput>, crate::backend::ai_driven::task::TaskRepositoryError> {
        self.repository
            .list_accepted_turn_inputs(self.task.project_id, self.task.thread_id.clone(), self.task.turn_id.clone(), INBOX_BATCH_LIMIT)
            .await
    }
}

fn now_ms() -> i64 {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .min(i64::MAX as u128) as i64
}
