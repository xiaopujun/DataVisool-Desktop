/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Rig 动态 Tool 的统一单次执行超时边界。

use std::fmt::{Display, Formatter};
use std::time::Duration;

use rig_core::tool::{ToolCallExtensions, ToolDyn, ToolError, ToolExecutionResult, ToolFailure};
use rig_core::wasm_compat::WasmBoxedFuture;
use serde_json::Value;

use super::limits::AgentExecutionErrorCode;

const TOOL_TIMEOUT_MESSAGE: &str = "单次工具执行时间超过上限";

pub(super) fn with_tool_timeout(tools: Vec<Box<dyn ToolDyn>>, max_duration: Duration) -> Vec<Box<dyn ToolDyn>> {
    tools
        .into_iter()
        .map(|tool| Box::new(TimedTool { inner: tool, max_duration }) as Box<dyn ToolDyn>)
        .collect()
}

struct TimedTool {
    inner: Box<dyn ToolDyn>,
    max_duration: Duration,
}

impl ToolDyn for TimedTool {
    fn name(&self) -> String {
        self.inner.name()
    }

    fn description(&self) -> String {
        self.inner.description()
    }

    fn parameters(&self) -> Value {
        self.inner.parameters()
    }

    fn call<'a>(&'a self, args: String) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
        Box::pin(async move {
            tokio::time::timeout(self.max_duration, self.inner.call(args))
                .await
                .unwrap_or_else(|_| Err(timeout_error()))
        })
    }

    fn call_with_extensions<'a>(&'a self, args: String, extensions: &'a ToolCallExtensions) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
        Box::pin(async move {
            tokio::time::timeout(self.max_duration, self.inner.call_with_extensions(args, extensions))
                .await
                .unwrap_or_else(|_| Err(timeout_error()))
        })
    }

    fn call_structured<'a>(&'a self, args: String, extensions: &'a ToolCallExtensions) -> WasmBoxedFuture<'a, ToolExecutionResult> {
        Box::pin(async move {
            match tokio::time::timeout(self.max_duration, self.inner.call_structured(args, extensions)).await {
                Ok(result) => result,
                Err(_) => timeout_result(),
            }
        })
    }
}

fn timeout_result() -> ToolExecutionResult {
    let code = AgentExecutionErrorCode::ToolDurationLimit.as_str();
    ToolExecutionResult::failed(
        format!("{code}: {TOOL_TIMEOUT_MESSAGE}"),
        ToolFailure::timeout(TOOL_TIMEOUT_MESSAGE).with_retryable(false).with_code(code),
    )
}

fn timeout_error() -> ToolError {
    ToolError::ToolCallError(Box::new(ToolDurationLimitError))
}

#[derive(Debug)]
struct ToolDurationLimitError;

impl Display for ToolDurationLimitError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {TOOL_TIMEOUT_MESSAGE}", AgentExecutionErrorCode::ToolDurationLimit.as_str())
    }
}

impl std::error::Error for ToolDurationLimitError {}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use rig_core::tool::{ToolFailureKind, ToolOutcome};

    use super::*;

    struct PendingTool {
        dropped: Arc<AtomicBool>,
    }

    impl ToolDyn for PendingTool {
        fn name(&self) -> String {
            "pending_tool".to_string()
        }

        fn description(&self) -> String {
            "never returns".to_string()
        }

        fn parameters(&self) -> Value {
            serde_json::json!({ "type": "object" })
        }

        fn call<'a>(&'a self, _args: String) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
            let dropped = self.dropped.clone();
            Box::pin(async move {
                let _probe = DropProbe(dropped);
                pending().await
            })
        }
    }

    struct DropProbe(Arc<AtomicBool>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[tokio::test]
    async fn timeout_drops_a_tool_future_that_never_returns() {
        let dropped = Arc::new(AtomicBool::new(false));
        let wrapped = TimedTool { inner: Box::new(PendingTool { dropped: dropped.clone() }), max_duration: Duration::from_millis(5) };

        let result = wrapped.call_structured("{}".to_string(), &ToolCallExtensions::new()).await;

        let ToolOutcome::Error(failure) = result.outcome() else {
            panic!("pending tool must end with a structured timeout");
        };
        assert_eq!(failure.kind, ToolFailureKind::Timeout);
        assert_eq!(failure.code.as_deref(), Some(AgentExecutionErrorCode::ToolDurationLimit.as_str()));
        assert_eq!(failure.retryable, Some(false));
        assert!(result
            .model_output()
            .contains(AgentExecutionErrorCode::ToolDurationLimit.as_str()));
        assert!(dropped.load(Ordering::Acquire), "timeout must drop the running tool future");
    }
}
