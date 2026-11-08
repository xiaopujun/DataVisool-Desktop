/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 宿主拥有的完整 Workspace 内容快照与恢复。

use std::fs;

use serde::{Deserialize, Serialize};

use super::file_edit::{WorkspaceApplyPatchResult, WorkspaceCapturedFile, WorkspaceFileLimits, WorkspaceFileService};
use super::manager::{write_bytes_create_once, WorkspaceHandle};
use super::types::{content_sha256, is_safe_name, WorkspaceError, WorkspaceResult};
use super::WorkspaceLayout;
use crate::backend::ai_driven::time::now_ms;

const WORKSPACE_SNAPSHOT_SCHEMA: &str = "datavisool-workspace-snapshot/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WorkspaceSnapshotReason {
    TurnStart,
    PrePublish,
    PostPublish,
}

impl WorkspaceSnapshotReason {
    fn name(self) -> &'static str {
        match self {
            Self::TurnStart => "turn-start",
            Self::PrePublish => "pre-publish",
            Self::PostPublish => "post-publish",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkspaceContentSnapshot {
    pub schema_version: String,
    pub snapshot_id: String,
    pub run_id: String,
    pub reason: WorkspaceSnapshotReason,
    pub workspace_revision: u64,
    pub files: Vec<WorkspaceCapturedFile>,
    pub created_at: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceSnapshotStore {
    layout: WorkspaceLayout,
    files: WorkspaceFileService,
}

impl WorkspaceSnapshotStore {
    pub(crate) fn new(workspace: &WorkspaceHandle, limits: WorkspaceFileLimits) -> Self {
        Self { layout: workspace.layout.clone(), files: WorkspaceFileService::new(workspace.layout.clone(), limits) }
    }

    pub(crate) fn capture(&self, run_id: &str, reason: WorkspaceSnapshotReason, expected_revision: u64) -> WorkspaceResult<WorkspaceContentSnapshot> {
        if !is_safe_name(run_id, 160) {
            return Err(WorkspaceError::InvalidWorkspaceId(format!("runId={run_id}")));
        }
        let files = self.files.capture_all(expected_revision)?;
        let identity = serde_json::to_vec(&(
            run_id,
            reason,
            expected_revision,
            files
                .iter()
                .map(|file| (&file.path, &file.content_sha256, file.byte_size))
                .collect::<Vec<_>>(),
        ))
        .map_err(|error| WorkspaceError::InvalidManifest(format!("serialize snapshot identity: {error}")))?;
        let digest = content_sha256(&identity);
        let snapshot_id = format!("{}-{}-{}-{}", run_id, reason.name(), expected_revision, &digest[..16]);
        let proposed = WorkspaceContentSnapshot {
            schema_version: WORKSPACE_SNAPSHOT_SCHEMA.to_string(),
            snapshot_id,
            run_id: run_id.to_string(),
            reason,
            workspace_revision: expected_revision,
            files,
            created_at: now_ms(),
        };
        self.persist(proposed)
    }

    pub(crate) fn read(&self, snapshot_id: &str) -> WorkspaceResult<WorkspaceContentSnapshot> {
        if !is_safe_name(snapshot_id, 240) {
            return Err(WorkspaceError::InvalidWorkspaceId(format!("snapshotId={snapshot_id}")));
        }
        let path = self.layout.workspace_snapshots_dir().join(format!("{snapshot_id}.json"));
        let metadata = fs::symlink_metadata(&path).map_err(|error| WorkspaceError::Storage {
            operation: "inspectWorkspaceSnapshot",
            path: path.clone(),
            message: error.to_string(),
        })?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(WorkspaceError::InvalidManifest(format!("snapshot is not a regular file: {path:?}")));
        }
        let bytes =
            fs::read(&path).map_err(|error| WorkspaceError::Storage { operation: "readWorkspaceSnapshot", path, message: error.to_string() })?;
        let snapshot: WorkspaceContentSnapshot =
            serde_json::from_slice(&bytes).map_err(|error| WorkspaceError::InvalidManifest(format!("invalid Workspace snapshot: {error}")))?;
        validate_snapshot(&snapshot, snapshot_id)?;
        Ok(snapshot)
    }

    pub(crate) fn restore(&self, snapshot_id: &str, expected_revision: u64) -> WorkspaceResult<WorkspaceApplyPatchResult> {
        let snapshot = self.read(snapshot_id)?;
        self.files.restore_files(&snapshot.files, expected_revision)
    }

    fn persist(&self, proposed: WorkspaceContentSnapshot) -> WorkspaceResult<WorkspaceContentSnapshot> {
        ensure_regular_directory(&self.layout.workspace_snapshots_dir())?;
        let path = self
            .layout
            .workspace_snapshots_dir()
            .join(format!("{}.json", proposed.snapshot_id));
        let bytes = serde_json::to_vec_pretty(&proposed).map_err(|error| WorkspaceError::Storage {
            operation: "serializeWorkspaceSnapshot",
            path: path.clone(),
            message: error.to_string(),
        })?;
        if write_bytes_create_once(&path, &bytes)? {
            return Ok(proposed);
        }
        let existing = self.read(&proposed.snapshot_id)?;
        if same_snapshot(&existing, &proposed) {
            Ok(existing)
        } else {
            Err(WorkspaceError::InvalidManifest(format!("snapshot identity collision: snapshotId={}", proposed.snapshot_id)))
        }
    }
}

fn validate_snapshot(snapshot: &WorkspaceContentSnapshot, expected_id: &str) -> WorkspaceResult<()> {
    if snapshot.schema_version != WORKSPACE_SNAPSHOT_SCHEMA || snapshot.snapshot_id != expected_id {
        return Err(WorkspaceError::InvalidManifest("Workspace snapshot identity mismatch".to_string()));
    }
    for file in &snapshot.files {
        if file.byte_size != u64::try_from(file.content.len()).unwrap_or(u64::MAX) || file.content_sha256 != content_sha256(file.content.as_bytes()) {
            return Err(WorkspaceError::InvalidManifest(format!("Workspace snapshot content mismatch: path={}", file.path)));
        }
    }
    Ok(())
}

fn same_snapshot(left: &WorkspaceContentSnapshot, right: &WorkspaceContentSnapshot) -> bool {
    left.schema_version == right.schema_version
        && left.snapshot_id == right.snapshot_id
        && left.run_id == right.run_id
        && left.reason == right.reason
        && left.workspace_revision == right.workspace_revision
        && left.files == right.files
}

fn ensure_regular_directory(path: &std::path::Path) -> WorkspaceResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => return Ok(()),
        Ok(_) => return Err(WorkspaceError::InvalidManifest(format!("snapshot path is not a regular directory: {path:?}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(WorkspaceError::Storage {
                operation: "inspectWorkspaceSnapshotDirectory",
                path: path.to_path_buf(),
                message: error.to_string(),
            })
        }
    }
    fs::create_dir(path).map_err(|error| WorkspaceError::Storage {
        operation: "createWorkspaceSnapshotDirectory",
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::backend::ai_driven::ai_task_execution::workspace::{WorkspaceFileService, WorkspaceManager};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn restores_deleted_directory_as_a_new_revision() {
        let root =
            std::env::temp_dir().join(format!("datavisool-workspace-snapshot-{}-{}", std::process::id(), TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)));
        let workspace = WorkspaceManager::new(&root).create("project-1").unwrap();
        let limits = WorkspaceFileLimits::default();
        let files = WorkspaceFileService::new(workspace.layout.clone(), limits.clone());
        let added = files
            .apply_patch(
                "*** Begin Patch\n*** Add File: drafts/a.txt\n+alpha\n*** Add File: drafts/b.txt\n+beta\n*** End Patch",
                workspace.manifest.workspace_revision,
            )
            .unwrap();
        let snapshots = WorkspaceSnapshotStore::new(&workspace, limits);
        let snapshot = snapshots
            .capture("run-1", WorkspaceSnapshotReason::TurnStart, added.workspace_revision)
            .unwrap();
        assert_eq!(snapshot.schema_version, "datavisool-workspace-snapshot/v1");
        let mut old_snapshot = snapshot.clone();
        old_snapshot.schema_version = "lcp-workspace-snapshot/v1".to_string();
        assert!(validate_snapshot(&old_snapshot, &snapshot.snapshot_id).is_err());
        let deleted = files.delete_path("drafts", added.workspace_revision).unwrap();
        assert!(files.read("drafts/a.txt", 1, 1).is_err());
        let restored = snapshots.restore(&snapshot.snapshot_id, deleted.workspace_revision).unwrap();

        assert!(restored.workspace_revision > deleted.workspace_revision);
        assert_eq!(files.read("drafts/a.txt", 1, 1).unwrap().content, "alpha");
        assert_eq!(files.read("drafts/b.txt", 1, 1).unwrap().content, "beta");
        let _ = fs::remove_dir_all(root);
    }
}
