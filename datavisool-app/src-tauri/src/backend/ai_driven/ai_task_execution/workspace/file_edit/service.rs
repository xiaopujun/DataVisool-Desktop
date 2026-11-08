/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Workspace 文件列表、分段读取和事务 Patch 服务。

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::parser::{apply_hunks, parse_patch, FilePatchOperation};
use super::path::WorkspaceRoot;
use super::reconcile::reconcile_manifest_files;
use super::transaction::{commit_file_transaction, derive_target_manifest, recover_pending_file_commit, FileJournalChange, FileTransactionJournal};
use crate::backend::ai_driven::ai_task_execution::workspace::artifact_store::{resolve_line_range, workspace_lock};
use crate::backend::ai_driven::ai_task_execution::workspace::manager::read_manifest;
use crate::backend::ai_driven::ai_task_execution::workspace::types::content_sha256;
use crate::backend::ai_driven::ai_task_execution::workspace::{WorkspaceError, WorkspaceLayout, WorkspaceRelativePath, WorkspaceResult};
use crate::backend::ai_driven::time::now_ms;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct WorkspaceFileLimits {
    pub max_patch_bytes: usize,
    pub max_patch_operations: usize,
    pub max_file_bytes: u64,
    pub max_tool_output_bytes: usize,
    pub max_workspace_files: usize,
    pub max_workspace_bytes: u64,
}

impl Default for WorkspaceFileLimits {
    fn default() -> Self {
        Self {
            max_patch_bytes: 512 * 1024,
            max_patch_operations: 64,
            max_file_bytes: 2 * 1024 * 1024,
            max_tool_output_bytes: 512 * 1024,
            max_workspace_files: 2_000,
            max_workspace_bytes: 100 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceFileListEntry {
    pub path: String,
    pub entry_type: WorkspaceFileEntryType,
    pub byte_size: u64,
    pub modified_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WorkspaceFileEntryType {
    File,
    Directory,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceFileExcerpt {
    pub path: WorkspaceRelativePath,
    pub start_line: usize,
    pub end_line: usize,
    pub total_lines: usize,
    pub content: String,
    pub content_sha256: String,
    pub modified_revision: u64,
    pub workspace_revision: u64,
}

/// 在同一 Workspace revision 下捕获的不可变文本文件。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceCapturedFile {
    pub path: WorkspaceRelativePath,
    pub byte_size: u64,
    pub content_sha256: String,
    pub modified_revision: u64,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceMovedFile {
    pub from: WorkspaceRelativePath,
    pub to: WorkspaceRelativePath,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceApplyPatchResult {
    pub added: Vec<WorkspaceRelativePath>,
    pub modified: Vec<WorkspaceRelativePath>,
    pub deleted: Vec<WorkspaceRelativePath>,
    pub moved: Vec<WorkspaceMovedFile>,
    pub workspace_revision: u64,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceFileTraceSnapshot {
    pub read_files: Vec<WorkspaceRelativePath>,
    pub added_files: Vec<WorkspaceRelativePath>,
    pub modified_files: Vec<WorkspaceRelativePath>,
    pub deleted_files: Vec<WorkspaceRelativePath>,
    pub moved_files: Vec<WorkspaceMovedFile>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct WorkspaceFileTrace {
    state: Arc<Mutex<WorkspaceFileTraceState>>,
}

#[derive(Debug, Default)]
struct WorkspaceFileTraceState {
    read_files: BTreeSet<WorkspaceRelativePath>,
    added_files: BTreeSet<WorkspaceRelativePath>,
    modified_files: BTreeSet<WorkspaceRelativePath>,
    deleted_files: BTreeSet<WorkspaceRelativePath>,
    moved_files: Vec<WorkspaceMovedFile>,
}

impl WorkspaceFileTrace {
    pub(crate) fn snapshot(&self) -> WorkspaceFileTraceSnapshot {
        let state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        WorkspaceFileTraceSnapshot {
            read_files: state.read_files.iter().cloned().collect(),
            added_files: state.added_files.iter().cloned().collect(),
            modified_files: state.modified_files.iter().cloned().collect(),
            deleted_files: state.deleted_files.iter().cloned().collect(),
            moved_files: state.moved_files.clone(),
        }
    }

    fn record_read(&self, path: WorkspaceRelativePath) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .read_files
            .insert(path);
    }

    fn record_patch(&self, result: &WorkspaceApplyPatchResult) {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.added_files.extend(result.added.iter().cloned());
        state.modified_files.extend(result.modified.iter().cloned());
        state.deleted_files.extend(result.deleted.iter().cloned());
        state.moved_files.extend(result.moved.iter().cloned());
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceFileService {
    layout: WorkspaceLayout,
    limits: WorkspaceFileLimits,
    trace: WorkspaceFileTrace,
}

impl WorkspaceFileService {
    pub(crate) fn new(layout: WorkspaceLayout, limits: WorkspaceFileLimits) -> Self {
        Self::new_with_trace(layout, limits, WorkspaceFileTrace::default())
    }

    pub(crate) fn new_with_trace(layout: WorkspaceLayout, limits: WorkspaceFileLimits, trace: WorkspaceFileTrace) -> Self {
        Self { layout, limits, trace }
    }

    /// 返回逻辑 Workspace 标识；不会暴露物理目录。
    pub(crate) fn workspace_id(&self) -> &str {
        self.layout.workspace_id()
    }

    pub(crate) fn workspace_revision(&self) -> WorkspaceResult<u64> {
        let lock = workspace_lock(self.layout.workspace_dir().to_path_buf());
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        recover_pending_file_commit(&self.layout)?;
        let mut manifest = read_manifest(&self.layout)?;
        reconcile_manifest_files(&self.layout, &mut manifest, &self.limits)?;
        Ok(manifest.workspace_revision)
    }

    pub(crate) fn list(&self, path: Option<&str>, depth: usize) -> WorkspaceResult<Vec<WorkspaceFileListEntry>> {
        let lock = workspace_lock(self.layout.workspace_dir().to_path_buf());
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        recover_pending_file_commit(&self.layout)?;
        if depth == 0 || depth > 32 {
            return Err(WorkspaceError::ResourceLimit("list depth must be between 1 and 32".to_string()));
        }
        let prefix = optional_prefix(path)?;
        let mut manifest = read_manifest(&self.layout)?;
        reconcile_manifest_files(&self.layout, &mut manifest, &self.limits)?;
        let mut directories = BTreeSet::new();
        let mut result = Vec::new();
        for file in &manifest.files {
            let Some(relative) = strip_scope(&file.path, prefix.as_ref()) else {
                continue;
            };
            let component_count = relative.split('/').count();
            if component_count <= depth {
                result.push(WorkspaceFileListEntry {
                    path: file.path.to_string(),
                    entry_type: WorkspaceFileEntryType::File,
                    byte_size: file.byte_size,
                    modified_revision: file.modified_revision,
                });
            }
            let mut current = prefix.as_ref().map(ToString::to_string).unwrap_or_default();
            for component in relative.split('/').take(component_count.saturating_sub(1).min(depth)) {
                if !current.is_empty() {
                    current.push('/');
                }
                current.push_str(component);
                directories.insert(current.clone());
            }
        }
        result.extend(directories.into_iter().map(|path| WorkspaceFileListEntry {
            path,
            entry_type: WorkspaceFileEntryType::Directory,
            byte_size: 0,
            modified_revision: 0,
        }));
        result.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| entry_rank(left.entry_type).cmp(&entry_rank(right.entry_type)))
        });
        ensure_serialized_output(&result, self.limits.max_tool_output_bytes)?;
        Ok(result)
    }

    pub(crate) fn read(&self, path: &str, start_line: usize, end_line: usize) -> WorkspaceResult<WorkspaceFileExcerpt> {
        let lock = workspace_lock(self.layout.workspace_dir().to_path_buf());
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        recover_pending_file_commit(&self.layout)?;
        let path = WorkspaceRelativePath::new(path)?;
        let mut manifest = read_manifest(&self.layout)?;
        reconcile_manifest_files(&self.layout, &mut manifest, &self.limits)?;
        let metadata = manifest
            .files
            .iter()
            .find(|file| file.path == path)
            .ok_or_else(|| WorkspaceError::InvalidFilePatch(format!("file does not exist: {path}")))?;
        let bytes = WorkspaceRoot::open(self.layout.workspace_dir())?.read_file(&path, self.limits.max_file_bytes)?;
        let content = String::from_utf8(bytes).map_err(|_| WorkspaceError::InvalidFilePatch(format!("file is not UTF-8 text: {path}")))?;
        self.trace.record_read(path.clone());
        let range = resolve_line_range(&content, start_line, end_line)?;
        let excerpt = WorkspaceFileExcerpt {
            path,
            start_line,
            end_line: range.end_line,
            total_lines: range.total_lines,
            content: content[range.start_byte..range.end_byte].to_string(),
            content_sha256: metadata.content_sha256.clone(),
            modified_revision: metadata.modified_revision,
            workspace_revision: manifest.workspace_revision,
        };
        ensure_serialized_output(&excerpt, self.limits.max_tool_output_bytes)?;
        Ok(excerpt)
    }

    /// 在共享锁内按指定 Workspace revision 捕获多个文件。
    pub(crate) fn capture(&self, paths: &[WorkspaceRelativePath], expected_revision: u64) -> WorkspaceResult<Vec<WorkspaceCapturedFile>> {
        const MAX_CAPTURED_FILES: usize = 32;
        if paths.is_empty() || paths.len() > MAX_CAPTURED_FILES {
            return Err(WorkspaceError::ResourceLimit(format!("captured file count must be between 1 and {MAX_CAPTURED_FILES}")));
        }
        let mut unique = HashSet::with_capacity(paths.len());
        if paths.iter().any(|path| !unique.insert(path.clone())) {
            return Err(WorkspaceError::InvalidToolCall {
                tool: "workspace_capture".to_string(),
                message: "captured paths must be unique".to_string(),
            });
        }

        let lock = workspace_lock(self.layout.workspace_dir().to_path_buf());
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        recover_pending_file_commit(&self.layout)?;
        let mut manifest = read_manifest(&self.layout)?;
        reconcile_manifest_files(&self.layout, &mut manifest, &self.limits)?;
        if manifest.workspace_revision != expected_revision {
            return Err(WorkspaceError::RevisionConflict {
                artifact_id: "workspace".to_string(),
                expected: expected_revision,
                actual: manifest.workspace_revision,
            });
        }

        let root = WorkspaceRoot::open(self.layout.workspace_dir())?;
        let mut captured = Vec::with_capacity(paths.len());
        for path in paths {
            let metadata = manifest
                .files
                .iter()
                .find(|file| file.path == *path)
                .ok_or_else(|| WorkspaceError::InvalidFilePatch(format!("file does not exist: {path}")))?;
            let bytes = root.read_file(path, self.limits.max_file_bytes)?;
            if content_sha256(&bytes) != metadata.content_sha256 {
                return Err(WorkspaceError::InvalidManifest(format!("workspace file content hash mismatch: path={path}")));
            }
            let content = String::from_utf8(bytes).map_err(|_| WorkspaceError::InvalidFilePatch(format!("file is not UTF-8 text: {path}")))?;
            captured.push(WorkspaceCapturedFile {
                path: path.clone(),
                byte_size: metadata.byte_size,
                content_sha256: metadata.content_sha256.clone(),
                modified_revision: metadata.modified_revision,
                content,
            });
        }
        Ok(captured)
    }

    /// 捕获完整 Workspace 内容，用于宿主可恢复快照。
    pub(crate) fn capture_all(&self, expected_revision: u64) -> WorkspaceResult<Vec<WorkspaceCapturedFile>> {
        let lock = workspace_lock(self.layout.workspace_dir().to_path_buf());
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        recover_pending_file_commit(&self.layout)?;
        let mut manifest = read_manifest(&self.layout)?;
        reconcile_manifest_files(&self.layout, &mut manifest, &self.limits)?;
        validate_expected_revision(&manifest, expected_revision)?;
        let root = WorkspaceRoot::open(self.layout.workspace_dir())?;
        let mut captured = Vec::with_capacity(manifest.files.len());
        for metadata in &manifest.files {
            let bytes = root.read_file(&metadata.path, self.limits.max_workspace_bytes)?;
            let content =
                String::from_utf8(bytes).map_err(|_| WorkspaceError::InvalidFilePatch(format!("file is not UTF-8 text: {}", metadata.path)))?;
            captured.push(WorkspaceCapturedFile {
                path: metadata.path.clone(),
                byte_size: metadata.byte_size,
                content_sha256: metadata.content_sha256.clone(),
                modified_revision: metadata.modified_revision,
                content,
            });
        }
        Ok(captured)
    }

    /// 将完整内容快照恢复为新的 Workspace revision，不回退版本号。
    pub(crate) fn restore_files(&self, snapshot: &[WorkspaceCapturedFile], expected_revision: u64) -> WorkspaceResult<WorkspaceApplyPatchResult> {
        let lock = workspace_lock(self.layout.workspace_dir().to_path_buf());
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        recover_pending_file_commit(&self.layout)?;
        let mut old_manifest = read_manifest(&self.layout)?;
        reconcile_manifest_files(&self.layout, &mut old_manifest, &self.limits)?;
        validate_expected_revision(&old_manifest, expected_revision)?;
        let mut desired = BTreeMap::new();
        for file in snapshot {
            if file.byte_size != u64::try_from(file.content.len()).unwrap_or(u64::MAX)
                || file.content_sha256 != content_sha256(file.content.as_bytes())
                || desired.insert(file.path.clone(), file.content.clone()).is_some()
            {
                return Err(WorkspaceError::InvalidManifest(format!("invalid snapshot file: path={}", file.path)));
            }
            validate_content(&file.path, &file.content, self.limits.max_file_bytes)?;
        }
        let root = WorkspaceRoot::open(self.layout.workspace_dir())?;
        let current = old_manifest
            .files
            .iter()
            .map(|file| (file.path.clone(), file))
            .collect::<BTreeMap<_, _>>();
        let all_paths = current.keys().cloned().chain(desired.keys().cloned()).collect::<BTreeSet<_>>();
        let mut changes = BTreeMap::new();
        let mut added = Vec::new();
        let mut modified = Vec::new();
        let mut deleted = Vec::new();
        for path in all_paths {
            let old_content = current
                .get(&path)
                .map(|_| read_utf8(&root, &path, self.limits.max_workspace_bytes))
                .transpose()?;
            let new_content = desired.get(&path).cloned();
            if old_content == new_content {
                continue;
            }
            match (&old_content, &new_content) {
                (None, Some(_)) => added.push(path.clone()),
                (Some(_), Some(_)) => modified.push(path.clone()),
                (Some(_), None) => deleted.push(path.clone()),
                (None, None) => continue,
            }
            changes.insert(path.clone(), FileJournalChange { path, old_content, new_content });
        }
        if changes.is_empty() {
            return Ok(WorkspaceApplyPatchResult {
                added,
                modified,
                deleted,
                moved: Vec::new(),
                workspace_revision: expected_revision,
                diagnostics: Vec::new(),
            });
        }
        self.commit_changes(old_manifest, changes, added, modified, deleted, Vec::new())
    }

    pub(crate) fn apply_patch(&self, patch: &str, expected_revision: u64) -> WorkspaceResult<WorkspaceApplyPatchResult> {
        let lock = workspace_lock(self.layout.workspace_dir().to_path_buf());
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        recover_pending_file_commit(&self.layout)?;
        let mut old_manifest = read_manifest(&self.layout)?;
        reconcile_manifest_files(&self.layout, &mut old_manifest, &self.limits)?;
        if old_manifest.workspace_revision != expected_revision {
            return Err(WorkspaceError::RevisionConflict {
                artifact_id: "workspace".to_string(),
                expected: expected_revision,
                actual: old_manifest.workspace_revision,
            });
        }
        let parsed = parse_patch(patch, self.limits.max_patch_bytes, self.limits.max_patch_operations)?;
        let root = WorkspaceRoot::open(self.layout.workspace_dir())?;
        let mut current_paths = old_manifest.files.iter().map(|file| file.path.clone()).collect::<HashSet<_>>();
        let mut changes = BTreeMap::<WorkspaceRelativePath, FileJournalChange>::new();
        let mut touched = HashSet::new();
        let mut added = Vec::new();
        let mut modified = Vec::new();
        let mut deleted = Vec::new();
        let mut moved = Vec::new();

        for operation in parsed.operations {
            match operation {
                FilePatchOperation::Add { path, content } => {
                    touch_once(&mut touched, &path)?;
                    if current_paths.contains(&path) {
                        return Err(invalid(format!("Add File target already exists: {path}")));
                    }
                    validate_content(&path, &content, self.limits.max_file_bytes)?;
                    current_paths.insert(path.clone());
                    changes.insert(path.clone(), FileJournalChange { path: path.clone(), old_content: None, new_content: Some(content) });
                    added.push(path);
                }
                FilePatchOperation::Update { path, move_to, hunks } => {
                    touch_once(&mut touched, &path)?;
                    if !current_paths.contains(&path) {
                        return Err(invalid(format!("Update File source does not exist: {path}")));
                    }
                    let old_content = read_utf8(&root, &path, self.limits.max_file_bytes)?;
                    let new_content = apply_hunks(&old_content, &hunks)?;
                    let target = move_to.unwrap_or_else(|| path.clone());
                    if target != path {
                        touch_once(&mut touched, &target)?;
                        if current_paths.contains(&target) {
                            return Err(invalid(format!("Move target already exists: {target}")));
                        }
                    }
                    validate_content(&target, &new_content, self.limits.max_file_bytes)?;
                    if target == path {
                        changes.insert(
                            path.clone(),
                            FileJournalChange { path: path.clone(), old_content: Some(old_content), new_content: Some(new_content) },
                        );
                        modified.push(path);
                    } else {
                        current_paths.remove(&path);
                        current_paths.insert(target.clone());
                        changes.insert(path.clone(), FileJournalChange { path: path.clone(), old_content: Some(old_content), new_content: None });
                        changes.insert(target.clone(), FileJournalChange { path: target.clone(), old_content: None, new_content: Some(new_content) });
                        moved.push(WorkspaceMovedFile { from: path, to: target });
                    }
                }
                FilePatchOperation::Delete { path } => {
                    touch_once(&mut touched, &path)?;
                    if !current_paths.remove(&path) {
                        return Err(invalid(format!("Delete File target does not exist: {path}")));
                    }
                    let old_content = read_utf8(&root, &path, self.limits.max_file_bytes)?;
                    changes.insert(path.clone(), FileJournalChange { path: path.clone(), old_content: Some(old_content), new_content: None });
                    deleted.push(path);
                }
            }
        }

        let new_revision = old_manifest.workspace_revision.saturating_add(1);
        let timestamp = now_ms();
        let new_manifest = derive_target_manifest(&old_manifest, &changes.values().cloned().collect::<Vec<_>>(), new_revision, timestamp)?;
        enforce_workspace_limits(&new_manifest, &self.limits)?;
        let journal = FileTransactionJournal::new(&self.layout, old_manifest, new_manifest, changes.into_values().collect());
        commit_file_transaction(&self.layout, &journal)?;
        let result = WorkspaceApplyPatchResult { added, modified, deleted, moved, workspace_revision: new_revision, diagnostics: Vec::new() };
        self.trace.record_patch(&result);
        Ok(result)
    }

    /// 原子移动单个文件或以该路径为前缀的目录树。
    pub(crate) fn move_path(&self, from: &str, to: &str, expected_revision: u64) -> WorkspaceResult<WorkspaceApplyPatchResult> {
        let from = WorkspaceRelativePath::new(from)?;
        let to = WorkspaceRelativePath::new(to)?;
        if from == to || to.as_str().starts_with(&format!("{from}/")) || from.as_str().starts_with(&format!("{to}/")) {
            return Err(invalid("move target must differ from source and cannot be inside or an ancestor of it".to_string()));
        }
        let lock = workspace_lock(self.layout.workspace_dir().to_path_buf());
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        recover_pending_file_commit(&self.layout)?;
        let mut old_manifest = read_manifest(&self.layout)?;
        reconcile_manifest_files(&self.layout, &mut old_manifest, &self.limits)?;
        validate_expected_revision(&old_manifest, expected_revision)?;
        let sources = scoped_files(&old_manifest, &from);
        if sources.is_empty() {
            return Err(invalid(format!("move source does not exist: {from}")));
        }
        if sources.len() > self.limits.max_workspace_files {
            return Err(WorkspaceError::ResourceLimit(format!("move contains more than {} files", self.limits.max_workspace_files)));
        }
        let source_paths = sources.iter().map(|file| file.path.clone()).collect::<HashSet<_>>();
        let all_paths = old_manifest.files.iter().map(|file| file.path.clone()).collect::<HashSet<_>>();
        let target_prefix = format!("{to}/");
        if all_paths
            .iter()
            .any(|path| !source_paths.contains(path) && (path == &to || path.as_str().starts_with(&target_prefix)))
        {
            return Err(invalid(format!("move target already exists: {to}")));
        }

        let root = WorkspaceRoot::open(self.layout.workspace_dir())?;
        let mut changes = BTreeMap::new();
        let mut moved = Vec::with_capacity(sources.len());
        for file in sources {
            let suffix = file
                .path
                .as_str()
                .strip_prefix(from.as_str())
                .expect("scoped path must share prefix");
            let target = WorkspaceRelativePath::new(format!("{}{suffix}", to.as_str()))?;
            if all_paths.contains(&target) && !source_paths.contains(&target) {
                return Err(invalid(format!("move target already exists: {target}")));
            }
            let content = read_utf8(&root, &file.path, self.limits.max_file_bytes)?;
            validate_content(&target, &content, self.limits.max_file_bytes)?;
            changes.insert(
                file.path.clone(),
                FileJournalChange { path: file.path.clone(), old_content: Some(content.clone()), new_content: None },
            );
            changes.insert(target.clone(), FileJournalChange { path: target.clone(), old_content: None, new_content: Some(content) });
            moved.push(WorkspaceMovedFile { from: file.path.clone(), to: target });
        }
        self.commit_changes(old_manifest, changes, Vec::new(), Vec::new(), Vec::new(), moved)
    }

    /// 原子删除单个文件或以该路径为前缀的目录树。
    pub(crate) fn delete_path(&self, path: &str, expected_revision: u64) -> WorkspaceResult<WorkspaceApplyPatchResult> {
        let path = WorkspaceRelativePath::new(path)?;
        let lock = workspace_lock(self.layout.workspace_dir().to_path_buf());
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        recover_pending_file_commit(&self.layout)?;
        let mut old_manifest = read_manifest(&self.layout)?;
        reconcile_manifest_files(&self.layout, &mut old_manifest, &self.limits)?;
        validate_expected_revision(&old_manifest, expected_revision)?;
        let files = scoped_files(&old_manifest, &path);
        if files.is_empty() {
            return Err(invalid(format!("delete target does not exist: {path}")));
        }
        if files.len() > self.limits.max_workspace_files {
            return Err(WorkspaceError::ResourceLimit(format!("delete contains more than {} files", self.limits.max_workspace_files)));
        }
        let root = WorkspaceRoot::open(self.layout.workspace_dir())?;
        let mut changes = BTreeMap::new();
        let mut deleted = Vec::with_capacity(files.len());
        for file in files {
            let content = read_utf8(&root, &file.path, self.limits.max_file_bytes)?;
            changes.insert(
                file.path.clone(),
                FileJournalChange { path: file.path.clone(), old_content: Some(content), new_content: None },
            );
            deleted.push(file.path.clone());
        }
        self.commit_changes(old_manifest, changes, Vec::new(), Vec::new(), deleted, Vec::new())
    }

    fn commit_changes(
        &self,
        old_manifest: crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceManifest,
        changes: BTreeMap<WorkspaceRelativePath, FileJournalChange>,
        added: Vec<WorkspaceRelativePath>,
        modified: Vec<WorkspaceRelativePath>,
        deleted: Vec<WorkspaceRelativePath>,
        moved: Vec<WorkspaceMovedFile>,
    ) -> WorkspaceResult<WorkspaceApplyPatchResult> {
        let new_revision = old_manifest.workspace_revision.saturating_add(1);
        let new_manifest = derive_target_manifest(&old_manifest, &changes.values().cloned().collect::<Vec<_>>(), new_revision, now_ms())?;
        enforce_workspace_limits(&new_manifest, &self.limits)?;
        let journal = FileTransactionJournal::new(&self.layout, old_manifest, new_manifest, changes.into_values().collect());
        commit_file_transaction(&self.layout, &journal)?;
        let result = WorkspaceApplyPatchResult { added, modified, deleted, moved, workspace_revision: new_revision, diagnostics: Vec::new() };
        self.trace.record_patch(&result);
        Ok(result)
    }
}

fn validate_expected_revision(
    manifest: &crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceManifest,
    expected_revision: u64,
) -> WorkspaceResult<()> {
    if manifest.workspace_revision != expected_revision {
        return Err(WorkspaceError::RevisionConflict {
            artifact_id: "workspace".to_string(),
            expected: expected_revision,
            actual: manifest.workspace_revision,
        });
    }
    Ok(())
}

fn scoped_files<'a>(
    manifest: &'a crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceManifest,
    path: &WorkspaceRelativePath,
) -> Vec<&'a crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceFileMetadata> {
    let prefix = format!("{path}/");
    manifest
        .files
        .iter()
        .filter(|file| file.path == *path || file.path.as_str().starts_with(&prefix))
        .collect()
}

fn validate_content(path: &WorkspaceRelativePath, content: &str, max_bytes: u64) -> WorkspaceResult<()> {
    if u64::try_from(content.len()).unwrap_or(u64::MAX) > max_bytes {
        return Err(WorkspaceError::ResourceLimit(format!("file exceeds {max_bytes} bytes: {path}")));
    }
    if path.as_str().ends_with(".json") {
        serde_json::from_str::<serde_json::Value>(content)
            .map_err(|error| WorkspaceError::InvalidFilePatch(format!("invalid JSON: path={path}, error={error}")))?;
    }
    Ok(())
}

fn enforce_workspace_limits(
    manifest: &crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceManifest,
    limits: &WorkspaceFileLimits,
) -> WorkspaceResult<()> {
    if manifest.files.len() > limits.max_workspace_files {
        return Err(WorkspaceError::ResourceLimit(format!("workspace exceeds {} files", limits.max_workspace_files)));
    }
    let bytes = manifest
        .files
        .iter()
        .fold(0u64, |total, file| total.saturating_add(file.byte_size));
    if bytes > limits.max_workspace_bytes {
        return Err(WorkspaceError::ResourceLimit(format!("workspace exceeds {} bytes", limits.max_workspace_bytes)));
    }
    Ok(())
}

fn read_utf8(root: &WorkspaceRoot, path: &WorkspaceRelativePath, max_bytes: u64) -> WorkspaceResult<String> {
    String::from_utf8(root.read_file(path, max_bytes)?).map_err(|_| WorkspaceError::InvalidFilePatch(format!("file is not UTF-8 text: {path}")))
}

fn optional_prefix(path: Option<&str>) -> WorkspaceResult<Option<WorkspaceRelativePath>> {
    match path.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => WorkspaceRelativePath::new(value).map(Some),
        None => Ok(None),
    }
}

fn strip_scope<'a>(path: &'a WorkspaceRelativePath, prefix: Option<&WorkspaceRelativePath>) -> Option<&'a str> {
    let Some(prefix) = prefix else { return Some(path.as_str()) };
    if path == prefix {
        return Some(path.as_str().rsplit('/').next().unwrap_or(path.as_str()));
    }
    path.as_str().strip_prefix(prefix.as_str())?.strip_prefix('/')
}

fn touch_once(paths: &mut HashSet<WorkspaceRelativePath>, path: &WorkspaceRelativePath) -> WorkspaceResult<()> {
    if !paths.insert(path.clone()) {
        return Err(invalid(format!("path appears in multiple operations: {path}")));
    }
    Ok(())
}

fn invalid(message: String) -> WorkspaceError {
    WorkspaceError::InvalidFilePatch(message)
}

fn ensure_serialized_output<T: Serialize>(value: &T, max_bytes: usize) -> WorkspaceResult<()> {
    let bytes = serde_json::to_vec(value).map_err(|error| WorkspaceError::InvalidManifest(error.to_string()))?;
    if bytes.len() > max_bytes {
        return Err(WorkspaceError::ResourceLimit(format!("tool output exceeds {max_bytes} bytes")));
    }
    Ok(())
}

fn entry_rank(entry_type: WorkspaceFileEntryType) -> u8 {
    match entry_type {
        WorkspaceFileEntryType::Directory => 0,
        WorkspaceFileEntryType::File => 1,
    }
}
