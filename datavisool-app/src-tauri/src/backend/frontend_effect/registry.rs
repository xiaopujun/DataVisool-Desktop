/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 前端异步交互等待注册表。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;

use super::error::FrontendEffectError;
use super::types::{FrontendEffectBinding, FrontendEffectResponse};

/// effectId 到一次性响应通道的进程内映射。
#[derive(Clone, Default)]
pub(crate) struct FrontendEffectRegistry {
    /// 等待前端响应的 effect 映射，key 是 effect id。
    pending: Arc<Mutex<HashMap<String, PendingEffect>>>,
}

struct PendingEffect {
    binding: FrontendEffectBinding,
    sender: oneshot::Sender<FrontendEffectResponse>,
}

impl FrontendEffectRegistry {
    /// 注册一次等待中的前端交互，并返回用于等待结果的 receiver。
    pub(crate) fn register(
        &self,
        effect_id: String,
        binding: FrontendEffectBinding,
    ) -> Result<oneshot::Receiver<FrontendEffectResponse>, FrontendEffectError> {
        let (sender, receiver) = oneshot::channel();
        let mut pending = self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        if pending.contains_key(&effect_id) {
            return Err(FrontendEffectError::DuplicateEffectId(effect_id));
        }

        pending.insert(effect_id, PendingEffect { binding, sender });
        Ok(receiver)
    }

    /// 完成一次等待中的前端交互，并把结果发送给对应 Future。
    pub(crate) fn complete(&self, response: FrontendEffectResponse) -> Result<(), FrontendEffectError> {
        let effect_id = response.effect_id.clone();
        let sender = {
            let mut pending = self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            pending.remove(&effect_id)
        };

        match sender {
            Some(pending) if pending.binding == response.binding => pending.sender.send(response).map_err(|_| FrontendEffectError::WaiterDropped),
            Some(_) => Err(FrontendEffectError::BindingMismatch(effect_id)),
            None => Err(FrontendEffectError::UnknownEffectId(effect_id)),
        }
    }

    /// 清理指定 effect，通常用于 emit 失败、等待超时或调用方取消。
    pub(crate) fn remove(&self, effect_id: &str) {
        let mut pending = self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.remove(effect_id);
    }

    /// 读取当前等待数量，仅用于测试和诊断。
    #[cfg(test)]
    pub(crate) fn pending_count(&self) -> usize {
        let pending = self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.len()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::FrontendEffectRegistry;
    use crate::backend::frontend_effect::types::{FrontendEffectBinding, FrontendEffectResponse};

    fn binding() -> FrontendEffectBinding {
        FrontendEffectBinding {
            request_hash: "hash".to_string(),
            tool_name: "tool".to_string(),
            tool_call_id: "effect".to_string(),
            task_id: Some("task".to_string()),
            run_id: Some("run".to_string()),
            turn_id: Some("turn".to_string()),
        }
    }

    #[tokio::test]
    async fn complete_routes_response_to_matching_waiter() {
        let registry = FrontendEffectRegistry::default();
        let binding = binding();
        let receiver = registry
            .register("effect-1".to_string(), binding.clone())
            .expect("effect should register");

        registry
            .complete(FrontendEffectResponse { effect_id: "effect-1".to_string(), binding, ok: true, data: Some(json!({ "value": 1 })), error: None })
            .expect("effect should complete");

        let response = receiver.await.expect("response should arrive");
        assert_eq!(response.effect_id, "effect-1");
        assert_eq!(response.data.unwrap()["value"], 1);
        assert_eq!(registry.pending_count(), 0);
    }

    #[test]
    fn duplicate_effect_id_is_rejected() {
        let registry = FrontendEffectRegistry::default();

        registry
            .register("effect-1".to_string(), binding())
            .expect("first register should succeed");

        let error = registry
            .register("effect-1".to_string(), binding())
            .expect_err("duplicate effect id should fail");

        assert!(error.to_string().contains("重复"));
        assert_eq!(registry.pending_count(), 1);
    }

    #[test]
    fn unknown_effect_id_is_rejected() {
        let registry = FrontendEffectRegistry::default();
        let error = registry
            .complete(FrontendEffectResponse { effect_id: "missing".to_string(), binding: binding(), ok: true, data: Some(json!(null)), error: None })
            .expect_err("unknown effect id should fail");

        assert!(error.to_string().contains("不存在"));
    }

    #[test]
    fn mismatched_binding_is_rejected() {
        let registry = FrontendEffectRegistry::default();
        registry
            .register("effect-1".to_string(), binding())
            .expect("effect should register");
        let error = registry
            .complete(FrontendEffectResponse {
                effect_id: "effect-1".to_string(),
                binding: FrontendEffectBinding { request_hash: "other".to_string(), ..binding() },
                ok: true,
                data: Some(json!(null)),
                error: None,
            })
            .expect_err("cross-request response must fail");
        assert!(error.to_string().contains("绑定"));
    }
}
