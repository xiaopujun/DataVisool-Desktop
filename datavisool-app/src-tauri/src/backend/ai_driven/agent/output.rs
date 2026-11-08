/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 与 Rig 响应格式隔离的项目运行输出。

use serde::Serialize;

use crate::backend::ai_driven::domain::operation::CanvasOperationReceipt;

use super::ExecutionUsageSnapshot;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentRunOutput {
    /// 仅保留最终简短文本，不把它当作任务完成凭证。
    pub final_text: String,
    /// 只能由持久化 operation journal 填充。
    pub operations: Vec<CanvasOperationReceipt>,
    pub usage: ExecutionUsageSnapshot,
}

#[derive(Clone, Debug)]
pub(crate) struct AgentRunFailure {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for AgentRunFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AgentRunFailure {}
