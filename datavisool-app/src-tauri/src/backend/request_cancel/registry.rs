/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 本地 HTTP 请求取消注册表。
//!
//! 前端可以为长耗时请求注册 requestId，再通过取消接口广播取消信号。
//! 模型调用等异步流程会订阅对应的 `watch` channel，在等待重试或发送过程中响应取消。

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::watch;

/// requestId 到取消广播通道的映射。
#[derive(Default)]
pub(crate) struct HttpRequestCancelRegistry {
    requests: Mutex<HashMap<u64, watch::Sender<bool>>>,
}

impl HttpRequestCancelRegistry {
    /// 订阅已有 requestId 的取消信号；不存在时创建新的取消通道。
    pub(crate) fn subscribe_or_register(&self, request_id: u64) -> watch::Receiver<bool> {
        let mut requests = self.requests.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(sender) = requests.get(&request_id) {
            return sender.subscribe();
        }

        // 初始值为 false，调用 cancel 后广播 true。
        let (sender, receiver) = watch::channel(false);
        requests.insert(request_id, sender);
        receiver
    }

    /// 标记 requestId 已取消，并通知所有订阅者。
    pub(crate) fn cancel(&self, request_id: u64) -> bool {
        let sender = {
            let requests = self.requests.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            requests.get(&request_id).cloned()
        };

        match sender {
            Some(sender) => {
                // 订阅方只关心最终取消状态，发送失败不影响 cancel 接口的语义。
                let _ = sender.send(true);
                true
            }
            None => false,
        }
    }

    /// 请求完成后移除注册项，避免长期占用内存。
    pub(crate) fn unregister(&self, request_id: u64) {
        let mut requests = self.requests.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        requests.remove(&request_id);
    }
}

#[cfg(test)]
mod tests {
    use super::HttpRequestCancelRegistry;

    #[tokio::test]
    async fn cancel_signal_is_visible_to_subscribers() {
        let registry = HttpRequestCancelRegistry::default();
        let mut receiver = registry.subscribe_or_register(42);

        assert!(!*receiver.borrow());
        assert!(registry.cancel(42));

        receiver.changed().await.expect("cancel signal should arrive");
        assert!(*receiver.borrow());

        registry.unregister(42);
    }
}
