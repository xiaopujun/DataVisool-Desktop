/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Agent 最终回复与宿主事实检查之间的稳定边界。

use std::future::Future;
use std::pin::Pin;

use super::AgentRunFailure;

pub(crate) type CompletionGateFuture<'a> = Pin<Box<dyn Future<Output = Result<AgentCompletionDecision, AgentRunFailure>> + Send + 'a>>;

/// 模型决定任务是否完成；宿主只返回可确定的运行事实结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AgentCompletionDecision {
    Complete,
    Continue { prompt: String },
    Fail { code: String, message: String },
}

pub(crate) trait AgentCompletionGate: Send + Sync {
    fn evaluate<'a>(&'a self, final_text: &'a str) -> CompletionGateFuture<'a>;
}
