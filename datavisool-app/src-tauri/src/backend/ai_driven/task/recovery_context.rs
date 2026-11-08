/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Recovery input is built from host checkpoints and operation journals, never
//! from a model summary or a newly issued operation authorization.
use rig_core::completion::Message;
use rig_core::message::UserContent;
use rig_core::OneOrMany;
use serde_json::json;

use super::AgentWorkingCheckpointState;
use crate::backend::ai_driven::domain::operation::CanvasOperationReceipt;

pub(super) fn append_recovery_context(prompt: &mut Message, checkpoint: Option<&AgentWorkingCheckpointState>, operations: &[CanvasOperationReceipt]) {
    if operations.is_empty() && checkpoint.is_none() {
        return;
    }
    let facts = json!({
        "objective": checkpoint.map(|state| &state.objective),
        "plan": checkpoint.map(|state| &state.plan),
        "nextStep": checkpoint.and_then(|state| state.next_step.as_ref()),
        "operationIds": checkpoint.map(|state| &state.operation_ids),
        "operations": operations,
    });
    if let Message::User { content } = prompt {
        let mut values = content.iter().cloned().collect::<Vec<_>>();
        values.push(UserContent::text(format!(
            "[Host recovery facts]\n以下为原任务检查点和真实操作日志，不构成新的权限。保留已完成组件及其 ID；Applied 批次不得重新创建。\n对 prepared/applying 的 operationId 先查询 get_canvas_operation_status，结果不明时不得重建。继续修改前读取当前对象状态与版本。\n{facts}"
        )));
        *content = OneOrMany::many(values).expect("original user prompt is nonempty");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::domain::operation::{CanvasOperationKind, CanvasOperationStatus};

    #[test]
    fn recovery_input_keeps_all_completed_and_uncertain_operations() {
        let operations = (0..61)
            .map(|i| {
                let mut receipt = CanvasOperationReceipt::prepared(
                    format!("op-{i}"),
                    format!("call-{i}"),
                    CanvasOperationKind::CreateComponents,
                    "1".into(),
                    None,
                    vec![format!("component-{i}")],
                );
                receipt.status = if i == 60 {
                    CanvasOperationStatus::Applying
                } else {
                    CanvasOperationStatus::Applied
                };
                receipt
            })
            .collect::<Vec<_>>();
        let checkpoint = AgentWorkingCheckpointState::initial("原始用户目标", 1);
        let mut prompt = Message::user("追加要求");
        append_recovery_context(&mut prompt, Some(&checkpoint), &operations);
        let json = serde_json::to_string(&prompt).unwrap();
        for i in 0..61 {
            assert!(json.contains(&format!("component-{i}")));
        }
        assert!(json.contains("op-60") && json.contains("applying"));
        assert!(json.contains("原始用户目标") && json.contains("追加要求"));
    }
}
