/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 任务执行存储区的目录布局和逻辑路径映射。
//!
//! 所有路径都由受校验的 `workspaceId` 和 `ArtifactId` 派生。上层不能把模型提供的
//! 字符串直接拼接到文件系统路径，从源头限制任务间越界访问。

use std::fs;
use std::path::{Path, PathBuf};

use super::types::{is_safe_name, ArtifactId, WorkspaceError, WorkspaceResult};

/// 一个项目唯一的 Agent Workspace 布局。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceLayout {
    workspace_id: String,
    project_dir: PathBuf,
    workspace_dir: PathBuf,
    runtime_dir: PathBuf,
}

impl WorkspaceLayout {
    /// 根据工作区总根目录和 `workspaceId` 构造布局。
    pub(crate) fn new(workspaces_root: impl AsRef<Path>, workspace_id: impl AsRef<str>) -> WorkspaceResult<Self> {
        let workspace_id = validate_workspace_id(workspace_id.as_ref())?;
        let project_dir = workspaces_root.as_ref().join(&workspace_id);
        Ok(Self::at_path(project_dir, workspace_id))
    }

    /// 为创建阶段的临时目录构造相同布局。
    pub(super) fn at_path(project_dir: PathBuf, workspace_id: String) -> Self {
        let workspace_dir = project_dir.join("workspace");
        let runtime_dir = project_dir.join("runtime");
        Self { workspace_id, project_dir, workspace_dir, runtime_dir }
    }

    /// 返回当前工作区的稳定逻辑 ID。
    pub(crate) fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    /// 返回宿主拥有的项目 Agent 存储根目录。
    pub(crate) fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    /// 返回当前任务工作区根目录。
    pub(crate) fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }

    /// 返回仅宿主可访问的运行目录。
    pub(crate) fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// 返回兼容旧工作区的可选索引文件。
    /// TODO(legacy-compat): 移除旧工作区 INDEX.json 协议时，同步移除此入口。
    pub(crate) fn index_file(&self) -> PathBuf {
        self.workspace_dir.join("INDEX.json")
    }

    /// 返回工作区清单文件。
    pub(crate) fn manifest_file(&self) -> PathBuf {
        self.runtime_dir.join("manifest.json")
    }

    /// 返回只读输入快照目录。
    pub(crate) fn input_dir(&self) -> PathBuf {
        self.runtime_dir.join("input")
    }

    /// 返回全部可修改中间产物目录。
    pub(crate) fn artifacts_dir(&self) -> PathBuf {
        self.workspace_dir.join("artifacts")
    }

    /// 返回校验错误和修复记录目录。
    pub(crate) fn diagnostics_dir(&self) -> PathBuf {
        self.runtime_dir.join("diagnostics")
    }

    /// 返回已通过全部校验的最终结果目录。
    pub(crate) fn final_dir(&self) -> PathBuf {
        self.runtime_dir.join("final")
    }

    /// 返回按 Turn 保存不可变输入、基线与运行结果快照的目录。
    pub(crate) fn turn_snapshots_dir(&self) -> PathBuf {
        self.runtime_dir.join("turn-snapshots")
    }

    /// 返回 Agent 最终产物声明的不可变快照目录。
    pub(crate) fn submissions_dir(&self) -> PathBuf {
        self.runtime_dir.join("submissions")
    }

    /// 返回可恢复 Workspace 内容快照目录。
    pub(crate) fn workspace_snapshots_dir(&self) -> PathBuf {
        self.runtime_dir.join("snapshots")
    }

    /// 返回 Workspace 事务 journal 目录。
    pub(crate) fn transaction_journal_dir(&self) -> PathBuf {
        self.runtime_dir.join("transaction-journal")
    }

    /// 将逻辑产物 ID 映射为固定存储区内的 JSON 文件。
    pub(crate) fn artifact_file(&self, artifact_id: &ArtifactId) -> PathBuf {
        self.artifacts_dir().join(format!("{}.json", artifact_id.name()))
    }

    /// 创建完整固定目录结构。
    pub(super) fn create_directories(&self) -> WorkspaceResult<()> {
        for directory in [
            self.workspace_dir.clone(),
            self.artifacts_dir(),
            self.runtime_dir.clone(),
            self.input_dir(),
            self.diagnostics_dir(),
            self.final_dir(),
            self.turn_snapshots_dir(),
            self.submissions_dir(),
            self.workspace_snapshots_dir(),
            self.transaction_journal_dir(),
        ] {
            fs::create_dir_all(&directory).map_err(|error| WorkspaceError::Storage {
                operation: "createDirectory",
                path: directory,
                message: error.to_string(),
            })?;
        }
        Ok(())
    }

    /// 检查打开的工作区是否包含全部固定目录。
    pub(super) fn validate_directories(&self) -> WorkspaceResult<()> {
        let required = [
            self.project_dir.clone(),
            self.workspace_dir.clone(),
            self.artifacts_dir(),
            self.runtime_dir.clone(),
            self.input_dir(),
            self.diagnostics_dir(),
            self.final_dir(),
            self.turn_snapshots_dir(),
            self.transaction_journal_dir(),
        ];
        for directory in &required {
            let metadata = fs::symlink_metadata(&directory).map_err(|error| {
                WorkspaceError::InvalidManifest(format!("required directory cannot be inspected: path={directory:?}, error={error}"))
            })?;
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(WorkspaceError::InvalidManifest(format!("required directory is missing: {directory:?}")));
            }
        }
        Ok(())
    }
}

/// 校验并返回可安全作为单级目录名的 `workspaceId`。
pub(super) fn validate_workspace_id(workspace_id: &str) -> WorkspaceResult<String> {
    if !is_safe_name(workspace_id, 160) {
        return Err(WorkspaceError::InvalidWorkspaceId(workspace_id.to_string()));
    }
    Ok(workspace_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_workspace_id_path_traversal() {
        let error = WorkspaceLayout::new("/tmp/workspaces", "../other").expect_err("path traversal must be rejected");
        assert!(matches!(error, WorkspaceError::InvalidWorkspaceId(_)));
    }

    #[test]
    fn maps_artifact_id_to_controlled_directory() {
        let layout = WorkspaceLayout::new("/tmp/workspaces", "run-1").expect("layout should be valid");
        let artifact_id = ArtifactId::new("result").expect("artifactId should be valid");
        assert_eq!(layout.project_dir(), Path::new("/tmp/workspaces/run-1"));
        assert_eq!(layout.workspace_dir(), Path::new("/tmp/workspaces/run-1/workspace"));
        assert_eq!(layout.runtime_dir(), Path::new("/tmp/workspaces/run-1/runtime"));
        assert_eq!(layout.artifact_file(&artifact_id), PathBuf::from("/tmp/workspaces/run-1/workspace/artifacts/result.json"));
    }

    #[test]
    fn rejects_artifact_id_path_traversal() {
        let error = ArtifactId::new("../other").expect_err("path traversal must be rejected");
        assert!(matches!(error, WorkspaceError::InvalidArtifactId(_)));
    }
}
