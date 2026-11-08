/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 任务执行存储区的通用数据类型。
//!
//! 本模块只描述执行存储区、产物版本、补丁和诊断等基础数据。调用方通过逻辑
//! 产物 ID 使用这些能力，不能传入物理路径。

use std::fmt::{Display, Formatter};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::WORKSPACE_TEMP_MARKER;

/// 当前工作区清单格式版本。
pub(crate) const WORKSPACE_FORMAT_VERSION: u32 = 3;

/// 工作区模块统一结果类型。
pub(crate) type WorkspaceResult<T> = Result<T, WorkspaceError>;

/// 工作区基础设施错误。
#[derive(Debug)]
pub(crate) enum WorkspaceError {
    /// `workspaceId` 为空、过长或包含不安全字符。
    InvalidWorkspaceId(String),
    /// `artifactId` 格式错误或包含不安全字符。
    InvalidArtifactId(String),
    /// 同一任务的工作区已经存在。
    WorkspaceAlreadyExists(String),
    /// 指定任务工作区不存在。
    WorkspaceNotFound(String),
    /// 工作区清单无法读取、解析或与目录不一致。
    InvalidManifest(String),
    /// Agent 提供的逻辑路径不属于当前项目 Workspace。
    PathOutOfScope(String),
    /// Workspace 强制索引缺失、损坏或与真实文件不一致。
    InvalidIndex(String),
    /// Codex 文本 Patch 无法解析或预检失败。
    InvalidFilePatch(String),
    /// 文件 Tool 请求超过统一资源限制。
    ResourceLimit(String),
    /// 指定产物已经存在。
    ArtifactAlreadyExists(String),
    /// 指定产物不存在。
    ArtifactNotFound(String),
    /// 调用方基于旧版本提交修改。
    RevisionConflict { artifact_id: String, expected: u64, actual: u64 },
    /// 产物不是有效 JSON，不能执行结构化操作。
    InvalidJson { artifact_id: String, message: String },
    /// JSON Pointer 没有定位到值。
    JsonPointerNotFound { artifact_id: String, pointer: String },
    /// JSON Patch 无法应用到当前产物。
    JsonPatchFailed { artifact_id: String, message: String },
    /// 有效 JSON 产物被错误地请求使用文本补丁。
    TextPatchNotAllowed(String),
    /// 文本补丁携带的原文与指定行范围不一致。
    TextPatchContentMismatch(String),
    /// 请求的行范围不合法。
    InvalidLineRange { start_line: usize, end_line: usize },
    /// 模型提交了未知工具或不符合协议的工具参数。
    InvalidToolCall { tool: String, message: String },
    /// 文件系统或序列化操作失败。
    Storage { operation: &'static str, path: PathBuf, message: String },
}

impl Display for WorkspaceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidWorkspaceId(workspace_id) => write!(formatter, "invalid agent workspace workspaceId: {workspace_id}"),
            Self::InvalidArtifactId(artifact_id) => write!(formatter, "invalid agent workspace artifactId: {artifact_id}"),
            Self::WorkspaceAlreadyExists(run_id) => write!(formatter, "agent workspace already exists: {run_id}"),
            Self::WorkspaceNotFound(run_id) => write!(formatter, "agent workspace not found: {run_id}"),
            Self::InvalidManifest(message) => write!(formatter, "invalid agent workspace manifest: {message}"),
            Self::PathOutOfScope(path) => {
                write!(formatter, "agent.workspace.path_out_of_scope: only current project Workspace paths are allowed: {path}")
            }
            Self::InvalidIndex(message) => write!(formatter, "invalid agent workspace index: {message}"),
            Self::InvalidFilePatch(message) => write!(formatter, "invalid agent workspace file patch: {message}"),
            Self::ResourceLimit(message) => write!(formatter, "agent workspace resource limit exceeded: {message}"),
            Self::ArtifactAlreadyExists(artifact_id) => write!(formatter, "agent workspace artifact already exists: {artifact_id}"),
            Self::ArtifactNotFound(artifact_id) => write!(formatter, "agent workspace artifact not found: {artifact_id}"),
            Self::RevisionConflict { artifact_id, expected, actual } => {
                write!(
                    formatter,
                    "agent workspace artifact revision conflict: artifactId={artifact_id}, expected={expected}, actual={actual}"
                )
            }
            Self::InvalidJson { artifact_id, message } => {
                write!(formatter, "agent workspace artifact contains invalid JSON: artifactId={artifact_id}, error={message}")
            }
            Self::JsonPointerNotFound { artifact_id, pointer } => {
                write!(formatter, "agent workspace JSON pointer not found: artifactId={artifact_id}, pointer={pointer}")
            }
            Self::JsonPatchFailed { artifact_id, message } => {
                write!(formatter, "agent workspace JSON patch failed: artifactId={artifact_id}, error={message}")
            }
            Self::TextPatchNotAllowed(artifact_id) => {
                write!(
                    formatter,
                    "agent workspace text patch is only allowed for invalid JSON or text artifacts: artifactId={artifact_id}"
                )
            }
            Self::TextPatchContentMismatch(artifact_id) => {
                write!(formatter, "agent workspace text patch expectedText mismatch: artifactId={artifact_id}")
            }
            Self::InvalidLineRange { start_line, end_line } => {
                write!(formatter, "invalid agent workspace line range: startLine={start_line}, endLine={end_line}")
            }
            Self::InvalidToolCall { tool, message } => {
                write!(formatter, "invalid agent workspace tool call: tool={tool}, error={message}")
            }
            Self::Storage { operation, path, message } => {
                write!(formatter, "agent workspace storage failed: operation={operation}, path={path:?}, error={message}")
            }
        }
    }
}

impl std::error::Error for WorkspaceError {}

/// AI 和业务模块使用的逻辑产物 ID。
///
/// ID 只允许 ASCII 字母、数字、`-` 和 `_`，因此不能携带目录分隔符或执行路径穿越。
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct ArtifactId(String);

impl ArtifactId {
    /// 创建并校验逻辑产物 ID。
    pub(crate) fn new(name: impl AsRef<str>) -> WorkspaceResult<Self> {
        let name = name.as_ref();
        if !is_safe_name(name, 128) {
            return Err(WorkspaceError::InvalidArtifactId(name.to_string()));
        }
        Ok(Self(name.to_string()))
    }

    /// 解析并校验序列化后的逻辑产物 ID。
    pub(crate) fn parse(value: impl AsRef<str>) -> WorkspaceResult<Self> {
        Self::new(value)
    }

    /// 返回可安全映射到存储文件的名称。
    pub(crate) fn name(&self) -> &str {
        &self.0
    }
}

impl Display for ArtifactId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ArtifactId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// 工作区清单。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceManifest {
    /// 清单格式版本。
    pub format_version: u32,
    /// 当前项目工作区的稳定逻辑 ID。
    pub workspace_id: String,
    /// 每次成功文件事务只递增一次的 Workspace 总版本。
    pub workspace_revision: u64,
    /// 工作区创建时间，Unix 毫秒时间戳。
    pub created_at: i64,
    /// 清单最近更新时间，Unix 毫秒时间戳。
    pub updated_at: i64,
    /// Agent 可见文件的宿主事实清单；不信任 INDEX.json 中的安全属性。
    pub files: Vec<WorkspaceFileMetadata>,
    /// 当前工作区已登记产物。
    pub artifacts: Vec<ArtifactMetadata>,
}

/// Agent 可见 Workspace 内的安全相对路径。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct WorkspaceRelativePath(String);

impl WorkspaceRelativePath {
    pub(crate) fn new(value: impl AsRef<str>) -> WorkspaceResult<Self> {
        let value = value.as_ref();
        if value.is_empty()
            || value.len() > 1024
            || value.starts_with('/')
            || value.starts_with('\\')
            || value.contains('\\')
            || value.split('/').any(str::is_empty)
        {
            return Err(WorkspaceError::PathOutOfScope(value.to_string()));
        }
        if value.as_bytes().iter().any(|byte| byte.is_ascii_control()) || has_windows_prefix(value) {
            return Err(WorkspaceError::PathOutOfScope(value.to_string()));
        }
        let path = Path::new(value);
        let mut component_count = 0usize;
        for component in path.components() {
            let Component::Normal(component) = component else {
                return Err(WorkspaceError::PathOutOfScope(value.to_string()));
            };
            let component = component
                .to_str()
                .ok_or_else(|| WorkspaceError::PathOutOfScope(value.to_string()))?;
            if !is_portable_component(component) {
                return Err(WorkspaceError::PathOutOfScope(value.to_string()));
            }
            component_count += 1;
        }
        if component_count == 0 {
            return Err(WorkspaceError::PathOutOfScope(value.to_string()));
        }
        Ok(Self(value.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn join_to(&self, root: &Path) -> PathBuf {
        root.join(&self.0)
    }
}

impl Display for WorkspaceRelativePath {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for WorkspaceRelativePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// runtime/manifest.json 中由宿主维护的真实文件元数据。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkspaceFileMetadata {
    pub path: WorkspaceRelativePath,
    pub byte_size: u64,
    pub content_sha256: String,
    pub modified_revision: u64,
}

/// 产物内容预期格式。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ArtifactContentFormat {
    /// 内容应当是 JSON；允许保存并分多次文本补丁修复无效 JSON。
    Json,
    /// 普通 UTF-8 文本内容。
    Text,
}

/// 产物校验和提交状态。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ArtifactStatus {
    /// 已落盘但尚未完成校验。
    Draft,
    /// 最近一次校验失败。
    Invalid,
    /// 当前内容已通过通用或业务校验。
    Valid,
    /// 当前产物已被业务流程接受，可参与最终聚合。
    Accepted,
}

/// 工作区清单中的产物元数据。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactMetadata {
    /// 逻辑产物 ID。
    pub artifact_id: ArtifactId,
    /// 当前版本，从 1 开始递增。
    pub revision: u64,
    /// 内容预期格式。
    pub content_format: ArtifactContentFormat,
    /// 当前正式内容的 SHA-256，用于发现进程异常造成的清单与文件不一致。
    pub content_sha256: String,
    /// 当前校验和提交状态。
    pub status: ArtifactStatus,
    /// 产物创建时间，Unix 毫秒时间戳。
    pub created_at: i64,
    /// 产物最近更新时间，Unix 毫秒时间戳。
    pub updated_at: i64,
}

/// 读取产物后返回的内容和版本快照。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactSnapshot {
    /// 产物元数据。
    pub metadata: ArtifactMetadata,
    /// 产物的原始 UTF-8 内容。
    pub content: String,
}

/// 最近一次确认已发布的工作区产物头。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublishedArtifactHead {
    /// 发布成功时对应的完整产物快照。
    pub artifact: ArtifactSnapshot,
    /// 写入业务页面的 Run；旧 head 缺失时仅保留内容级幂等能力。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// 前端发布成功后的记录时间，Unix 毫秒时间戳。
    pub published_at: i64,
}

/// 按行读取产物时返回的有限上下文。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactExcerpt {
    /// 产物逻辑 ID。
    pub artifact_id: ArtifactId,
    /// 读取时对应的产物版本。
    pub revision: u64,
    /// 实际返回的起始行，按 1 开始计数。
    pub start_line: usize,
    /// 实际返回的结束行，包含该行。
    pub end_line: usize,
    /// 产物总行数。
    pub total_lines: usize,
    /// 指定范围内的原始文本。
    pub content: String,
}

/// 有效 JSON 产物的结构化补丁请求。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JsonPatchRequest {
    /// 目标逻辑产物 ID。
    pub artifact_id: ArtifactId,
    /// 调用方读取内容时的版本。
    pub base_revision: u64,
    /// RFC 6902 JSON Patch 操作列表。
    pub operations: json_patch::Patch,
}

/// 无效 JSON 或普通文本产物的局部文本补丁请求。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TextPatchRequest {
    /// 目标逻辑产物 ID。
    pub artifact_id: ArtifactId,
    /// 调用方读取内容时的版本。
    pub base_revision: u64,
    /// 需要替换的起始行，按 1 开始计数。
    pub start_line: usize,
    /// 需要替换的结束行，包含该行。
    pub end_line: usize,
    /// 最近一次按行读取返回的完整原文。
    pub expected_text: String,
    /// 指定范围替换后的完整内容。
    pub new_text: String,
}

/// 补丁成功提交后的结果。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PatchResult {
    /// 补丁提交后的最新产物快照。
    pub artifact: ArtifactSnapshot,
}

/// 诊断严重程度。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagnosticSeverity {
    /// 提示性信息。
    Info,
    /// 不阻止提交但需要关注的问题。
    Warning,
    /// 阻止产物提交或聚合的错误。
    Error,
}

/// 诊断发生的校验阶段。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagnosticStage {
    /// JSON 或文本语法检查。
    Syntax,
    /// Schema 结构检查。
    Schema,
    /// 单个产物的业务规则检查。
    Business,
    /// 聚合后的跨产物检查。
    CrossArtifact,
    /// 补丁定位或应用阶段。
    Patch,
}

/// 单条产物校验诊断。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactDiagnostic {
    /// 诊断唯一 ID。
    pub diagnostic_id: String,
    /// 关联产物；工作区级诊断可以为空。
    pub artifact_id: Option<ArtifactId>,
    /// 诊断对应的产物版本。
    pub revision: Option<u64>,
    /// 严重程度。
    pub severity: DiagnosticSeverity,
    /// 校验阶段。
    pub stage: DiagnosticStage,
    /// 稳定的机器可读错误码。
    pub code: String,
    /// 面向开发者或模型的错误说明。
    pub message: String,
    /// 可选的字段位置、解析行列等结构化细节。
    pub details: Option<Value>,
    /// 记录创建时间，Unix 毫秒时间戳。
    pub created_at: i64,
}

/// 单次确定性修复或 AI 补丁修复记录。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactRepairRecord {
    /// 修复记录唯一 ID。
    pub repair_id: String,
    /// 被修复的逻辑产物 ID。
    pub artifact_id: ArtifactId,
    /// 修复前版本。
    pub base_revision: u64,
    /// 修复成功后的版本；修复失败时为空。
    pub result_revision: Option<u64>,
    /// 修复方式，例如 `jsonPatch`、`textPatch` 或业务确定性修复名称。
    pub strategy: String,
    /// 修复是否成功提交。
    pub succeeded: bool,
    /// 修复结果说明。
    pub message: String,
    /// 记录创建时间，Unix 毫秒时间戳。
    pub created_at: i64,
}

/// 诊断目录中保存的记录类型。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "recordType", content = "record", rename_all = "camelCase")]
pub(crate) enum DiagnosticRecord {
    /// 校验诊断记录。
    Diagnostic(ArtifactDiagnostic),
    /// 修复过程记录。
    Repair(ArtifactRepairRecord),
}

/// 判断名称是否能安全用作单级文件名。
pub(super) fn is_safe_name(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn has_windows_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn is_portable_component(component: &str) -> bool {
    if component.is_empty()
        || component == "."
        || component == ".."
        || component.len() > 255
        || component.contains(WORKSPACE_TEMP_MARKER)
        || component.ends_with(['.', ' '])
        || component
            .chars()
            .any(|character| matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return false;
    }
    let stem = component.split('.').next().unwrap_or(component).to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

/// 计算产物内容的稳定 SHA-256 十六进制摘要。
pub(super) fn content_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod workspace_path_tests {
    use super::*;

    #[test]
    fn accepts_portable_workspace_relative_paths() {
        for path in ["INDEX.json", "docs/overview.md", "outputs/页面.json"] {
            assert_eq!(WorkspaceRelativePath::new(path).unwrap().as_str(), path);
        }
    }

    #[test]
    fn rejects_absolute_traversal_windows_and_device_paths() {
        for path in [
            "",
            "/etc/passwd",
            "../outside",
            "docs/../outside",
            "./file",
            "C:/Windows/system.ini",
            r"C:\\Windows\\system.ini",
            r"\\server\\share",
            "docs//file",
            "docs/NUL.txt",
            "docs/file?.txt",
            "docs/.file.datavisool-tmp-1-1",
        ] {
            assert!(WorkspaceRelativePath::new(path).is_err(), "path should be rejected: {path:?}");
        }
    }
}
