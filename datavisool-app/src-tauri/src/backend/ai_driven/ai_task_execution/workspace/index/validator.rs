/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! INDEX.json 的宿主读取和真实文件一致性校验。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::schema::{WorkspaceIndex, WORKSPACE_INDEX_SCHEMA_VERSION};
use crate::backend::ai_driven::ai_task_execution::workspace::{WorkspaceError, WorkspaceRelativePath, WorkspaceResult, WORKSPACE_TEMP_MARKER};

pub(crate) const WORKSPACE_INDEX_FILE: &str = "INDEX.json";
const MAX_INDEX_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceIndexValidationMode {
    ReferencesOnly,
    Terminal,
}

pub(crate) fn serialize_initial_index() -> WorkspaceResult<Vec<u8>> {
    serde_json::to_vec_pretty(&WorkspaceIndex::default())
        .map_err(|error| WorkspaceError::InvalidIndex(format!("cannot serialize initial index: {error}")))
}

pub(crate) fn read_and_validate_index(workspace_root: &Path, mode: WorkspaceIndexValidationMode) -> WorkspaceResult<WorkspaceIndex> {
    let index_path = workspace_root.join(WORKSPACE_INDEX_FILE);
    let metadata =
        fs::symlink_metadata(&index_path).map_err(|error| WorkspaceError::InvalidIndex(format!("INDEX.json cannot be inspected: {error}")))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(WorkspaceError::InvalidIndex("INDEX.json must be a regular file".to_string()));
    }
    if metadata.len() > MAX_INDEX_BYTES {
        return Err(WorkspaceError::ResourceLimit(format!("INDEX.json exceeds {MAX_INDEX_BYTES} bytes")));
    }
    let bytes = fs::read(&index_path).map_err(|error| WorkspaceError::Storage {
        operation: "readWorkspaceIndex",
        path: index_path,
        message: error.to_string(),
    })?;
    let index: WorkspaceIndex =
        serde_json::from_slice(&bytes).map_err(|error| WorkspaceError::InvalidIndex(format!("INDEX.json is invalid JSON: {error}")))?;
    validate_index(workspace_root, &index, mode)?;
    Ok(index)
}

pub(crate) fn validate_index(workspace_root: &Path, index: &WorkspaceIndex, mode: WorkspaceIndexValidationMode) -> WorkspaceResult<()> {
    let workspace_paths = scan_workspace_files(workspace_root)?.into_iter().collect::<HashSet<_>>();
    validate_index_against_paths(index, &workspace_paths, mode)?;
    for entry in &index.entries {
        ensure_regular_workspace_file(workspace_root, &entry.path)?;
    }
    Ok(())
}

pub(crate) fn validate_index_against_paths(
    index: &WorkspaceIndex,
    workspace_paths: &HashSet<WorkspaceRelativePath>,
    mode: WorkspaceIndexValidationMode,
) -> WorkspaceResult<()> {
    if index.schema_version != WORKSPACE_INDEX_SCHEMA_VERSION {
        return Err(WorkspaceError::InvalidIndex(format!(
            "unsupported schemaVersion {}, expected {WORKSPACE_INDEX_SCHEMA_VERSION}",
            index.schema_version
        )));
    }
    let mut entry_paths = HashSet::with_capacity(index.entries.len());
    for entry in &index.entries {
        validate_summary("entry summary", &entry.summary)?;
        validate_text("updatedByTurnId", &entry.updated_by_turn_id, 160)?;
        if entry.path.as_str() == WORKSPACE_INDEX_FILE {
            return Err(WorkspaceError::InvalidIndex("INDEX.json must not index itself".to_string()));
        }
        if !entry_paths.insert(entry.path.clone()) {
            return Err(WorkspaceError::InvalidIndex(format!("duplicate entry path: {}", entry.path)));
        }
        if !workspace_paths.contains(&entry.path) {
            return Err(WorkspaceError::InvalidIndex(format!("indexed path does not exist: {}", entry.path)));
        }
    }

    let mut turn_ids = HashSet::with_capacity(index.turns.len());
    for turn in &index.turns {
        validate_text("turnId", &turn.turn_id, 160)?;
        validate_summary("turn requirement", &turn.requirement)?;
        if !turn_ids.insert(turn.turn_id.as_str()) {
            return Err(WorkspaceError::InvalidIndex(format!("duplicate turnId: {}", turn.turn_id)));
        }
        validate_turn_paths("inputs", &turn.inputs, &entry_paths)?;
        validate_turn_paths("outputs", &turn.outputs, &entry_paths)?;
    }

    if mode == WorkspaceIndexValidationMode::Terminal {
        for path in workspace_paths {
            if path.as_str() != WORKSPACE_INDEX_FILE && !entry_paths.contains(&path) {
                return Err(WorkspaceError::InvalidIndex(format!("persistent file is not registered in entries: {path}")));
            }
        }
    }
    Ok(())
}

pub(crate) fn ensure_regular_workspace_file(root: &Path, relative: &WorkspaceRelativePath) -> WorkspaceResult<()> {
    let mut current = root.to_path_buf();
    let components = relative.as_str().split('/').collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| WorkspaceError::InvalidIndex(format!("indexed path cannot be inspected: path={relative}, error={error}")))?;
        if metadata.file_type().is_symlink() {
            return Err(WorkspaceError::InvalidIndex(format!("indexed path contains a symbolic link: {relative}")));
        }
        let is_last = index + 1 == components.len();
        if (is_last && !metadata.file_type().is_file()) || (!is_last && !metadata.file_type().is_dir()) {
            return Err(WorkspaceError::InvalidIndex(format!("indexed path is not a regular file: {relative}")));
        }
    }
    Ok(())
}

pub(crate) fn scan_workspace_files(root: &Path) -> WorkspaceResult<Vec<WorkspaceRelativePath>> {
    let root_metadata = fs::symlink_metadata(root).map_err(|error| WorkspaceError::Storage {
        operation: "inspectWorkspaceRoot",
        path: root.to_path_buf(),
        message: error.to_string(),
    })?;
    if !root_metadata.file_type().is_dir() || root_metadata.file_type().is_symlink() {
        return Err(WorkspaceError::InvalidManifest("workspace root must be a regular directory".to_string()));
    }
    let mut files = Vec::new();
    scan_directory(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn scan_directory(root: &Path, directory: &Path, files: &mut Vec<WorkspaceRelativePath>) -> WorkspaceResult<()> {
    let entries = fs::read_dir(directory).map_err(|error| WorkspaceError::Storage {
        operation: "listWorkspaceDirectory",
        path: directory.to_path_buf(),
        message: error.to_string(),
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| WorkspaceError::Storage {
            operation: "readWorkspaceDirectoryEntry",
            path: directory.to_path_buf(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| WorkspaceError::Storage {
            operation: "inspectWorkspaceEntry",
            path: path.clone(),
            message: error.to_string(),
        })?;
        if metadata.file_type().is_symlink() {
            return Err(WorkspaceError::PathOutOfScope(logical_path(root, &path)?));
        }
        if metadata.file_type().is_dir() {
            scan_directory(root, &path, files)?;
        } else if metadata.file_type().is_file() {
            if is_ignored_workspace_file(&path) {
                continue;
            }
            files.push(WorkspaceRelativePath::new(logical_path(root, &path)?)?);
        } else {
            return Err(WorkspaceError::PathOutOfScope(logical_path(root, &path)?));
        }
    }
    Ok(())
}

/// Finder、Windows Explorer 和 Workspace 原子写入产生的文件不属于 Agent 内容。
fn is_ignored_workspace_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(name, ".DS_Store" | "Thumbs.db" | "desktop.ini")
        || name.starts_with("._")
        || (name.starts_with('.') && name.contains(WORKSPACE_TEMP_MARKER))
}

fn logical_path(root: &Path, path: &Path) -> WorkspaceResult<String> {
    path.strip_prefix(root)
        .map_err(|_| WorkspaceError::PathOutOfScope(path.display().to_string()))?
        .to_str()
        .map(|value| value.replace(std::path::MAIN_SEPARATOR, "/"))
        .ok_or_else(|| WorkspaceError::PathOutOfScope(path.display().to_string()))
}

fn validate_turn_paths(field: &str, paths: &[WorkspaceRelativePath], entry_paths: &HashSet<WorkspaceRelativePath>) -> WorkspaceResult<()> {
    let mut unique = HashSet::with_capacity(paths.len());
    for path in paths {
        if !unique.insert(path) {
            return Err(WorkspaceError::InvalidIndex(format!("duplicate {field} path: {path}")));
        }
        if !entry_paths.contains(path) {
            return Err(WorkspaceError::InvalidIndex(format!("{field} path is not registered in entries: {path}")));
        }
    }
    Ok(())
}

fn validate_summary(field: &str, value: &str) -> WorkspaceResult<()> {
    validate_text(field, value, 4000)
}

fn validate_text(field: &str, value: &str, max_len: usize) -> WorkspaceResult<()> {
    if value.trim().is_empty() || value.len() > max_len {
        return Err(WorkspaceError::InvalidIndex(format!("{field} must be non-blank and at most {max_len} bytes")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::ai_task_execution::workspace::index::schema::{
        WorkspaceIndexEntry, WorkspaceIndexRole, WorkspaceIndexTurn, WorkspaceIndexTurnStatus,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("datavisool-workspace-index-{name}-{}-{}", std::process::id(), TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)))
    }

    fn valid_index() -> WorkspaceIndex {
        WorkspaceIndex {
            schema_version: WORKSPACE_INDEX_SCHEMA_VERSION.to_string(),
            project_summary: "summary".to_string(),
            entries: vec![WorkspaceIndexEntry {
                path: WorkspaceRelativePath::new("docs/overview.md").unwrap(),
                role: WorkspaceIndexRole::Reference,
                summary: "overview".to_string(),
                updated_by_turn_id: "turn-1".to_string(),
            }],
            turns: vec![WorkspaceIndexTurn {
                turn_id: "turn-1".to_string(),
                requirement: "write overview".to_string(),
                inputs: vec![],
                outputs: vec![WorkspaceRelativePath::new("docs/overview.md").unwrap()],
                status: WorkspaceIndexTurnStatus::Completed,
            }],
        }
    }

    #[test]
    fn validates_index_against_real_files() {
        let root = root("valid");
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("INDEX.json"), serialize_initial_index().unwrap()).unwrap();
        fs::write(root.join("docs/overview.md"), "overview").unwrap();
        let initial: WorkspaceIndex = serde_json::from_slice(&serialize_initial_index().unwrap()).unwrap();
        assert_eq!(initial.schema_version, "datavisool-workspace-index/v1");
        validate_index(&root, &valid_index(), WorkspaceIndexValidationMode::Terminal).unwrap();
        let mut old_index = valid_index();
        old_index.schema_version = "lcp-workspace-index/v1".to_string();
        assert!(validate_index(&root, &old_index, WorkspaceIndexValidationMode::Terminal).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_validation_rejects_unindexed_files() {
        let root = root("unindexed");
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("INDEX.json"), serialize_initial_index().unwrap()).unwrap();
        fs::write(root.join("docs/overview.md"), "overview").unwrap();
        fs::write(root.join("orphan.md"), "orphan").unwrap();
        let error = validate_index(&root, &valid_index(), WorkspaceIndexValidationMode::Terminal).unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIndex(_)));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_links_in_index_and_scan() {
        use std::os::unix::fs::symlink;

        let root = root("symlink");
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("INDEX.json"), serialize_initial_index().unwrap()).unwrap();
        let outside = root.with_extension("outside");
        fs::write(&outside, "outside").unwrap();
        symlink(&outside, root.join("docs/overview.md")).unwrap();
        assert!(validate_index(&root, &valid_index(), WorkspaceIndexValidationMode::ReferencesOnly).is_err());
        assert!(scan_workspace_files(&root).is_err());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_file(outside);
    }

    #[test]
    fn scan_ignores_host_metadata_and_workspace_temporary_files() {
        let root = root("ignored-host-files");
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("INDEX.json"), serialize_initial_index().unwrap()).unwrap();
        fs::write(root.join("docs/overview.md"), "overview").unwrap();
        fs::write(root.join(".DS_Store"), [0_u8; 32]).unwrap();
        fs::write(root.join("docs/._overview.md"), [0_u8; 16]).unwrap();
        fs::write(root.join("Thumbs.db"), [0_u8; 8]).unwrap();
        fs::write(root.join("desktop.ini"), "metadata").unwrap();
        fs::write(root.join("docs/.overview.md.datavisool-tmp-1-1"), "temporary").unwrap();

        let paths = scan_workspace_files(&root).unwrap();
        assert_eq!(
            paths,
            vec![WorkspaceRelativePath::new("INDEX.json").unwrap(), WorkspaceRelativePath::new("docs/overview.md").unwrap()]
        );
        let _ = fs::remove_dir_all(root);
    }
}
