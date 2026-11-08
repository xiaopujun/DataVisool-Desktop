/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use rig_core::tool::{Tool, ToolFailure};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::ai_driven::tools::error::AgentToolError;
use crate::backend::ai_driven::tools::output::{fit_model_output, ModelOutputBudget};
use crate::backend::ai_driven::tools::policy::AgentToolEffect;
use crate::backend::ai_driven::tools::runtime::AgentToolRuntime;

pub(crate) const GET_CURRENT_CANVAS_SELECTION_TOOL: &str = "get_current_canvas_selection";

#[derive(Clone)]
pub(crate) struct GetCurrentCanvasSelectionTool(pub(crate) AgentToolRuntime);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetCurrentCanvasSelectionArgs {}

impl Tool for GetCurrentCanvasSelectionTool {
    const NAME: &'static str = GET_CURRENT_CANVAS_SELECTION_TOOL;
    type Error = AgentToolError;
    type Args = GetCurrentCanvasSelectionArgs;
    type Output = Value;

    fn description(&self) -> String {
        "读取调用时刻设计器的真实选区；不要把任务启动时的选区当作持续有效状态。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0
            .context()
            .authorize(Self::NAME, AgentToolEffect::FrontendInteraction, false)?;
        let response: Value = self
            .0
            .frontend()?
            .call("getCurrentCanvasSelection", json!({}), Some(std::time::Duration::from_secs(10)), self.0.context())
            .await?;
        fit_model_output(response, ModelOutputBudget::Snapshot, "缩小画布选区后重试")
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}
