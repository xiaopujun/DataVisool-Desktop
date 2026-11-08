/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use rig_core::tool::{Tool, ToolFailure};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::ai_driven::domain::operation::{CanvasOperationKind, CanvasOperationReceipt, CanvasOperationStatus};
use crate::backend::ai_driven::tools::error::AgentToolError;
use crate::backend::ai_driven::tools::policy::AgentToolEffect;
use crate::backend::ai_driven::tools::runtime::AgentToolRuntime;

use super::{sync_security_resource, CreateCanvasComponentsTool, DeleteCanvasComponentsTool, PatchCanvasComponentsTool};

pub(crate) const GET_CANVAS_OPERATION_STATUS_TOOL: &str = "get_canvas_operation_status";

#[derive(Clone)]
pub(crate) struct GetCanvasOperationStatusTool(pub(crate) AgentToolRuntime);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GetCanvasOperationStatusArgs {
    operation_id: String,
}

impl Tool for GetCanvasOperationStatusTool {
    const NAME: &'static str = GET_CANVAS_OPERATION_STATUS_TOOL;
    type Error = AgentToolError;
    type Args = GetCanvasOperationStatusArgs;
    type Output = CanvasOperationReceipt;

    fn description(&self) -> String {
        "查询并校准 prepared/applying 画布动作的真实状态；只能查询当前任务已有的 operationId。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"operationId": {"type": "string", "minLength": 1}},
            "required": ["operationId"],
            "additionalProperties": false
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0
            .context()
            .authorize(Self::NAME, AgentToolEffect::FrontendInteraction, false)?;
        let operation_id = args.operation_id.trim();
        if operation_id.is_empty() {
            return Err(AgentToolError::invalid("agent.operation.id_required", "operationId 不能为空"));
        }
        let receipt = self.0.operation_journal()?.require(operation_id).await?.receipt;
        if receipt.status != CanvasOperationStatus::Applying {
            sync_security_resource(&self.0, &receipt)?;
            return Ok(receipt);
        }
        match receipt.kind {
            CanvasOperationKind::CreateComponents => CreateCanvasComponentsTool(self.0.clone()).recover_applying(receipt).await,
            CanvasOperationKind::PatchComponents => PatchCanvasComponentsTool(self.0.clone()).recover_applying(receipt).await,
            CanvasOperationKind::DeleteComponents => DeleteCanvasComponentsTool(self.0.clone()).recover_applying(receipt).await,
        }
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}
