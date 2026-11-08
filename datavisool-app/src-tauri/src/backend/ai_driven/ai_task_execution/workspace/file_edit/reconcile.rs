/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 以真实 Workspace 文件树协调宿主 manifest。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;

use super::path::WorkspaceRoot;
use super::service::WorkspaceFileLimits;
use super::transaction::artifact_id_for_path;
use crate::backend::ai_driven::ai_task_execution::workspace::index::scan_workspace_files;
use crate::backend::ai_driven::ai_task_execution::workspace::manager::write_manifest;
use crate::backend::ai_driven::ai_task_execution::workspace::types::content_sha256;
use crate::backend::ai_driven::ai_task_execution::workspace::{
    ArtifactContentFormat, ArtifactId, ArtifactMetadata, ArtifactStatus, WorkspaceError, WorkspaceFileMetadata, WorkspaceLayout, WorkspaceManifest,
    WorkspaceRelativePath, WorkspaceResult,
};
use crate::backend::ai_driven::time::now_ms;

/// 在调用方持有 Workspace 共享锁时，把真实普通文件协调进 manifest。
///
/// 外部变化按一个批次只推进一次 Workspace revision；宿主元数据已由扫描器过滤。
pub(crate) fn reconcile_manifest_files(
    layout: &WorkspaceLayout,
    manifest: &mut WorkspaceManifest,
    limits: &WorkspaceFileLimits,
) -> WorkspaceResult<()> {
    let old_files = validated_file_map(manifest)?;
    validate_unique_artifacts(manifest)?;

    let paths = scan_workspace_files(layout.workspace_dir())?;
    if paths.len() > limits.max_workspace_files {
        return Err(WorkspaceError::ResourceLimit(format!("workspace exceeds {} files", limits.max_workspace_files)));
    }

    let root = WorkspaceRoot::open(layout.workspace_dir())?;
    let mut next_files = BTreeMap::new();
    let mut changed_paths = HashSet::new();
    let mut total_bytes = 0_u64;
    for path in paths {
        let absolute_path = path.join_to(layout.workspace_dir());
        let metadata = inspect_regular_file(&absolute_path, &path)?;
        if metadata.len() > limits.max_file_bytes {
            return Err(WorkspaceError::ResourceLimit(format!("file exceeds {} bytes: {path}", limits.max_file_bytes)));
        }
        let bytes = root.read_file(&path, limits.max_file_bytes)?;
        let final_metadata = inspect_regular_file(&absolute_path, &path)?;
        let byte_size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if final_metadata.len() != byte_size {
            return Err(WorkspaceError::Storage {
                operation: "reconcileWorkspaceFile",
                path: absolute_path,
                message: "file changed while manifest was being reconciled".to_string(),
            });
        }
        total_bytes = total_bytes.saturating_add(byte_size);
        if total_bytes > limits.max_workspace_bytes {
            return Err(WorkspaceError::ResourceLimit(format!("workspace exceeds {} bytes", limits.max_workspace_bytes)));
        }

        let hash = content_sha256(&bytes);
        let previous = old_files.get(&path);
        let changed = previous.is_none_or(|file| file.byte_size != byte_size || file.content_sha256 != hash);
        if changed {
            changed_paths.insert(path.clone());
        }
        next_files.insert(
            path.clone(),
            WorkspaceFileMetadata {
                path,
                byte_size,
                content_sha256: hash,
                modified_revision: previous.map_or(0, |file| file.modified_revision),
            },
        );
    }

    for path in old_files.keys() {
        if !next_files.contains_key(path) {
            changed_paths.insert(path.clone());
        }
    }

    let timestamp = now_ms().max(manifest.updated_at);
    let next_artifacts = reconcile_artifacts(manifest, &next_files, timestamp)?;
    if changed_paths.is_empty() && next_artifacts == manifest.artifacts {
        return Ok(());
    }

    let revision = manifest
        .workspace_revision
        .checked_add(1)
        .ok_or_else(|| WorkspaceError::InvalidManifest("workspace revision overflow".to_string()))?;
    for path in &changed_paths {
        if let Some(file) = next_files.get_mut(path) {
            file.modified_revision = revision;
        }
    }
    manifest.workspace_revision = revision;
    manifest.updated_at = timestamp;
    manifest.files = next_files.into_values().collect();
    manifest.artifacts = next_artifacts;
    write_manifest(layout, manifest)
}

fn validated_file_map(manifest: &WorkspaceManifest) -> WorkspaceResult<BTreeMap<WorkspaceRelativePath, WorkspaceFileMetadata>> {
    let mut files = BTreeMap::new();
    for file in &manifest.files {
        if file.modified_revision > manifest.workspace_revision {
            return Err(WorkspaceError::InvalidManifest(format!(
                "file revision exceeds workspace revision: path={}, modifiedRevision={}, workspaceRevision={}",
                file.path, file.modified_revision, manifest.workspace_revision
            )));
        }
        if files.insert(file.path.clone(), file.clone()).is_some() {
            return Err(WorkspaceError::InvalidManifest(format!("duplicate file metadata: path={}", file.path)));
        }
    }
    Ok(files)
}

fn validate_unique_artifacts(manifest: &WorkspaceManifest) -> WorkspaceResult<()> {
    let mut artifact_ids = HashSet::with_capacity(manifest.artifacts.len());
    for artifact in &manifest.artifacts {
        if !artifact_ids.insert(artifact.artifact_id.clone()) {
            return Err(WorkspaceError::InvalidManifest(format!("duplicate artifact metadata: artifactId={}", artifact.artifact_id)));
        }
    }
    Ok(())
}

fn reconcile_artifacts(
    manifest: &WorkspaceManifest,
    files: &BTreeMap<WorkspaceRelativePath, WorkspaceFileMetadata>,
    timestamp: i64,
) -> WorkspaceResult<Vec<ArtifactMetadata>> {
    let mut old_artifacts = manifest
        .artifacts
        .iter()
        .cloned()
        .map(|metadata| (metadata.artifact_id.clone(), metadata))
        .collect::<HashMap<_, _>>();
    let mut artifacts = Vec::new();
    for file in files.values() {
        let Some(artifact_id) = artifact_id_for_path(&file.path)? else {
            continue;
        };
        let metadata = match old_artifacts.remove(&artifact_id) {
            Some(mut metadata) if metadata.content_sha256 != file.content_sha256 => {
                metadata.revision = metadata
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| WorkspaceError::InvalidManifest(format!("artifact revision overflow: artifactId={artifact_id}")))?;
                metadata.content_sha256 = file.content_sha256.clone();
                metadata.status = ArtifactStatus::Draft;
                metadata.updated_at = timestamp;
                metadata
            }
            Some(metadata) => metadata,
            None => ArtifactMetadata {
                artifact_id,
                revision: 1,
                content_format: ArtifactContentFormat::Json,
                content_sha256: file.content_sha256.clone(),
                status: ArtifactStatus::Draft,
                created_at: timestamp,
                updated_at: timestamp,
            },
        };
        artifacts.push(metadata);
    }
    artifacts.sort_by(|left, right| left.artifact_id.name().cmp(right.artifact_id.name()));
    Ok(artifacts)
}

fn inspect_regular_file(path: &std::path::Path, logical_path: &WorkspaceRelativePath) -> WorkspaceResult<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).map_err(|error| WorkspaceError::Storage {
        operation: "inspectWorkspaceFileForReconciliation",
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(WorkspaceError::PathOutOfScope(logical_path.to_string()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(WorkspaceError::InvalidManifest(format!("workspace file must not be a hard link: path={logical_path}")));
        }
    }
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::backend::ai_driven::ai_task_execution::workspace::manager::read_manifest;
    use crate::backend::ai_driven::ai_task_execution::workspace::{WorkspaceFileService, WorkspaceManager};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "datavisool-workspace-reconcile-{name}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn ignores_host_metadata_without_changing_revision_or_capture() {
        let root = root("host-metadata");
        let workspace = WorkspaceManager::new(&root).create("workspace").unwrap();
        let service = WorkspaceFileService::new(workspace.layout.clone(), WorkspaceFileLimits::default());
        fs::write(workspace.layout.workspace_dir().join(".DS_Store"), [0_u8; 64]).unwrap();

        assert_eq!(service.workspace_revision().unwrap(), 0);
        let index = WorkspaceRelativePath::new("INDEX.json").unwrap();
        assert_eq!(service.capture(&[index], 0).unwrap().len(), 1);
        assert!(service.list(None, 2).unwrap().iter().all(|entry| entry.path != ".DS_Store"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reconciles_external_file_and_artifact_changes_as_single_revisions() {
        let root = root("external-changes");
        let workspace = WorkspaceManager::new(&root).create("workspace").unwrap();
        let service = WorkspaceFileService::new(workspace.layout.clone(), WorkspaceFileLimits::default());
        fs::create_dir_all(workspace.layout.workspace_dir().join("notes")).unwrap();
        fs::write(workspace.layout.workspace_dir().join("notes/context.md"), "first").unwrap();
        fs::create_dir_all(workspace.layout.workspace_dir().join("artifacts")).unwrap();
        fs::write(workspace.layout.workspace_dir().join("artifacts/result.json"), r#"{"value":1}"#).unwrap();

        assert_eq!(service.workspace_revision().unwrap(), 1);
        let first = read_manifest(&workspace.layout).unwrap();
        assert_eq!(first.files.iter().filter(|file| file.modified_revision == 1).count(), 2);
        assert_eq!(first.artifacts.len(), 1);
        assert_eq!(first.artifacts[0].revision, 1);

        fs::write(workspace.layout.workspace_dir().join("notes/context.md"), "second").unwrap();
        fs::write(workspace.layout.workspace_dir().join("artifacts/result.json"), r#"{"value":2}"#).unwrap();
        assert_eq!(service.workspace_revision().unwrap(), 2);
        let second = read_manifest(&workspace.layout).unwrap();
        assert_eq!(second.artifacts[0].revision, 2);
        assert_eq!(second.artifacts[0].status, ArtifactStatus::Draft);

        fs::remove_file(workspace.layout.workspace_dir().join("notes/context.md")).unwrap();
        fs::remove_file(workspace.layout.workspace_dir().join("artifacts/result.json")).unwrap();
        assert_eq!(service.workspace_revision().unwrap(), 3);
        let third = read_manifest(&workspace.layout).unwrap();
        assert_eq!(third.files.len(), 1);
        assert!(third.artifacts.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_external_files_over_the_configured_limit() {
        let root = root("file-limit");
        let workspace = WorkspaceManager::new(&root).create("workspace").unwrap();
        let limits = WorkspaceFileLimits { max_file_bytes: 1024, ..WorkspaceFileLimits::default() };
        let service = WorkspaceFileService::new(workspace.layout.clone(), limits);
        fs::write(workspace.layout.workspace_dir().join("oversized.txt"), vec![0_u8; 1025]).unwrap();

        assert!(matches!(service.workspace_revision(), Err(WorkspaceError::ResourceLimit(_))));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_external_hard_links() {
        let root = root("hard-link");
        let workspace = WorkspaceManager::new(&root).create("workspace").unwrap();
        let service = WorkspaceFileService::new(workspace.layout.clone(), WorkspaceFileLimits::default());
        let outside = root.with_extension("outside");
        fs::write(&outside, "outside").unwrap();
        fs::hard_link(&outside, workspace.layout.workspace_dir().join("linked.txt")).unwrap();

        assert!(matches!(service.workspace_revision(), Err(WorkspaceError::InvalidManifest(_))));
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_file(outside);
    }
}
