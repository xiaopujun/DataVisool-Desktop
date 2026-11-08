/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI Agent 入口输入上下文结构。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::ai_driven::model_invocation::ModelAttachmentRole;

/// AI 驱动任务提交协议版本。
pub(crate) const AI_DRIVEN_TASK_SCHEMA_VERSION: &str = "ai-driven-task/v3";

/// 用户入口请求和初始上下文快照。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentRunInput {
    /// 入口请求协议版本。
    #[serde(default = "default_source_schema_version")]
    pub source_schema_version: String,
    /// 当前可见的用户需求。用户补充信息会合并到该字段。
    #[serde(default)]
    pub requirement: String,
    /// 用户最初输入的需求。只有补充信息改写 requirement 后才会写入。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_requirement: Option<String>,
    /// 用户补充信息记录。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_input_responses: Vec<AgentUserInputResponse>,
    /// 用户上传或前端生成的附件引用。
    #[serde(default)]
    pub attachments: Vec<AgentInputAttachment>,
    /// 请求发起时的画布宽高快照。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canvas_snapshot: Option<AgentCanvasSnapshot>,
    /// 请求发起时的选中组件上下文。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<AgentSelectionContext>,
    /// 前端环境信息。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_context: Option<AgentClientContext>,
    /// 快捷指令上下文；仅作为统一 Agent 的意图提示。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<AgentShortcutContext>,
}

impl AgentRunInput {
    /// 创建最小入口输入上下文。
    pub(crate) fn new(requirement: impl Into<String>) -> Self {
        Self { requirement: requirement.into(), ..Self::default() }
    }

    /// 读取原始需求；没有补充信息时返回当前需求。
    pub(crate) fn original_requirement_text(&self) -> String {
        self.original_requirement
            .as_deref()
            .unwrap_or(&self.requirement)
            .trim()
            .to_string()
    }

    /// 读取选中组件列表。
    pub(crate) fn selected_components(&self) -> &[AgentSelectedComponent] {
        self.selection
            .as_ref()
            .map(|selection| selection.components.as_slice())
            .unwrap_or_default()
    }

    /// 读取入口附件 id。
    pub(crate) fn attachment_ids(&self) -> Vec<String> {
        self.attachments
            .iter()
            .map(|attachment| attachment.attachment_id.trim())
            .filter(|attachment_id| !attachment_id.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    /// 追加用户补充信息，并把当前需求改写成“原始需求 + 补充信息”。
    pub(crate) fn append_user_input_response(&mut self, round: usize, source: impl Into<String>, response: Value, received_at: i64) {
        let base_requirement = self.original_requirement_text();
        if self.original_requirement.is_none() {
            self.original_requirement = Some(base_requirement.clone());
        }
        self.user_input_responses
            .push(AgentUserInputResponse { round, source: source.into(), response, received_at });

        let supplement = serde_json::to_string_pretty(&self.user_input_responses).unwrap_or_else(|_| "[]".to_string());
        self.requirement = format!("{base_requirement}\n\n用户补充信息：\n{supplement}");
    }
}

impl Default for AgentRunInput {
    fn default() -> Self {
        Self {
            source_schema_version: default_source_schema_version(),
            requirement: String::new(),
            original_requirement: None,
            user_input_responses: Vec::new(),
            attachments: Vec::new(),
            canvas_snapshot: None,
            selection: None,
            client_context: None,
            shortcut: None,
        }
    }
}

/// 用户补充信息记录。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentUserInputResponse {
    /// 第几轮补充。
    pub round: usize,
    /// 补充来源。
    pub source: String,
    /// 用户补充内容，形态不固定，保留 JSON。
    pub response: Value,
    /// 接收时间。
    pub received_at: i64,
}

/// AI 任务附件引用。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentInputAttachment {
    /// 附件用途。
    pub role: ModelAttachmentRole,
    /// 附件主键。
    pub attachment_id: String,
    /// 上传请求 id。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_request_id: Option<String>,
    /// 用户上传时的原始文件名。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_name: Option<String>,
    /// 后端归一化后的 MIME 类型。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// 文件内容 SHA-256 哈希。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// 图片宽度。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    /// 图片高度。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
}

/// 请求发起时的画布快照。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentCanvasSnapshot {
    /// 画布宽度。
    pub width: f64,
    /// 画布高度。
    pub height: f64,
    /// 请求发起时的设计器 Manager ID，用于阻止运行期间切换画布后误写。
    #[serde(default)]
    pub manager_id: Option<String>,
}

/// 请求发起时的选中组件上下文。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSelectionContext {
    /// 选区模式。
    #[serde(default)]
    pub mode: String,
    /// 选中组件 id 列表。
    #[serde(default)]
    pub target_ids: Vec<String>,
    /// 当前选中组件快照。
    #[serde(default)]
    pub components: Vec<AgentSelectedComponent>,
}

/// 选中组件快照。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSelectedComponent {
    /// 组件图层 id。
    #[serde(default)]
    pub id: String,
    /// 组件类型。
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub component_type: Option<String>,
    /// 组件名称。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 图层布局快照。任务规划会读取位置、尺寸和层级信息。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<AgentSelectedComponentLayer>,
}

impl AgentSelectedComponent {
    /// 读取组件类型文本。
    pub(crate) fn component_type_text(&self) -> String {
        self.component_type.as_deref().unwrap_or_default().trim().to_string()
    }
}

/// 选中组件图层布局快照。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSelectedComponentLayer {
    /// 图层左上角横坐标。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    /// 图层左上角纵坐标。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// 图层宽度。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    /// 图层高度。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// 图层顺序。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<i64>,
}

/// 前端环境信息。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentClientContext {
    /// 前端当前语言。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// 客户端时区。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// 前端版本或构建号。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
}

/// AI 驱动快捷指令提交上下文。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentShortcutContext {
    /// 用户选择的快捷指令。
    pub command: AgentShortcutCommand,
    /// 用户输入区原始文本。
    pub raw_text: String,
}

/// AI 驱动快捷指令名称。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AgentShortcutCommand {
    /// 样式快捷指令。
    Style,
    /// 数据快捷指令。
    Data,
}

fn default_source_schema_version() -> String {
    AI_DRIVEN_TASK_SCHEMA_VERSION.to_string()
}
