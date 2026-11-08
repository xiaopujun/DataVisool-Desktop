/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 当前项目 Workspace 的受限文件 Tool。

use rig_core::tool::{Tool, ToolFailure};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceSearchRequest;
use crate::backend::ai_driven::ripgrep::{RipgrepSearchOptions, SearchCaseMode, SearchMode};
use crate::backend::ai_driven::security::AgentOperation;

use super::error::AgentToolError;
use super::output::{fit_model_output, ModelOutputBudget};
use super::policy::AgentToolEffect;
use super::runtime::AgentToolRuntime;

pub(crate) const LIST_WORKSPACE_FILES_TOOL: &str = "list_workspace_files";
pub(crate) const SEARCH_WORKSPACE_FILES_TOOL: &str = "search_workspace_files";
pub(crate) const READ_WORKSPACE_FILE_TOOL: &str = "read_workspace_file";
pub(crate) const APPLY_WORKSPACE_PATCH_TOOL: &str = "apply_workspace_patch";
pub(crate) const MOVE_WORKSPACE_PATH_TOOL: &str = "move_workspace_path";
pub(crate) const DELETE_WORKSPACE_PATH_TOOL: &str = "delete_workspace_path";

#[derive(Clone)]
pub(crate) struct ListWorkspaceFilesTool(pub(crate) AgentToolRuntime);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ListWorkspaceFilesArgs {
    #[serde(default)]
    path: String,
    #[serde(default = "default_depth")]
    depth: usize,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_list_limit")]
    limit: usize,
}

impl Tool for ListWorkspaceFilesTool {
    const NAME: &'static str = LIST_WORKSPACE_FILES_TOOL;
    type Error = AgentToolError;
    type Args = ListWorkspaceFilesArgs;
    type Output = Value;

    fn description(&self) -> String {
        "列出当前项目 Workspace 的受限相对路径、文件类型、大小和修改版本；不会返回物理路径。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "可选 Workspace 相对目录；根目录传空字符串" },
                "depth": { "type": "integer", "minimum": 1, "maximum": 32, "default": 3 },
                "offset": {"type": "integer", "minimum": 0, "default": 0},
                "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 100}
            },
            "additionalProperties": false
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0.context().authorize(Self::NAME, AgentToolEffect::Read, false)?;
        let entries = self.0.files().list(Some(&args.path), args.depth)?;
        let total_entries = entries.len();
        let limit = args.limit.clamp(1, 200);
        let page = entries.into_iter().skip(args.offset).take(limit).collect::<Vec<_>>();
        let has_more = args.offset.saturating_add(page.len()) < total_entries;
        bounded_output(json!({
            "workspaceRevision": self.0.files().workspace_revision()?,
            "entries": page,
            "totalEntries": total_entries,
            "offset": args.offset,
            "hasMore": has_more,
            "nextOffset": has_more.then_some(args.offset.saturating_add(limit))
        }))
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

#[derive(Clone)]
pub(crate) struct SearchWorkspaceFilesTool(pub(crate) AgentToolRuntime);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SearchWorkspaceFilesArgs {
    query: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    mode: SearchMode,
    #[serde(default)]
    case_mode: SearchCaseMode,
    #[serde(default)]
    globs: Vec<String>,
    #[serde(default)]
    include_hidden: bool,
    #[serde(default = "default_true")]
    respect_ignore_files: bool,
    #[serde(default = "default_search_results")]
    max_results: usize,
}

impl Tool for SearchWorkspaceFilesTool {
    const NAME: &'static str = SEARCH_WORKSPACE_FILES_TOOL;
    type Error = AgentToolError;
    type Args = SearchWorkspaceFilesArgs;
    type Output = Value;

    fn description(&self) -> String {
        "使用受控 ripgrep 按文件路径和内容搜索当前项目 Workspace；支持普通文本、正则、大小写、Glob 和忽略文件规则。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "minLength": 1, "maxLength": 1024 },
                "path": { "type": "string", "description": "可选 Workspace 相对目录" },
                "mode": { "type": "string", "enum": ["literal", "regex"], "default": "literal" },
                "caseMode": {
                    "type": "string",
                    "enum": ["sensitive", "insensitive", "smart"],
                    "default": "sensitive"
                },
                "globs": {
                    "type": "array",
                    "items": { "type": "string", "minLength": 1, "maxLength": 256 },
                    "maxItems": 20,
                    "default": []
                },
                "includeHidden": { "type": "boolean", "default": false },
                "respectIgnoreFiles": { "type": "boolean", "default": true },
                "maxResults": { "type": "integer", "minimum": 1, "maximum": 100, "default": 50 }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0.context().authorize(Self::NAME, AgentToolEffect::Read, false)?;
        let response = self
            .0
            .workspace_search()
            .search(
                WorkspaceSearchRequest {
                    path: args.path,
                    options: RipgrepSearchOptions {
                        query: args.query,
                        mode: args.mode,
                        case_mode: args.case_mode,
                        globs: args.globs,
                        include_hidden: args.include_hidden,
                        respect_ignore_files: args.respect_ignore_files,
                        max_results: args.max_results,
                    },
                },
                self.0.context().cancel_receiver(),
            )
            .await?;
        serde_json::to_value(response).map_err(|error| AgentToolError::internal("agent.tool.serialize_failed", error.to_string()))
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

#[derive(Clone)]
pub(crate) struct ReadWorkspaceFileTool(pub(crate) AgentToolRuntime);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReadWorkspaceFileArgs {
    path: String,
    #[serde(default = "first_line")]
    start_line: usize,
    #[serde(default = "default_end_line")]
    end_line: usize,
}

impl Tool for ReadWorkspaceFileTool {
    const NAME: &'static str = READ_WORKSPACE_FILE_TOOL;
    type Error = AgentToolError;
    type Args = ReadWorkspaceFileArgs;
    type Output = Value;

    fn description(&self) -> String {
        "按行读取当前项目 Workspace 中的 UTF-8 文本文件，并返回内容哈希、文件修改版本和 Workspace 版本。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "minLength": 1 },
                "startLine": { "type": "integer", "minimum": 1, "default": 1 },
                "endLine": { "type": "integer", "minimum": 1, "default": 160 }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0.context().authorize(Self::NAME, AgentToolEffect::Read, false)?;
        if args.end_line < args.start_line || args.end_line.saturating_sub(args.start_line) >= 400 {
            return Err(AgentToolError::invalid("agent.workspace.read_range_too_large", "单次最多读取 400 行，请缩小范围"));
        }
        let excerpt = self.0.files().read(&args.path, args.start_line, args.end_line)?;
        let has_more = excerpt.end_line < excerpt.total_lines;
        bounded_output(json!({"excerpt": excerpt, "hasMore": has_more, "nextStartLine": has_more.then_some(args.end_line.saturating_add(1))}))
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

#[derive(Clone)]
pub(crate) struct ApplyWorkspacePatchTool(pub(crate) AgentToolRuntime);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApplyPatchArgs {
    expected_revision: u64,
    patch: String,
}

impl Tool for ApplyWorkspacePatchTool {
    const NAME: &'static str = APPLY_WORKSPACE_PATCH_TOOL;
    type Error = AgentToolError;
    type Args = ApplyPatchArgs;
    type Output = Value;

    fn description(&self) -> String {
        "以 Codex Patch 在一次事务中新增、更新、移动或删除多个 Workspace 文本或 JSON 文件。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "expectedRevision": { "type": "integer", "minimum": 0 },
                "patch": { "type": "string", "description": "以 *** Begin Patch 开始、*** End Patch 结束的完整 Patch" }
            },
            "required": ["expectedRevision", "patch"],
            "additionalProperties": false
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0.context().authorize(Self::NAME, AgentToolEffect::WorkspaceWrite, false)?;
        authorize_workspace_write(&self.0)?;
        let _mutation = self.0.begin_workspace_mutation()?;
        let result = self.0.files().apply_patch(&args.patch, args.expected_revision)?;
        self.0.completion_requirements().record_workspace_write();
        serde_json::to_value(result).map_err(|error| AgentToolError::internal("agent.tool.serialize_failed", error.to_string()))
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

#[derive(Clone)]
pub(crate) struct MoveWorkspacePathTool(pub(crate) AgentToolRuntime);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MoveWorkspacePathArgs {
    from: String,
    to: String,
    expected_revision: u64,
}

impl Tool for MoveWorkspacePathTool {
    const NAME: &'static str = MOVE_WORKSPACE_PATH_TOOL;
    type Error = AgentToolError;
    type Args = MoveWorkspacePathArgs;
    type Output = Value;

    fn description(&self) -> String {
        "在一次 Workspace 事务中移动或重命名单个文件或整个目录树。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "minLength": 1 },
                "to": { "type": "string", "minLength": 1 },
                "expectedRevision": { "type": "integer", "minimum": 0 }
            },
            "required": ["from", "to", "expectedRevision"],
            "additionalProperties": false
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0.context().authorize(Self::NAME, AgentToolEffect::WorkspaceWrite, false)?;
        authorize_workspace_write(&self.0)?;
        let _mutation = self.0.begin_workspace_mutation()?;
        let result = self.0.files().move_path(&args.from, &args.to, args.expected_revision)?;
        self.0.completion_requirements().record_workspace_write();
        serde_json::to_value(result).map_err(|error| AgentToolError::internal("agent.tool.serialize_failed", error.to_string()))
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

#[derive(Clone)]
pub(crate) struct DeleteWorkspacePathTool(pub(crate) AgentToolRuntime);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeleteWorkspacePathArgs {
    path: String,
    expected_revision: u64,
}

impl Tool for DeleteWorkspacePathTool {
    const NAME: &'static str = DELETE_WORKSPACE_PATH_TOOL;
    type Error = AgentToolError;
    type Args = DeleteWorkspacePathArgs;
    type Output = Value;

    fn description(&self) -> String {
        "在一次 Workspace 事务中删除单个文件或整个目录树；宿主快照可用于恢复。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "minLength": 1 },
                "expectedRevision": { "type": "integer", "minimum": 0 }
            },
            "required": ["path", "expectedRevision"],
            "additionalProperties": false
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.0.context().authorize(Self::NAME, AgentToolEffect::WorkspaceWrite, false)?;
        authorize_workspace_write(&self.0)?;
        let _mutation = self.0.begin_workspace_mutation()?;
        let result = self.0.files().delete_path(&args.path, args.expected_revision)?;
        self.0.completion_requirements().record_workspace_write();
        serde_json::to_value(result).map_err(|error| AgentToolError::internal("agent.tool.serialize_failed", error.to_string()))
    }

    fn classify_error(&self, error: &Self::Error) -> ToolFailure {
        error.to_tool_failure()
    }
}

fn default_depth() -> usize {
    3
}

fn authorize_workspace_write(runtime: &AgentToolRuntime) -> Result<(), AgentToolError> {
    let context = runtime.context();
    context.security().grant().validate_scope(context.scope())?;
    context.security().grant().authorize_operation(AgentOperation::WorkspaceWrite)?;
    context.security().grant().authorize_workspace(runtime.files().workspace_id())?;
    Ok(())
}

fn default_list_limit() -> usize {
    100
}

fn default_search_results() -> usize {
    50
}

fn default_true() -> bool {
    true
}

fn first_line() -> usize {
    1
}

fn default_end_line() -> usize {
    160
}

fn bounded_output(value: Value) -> Result<Value, AgentToolError> {
    fit_model_output(value, ModelOutputBudget::Workspace, "缩小 limit、行范围或搜索目录")
}
