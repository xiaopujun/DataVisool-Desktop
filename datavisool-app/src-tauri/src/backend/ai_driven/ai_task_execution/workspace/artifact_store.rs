/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 任务产物存储、版本检查和原子提交。
//!
//! 产物仓库只接受经过校验的逻辑 `ArtifactId`。每次修改都在工作区共享锁内检查
//! `baseRevision`，并用持久化 journal 协调产物内容与清单提交，进程重启后可确定性恢复。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::layout::WorkspaceLayout;
use super::manager::{read_manifest, write_bytes_atomically, write_bytes_create_once, write_manifest, WorkspaceHandle};
use super::types::{
    content_sha256, ArtifactContentFormat, ArtifactExcerpt, ArtifactId, ArtifactMetadata, ArtifactSnapshot, ArtifactStatus, WorkspaceError,
    WorkspaceFileMetadata, WorkspaceManifest, WorkspaceRelativePath, WorkspaceResult,
};
use crate::backend::ai_driven::time::now_ms;

/// 进程内工作区写锁注册表。
///
/// 同一物理工作区创建的多个 `ArtifactStore` 会共享一把锁，避免两个调用同时通过
/// 版本检查后互相覆盖。弱引用使不再使用的工作区锁可以自动释放。
static WORKSPACE_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();

const ARTIFACT_COMMIT_JOURNAL_FILE: &str = ".artifact-commit-journal.json";
const ARTIFACT_CREATE_JOURNAL_FILE: &str = ".artifact-create-journal.json";
const ARTIFACT_COMMIT_JOURNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactCreateJournal {
    schema_version: u32,
    workspace_id: String,
    artifact_id: ArtifactId,
    old_manifest: WorkspaceManifest,
    new_manifest: WorkspaceManifest,
    new_content: String,
}

/// 单个产物内容与清单元数据之间的持久化事务记录。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactCommitJournal {
    schema_version: u32,
    workspace_id: String,
    artifact_id: ArtifactId,
    old_manifest: WorkspaceManifest,
    old_content: String,
    new_manifest: WorkspaceManifest,
    new_content: String,
}

/// 工作区产物仓库。
#[derive(Clone, Debug)]
pub(crate) struct ArtifactStore {
    layout: WorkspaceLayout,
    write_lock: Arc<Mutex<()>>,
}

impl ArtifactStore {
    /// 从已创建或打开的工作区构造产物仓库。
    pub(crate) fn new(workspace: &WorkspaceHandle) -> Self {
        let layout = workspace.layout.clone();
        let write_lock = workspace_lock(layout.workspace_dir().to_path_buf());
        Self { layout, write_lock }
    }

    /// 创建一个版本为 1 的新产物草稿。
    ///
    /// `Json` 类型允许先保存语法不完整的模型输出，后续只能通过文本补丁修复为
    /// 有效 JSON；`Text` 类型不执行 JSON 语法约束。
    pub(crate) fn create(
        &self,
        artifact_id: ArtifactId,
        content_format: ArtifactContentFormat,
        content: impl AsRef<str>,
    ) -> WorkspaceResult<ArtifactSnapshot> {
        let _guard = self.lock_workspace()?;
        recover_pending_artifact_commit_locked(&self.layout)?;
        let old_manifest = read_manifest(&self.layout)?;
        if old_manifest.artifacts.iter().any(|item| item.artifact_id == artifact_id) {
            return Err(WorkspaceError::ArtifactAlreadyExists(artifact_id.to_string()));
        }

        let path = self.layout.artifact_file(&artifact_id);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                return Err(WorkspaceError::InvalidManifest(format!(
                    "artifact path exists without manifest metadata: artifactId={artifact_id}"
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(WorkspaceError::Storage { operation: "inspectArtifactCreateTarget", path, message: error.to_string() });
            }
        }

        let content = content.as_ref().to_string();
        let timestamp = now_ms();
        let metadata = ArtifactMetadata {
            artifact_id: artifact_id.clone(),
            revision: 1,
            content_format,
            content_sha256: content_sha256(content.as_bytes()),
            status: ArtifactStatus::Draft,
            created_at: timestamp,
            updated_at: timestamp,
        };
        let mut new_manifest = old_manifest.clone();
        let workspace_revision = next_workspace_revision(&new_manifest)?;
        new_manifest.workspace_revision = workspace_revision;
        new_manifest.updated_at = timestamp;
        upsert_file_metadata(&mut new_manifest, artifact_workspace_path(&artifact_id)?, content.as_bytes(), workspace_revision);
        new_manifest.artifacts.push(metadata.clone());
        let journal = ArtifactCreateJournal {
            schema_version: ARTIFACT_COMMIT_JOURNAL_SCHEMA_VERSION,
            workspace_id: self.layout.workspace_id().to_string(),
            artifact_id: artifact_id.clone(),
            old_manifest,
            new_manifest,
            new_content: content.clone(),
        };
        validate_artifact_create_journal(&self.layout, &journal)?;
        write_artifact_create_journal(&self.layout, &journal)?;
        write_bytes_atomically(&path, content.as_bytes())?;
        write_manifest(&self.layout, &journal.new_manifest)?;
        clear_artifact_create_journal(&self.layout)?;
        Ok(ArtifactSnapshot { metadata, content })
    }

    /// 读取产物原始内容及其版本元数据。
    pub(crate) fn read(&self, artifact_id: &ArtifactId) -> WorkspaceResult<ArtifactSnapshot> {
        let _guard = self.lock_workspace()?;
        self.read_locked(artifact_id)
    }

    /// 基于当前版本用新生成的完整内容替换单个产物。
    ///
    /// 该接口只用于重新生成一个独立业务分片，不用于局部修复；局部修复应使用
    /// `ArtifactPatchService`，以保留更精确的修改语义。
    pub(crate) fn replace(&self, artifact_id: &ArtifactId, base_revision: u64, content: impl Into<String>) -> WorkspaceResult<ArtifactSnapshot> {
        let _guard = self.lock_workspace()?;
        self.commit_content_locked(artifact_id, base_revision, content.into())
    }

    /// 将有效 JSON 产物解析为 `serde_json::Value`。
    pub(crate) fn read_json(&self, artifact_id: &ArtifactId) -> WorkspaceResult<(ArtifactMetadata, Value)> {
        let snapshot = self.read(artifact_id)?;
        let value = serde_json::from_str(&snapshot.content)
            .map_err(|error| WorkspaceError::InvalidJson { artifact_id: artifact_id.to_string(), message: format_json_error(&error) })?;
        Ok((snapshot.metadata, value))
    }

    /// 读取有效 JSON 产物中指定 JSON Pointer 的值。
    pub(crate) fn read_json_pointer(&self, artifact_id: &ArtifactId, pointer: &str) -> WorkspaceResult<(u64, Value)> {
        let (metadata, value) = self.read_json(artifact_id)?;
        let selected = value
            .pointer(pointer)
            .cloned()
            .ok_or_else(|| WorkspaceError::JsonPointerNotFound { artifact_id: artifact_id.to_string(), pointer: pointer.to_string() })?;
        Ok((metadata.revision, selected))
    }

    /// 按 1 开始的闭区间读取有限文本行。
    pub(crate) fn read_lines(&self, artifact_id: &ArtifactId, start_line: usize, end_line: usize) -> WorkspaceResult<ArtifactExcerpt> {
        let snapshot = self.read(artifact_id)?;
        let range = resolve_line_range(&snapshot.content, start_line, end_line)?;
        Ok(ArtifactExcerpt {
            artifact_id: artifact_id.clone(),
            revision: snapshot.metadata.revision,
            start_line,
            end_line: range.end_line,
            total_lines: range.total_lines,
            content: snapshot.content[range.start_byte..range.end_byte].to_string(),
        })
    }

    /// 在版本不变的前提下更新产物状态。
    pub(crate) fn update_status(&self, artifact_id: &ArtifactId, base_revision: u64, status: ArtifactStatus) -> WorkspaceResult<ArtifactMetadata> {
        let _guard = self.lock_workspace()?;
        recover_pending_artifact_commit_locked(&self.layout)?;
        let snapshot = self.read_locked(artifact_id)?;
        ensure_revision(&snapshot.metadata, base_revision)?;
        let mut manifest = read_manifest(&self.layout)?;
        let metadata = find_metadata_mut(&mut manifest.artifacts, artifact_id)?;
        metadata.revision += 1;
        metadata.status = status;
        metadata.updated_at = now_ms();
        let result = metadata.clone();
        manifest.updated_at = result.updated_at;
        write_manifest(&self.layout, &manifest)?;
        Ok(result)
    }

    /// 在共享锁内读取产物，供补丁服务组合原子操作。
    pub(super) fn read_locked(&self, artifact_id: &ArtifactId) -> WorkspaceResult<ArtifactSnapshot> {
        recover_pending_artifact_commit_locked(&self.layout)?;
        let manifest = read_manifest(&self.layout)?;
        let metadata = find_metadata(&manifest.artifacts, artifact_id)?.clone();
        let path = self.layout.artifact_file(artifact_id);
        ensure_regular_artifact_file(&path, artifact_id)?;
        let content =
            fs::read_to_string(&path).map_err(|error| WorkspaceError::Storage { operation: "readArtifact", path, message: error.to_string() })?;
        ensure_content_hash(&metadata, content.as_bytes())?;
        Ok(ArtifactSnapshot { metadata, content })
    }

    /// 在共享锁内提交新内容并递增版本。
    pub(super) fn commit_content_locked(&self, artifact_id: &ArtifactId, base_revision: u64, content: String) -> WorkspaceResult<ArtifactSnapshot> {
        recover_pending_artifact_commit_locked(&self.layout)?;
        let old_manifest = read_manifest(&self.layout)?;
        let old_metadata = find_metadata(&old_manifest.artifacts, artifact_id)?;
        ensure_revision(old_metadata, base_revision)?;

        let path = self.layout.artifact_file(artifact_id);
        ensure_regular_artifact_file(&path, artifact_id)?;
        let old_content = fs::read_to_string(&path).map_err(|error| WorkspaceError::Storage {
            operation: "readArtifactBeforeCommit",
            path: path.clone(),
            message: error.to_string(),
        })?;
        ensure_content_hash(old_metadata, old_content.as_bytes())?;

        let timestamp = now_ms();
        let journal = prepare_artifact_commit_journal(&self.layout, artifact_id, old_manifest, old_content, content, timestamp)?;
        let updated_metadata = find_metadata(&journal.new_manifest.artifacts, artifact_id)?.clone();
        write_artifact_commit_journal(&self.layout, &journal)?;
        write_bytes_atomically(&path, journal.new_content.as_bytes())?;
        write_manifest(&self.layout, &journal.new_manifest)?;
        clear_artifact_commit_journal(&self.layout)?;
        let content = journal.new_content;
        Ok(ArtifactSnapshot { metadata: updated_metadata, content })
    }

    /// 获取当前工作区共享锁，锁中毒时返回明确存储错误。
    pub(super) fn lock_workspace(&self) -> WorkspaceResult<std::sync::MutexGuard<'_, ()>> {
        self.write_lock.lock().map_err(|_| WorkspaceError::Storage {
            operation: "lockWorkspace",
            path: self.layout.workspace_dir().to_path_buf(),
            message: "workspace lock is poisoned".to_string(),
        })
    }
}

/// 打开工作区时在共享锁内恢复尚未完成的产物事务。
pub(super) fn recover_pending_artifact_commit(layout: &WorkspaceLayout) -> WorkspaceResult<()> {
    let write_lock = workspace_lock(layout.workspace_dir().to_path_buf());
    let _guard = write_lock.lock().map_err(|_| WorkspaceError::Storage {
        operation: "lockWorkspaceForRecovery",
        path: layout.workspace_dir().to_path_buf(),
        message: "workspace lock is poisoned".to_string(),
    })?;
    recover_pending_artifact_commit_locked(layout)
}

/// 恢复时只接受 journal 声明的 old/new 两组完整状态，拒绝猜测未知内容。
fn recover_pending_artifact_commit_locked(layout: &WorkspaceLayout) -> WorkspaceResult<()> {
    recover_pending_artifact_create_locked(layout)?;
    let Some(journal) = read_artifact_commit_journal(layout)? else {
        return Ok(());
    };
    validate_artifact_commit_journal(layout, &journal)?;

    let manifest = read_manifest(layout)?;
    let manifest_is_old = manifest == journal.old_manifest;
    let manifest_is_new = manifest == journal.new_manifest;
    if !manifest_is_old && !manifest_is_new {
        return Err(invalid_journal("current manifest matches neither journal state"));
    }

    let artifact_path = layout.artifact_file(&journal.artifact_id);
    cleanup_artifact_temporary_files(layout, &journal.artifact_id)?;
    ensure_regular_artifact_file(&artifact_path, &journal.artifact_id)?;
    let content = fs::read(&artifact_path).map_err(|error| WorkspaceError::Storage {
        operation: "readArtifactForRecovery",
        path: artifact_path.clone(),
        message: error.to_string(),
    })?;
    let content_is_old = content == journal.old_content.as_bytes();
    let content_is_new = content == journal.new_content.as_bytes();
    if !content_is_old && !content_is_new {
        return Err(invalid_journal("current artifact matches neither journal state"));
    }

    // 清单或不同的新内容已公开时一律前滚；仍是完整旧状态时保留旧版本。
    let complete_new = manifest_is_new || (content_is_new && journal.new_content != journal.old_content);
    if complete_new {
        if !content_is_new {
            write_bytes_atomically(&artifact_path, journal.new_content.as_bytes())?;
        }
        if !manifest_is_new {
            write_manifest(layout, &journal.new_manifest)?;
        }
    }
    clear_artifact_commit_journal(layout)
}

fn recover_pending_artifact_create_locked(layout: &WorkspaceLayout) -> WorkspaceResult<()> {
    let Some(journal) = read_artifact_create_journal(layout)? else {
        return Ok(());
    };
    validate_artifact_create_journal(layout, &journal)?;
    let manifest = read_manifest(layout)?;
    let manifest_is_old = manifest == journal.old_manifest;
    let manifest_is_new = manifest == journal.new_manifest;
    if !manifest_is_old && !manifest_is_new {
        return Err(invalid_journal("current manifest matches neither artifact create journal state"));
    }
    let path = layout.artifact_file(&journal.artifact_id);
    cleanup_artifact_temporary_files(layout, &journal.artifact_id)?;
    let content_is_new = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            fs::read(&path).map_err(|error| WorkspaceError::Storage {
                operation: "readCreatedArtifactForRecovery",
                path: path.clone(),
                message: error.to_string(),
            })? == journal.new_content.as_bytes()
        }
        Ok(_) => return Err(invalid_journal("created artifact path is not a regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(WorkspaceError::Storage { operation: "inspectCreatedArtifactForRecovery", path, message: error.to_string() }),
    };
    if manifest_is_new || content_is_new {
        if !content_is_new {
            write_bytes_atomically(&path, journal.new_content.as_bytes())?;
        }
        if !manifest_is_new {
            write_manifest(layout, &journal.new_manifest)?;
        }
    }
    clear_artifact_create_journal(layout)
}

fn prepare_artifact_commit_journal(
    layout: &WorkspaceLayout,
    artifact_id: &ArtifactId,
    old_manifest: WorkspaceManifest,
    old_content: String,
    new_content: String,
    timestamp: i64,
) -> WorkspaceResult<ArtifactCommitJournal> {
    let mut new_manifest = old_manifest.clone();
    let metadata = find_metadata_mut(&mut new_manifest.artifacts, artifact_id)?;
    metadata.revision = metadata
        .revision
        .checked_add(1)
        .ok_or_else(|| WorkspaceError::InvalidManifest(format!("artifact revision overflow: artifactId={artifact_id}")))?;
    metadata.content_sha256 = content_sha256(new_content.as_bytes());
    metadata.status = ArtifactStatus::Draft;
    metadata.updated_at = timestamp;
    let workspace_revision = next_workspace_revision(&new_manifest)?;
    new_manifest.workspace_revision = workspace_revision;
    new_manifest.updated_at = timestamp;
    upsert_file_metadata(&mut new_manifest, artifact_workspace_path(artifact_id)?, new_content.as_bytes(), workspace_revision);
    let journal = ArtifactCommitJournal {
        schema_version: ARTIFACT_COMMIT_JOURNAL_SCHEMA_VERSION,
        workspace_id: layout.workspace_id().to_string(),
        artifact_id: artifact_id.clone(),
        old_manifest,
        old_content,
        new_manifest,
        new_content,
    };
    validate_artifact_commit_journal(layout, &journal)?;
    Ok(journal)
}

fn validate_artifact_commit_journal(layout: &WorkspaceLayout, journal: &ArtifactCommitJournal) -> WorkspaceResult<()> {
    if journal.schema_version != ARTIFACT_COMMIT_JOURNAL_SCHEMA_VERSION {
        return Err(invalid_journal("unsupported schemaVersion"));
    }
    if journal.workspace_id != layout.workspace_id()
        || journal.old_manifest.workspace_id != journal.workspace_id
        || journal.new_manifest.workspace_id != journal.workspace_id
    {
        return Err(invalid_journal("workspaceId mismatch"));
    }
    ensure_unique_artifact_metadata(&journal.old_manifest)?;
    ensure_unique_artifact_metadata(&journal.new_manifest)?;

    let old_metadata = find_metadata(&journal.old_manifest.artifacts, &journal.artifact_id)
        .map_err(|_| invalid_journal("old manifest is missing target artifact"))?;
    let new_metadata = find_metadata(&journal.new_manifest.artifacts, &journal.artifact_id)
        .map_err(|_| invalid_journal("new manifest is missing target artifact"))?;
    ensure_content_hash(old_metadata, journal.old_content.as_bytes()).map_err(|_| invalid_journal("old content hash mismatch"))?;
    ensure_content_hash(new_metadata, journal.new_content.as_bytes()).map_err(|_| invalid_journal("new content hash mismatch"))?;

    let mut expected_manifest = journal.old_manifest.clone();
    let expected_metadata = find_metadata_mut(&mut expected_manifest.artifacts, &journal.artifact_id)
        .map_err(|_| invalid_journal("old manifest is missing target artifact"))?;
    expected_metadata.revision = expected_metadata
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid_journal("artifact revision overflow"))?;
    expected_metadata.content_sha256 = content_sha256(journal.new_content.as_bytes());
    expected_metadata.status = ArtifactStatus::Draft;
    expected_metadata.updated_at = new_metadata.updated_at;
    let workspace_revision = next_workspace_revision(&expected_manifest).map_err(|_| invalid_journal("workspace revision overflow"))?;
    expected_manifest.workspace_revision = workspace_revision;
    expected_manifest.updated_at = journal.new_manifest.updated_at;
    upsert_file_metadata(
        &mut expected_manifest,
        artifact_workspace_path(&journal.artifact_id).map_err(|_| invalid_journal("invalid artifact path"))?,
        journal.new_content.as_bytes(),
        workspace_revision,
    );
    if new_metadata.updated_at != journal.new_manifest.updated_at || expected_manifest != journal.new_manifest {
        return Err(invalid_journal("new manifest is not the next target revision"));
    }
    Ok(())
}

fn validate_artifact_create_journal(layout: &WorkspaceLayout, journal: &ArtifactCreateJournal) -> WorkspaceResult<()> {
    if journal.schema_version != ARTIFACT_COMMIT_JOURNAL_SCHEMA_VERSION
        || journal.workspace_id != layout.workspace_id()
        || journal.old_manifest.workspace_id != journal.workspace_id
        || journal.new_manifest.workspace_id != journal.workspace_id
    {
        return Err(invalid_journal("invalid artifact create journal identity"));
    }
    ensure_unique_artifact_metadata(&journal.old_manifest)?;
    ensure_unique_artifact_metadata(&journal.new_manifest)?;
    let artifact_path = artifact_workspace_path(&journal.artifact_id)?;
    if journal
        .old_manifest
        .artifacts
        .iter()
        .any(|metadata| metadata.artifact_id == journal.artifact_id)
        || journal.old_manifest.files.iter().any(|file| file.path == artifact_path)
    {
        return Err(invalid_journal("artifact create base already contains target"));
    }
    let new_metadata = find_metadata(&journal.new_manifest.artifacts, &journal.artifact_id)
        .map_err(|_| invalid_journal("artifact create target metadata is missing"))?;
    if new_metadata.revision != 1
        || new_metadata.status != ArtifactStatus::Draft
        || new_metadata.created_at != new_metadata.updated_at
        || new_metadata.updated_at != journal.new_manifest.updated_at
    {
        return Err(invalid_journal("artifact create target metadata is invalid"));
    }
    ensure_content_hash(new_metadata, journal.new_content.as_bytes()).map_err(|_| invalid_journal("artifact create content hash mismatch"))?;
    let mut expected = journal.old_manifest.clone();
    let workspace_revision = next_workspace_revision(&expected).map_err(|_| invalid_journal("workspace revision overflow"))?;
    expected.workspace_revision = workspace_revision;
    expected.updated_at = journal.new_manifest.updated_at;
    upsert_file_metadata(&mut expected, artifact_path, journal.new_content.as_bytes(), workspace_revision);
    expected.artifacts.push(new_metadata.clone());
    if expected != journal.new_manifest {
        return Err(invalid_journal("artifact create manifest is not the next target revision"));
    }
    Ok(())
}

fn ensure_unique_artifact_metadata(manifest: &WorkspaceManifest) -> WorkspaceResult<()> {
    let mut artifact_ids = std::collections::HashSet::with_capacity(manifest.artifacts.len());
    if manifest
        .artifacts
        .iter()
        .any(|metadata| !artifact_ids.insert(metadata.artifact_id.clone()))
    {
        return Err(invalid_journal("manifest contains duplicate artifact metadata"));
    }
    Ok(())
}

fn artifact_commit_journal_path(layout: &WorkspaceLayout) -> PathBuf {
    layout.transaction_journal_dir().join(ARTIFACT_COMMIT_JOURNAL_FILE)
}

fn artifact_create_journal_path(layout: &WorkspaceLayout) -> PathBuf {
    layout.transaction_journal_dir().join(ARTIFACT_CREATE_JOURNAL_FILE)
}

fn write_artifact_create_journal(layout: &WorkspaceLayout, journal: &ArtifactCreateJournal) -> WorkspaceResult<()> {
    let path = artifact_create_journal_path(layout);
    let bytes = serde_json::to_vec_pretty(journal).map_err(|error| WorkspaceError::Storage {
        operation: "serializeArtifactCreateJournal",
        path: path.clone(),
        message: error.to_string(),
    })?;
    if !write_bytes_create_once(&path, &bytes)? {
        return Err(invalid_journal("artifact create journal already exists"));
    }
    Ok(())
}

fn read_artifact_create_journal(layout: &WorkspaceLayout) -> WorkspaceResult<Option<ArtifactCreateJournal>> {
    super::manager::read_json_if_exists(&artifact_create_journal_path(layout), "readArtifactCreateJournal")
}

fn clear_artifact_create_journal(layout: &WorkspaceLayout) -> WorkspaceResult<()> {
    let path = artifact_create_journal_path(layout);
    match fs::remove_file(&path) {
        Ok(()) => sync_workspace_directory(layout),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(WorkspaceError::Storage { operation: "removeArtifactCreateJournal", path, message: error.to_string() }),
    }
}

fn write_artifact_commit_journal(layout: &WorkspaceLayout, journal: &ArtifactCommitJournal) -> WorkspaceResult<()> {
    let path = artifact_commit_journal_path(layout);
    let bytes = serde_json::to_vec_pretty(journal).map_err(|error| WorkspaceError::Storage {
        operation: "serializeArtifactCommitJournal",
        path: path.clone(),
        message: error.to_string(),
    })?;
    if !write_bytes_create_once(&path, &bytes)? {
        return Err(invalid_journal("journal already exists"));
    }
    Ok(())
}

fn read_artifact_commit_journal(layout: &WorkspaceLayout) -> WorkspaceResult<Option<ArtifactCommitJournal>> {
    let path = artifact_commit_journal_path(layout);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(WorkspaceError::Storage { operation: "inspectArtifactCommitJournal", path, message: error.to_string() });
        }
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(invalid_journal("journal path is not a regular file"));
    }
    let bytes = fs::read(&path).map_err(|error| WorkspaceError::Storage {
        operation: "readArtifactCommitJournal",
        path: path.clone(),
        message: error.to_string(),
    })?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| invalid_journal(&format!("journal JSON is invalid: {error}")))
}

fn clear_artifact_commit_journal(layout: &WorkspaceLayout) -> WorkspaceResult<()> {
    let path = artifact_commit_journal_path(layout);
    match fs::remove_file(&path) {
        Ok(()) => sync_workspace_directory(layout),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(WorkspaceError::Storage { operation: "removeArtifactCommitJournal", path, message: error.to_string() }),
    }
}

#[cfg(not(target_os = "windows"))]
fn sync_workspace_directory(layout: &WorkspaceLayout) -> WorkspaceResult<()> {
    std::fs::File::open(layout.transaction_journal_dir())
        .and_then(|directory| directory.sync_all())
        .map_err(|error| WorkspaceError::Storage {
            operation: "syncWorkspaceAfterJournalRemoval",
            path: layout.transaction_journal_dir(),
            message: error.to_string(),
        })
}

#[cfg(target_os = "windows")]
fn sync_workspace_directory(_layout: &WorkspaceLayout) -> WorkspaceResult<()> {
    Ok(())
}

fn invalid_journal(message: &str) -> WorkspaceError {
    WorkspaceError::InvalidManifest(format!("invalid artifact commit journal: {message}"))
}

fn cleanup_artifact_temporary_files(layout: &WorkspaceLayout, artifact_id: &ArtifactId) -> WorkspaceResult<()> {
    let directory = layout.artifacts_dir();
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(WorkspaceError::Storage { operation: "listArtifactTemporaryFiles", path: directory, message: error.to_string() });
        }
    };
    let prefix = format!(".{}.json.tmp-", artifact_id.name());
    for entry in entries {
        let entry = entry.map_err(|error| WorkspaceError::Storage {
            operation: "readArtifactTemporaryFile",
            path: directory.clone(),
            message: error.to_string(),
        })?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else { continue };
        if !file_name.starts_with(&prefix) {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| WorkspaceError::Storage {
            operation: "inspectArtifactTemporaryFile",
            path: path.clone(),
            message: error.to_string(),
        })?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(WorkspaceError::InvalidManifest("artifact temporary path is not a regular file".to_string()));
        }
        fs::remove_file(&path).map_err(|error| WorkspaceError::Storage {
            operation: "removeArtifactTemporaryFile",
            path,
            message: error.to_string(),
        })?;
    }
    Ok(())
}

fn artifact_workspace_path(artifact_id: &ArtifactId) -> WorkspaceResult<WorkspaceRelativePath> {
    WorkspaceRelativePath::new(format!("artifacts/{}.json", artifact_id.name()))
}

fn next_workspace_revision(manifest: &WorkspaceManifest) -> WorkspaceResult<u64> {
    manifest
        .workspace_revision
        .checked_add(1)
        .ok_or_else(|| WorkspaceError::InvalidManifest("workspace revision overflow".to_string()))
}

fn upsert_file_metadata(manifest: &mut WorkspaceManifest, path: WorkspaceRelativePath, content: &[u8], revision: u64) {
    let metadata = WorkspaceFileMetadata {
        path: path.clone(),
        byte_size: u64::try_from(content.len()).unwrap_or(u64::MAX),
        content_sha256: content_sha256(content),
        modified_revision: revision,
    };
    if let Some(existing) = manifest.files.iter_mut().find(|file| file.path == path) {
        *existing = metadata;
    } else {
        manifest.files.push(metadata);
    }
}

/// 已解析的精确行范围；末行的换行符不属于范围，范围内部换行符保持原样。
pub(super) struct ResolvedLineRange {
    pub(super) start_byte: usize,
    pub(super) end_byte: usize,
    pub(super) end_line: usize,
    pub(super) total_lines: usize,
}

/// 把 1 开始的闭区间行号解析为原始 UTF-8 字节范围。
pub(super) fn resolve_line_range(content: &str, start_line: usize, end_line: usize) -> WorkspaceResult<ResolvedLineRange> {
    if start_line == 0 || end_line < start_line || content.is_empty() {
        return Err(WorkspaceError::InvalidLineRange { start_line, end_line });
    }

    let mut spans = Vec::new();
    let mut offset = 0usize;
    for line in content.split_inclusive('\n') {
        let mut content_end = offset + line.len();
        if line.ends_with('\n') {
            content_end -= 1;
            if content[..content_end].ends_with('\r') {
                content_end -= 1;
            }
        }
        spans.push((offset, content_end));
        offset += line.len();
    }
    if offset < content.len() {
        spans.push((offset, content.len()));
    }

    let total_lines = spans.len();
    if start_line > total_lines {
        return Err(WorkspaceError::InvalidLineRange { start_line, end_line });
    }
    let actual_end = end_line.min(total_lines);
    Ok(ResolvedLineRange {
        start_byte: spans[start_line - 1].0,
        end_byte: spans[actual_end - 1].1,
        end_line: actual_end,
        total_lines,
    })
}

/// 获取或创建指定物理工作区对应的进程内共享锁。
pub(super) fn workspace_lock(workspace_dir: PathBuf) -> Arc<Mutex<()>> {
    let registry = WORKSPACE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = registry.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = guard.get(&workspace_dir).and_then(Weak::upgrade) {
        return existing;
    }
    let lock = Arc::new(Mutex::new(()));
    guard.insert(workspace_dir, Arc::downgrade(&lock));
    lock
}

/// 在清单中查找产物元数据。
fn find_metadata<'a>(artifacts: &'a [ArtifactMetadata], artifact_id: &ArtifactId) -> WorkspaceResult<&'a ArtifactMetadata> {
    artifacts
        .iter()
        .find(|item| &item.artifact_id == artifact_id)
        .ok_or_else(|| WorkspaceError::ArtifactNotFound(artifact_id.to_string()))
}

/// 在清单中查找可修改的产物元数据。
fn find_metadata_mut<'a>(artifacts: &'a mut [ArtifactMetadata], artifact_id: &ArtifactId) -> WorkspaceResult<&'a mut ArtifactMetadata> {
    artifacts
        .iter_mut()
        .find(|item| &item.artifact_id == artifact_id)
        .ok_or_else(|| WorkspaceError::ArtifactNotFound(artifact_id.to_string()))
}

/// 检查调用方读取版本是否仍是当前版本。
fn ensure_revision(metadata: &ArtifactMetadata, base_revision: u64) -> WorkspaceResult<()> {
    if metadata.revision != base_revision {
        return Err(WorkspaceError::RevisionConflict {
            artifact_id: metadata.artifact_id.to_string(),
            expected: base_revision,
            actual: metadata.revision,
        });
    }
    Ok(())
}

/// 拒绝读取符号链接或非普通文件，防止产物路径被替换后越出工作区。
fn ensure_regular_artifact_file(path: &std::path::Path, artifact_id: &ArtifactId) -> WorkspaceResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| WorkspaceError::Storage {
        operation: "inspectArtifact",
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(WorkspaceError::InvalidManifest(format!(
            "artifact path is not a regular file: artifactId={artifact_id}, path={path:?}"
        )));
    }
    Ok(())
}

/// 检查正式产物内容是否仍与清单记录一致。
fn ensure_content_hash(metadata: &ArtifactMetadata, bytes: &[u8]) -> WorkspaceResult<()> {
    let actual = content_sha256(bytes);
    if actual != metadata.content_sha256 {
        return Err(WorkspaceError::InvalidManifest(format!(
            "artifact content hash mismatch: artifactId={}, revision={}",
            metadata.artifact_id, metadata.revision
        )));
    }
    Ok(())
}

/// 为 JSON 解析错误补充行列位置，便于后续读取局部上下文。
fn format_json_error(error: &serde_json::Error) -> String {
    format!("{} at line {}, column {}", error, error.line(), error.column())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceManager;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn test_store(name: &str) -> (PathBuf, ArtifactStore) {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("datavisool-artifact-store-{name}-{}-{sequence}", std::process::id()));
        let workspace = WorkspaceManager::new(&root)
            .create("run-1")
            .expect("workspace should be created");
        (root, ArtifactStore::new(&workspace))
    }

    #[test]
    fn creates_and_reads_json_artifact() {
        let (root, store) = test_store("create");
        let artifact_id = ArtifactId::new("result").expect("artifactId should be valid");
        let created = store
            .create(artifact_id.clone(), ArtifactContentFormat::Json, r#"{"title":"首页"}"#)
            .expect("artifact should be created");
        assert_eq!(created.metadata.revision, 1);
        assert_eq!(
            store.read_json_pointer(&artifact_id, "/title").expect("pointer should exist").1,
            Value::String("首页".to_string())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reads_limited_line_range() {
        let (root, store) = test_store("lines");
        let artifact_id = ArtifactId::new("sample").expect("artifactId should be valid");
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Text, "one\ntwo\nthree\nfour")
            .expect("artifact should be created");
        let excerpt = store.read_lines(&artifact_id, 2, 3).expect("lines should be read");
        assert_eq!(excerpt.content, "two\nthree");
        assert_eq!(excerpt.total_lines, 4);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn status_update_checks_revision() {
        let (root, store) = test_store("status-revision");
        let artifact_id = ArtifactId::new("sample").expect("artifactId should be valid");
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Json, "{}")
            .expect("artifact should be created");
        let error = store
            .update_status(&artifact_id, 0, ArtifactStatus::Valid)
            .expect_err("stale revision must be rejected");
        assert!(matches!(error, WorkspaceError::RevisionConflict { actual: 1, .. }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn status_update_increments_revision_and_rejects_second_stale_update() {
        let (root, store) = test_store("status-update");
        let artifact_id = ArtifactId::new("sample").expect("artifactId should be valid");
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Json, "{}")
            .expect("artifact should be created");
        let metadata = store
            .update_status(&artifact_id, 1, ArtifactStatus::Valid)
            .expect("first status update should succeed");
        assert_eq!(metadata.revision, 2);
        let error = store
            .update_status(&artifact_id, 1, ArtifactStatus::Accepted)
            .expect_err("second update based on stale revision must fail");
        assert!(matches!(error, WorkspaceError::RevisionConflict { actual: 2, .. }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_content_that_does_not_match_manifest_hash() {
        let (root, store) = test_store("content-hash");
        let artifact_id = ArtifactId::new("sample").expect("artifactId should be valid");
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Json, r#"{"value":1}"#)
            .expect("artifact should be created");
        fs::write(store.layout.artifact_file(&artifact_id), r#"{"value":2}"#).expect("test should alter artifact content");

        let error = store.read(&artifact_id).expect_err("hash mismatch must be rejected");
        assert!(matches!(error, WorkspaceError::InvalidManifest(_)));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_artifact_symlink_that_points_outside_workspace() {
        use std::os::unix::fs::symlink;

        let (root, store) = test_store("symlink");
        let artifact_id = ArtifactId::new("sample").expect("artifactId should be valid");
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Json, "{}")
            .expect("artifact should be created");
        let artifact_path = store.layout.artifact_file(&artifact_id);
        let outside_path = root.join("outside.json");
        fs::write(&outside_path, "{}").expect("outside file should be created");
        fs::remove_file(&artifact_path).expect("artifact should be removed for test setup");
        symlink(&outside_path, &artifact_path).expect("artifact symlink should be created");

        let error = store.read(&artifact_id).expect_err("artifact symlink must be rejected");
        assert!(matches!(error, WorkspaceError::InvalidManifest(_)));
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(test)]
#[path = "artifact_store_recovery_tests.rs"]
mod recovery_tests;
