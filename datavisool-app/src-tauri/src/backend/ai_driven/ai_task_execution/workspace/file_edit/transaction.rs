/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 多文件写入 journal 与崩溃后的幂等前滚恢复。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::path::WorkspaceRoot;
use crate::backend::ai_driven::ai_task_execution::workspace::manager::{read_manifest, write_bytes_create_once, write_manifest};
use crate::backend::ai_driven::ai_task_execution::workspace::types::content_sha256;
use crate::backend::ai_driven::ai_task_execution::workspace::{
    ArtifactContentFormat, ArtifactId, ArtifactMetadata, ArtifactStatus, WorkspaceError, WorkspaceFileMetadata, WorkspaceLayout, WorkspaceManifest,
    WorkspaceRelativePath, WorkspaceResult,
};

const FILE_JOURNAL_SCHEMA: &str = "datavisool-workspace-file-transaction/v1";
const FILE_JOURNAL_NAME: &str = "workspace-files.json";
static COMMIT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct FileTransactionJournal {
    schema_version: String,
    workspace_id: String,
    old_manifest: WorkspaceManifest,
    new_manifest: WorkspaceManifest,
    changes: Vec<FileJournalChange>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct FileJournalChange {
    pub(super) path: WorkspaceRelativePath,
    pub(super) old_content: Option<String>,
    pub(super) new_content: Option<String>,
}

impl FileTransactionJournal {
    pub(super) fn new(
        layout: &WorkspaceLayout,
        old_manifest: WorkspaceManifest,
        new_manifest: WorkspaceManifest,
        changes: Vec<FileJournalChange>,
    ) -> Self {
        Self {
            schema_version: FILE_JOURNAL_SCHEMA.to_string(),
            workspace_id: layout.workspace_id().to_string(),
            old_manifest,
            new_manifest,
            changes,
        }
    }
}

pub(super) fn commit_file_transaction(layout: &WorkspaceLayout, journal: &FileTransactionJournal) -> WorkspaceResult<()> {
    validate_journal(layout, journal)?;
    let path = journal_path(layout);
    let bytes = serde_json::to_vec(journal).map_err(|error| WorkspaceError::Storage {
        operation: "serializeWorkspaceFileJournal",
        path: path.clone(),
        message: error.to_string(),
    })?;
    if !write_bytes_create_once(&path, &bytes)? {
        return Err(WorkspaceError::InvalidManifest("workspace file transaction journal already exists".to_string()));
    }
    apply_target_state(layout, journal)?;
    clear_journal(layout)
}

pub(crate) fn recover_pending_file_commit(layout: &WorkspaceLayout) -> WorkspaceResult<()> {
    let Some(journal) = read_file_journal(layout)? else {
        return Ok(());
    };
    validate_journal(layout, &journal)?;
    let manifest = read_manifest(layout)?;
    if manifest != journal.old_manifest && manifest != journal.new_manifest {
        return Err(WorkspaceError::InvalidManifest(
            "workspace file transaction manifest is neither the recorded base nor target".to_string(),
        ));
    }
    let root = WorkspaceRoot::open(layout.workspace_dir())?;
    for change in &journal.changes {
        root.cleanup_temporary_files(&change.path)?;
        let current = root.try_read_file(&change.path, recovery_max_bytes(change)?)?;
        let current = current
            .map(|bytes| String::from_utf8(bytes).map_err(|_| WorkspaceError::InvalidFilePatch(format!("non-UTF-8 file: {}", change.path))))
            .transpose()?;
        if current != change.old_content && current != change.new_content {
            return Err(WorkspaceError::InvalidManifest(format!(
                "workspace file transaction found unexpected content: path={}",
                change.path
            )));
        }
    }
    apply_target_state(layout, &journal)?;
    clear_journal(layout)
}

fn apply_target_state(layout: &WorkspaceLayout, journal: &FileTransactionJournal) -> WorkspaceResult<()> {
    let root = WorkspaceRoot::open(layout.workspace_dir())?;
    for change in &journal.changes {
        match &change.new_content {
            Some(content) => {
                let sequence = COMMIT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                root.write_file_atomically(&change.path, content.as_bytes(), sequence)?;
            }
            None => {
                if root.try_read_file(&change.path, recovery_max_bytes(change)?)?.is_some() {
                    root.remove_file(&change.path)?;
                }
            }
        }
    }
    write_manifest(layout, &journal.new_manifest)
}

fn recovery_max_bytes(change: &FileJournalChange) -> WorkspaceResult<u64> {
    const MAX_RECOVERY_FILE_BYTES: u64 = 100 * 1024 * 1024;
    let bytes = change
        .old_content
        .as_ref()
        .into_iter()
        .chain(change.new_content.as_ref())
        .map(|content| u64::try_from(content.len()).unwrap_or(u64::MAX))
        .max()
        .unwrap_or(0);
    if bytes > MAX_RECOVERY_FILE_BYTES {
        return Err(WorkspaceError::ResourceLimit(format!("transaction file exceeds {MAX_RECOVERY_FILE_BYTES} bytes")));
    }
    Ok(bytes)
}

fn validate_journal(layout: &WorkspaceLayout, journal: &FileTransactionJournal) -> WorkspaceResult<()> {
    if journal.schema_version != FILE_JOURNAL_SCHEMA || journal.workspace_id != layout.workspace_id() {
        return Err(WorkspaceError::InvalidManifest("invalid workspace file transaction journal identity".to_string()));
    }
    if journal.old_manifest.workspace_id != journal.workspace_id
        || journal.new_manifest.workspace_id != journal.workspace_id
        || journal.new_manifest.workspace_revision != journal.old_manifest.workspace_revision.saturating_add(1)
    {
        return Err(WorkspaceError::InvalidManifest("invalid workspace file transaction revision chain".to_string()));
    }
    if journal.changes.is_empty() {
        return Err(WorkspaceError::InvalidManifest("workspace file transaction has no changes".to_string()));
    }
    if journal.new_manifest.updated_at < journal.old_manifest.updated_at {
        return Err(WorkspaceError::InvalidManifest("workspace file transaction moved updatedAt backwards".to_string()));
    }
    let mut files = journal
        .old_manifest
        .files
        .iter()
        .cloned()
        .map(|file| (file.path.clone(), file))
        .collect::<HashMap<_, _>>();
    let mut unique = HashSet::with_capacity(journal.changes.len());
    for change in &journal.changes {
        if !unique.insert(change.path.clone()) {
            return Err(WorkspaceError::InvalidManifest("workspace file transaction contains a duplicate path".to_string()));
        }
        let old_metadata = files.get(&change.path);
        match (&change.old_content, old_metadata) {
            (Some(content), Some(metadata))
                if metadata.byte_size == u64::try_from(content.len()).unwrap_or(u64::MAX)
                    && metadata.content_sha256 == content_sha256(content.as_bytes()) => {}
            (None, None) => {}
            _ => return Err(WorkspaceError::InvalidManifest(format!("workspace file transaction base mismatch: path={}", change.path))),
        }
        match &change.new_content {
            Some(content) => {
                files.insert(
                    change.path.clone(),
                    WorkspaceFileMetadata {
                        path: change.path.clone(),
                        byte_size: u64::try_from(content.len()).unwrap_or(u64::MAX),
                        content_sha256: content_sha256(content.as_bytes()),
                        modified_revision: journal.new_manifest.workspace_revision,
                    },
                );
            }
            None => {
                files.remove(&change.path);
            }
        }
    }
    let expected = derive_target_manifest(
        &journal.old_manifest,
        &journal.changes,
        journal.new_manifest.workspace_revision,
        journal.new_manifest.updated_at,
    )?;
    if expected != journal.new_manifest {
        return Err(WorkspaceError::InvalidManifest("workspace file transaction target manifest mismatch".to_string()));
    }
    Ok(())
}

/// 根据文件事务唯一推导目标 manifest，确保普通文件 Tool 与 ArtifactStore 元数据一致。
pub(super) fn derive_target_manifest(
    old: &WorkspaceManifest,
    changes: &[FileJournalChange],
    revision: u64,
    updated_at: i64,
) -> WorkspaceResult<WorkspaceManifest> {
    let mut files = old
        .files
        .iter()
        .cloned()
        .map(|file| (file.path.clone(), file))
        .collect::<BTreeMap<_, _>>();
    let mut artifacts = old
        .artifacts
        .iter()
        .cloned()
        .map(|metadata| (metadata.artifact_id.clone(), metadata))
        .collect::<HashMap<_, _>>();

    for change in changes {
        match &change.new_content {
            Some(content) => {
                files.insert(
                    change.path.clone(),
                    WorkspaceFileMetadata {
                        path: change.path.clone(),
                        byte_size: u64::try_from(content.len()).unwrap_or(u64::MAX),
                        content_sha256: content_sha256(content.as_bytes()),
                        modified_revision: revision,
                    },
                );
            }
            None => {
                files.remove(&change.path);
            }
        }

        let Some(artifact_id) = artifact_id_for_path(&change.path)? else {
            continue;
        };
        match &change.new_content {
            Some(content) => {
                let hash = content_sha256(content.as_bytes());
                match artifacts.get_mut(&artifact_id) {
                    Some(metadata) => {
                        metadata.revision = metadata.revision.saturating_add(1);
                        metadata.content_sha256 = hash;
                        metadata.status = ArtifactStatus::Draft;
                        metadata.updated_at = updated_at;
                    }
                    None => {
                        artifacts.insert(
                            artifact_id.clone(),
                            ArtifactMetadata {
                                artifact_id,
                                revision: 1,
                                content_format: ArtifactContentFormat::Json,
                                content_sha256: hash,
                                status: ArtifactStatus::Draft,
                                created_at: updated_at,
                                updated_at,
                            },
                        );
                    }
                }
            }
            None => {
                artifacts.remove(&artifact_id);
            }
        }
    }

    let mut manifest = old.clone();
    manifest.workspace_revision = revision;
    manifest.updated_at = updated_at;
    manifest.files = files.into_values().collect();
    manifest.artifacts = artifacts.into_values().collect();
    manifest
        .artifacts
        .sort_by(|left, right| left.artifact_id.name().cmp(right.artifact_id.name()));
    Ok(manifest)
}

pub(super) fn artifact_id_for_path(path: &WorkspaceRelativePath) -> WorkspaceResult<Option<ArtifactId>> {
    let Some(name) = path
        .as_str()
        .strip_prefix("artifacts/")
        .and_then(|value| value.strip_suffix(".json"))
    else {
        return Ok(None);
    };
    if name.contains('/') {
        return Ok(None);
    }
    ArtifactId::parse(name).map(Some)
}

fn journal_path(layout: &WorkspaceLayout) -> std::path::PathBuf {
    layout.transaction_journal_dir().join(FILE_JOURNAL_NAME)
}

fn read_file_journal(layout: &WorkspaceLayout) -> WorkspaceResult<Option<FileTransactionJournal>> {
    const MAX_JOURNAL_BYTES: u64 = 256 * 1024 * 1024;
    let path = journal_path(layout);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(WorkspaceError::Storage { operation: "inspectWorkspaceFileJournal", path, message: error.to_string() }),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(WorkspaceError::InvalidManifest("workspace file transaction journal is not a regular file".to_string()));
    }
    if metadata.len() > MAX_JOURNAL_BYTES {
        return Err(WorkspaceError::ResourceLimit(format!("workspace file transaction journal exceeds {MAX_JOURNAL_BYTES} bytes")));
    }
    let bytes =
        fs::read(&path).map_err(|error| WorkspaceError::Storage { operation: "readWorkspaceFileJournal", path, message: error.to_string() })?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| WorkspaceError::InvalidManifest(format!("workspace file transaction journal is invalid JSON: {error}")))
}

fn clear_journal(layout: &WorkspaceLayout) -> WorkspaceResult<()> {
    let path = journal_path(layout);
    match fs::remove_file(&path) {
        Ok(()) => sync_journal_directory(layout),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(WorkspaceError::Storage { operation: "removeWorkspaceFileJournal", path, message: error.to_string() }),
    }
}

#[cfg(not(target_os = "windows"))]
fn sync_journal_directory(layout: &WorkspaceLayout) -> WorkspaceResult<()> {
    std::fs::File::open(layout.transaction_journal_dir())
        .and_then(|directory| directory.sync_all())
        .map_err(|error| WorkspaceError::Storage {
            operation: "syncWorkspaceFileJournalDirectory",
            path: layout.transaction_journal_dir(),
            message: error.to_string(),
        })
}

#[cfg(target_os = "windows")]
fn sync_journal_directory(_layout: &WorkspaceLayout) -> WorkspaceResult<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::ai_task_execution::workspace::manager::write_bytes_atomically;
    use crate::backend::ai_driven::ai_task_execution::workspace::{WorkspaceFileLimits, WorkspaceFileService, WorkspaceManager};
    use crate::backend::ai_driven::time::now_ms;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn recovery_forwards_a_journal_before_nested_parent_exists() {
        let root = std::env::temp_dir().join(format!(
            "datavisool-workspace-file-recovery-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let manager = WorkspaceManager::new(&root);
        let workspace = manager.create("project-1").unwrap();
        let old_manifest = workspace.manifest.clone();
        let path = WorkspaceRelativePath::new("nested/file.txt").unwrap();
        let content = "recovered\n".to_string();
        let mut new_manifest = old_manifest.clone();
        new_manifest.workspace_revision = 1;
        new_manifest.updated_at = now_ms();
        new_manifest.files.push(WorkspaceFileMetadata {
            path: path.clone(),
            byte_size: content.len() as u64,
            content_sha256: content_sha256(content.as_bytes()),
            modified_revision: 1,
        });
        new_manifest.files.sort_by(|left, right| left.path.cmp(&right.path));
        let journal = FileTransactionJournal::new(
            &workspace.layout,
            old_manifest,
            new_manifest,
            vec![FileJournalChange { path: path.clone(), old_content: None, new_content: Some(content.clone()) }],
        );
        assert_eq!(journal.schema_version, "datavisool-workspace-file-transaction/v1");
        let mut old_journal = journal.clone();
        old_journal.schema_version = "lcp-workspace-file-transaction/v1".to_string();
        assert!(validate_journal(&workspace.layout, &old_journal).is_err());
        validate_journal(&workspace.layout, &journal).unwrap();
        write_bytes_atomically(&journal_path(&workspace.layout), &serde_json::to_vec(&journal).unwrap()).unwrap();

        let reopened = manager
            .open("project-1")
            .expect("open should recover the pending file transaction");
        assert_eq!(reopened.manifest.workspace_revision, 1);
        assert_eq!(std::fs::read_to_string(workspace.layout.workspace_dir().join(path.as_str())).unwrap(), content);
        assert!(!journal_path(&workspace.layout).exists());
        assert_eq!(
            WorkspaceFileService::new(reopened.layout, WorkspaceFileLimits::default())
                .workspace_revision()
                .unwrap(),
            1
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
