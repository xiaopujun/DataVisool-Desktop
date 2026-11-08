/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 任务执行存储区生命周期管理。
//!
//! 管理器负责创建、打开和删除任务工作区。创建过程先在同级临时目录中完成，
//! 目录结构和清单全部写入成功后再整体提交，避免暴露半初始化工作区。

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::artifact_store::{recover_pending_artifact_commit, workspace_lock};
use super::file_edit::{reconcile_manifest_files, recover_pending_file_commit, WorkspaceFileLimits};
use super::index::{serialize_initial_index, WORKSPACE_INDEX_FILE};
use super::layout::WorkspaceLayout;
use super::types::{
    content_sha256, is_safe_name, ArtifactSnapshot, ArtifactStatus, PublishedArtifactHead, WorkspaceError, WorkspaceFileMetadata, WorkspaceManifest,
    WorkspaceRelativePath, WorkspaceResult, WORKSPACE_FORMAT_VERSION,
};
use crate::backend::ai_driven::time::now_ms;

/// 临时文件名序号，避免同一毫秒内的多次写入互相冲突。
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// 已创建或打开的工作区句柄。
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceHandle {
    /// 当前工作区固定路径布局。
    pub layout: WorkspaceLayout,
    /// 打开时读取到的清单快照。
    pub manifest: WorkspaceManifest,
}

impl WorkspaceHandle {
    /// 首次保存任务输入 JSON；文件已存在时保持原始快照不变。
    pub(crate) fn write_input_json<T: Serialize + ?Sized>(&self, name: &str, value: &T) -> WorkspaceResult<()> {
        let path = controlled_json_file(&self.layout.input_dir(), name)?;
        if path.exists() {
            return Ok(());
        }
        write_json_atomically(&path, value, "serializeInput")
    }

    /// 读取已经保存的任务输入 JSON；不存在时返回 `None`。
    pub(crate) fn read_input_json<T: DeserializeOwned>(&self, name: &str) -> WorkspaceResult<Option<T>> {
        read_json_if_exists(&controlled_json_file(&self.layout.input_dir(), name)?, "readInput")
    }

    /// 原子写入已通过全部校验的最终 JSON 结果。
    pub(crate) fn write_final_json<T: Serialize + ?Sized>(&self, name: &str, value: &T) -> WorkspaceResult<()> {
        let path = controlled_json_file(&self.layout.final_dir(), name)?;
        write_json_atomically(&path, value, "serializeFinalResult")
    }

    /// 读取已经提交的最终结果 JSON；不存在时返回 `None`。
    pub(crate) fn read_final_json<T: DeserializeOwned>(&self, name: &str) -> WorkspaceResult<Option<T>> {
        read_json_if_exists(&controlled_json_file(&self.layout.final_dir(), name)?, "readFinalResult")
    }

    /// 记录最近一次已经成功写入业务页面的 `result` 快照。
    pub(crate) fn write_published_result(&self, artifact: &ArtifactSnapshot) -> WorkspaceResult<PublishedArtifactHead> {
        self.write_published_result_for_run(artifact, None)
    }

    /// 记录带 Run 身份的发布证明，供项目 Workspace 跨进程恢复精确判断新旧顺序。
    pub(crate) fn write_published_result_for_run(&self, artifact: &ArtifactSnapshot, run_id: Option<&str>) -> WorkspaceResult<PublishedArtifactHead> {
        validate_published_result(artifact)?;
        if run_id.is_some_and(|value| !is_safe_name(value, 128)) {
            return Err(WorkspaceError::InvalidManifest("published result runId is invalid".to_string()));
        }
        let head = PublishedArtifactHead { artifact: artifact.clone(), run_id: run_id.map(str::to_string), published_at: now_ms() };
        self.write_final_json("published-result", &head)?;
        Ok(head)
    }

    /// 读取最近一次已经成功写入业务页面的 `result` 快照。
    pub(crate) fn read_published_result(&self) -> WorkspaceResult<Option<PublishedArtifactHead>> {
        let head = self.read_final_json::<PublishedArtifactHead>("published-result")?;
        if let Some(head) = &head {
            validate_published_result(&head.artifact)?;
            if head.run_id.as_deref().is_some_and(|value| !is_safe_name(value, 128)) {
                return Err(WorkspaceError::InvalidManifest("published result runId is invalid".to_string()));
            }
        }
        Ok(head)
    }
}

fn validate_published_result(artifact: &ArtifactSnapshot) -> WorkspaceResult<()> {
    if artifact.metadata.artifact_id.name() != "result" || artifact.metadata.status != ArtifactStatus::Accepted {
        return Err(WorkspaceError::InvalidManifest("published result must be the Accepted result artifact".to_string()));
    }
    if content_sha256(artifact.content.as_bytes()) != artifact.metadata.content_sha256 {
        return Err(WorkspaceError::InvalidManifest("published result content hash mismatch".to_string()));
    }
    Ok(())
}

/// AI 任务执行存储区管理器。
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceManager {
    workspaces_root: PathBuf,
}

impl WorkspaceManager {
    /// 使用 AppData 派生出的工作区总根目录创建管理器。
    pub(crate) fn new(workspaces_root: impl Into<PathBuf>) -> Self {
        Self { workspaces_root: workspaces_root.into() }
    }

    /// 创建一个完整的新逻辑工作区。
    pub(crate) fn create(&self, workspace_id: &str) -> WorkspaceResult<WorkspaceHandle> {
        self.create_atomically(workspace_id, |layout| self.initialize_workspace(layout))
    }

    /// 在同级临时目录完成初始化后，再原子提交到正式路径。
    fn create_atomically<F>(&self, workspace_id: &str, initialize: F) -> WorkspaceResult<WorkspaceHandle>
    where
        F: FnOnce(&WorkspaceLayout) -> WorkspaceResult<WorkspaceManifest>,
    {
        let final_layout = WorkspaceLayout::new(&self.workspaces_root, workspace_id)?;
        if final_layout.project_dir().exists() {
            return Err(WorkspaceError::WorkspaceAlreadyExists(workspace_id.to_string()));
        }
        fs::create_dir_all(&self.workspaces_root).map_err(|error| WorkspaceError::Storage {
            operation: "createWorkspacesRoot",
            path: self.workspaces_root.clone(),
            message: error.to_string(),
        })?;

        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp_dir = self
            .workspaces_root
            .join(format!(".{workspace_id}.creating-{}-{sequence}", std::process::id()));
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir).map_err(|error| WorkspaceError::Storage {
                operation: "removeStaleWorkspaceTemp",
                path: temp_dir.clone(),
                message: error.to_string(),
            })?;
        }

        let temp_layout = WorkspaceLayout::at_path(temp_dir.clone(), workspace_id.to_string());
        let result = initialize(&temp_layout).and_then(|manifest| {
            fs::rename(&temp_dir, final_layout.project_dir()).map_err(|error| {
                if final_layout.project_dir().exists() {
                    WorkspaceError::WorkspaceAlreadyExists(workspace_id.to_string())
                } else {
                    WorkspaceError::Storage {
                        operation: "commitWorkspace",
                        path: final_layout.project_dir().to_path_buf(),
                        message: error.to_string(),
                    }
                }
            })?;
            Ok(WorkspaceHandle { layout: final_layout, manifest })
        });

        if result.is_err() && temp_dir.exists() {
            let _ = fs::remove_dir_all(temp_dir);
        }
        result
    }

    /// 打开并验证当前格式的逻辑工作区。
    pub(crate) fn open(&self, workspace_id: &str) -> WorkspaceResult<WorkspaceHandle> {
        let layout = WorkspaceLayout::new(&self.workspaces_root, workspace_id)?;
        if !layout.project_dir().is_dir() {
            return Err(WorkspaceError::WorkspaceNotFound(workspace_id.to_string()));
        }
        layout.validate_directories()?;
        recover_pending_artifact_commit(&layout)?;
        recover_pending_file_commit(&layout)?;
        let lock = workspace_lock(layout.workspace_dir().to_path_buf());
        let _guard = lock.lock().map_err(|_| WorkspaceError::Storage {
            operation: "lockWorkspaceForReconciliation",
            path: layout.workspace_dir().to_path_buf(),
            message: "workspace lock is poisoned".to_string(),
        })?;
        let mut manifest = read_manifest(&layout)?;
        validate_manifest(&manifest, workspace_id, WORKSPACE_FORMAT_VERSION)?;
        reconcile_manifest_files(&layout, &mut manifest, &WorkspaceFileLimits::default())?;
        validate_manifest_artifacts(&layout, &manifest)?;
        Ok(WorkspaceHandle { layout, manifest })
    }

    /// 打开已有工作区；不存在时创建新工作区。
    pub(crate) fn open_or_create(&self, workspace_id: &str) -> WorkspaceResult<WorkspaceHandle> {
        match self.open(workspace_id) {
            Ok(workspace) => Ok(workspace),
            Err(WorkspaceError::WorkspaceNotFound(_)) => self.create(workspace_id),
            Err(error) => Err(error),
        }
    }

    /// 删除指定逻辑工作区及其中全部产物。
    pub(crate) fn remove(&self, workspace_id: &str) -> WorkspaceResult<()> {
        let layout = WorkspaceLayout::new(&self.workspaces_root, workspace_id)?;
        if !layout.project_dir().exists() {
            return Err(WorkspaceError::WorkspaceNotFound(workspace_id.to_string()));
        }
        fs::remove_dir_all(layout.project_dir()).map_err(|error| WorkspaceError::Storage {
            operation: "removeWorkspace",
            path: layout.project_dir().to_path_buf(),
            message: error.to_string(),
        })
    }

    /// 在临时目录中写入固定目录结构和初始清单。
    fn initialize_workspace(&self, layout: &WorkspaceLayout) -> WorkspaceResult<WorkspaceManifest> {
        layout.create_directories()?;
        let index = serialize_initial_index()?;
        write_bytes_atomically(&layout.index_file(), &index)?;
        let timestamp = now_ms();
        let manifest = WorkspaceManifest {
            format_version: WORKSPACE_FORMAT_VERSION,
            workspace_id: layout.workspace_id().to_string(),
            workspace_revision: 0,
            created_at: timestamp,
            updated_at: timestamp,
            files: vec![WorkspaceFileMetadata {
                path: WorkspaceRelativePath::new(WORKSPACE_INDEX_FILE)?,
                byte_size: u64::try_from(index.len()).unwrap_or(u64::MAX),
                content_sha256: content_sha256(&index),
                modified_revision: 0,
            }],
            artifacts: Vec::new(),
        };
        write_manifest(layout, &manifest)?;
        Ok(manifest)
    }
}

/// 构造受控单级 JSON 文件路径。
fn controlled_json_file(directory: &Path, name: &str) -> WorkspaceResult<PathBuf> {
    if !super::types::is_safe_name(name, 128) {
        return Err(WorkspaceError::InvalidArtifactId(name.to_string()));
    }
    Ok(directory.join(format!("{name}.json")))
}

/// 序列化并原子写入 JSON 文件。
fn write_json_atomically<T: Serialize + ?Sized>(path: &Path, value: &T, operation: &'static str) -> WorkspaceResult<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| WorkspaceError::Storage {
        operation,
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    write_bytes_atomically(path, &bytes)
}

/// 读取受控 JSON 文件，并拒绝符号链接和非普通文件。
pub(super) fn read_json_if_exists<T: DeserializeOwned>(path: &Path, operation: &'static str) -> WorkspaceResult<Option<T>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(WorkspaceError::Storage { operation, path: path.to_path_buf(), message: error.to_string() });
        }
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(WorkspaceError::Storage { operation, path: path.to_path_buf(), message: "JSON path is not a regular file".to_string() });
    }
    let bytes = fs::read(path).map_err(|error| WorkspaceError::Storage { operation, path: path.to_path_buf(), message: error.to_string() })?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| WorkspaceError::Storage { operation, path: path.to_path_buf(), message: error.to_string() })
}

/// 读取工作区清单。
pub(super) fn read_manifest(layout: &WorkspaceLayout) -> WorkspaceResult<WorkspaceManifest> {
    let path = layout.manifest_file();
    let metadata = fs::symlink_metadata(&path).map_err(|error| WorkspaceError::Storage {
        operation: "inspectManifest",
        path: path.clone(),
        message: error.to_string(),
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(WorkspaceError::InvalidManifest(format!("manifest is not a regular file: {path:?}")));
    }
    let bytes =
        fs::read(&path).map_err(|error| WorkspaceError::Storage { operation: "readManifest", path: path.clone(), message: error.to_string() })?;
    serde_json::from_slice(&bytes).map_err(|error| WorkspaceError::InvalidManifest(error.to_string()))
}

/// 原子保存工作区清单。
pub(super) fn write_manifest(layout: &WorkspaceLayout, manifest: &WorkspaceManifest) -> WorkspaceResult<()> {
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|error| WorkspaceError::Storage {
        operation: "serializeManifest",
        path: layout.manifest_file(),
        message: error.to_string(),
    })?;
    write_bytes_atomically(&layout.manifest_file(), &bytes)
}

/// 使用同目录临时文件原子替换目标文件。
///
/// 临时文件会先完整写入并同步。Unix 使用覆盖式 `rename`，Windows 使用
/// `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`，确保正式文件不会出现半写入内容。
pub(super) fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> WorkspaceResult<()> {
    let parent = path.parent().ok_or_else(|| WorkspaceError::Storage {
        operation: "resolveParentDirectory",
        path: path.to_path_buf(),
        message: "target path has no parent directory".to_string(),
    })?;
    fs::create_dir_all(parent).map_err(|error| WorkspaceError::Storage {
        operation: "createParentDirectory",
        path: parent.to_path_buf(),
        message: error.to_string(),
    })?;

    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("workspace-file");
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(".{file_name}.tmp-{}-{}-{sequence}", std::process::id(), now_ms()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|error| WorkspaceError::Storage { operation: "createTemporaryFile", path: temp_path.clone(), message: error.to_string() })?;
        file.write_all(bytes).map_err(|error| WorkspaceError::Storage {
            operation: "writeTemporaryFile",
            path: temp_path.clone(),
            message: error.to_string(),
        })?;
        file.sync_all().map_err(|error| WorkspaceError::Storage {
            operation: "syncTemporaryFile",
            path: temp_path.clone(),
            message: error.to_string(),
        })?;
        replace_file(&temp_path, path)?;
        sync_parent_directory(parent, path)
    })();

    if result.is_err() && temp_path.exists() {
        let _ = fs::remove_file(temp_path);
    }
    result
}

/// 将完整临时文件以“不覆盖已有目标”的方式原子提交。
///
/// 返回 `true` 表示本次创建成功，`false` 表示目标已经存在。硬链接只在临时文件
/// 完整写入并同步后公开目标路径，因此并发读取不会看到半个 JSON 文件。
pub(super) fn write_bytes_create_once(path: &Path, bytes: &[u8]) -> WorkspaceResult<bool> {
    let parent = path.parent().ok_or_else(|| WorkspaceError::Storage {
        operation: "resolveParentDirectory",
        path: path.to_path_buf(),
        message: "target path has no parent directory".to_string(),
    })?;
    fs::create_dir_all(parent).map_err(|error| WorkspaceError::Storage {
        operation: "createParentDirectory",
        path: parent.to_path_buf(),
        message: error.to_string(),
    })?;

    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("workspace-file");
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(".{file_name}.once-{}-{}-{sequence}", std::process::id(), now_ms()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|error| WorkspaceError::Storage { operation: "createTemporaryFile", path: temp_path.clone(), message: error.to_string() })?;
        file.write_all(bytes).map_err(|error| WorkspaceError::Storage {
            operation: "writeTemporaryFile",
            path: temp_path.clone(),
            message: error.to_string(),
        })?;
        file.sync_all().map_err(|error| WorkspaceError::Storage {
            operation: "syncTemporaryFile",
            path: temp_path.clone(),
            message: error.to_string(),
        })?;
        match fs::hard_link(&temp_path, path) {
            Ok(()) => {
                sync_parent_directory(parent, path)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(WorkspaceError::Storage { operation: "commitCreateOnceFile", path: path.to_path_buf(), message: error.to_string() }),
        }
    })();
    let cleanup = fs::remove_file(&temp_path);
    if result.is_ok() && cleanup.is_err() {
        return Err(WorkspaceError::Storage {
            operation: "removeTemporaryFile",
            path: temp_path,
            message: cleanup.expect_err("checked error").to_string(),
        });
    }
    result
}

#[cfg(not(target_os = "windows"))]
fn replace_file(temp_path: &Path, target_path: &Path) -> WorkspaceResult<()> {
    fs::rename(temp_path, target_path).map_err(|error| WorkspaceError::Storage {
        operation: "replaceFile",
        path: target_path.to_path_buf(),
        message: error.to_string(),
    })
}

#[cfg(target_os = "windows")]
fn replace_file(temp_path: &Path, target_path: &Path) -> WorkspaceResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};

    let temp_wide: Vec<u16> = temp_path.as_os_str().encode_wide().chain(Some(0)).collect();
    let target_wide: Vec<u16> = target_path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: 两个 UTF-16 缓冲区都以 NUL 结尾，并在系统调用完成前保持有效。
    let succeeded = unsafe { MoveFileExW(temp_wide.as_ptr(), target_wide.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) };
    if succeeded == 0 {
        return Err(WorkspaceError::Storage {
            operation: "replaceFile",
            path: target_path.to_path_buf(),
            message: std::io::Error::last_os_error().to_string(),
        });
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn sync_parent_directory(parent: &Path, target_path: &Path) -> WorkspaceResult<()> {
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| WorkspaceError::Storage { operation: "syncParentDirectory", path: target_path.to_path_buf(), message: error.to_string() })
}

#[cfg(target_os = "windows")]
fn sync_parent_directory(_parent: &Path, _target_path: &Path) -> WorkspaceResult<()> {
    Ok(())
}

/// 校验清单版本和逻辑工作区归属。
fn validate_manifest(manifest: &WorkspaceManifest, expected_workspace_id: &str, expected_format_version: u32) -> WorkspaceResult<()> {
    if manifest.format_version != expected_format_version {
        return Err(WorkspaceError::InvalidManifest(format!(
            "unsupported formatVersion {}, expected {}",
            manifest.format_version, expected_format_version
        )));
    }
    if manifest.workspace_id != expected_workspace_id {
        return Err(WorkspaceError::InvalidManifest(format!(
            "workspaceId mismatch: manifest={}, expected={expected_workspace_id}",
            manifest.workspace_id
        )));
    }
    Ok(())
}

/// 检查清单中的逻辑产物是否唯一且对应正式文件存在。
fn validate_manifest_artifacts(layout: &WorkspaceLayout, manifest: &WorkspaceManifest) -> WorkspaceResult<()> {
    let mut artifact_ids = HashSet::with_capacity(manifest.artifacts.len());
    for metadata in &manifest.artifacts {
        if !artifact_ids.insert(metadata.artifact_id.clone()) {
            return Err(WorkspaceError::InvalidManifest(format!("duplicate artifact metadata: artifactId={}", metadata.artifact_id)));
        }
        let path = layout.artifact_file(&metadata.artifact_id);
        let file_metadata = fs::symlink_metadata(&path).map_err(|error| {
            WorkspaceError::InvalidManifest(format!(
                "artifact file cannot be inspected: artifactId={}, path={path:?}, error={error}",
                metadata.artifact_id
            ))
        })?;
        if !file_metadata.file_type().is_file() || file_metadata.file_type().is_symlink() {
            return Err(WorkspaceError::InvalidManifest(format!(
                "artifact path is not a regular file: artifactId={}, path={path:?}",
                metadata.artifact_id
            )));
        }
        let bytes = fs::read(&path).map_err(|error| {
            WorkspaceError::InvalidManifest(format!(
                "artifact file cannot be read: artifactId={}, path={path:?}, error={error}",
                metadata.artifact_id
            ))
        })?;
        if content_sha256(&bytes) != metadata.content_sha256 {
            return Err(WorkspaceError::InvalidManifest(format!(
                "artifact content hash mismatch: artifactId={}, revision={}",
                metadata.artifact_id, metadata.revision
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::backend::ai_driven::ai_task_execution::workspace::{ArtifactContentFormat, ArtifactId, ArtifactStore};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn test_root(name: &str) -> PathBuf {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("datavisool-ai-task-workspace-{name}-{}-{sequence}", std::process::id()))
    }

    #[test]
    fn creates_opens_and_removes_workspace() {
        let root = test_root("lifecycle");
        let manager = WorkspaceManager::new(&root);

        let created = manager.create("run-1").expect("workspace should be created");
        assert!(created.layout.manifest_file().is_file());
        assert!(created.layout.index_file().is_file());
        assert!(created.layout.workspace_dir().is_dir());
        assert!(created.layout.runtime_dir().is_dir());

        let opened = manager.open("run-1").expect("workspace should reopen");
        assert_eq!(opened.manifest.workspace_id, "run-1");

        manager.remove("run-1").expect("workspace should be removed");
        assert!(!created.layout.project_dir().exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn open_reconciles_the_real_file_tree_and_ignores_host_metadata() {
        let root = test_root("open-reconciliation");
        let manager = WorkspaceManager::new(&root);
        let workspace = manager.create("run-1").unwrap();
        fs::write(workspace.layout.workspace_dir().join("notes.md"), "external content").unwrap();
        fs::write(workspace.layout.workspace_dir().join(".DS_Store"), [0_u8; 32]).unwrap();

        let reopened = manager.open("run-1").unwrap();
        assert_eq!(reopened.manifest.workspace_revision, 1);
        assert!(reopened.manifest.files.iter().any(|file| file.path.as_str() == "notes.md"));
        assert!(reopened.manifest.files.iter().all(|file| file.path.as_str() != ".DS_Store"));

        let persisted = read_manifest(&reopened.layout).unwrap();
        assert_eq!(persisted, reopened.manifest);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn refuses_to_replace_existing_workspace() {
        let root = test_root("duplicate");
        let manager = WorkspaceManager::new(&root);
        manager.create("run-1").expect("first workspace should be created");
        let error = manager.create("run-1").expect_err("duplicate workspace must be rejected");
        assert!(matches!(error, WorkspaceError::WorkspaceAlreadyExists(_)));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn atomic_write_replaces_complete_file() {
        let root = test_root("atomic-write");
        fs::create_dir_all(&root).expect("test root should be created");
        let target = root.join("value.json");
        write_bytes_atomically(&target, br#"{"revision":1}"#).expect("first write should succeed");
        write_bytes_atomically(&target, br#"{"revision":2}"#).expect("replacement should succeed");
        assert_eq!(fs::read_to_string(&target).expect("target should be readable"), r#"{"revision":2}"#);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn published_result_round_trips_as_a_typed_head() {
        let root = test_root("published-result");
        let workspace = WorkspaceManager::new(&root).create("workspace").unwrap();
        let artifact_id = ArtifactId::new("result").unwrap();
        let store = ArtifactStore::new(&workspace);
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Json, r#"{"value":1}"#)
            .unwrap();
        store.update_status(&artifact_id, 1, ArtifactStatus::Accepted).unwrap();
        let snapshot = store.read(&artifact_id).unwrap();

        let written = workspace.write_published_result(&snapshot).unwrap();
        let reopened = WorkspaceManager::new(&root).open("workspace").unwrap();
        let read = reopened.read_published_result().unwrap().unwrap();

        assert_eq!(written, read);
        assert_eq!(read.artifact, snapshot);
        assert!(workspace.layout.final_dir().join("published-result.json").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn published_result_rejects_unaccepted_or_tampered_snapshots() {
        let root = test_root("published-result-validation");
        let workspace = WorkspaceManager::new(&root).create("workspace").unwrap();
        let artifact_id = ArtifactId::new("result").unwrap();
        let store = ArtifactStore::new(&workspace);
        let draft = store
            .create(artifact_id.clone(), ArtifactContentFormat::Json, r#"{"value":1}"#)
            .unwrap();
        assert!(workspace.write_published_result(&draft).is_err());

        store.update_status(&artifact_id, 1, ArtifactStatus::Accepted).unwrap();
        let accepted = store.read(&artifact_id).unwrap();
        workspace.write_published_result(&accepted).unwrap();
        let mut tampered = workspace
            .read_final_json::<serde_json::Value>("published-result")
            .unwrap()
            .unwrap();
        tampered["artifact"]["content"] = serde_json::Value::String(r#"{"value":2}"#.to_string());
        workspace.write_final_json("published-result", &tampered).unwrap();

        assert!(workspace.read_published_result().is_err());
        let _ = fs::remove_dir_all(root);
    }
}
