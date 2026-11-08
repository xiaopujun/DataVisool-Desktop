/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 可版本化的 Task 工作状态，不承载 Thread 展示历史。

use serde::{
    de::{Deserializer, Error as DeError},
    Deserialize, Serialize,
};
use serde_json::Value;

use super::{AgentPendingSnapshot, RequiredCanvasEffect};

pub(crate) const WORKING_CHECKPOINT_SCHEMA_VERSION: u16 = 4;
const MAX_COMPONENT_CATALOG_CHECKPOINT_BYTES: usize = 6_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentWorkingPlanStep {
    pub step: String,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentWorkingCheckpointState {
    pub schema_version: u16,
    pub objective: String,
    #[serde(default)]
    pub plan: Vec<AgentWorkingPlanStep>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_step: Option<String>,
    #[serde(default)]
    pub operation_ids: Vec<String>,
    /// Host-owned recovery facts (budgets, error counters and compacted IDs).
    /// Object IDs here are diagnostic state only and never grant permissions.
    #[serde(default)]
    pub run_recovery_state: Value,
    #[serde(default)]
    pub required_canvas_effects: Vec<RequiredCanvasEffect>,
    /// Persisted facts used to restore completion-gate state after restart.
    #[serde(default)]
    pub canvas_write_started: bool,
    #[serde(default)]
    pub workspace_write_started: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canvas_revision: Option<String>,
    /// 首次组件目录读取成功后的模型可见回执；用于同一任务恢复时复用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_catalog_result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<AgentPendingSnapshot>,
    pub updated_at: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentWorkingCheckpointStateWire {
    schema_version: u16,
    objective: String,
    #[serde(default)]
    plan: Vec<AgentWorkingPlanStep>,
    #[serde(default)]
    next_step: Option<String>,
    #[serde(default)]
    operation_ids: Vec<String>,
    #[serde(default)]
    run_recovery_state: Value,
    #[serde(default)]
    required_canvas_effects: Vec<RequiredCanvasEffect>,
    #[serde(default)]
    canvas_write_started: bool,
    #[serde(default)]
    workspace_write_started: bool,
    #[serde(default)]
    workspace_revision: Option<u64>,
    #[serde(default)]
    canvas_revision: Option<String>,
    #[serde(default)]
    component_catalog_result: Option<String>,
    #[serde(default)]
    pending: Option<AgentPendingSnapshot>,
    updated_at: i64,
}

impl<'de> Deserialize<'de> for AgentWorkingCheckpointState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AgentWorkingCheckpointStateWire::deserialize(deserializer)?;
        if wire.schema_version != WORKING_CHECKPOINT_SCHEMA_VERSION {
            return Err(D::Error::custom(format!("不支持的工作检查点版本：{}", wire.schema_version)));
        }
        Ok(Self {
            schema_version: wire.schema_version,
            objective: wire.objective,
            plan: wire.plan,
            next_step: wire.next_step,
            operation_ids: wire.operation_ids,
            run_recovery_state: wire.run_recovery_state,
            required_canvas_effects: wire.required_canvas_effects,
            canvas_write_started: wire.canvas_write_started,
            workspace_write_started: wire.workspace_write_started,
            workspace_revision: wire.workspace_revision,
            canvas_revision: wire.canvas_revision,
            component_catalog_result: wire.component_catalog_result,
            pending: wire.pending,
            updated_at: wire.updated_at,
        })
    }
}

impl AgentWorkingCheckpointState {
    pub(crate) fn initial(objective: impl Into<String>, updated_at: i64) -> Self {
        Self {
            schema_version: WORKING_CHECKPOINT_SCHEMA_VERSION,
            objective: objective.into(),
            plan: Vec::new(),
            next_step: Some("可选更新计划；按需执行宿主允许的操作".to_string()),
            operation_ids: Vec::new(),
            run_recovery_state: Value::Null,
            required_canvas_effects: Vec::new(),
            canvas_write_started: false,
            workspace_write_started: false,
            workspace_revision: None,
            canvas_revision: None,
            component_catalog_result: None,
            pending: None,
            updated_at,
        }
    }

    /// Parse and validate a checkpoint at every persistence read boundary.
    ///
    /// Reject unknown versions and malformed records before restoring a task.
    pub(crate) fn from_json(value: &str) -> Result<Self, String> {
        let state: Self = serde_json::from_str(value).map_err(|error| format!("工作检查点 JSON 无效：{error}"))?;
        state.validate().map_err(|error| format!("工作检查点校验失败：{error}"))?;
        Ok(state)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != WORKING_CHECKPOINT_SCHEMA_VERSION {
            return Err(format!("不支持的工作检查点版本：{}", self.schema_version));
        }
        if !self.run_recovery_state.is_null() && !self.run_recovery_state.is_object() {
            return Err("工作检查点 runRecoveryState 必须是对象".to_string());
        }
        if self.objective.trim().is_empty() {
            return Err("工作检查点 objective 不能为空".to_string());
        }
        if self.updated_at < 0 {
            return Err("工作检查点 updatedAt 不能为负数".to_string());
        }
        if self
            .component_catalog_result
            .as_ref()
            .is_some_and(|result| result.len() > MAX_COMPONENT_CATALOG_CHECKPOINT_BYTES)
        {
            return Err("工作检查点组件目录超过大小限制".to_string());
        }
        if self.plan.len() > 20 || self.operation_ids.len() > 128 || self.required_canvas_effects.len() > 3 {
            return Err("工作检查点条目超过限制".to_string());
        }
        if self
            .required_canvas_effects
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != self.required_canvas_effects.len()
        {
            return Err("工作检查点 requiredCanvasEffects 不得重复".to_string());
        }
        if self
            .plan
            .iter()
            .any(|step| step.step.trim().is_empty() || !matches!(step.status.as_str(), "pending" | "in_progress" | "completed"))
        {
            return Err("工作检查点 plan 无效".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentWorkingCheckpoint {
    pub task_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub checkpoint_version: u64,
    pub state: AgentWorkingCheckpointState,
    pub updated_at: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct SaveAgentWorkingCheckpoint {
    pub project_id: i64,
    pub thread_id: String,
    pub task_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub expected_version: u64,
    pub state: AgentWorkingCheckpointState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_checkpoint_is_valid_and_has_next_step() {
        let checkpoint = AgentWorkingCheckpointState::initial("创建图表", 10);
        checkpoint.validate().unwrap();
        assert_eq!(checkpoint.next_step.as_deref(), Some("可选更新计划；按需执行宿主允许的操作"));
    }

    #[test]
    fn v4_checkpoint_preserves_recovery_state_as_non_authorizing_data() {
        let state: AgentWorkingCheckpointState = serde_json::from_value(serde_json::json!({
            "schemaVersion": 4,
            "objective": "继续处理",
            "operationIds": ["run-a:call-1"],
            "runRecoveryState": {
                "usage": {"inputTokens": 8},
                "compactedOperations": ["run-a:call-1"],
                "createdObjectIds": ["component-a"]
            },
            "updatedAt": 10
        }))
        .unwrap();

        assert_eq!(state.run_recovery_state["usage"]["inputTokens"], 8);
        assert_eq!(state.run_recovery_state["createdObjectIds"], serde_json::json!(["component-a"]));
        assert!(state.required_canvas_effects.is_empty());
        state.validate().unwrap();
    }

    #[test]
    fn unknown_checkpoint_version_is_rejected_during_deserialization() {
        let value = serde_json::json!({
            "schemaVersion": 99,
            "objective": "解释页面",
            "updatedAt": 10
        });
        let error = serde_json::from_value::<AgentWorkingCheckpointState>(value).unwrap_err();
        assert!(error.to_string().contains("不支持的工作检查点版本"));
    }

    #[test]
    fn rejects_old_checkpoint_versions() {
        for version in [1, 2, 3] {
            let json = format!(r#"{{"schemaVersion":{version},"objective":"创建图表","updatedAt":10}}"#);
            assert!(AgentWorkingCheckpointState::from_json(&json).is_err());
        }
    }
}
