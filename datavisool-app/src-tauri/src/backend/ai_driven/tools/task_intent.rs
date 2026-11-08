/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Agent 任务意图工具。
//!
//! 计划只负责向用户展示进度。真实写入由宿主能力和动作回执核验。

use rig_core::tool::{Tool, ToolFailure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::error::AgentToolError;
use super::policy::AgentToolEffect;
use super::runtime::AgentToolRuntime;

pub(crate) const UPDATE_PLAN_TOOL: &str = "update_plan";
const UPDATE_PLAN_DESCRIPTION: &str =
    "可选的任务计划工具，只保存并展示多步骤任务的进度。计划不是写入权限凭证；真实写入由宿主能力和结构化动作回执核验。若完成计划所需的关键用户决策尚未确认，应先单独调用 request_user_input，收到回答后再更新计划。";

#[derive(Clone)]
pub(crate) struct UpdatePlanTool(pub(crate) AgentToolRuntime);

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PlanStepStatus {
    Pending,
    // TODO(legacy-compat): 确认调用方统一使用 in_progress 后，移除旧状态别名。
    #[serde(alias = "inProgress")]
    InProgress,
    Completed,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdatePlanStep {
    step: String,
    status: PlanStepStatus,
}

/// `update_plan` 的输入和回执。完成要求已从计划中完全移除。
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UpdatePlanArgs {
    #[serde(default)]
    explanation: Option<String>,
    plan: Vec<UpdatePlanStep>,
}

impl Tool for UpdatePlanTool {
    const NAME: &'static str = UPDATE_PLAN_TOOL;
    type Error = AgentToolError;
    type Args = UpdatePlanArgs;
    type Output = UpdatePlanArgs;

    fn description(&self) -> String {
        UPDATE_PLAN_DESCRIPTION.to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "explanation": { "type": "string", "maxLength": 1000 },
                "plan": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 20,
                    "items": {
                        "type": "object",
                        "properties": {
                            "step": { "type": "string", "minLength": 1, "maxLength": 200 },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                        },
                        "required": ["step", "status"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["plan"],
            "additionalProperties": false
        })
    }

    async fn call(&self, mut args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0
            .context()
            .authorize(Self::NAME, AgentToolEffect::FrontendInteraction, false)?;
        normalize_plan(&mut args)?;
        Ok(args)
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

fn normalize_plan(args: &mut UpdatePlanArgs) -> Result<(), AgentToolError> {
    if args.plan.is_empty() || args.plan.len() > 20 {
        return Err(AgentToolError::invalid("agent.plan.invalid_step_count", "plan 必须包含 1 到 20 个步骤"));
    }
    let mut in_progress = 0usize;
    for item in &mut args.plan {
        item.step = required_text(std::mem::take(&mut item.step), "plan.step")?;
        if item.step.chars().count() > 200 {
            return Err(AgentToolError::invalid("agent.plan.step_too_long", "plan.step 不能超过 200 个字符"));
        }
        if matches!(item.status, PlanStepStatus::InProgress) {
            in_progress += 1;
        }
    }
    if in_progress > 1 {
        return Err(AgentToolError::invalid("agent.plan.multiple_in_progress", "同一时间最多一个计划步骤处于 in_progress"));
    }
    args.explanation = args
        .explanation
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if args.explanation.as_ref().is_some_and(|value| value.chars().count() > 1000) {
        return Err(AgentToolError::invalid("agent.plan.explanation_too_long", "explanation 不能超过 1000 个字符"));
    }
    Ok(())
}

fn required_text(value: String, field: &str) -> Result<String, AgentToolError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        Err(AgentToolError::invalid("agent.tool.empty_text", format!("{field} 不能为空")))
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_only_contains_display_fields() {
        let mut plan: UpdatePlanArgs = serde_json::from_value(json!({
            "explanation": "  分步处理  ",
            "plan": [{ "step": "  读取页面  ", "status": "in_progress" }]
        }))
        .unwrap();

        normalize_plan(&mut plan).unwrap();

        let value = serde_json::to_value(plan).unwrap();
        assert_eq!(value["explanation"], "分步处理");
        assert_eq!(value["plan"][0]["step"], "读取页面");
        assert_eq!(value["plan"][0]["status"], "in_progress");
        assert!(value.get("completionRequirements").is_none());
    }

    #[test]
    fn plan_rejects_completion_requirements_as_an_unknown_field() {
        let result = serde_json::from_value::<UpdatePlanArgs>(json!({
            "plan": [{ "step": "读取页面", "status": "pending" }],
            "completionRequirements": { "requiredCanvasEffects": [] }
        }));
        assert!(result.is_err());
    }

    #[test]
    fn plan_rejects_multiple_in_progress_steps() {
        let mut plan: UpdatePlanArgs = serde_json::from_value(json!({
            "plan": [
                { "step": "读取页面", "status": "in_progress" },
                { "step": "生成结果", "status": "in_progress" }
            ]
        }))
        .unwrap();

        let error = normalize_plan(&mut plan).unwrap_err();
        assert_eq!(error.code, "agent.plan.multiple_in_progress");
    }

    #[test]
    fn plan_description_keeps_progress_separate_from_authorization() {
        assert!(UPDATE_PLAN_DESCRIPTION.contains("计划不是写入权限凭证"));
        assert!(UPDATE_PLAN_DESCRIPTION.contains("request_user_input"));
        assert!(UPDATE_PLAN_DESCRIPTION.contains("宿主能力"));
    }
}
