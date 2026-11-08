/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceManager;

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn test_workspace(name: &str) -> (PathBuf, WorkspaceManager, WorkspaceHandle, ArtifactStore, ArtifactId) {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("datavisool-artifact-commit-recovery-{name}-{}-{sequence}", std::process::id()));
    let manager = WorkspaceManager::new(&root);
    let workspace = manager.create("workspace-1").expect("workspace should be created");
    let store = ArtifactStore::new(&workspace);
    let artifact_id = ArtifactId::new("result").expect("artifactId should be valid");
    store
        .create(artifact_id.clone(), ArtifactContentFormat::Json, r#"{"revision":1}"#)
        .expect("artifact should be created");
    (root, manager, workspace, store, artifact_id)
}

fn prepare_journal(workspace: &WorkspaceHandle, artifact_id: &ArtifactId, new_content: &str) -> ArtifactCommitJournal {
    let old_manifest = read_manifest(&workspace.layout).expect("old manifest should be readable");
    let old_content = fs::read_to_string(workspace.layout.artifact_file(artifact_id)).expect("old artifact should be readable");
    prepare_artifact_commit_journal(&workspace.layout, artifact_id, old_manifest, old_content, new_content.to_string(), now_ms().saturating_add(1))
        .expect("journal should be prepared")
}

fn stage_interrupted_commit(workspace: &WorkspaceHandle, artifact_id: &ArtifactId, new_content: &str) -> ArtifactCommitJournal {
    let journal = prepare_journal(workspace, artifact_id, new_content);
    write_artifact_commit_journal(&workspace.layout, &journal).expect("journal should be durable");
    write_bytes_atomically(&workspace.layout.artifact_file(artifact_id), new_content.as_bytes()).expect("new artifact should be published");
    journal
}

#[test]
fn artifact_commit_recovery_completes_new_state_and_is_idempotent() {
    let (root, _manager, workspace, store, artifact_id) = test_workspace("forward");
    let new_content = r#"{"revision":2}"#;
    stage_interrupted_commit(&workspace, &artifact_id, new_content);

    assert_eq!(
        find_metadata(&read_manifest(&workspace.layout).unwrap().artifacts, &artifact_id)
            .unwrap()
            .revision,
        1
    );
    let journal_path = artifact_commit_journal_path(&workspace.layout);
    assert!(journal_path.is_file());
    drop(store);
    drop(workspace);

    let manager = WorkspaceManager::new(&root);
    let reopened = manager.open("workspace-1").expect("open should recover interrupted commit");
    let snapshot = ArtifactStore::new(&reopened).read(&artifact_id).unwrap();
    assert_eq!(snapshot.metadata.revision, 2);
    assert_eq!(snapshot.content, new_content);
    assert!(!journal_path.exists());

    let reopened_again = manager.open("workspace-1").expect("repeated recovery should be a no-op");
    let replaced = ArtifactStore::new(&reopened_again)
        .replace(&artifact_id, 2, r#"{"revision":3}"#)
        .expect("normal commit should continue from recovered revision");
    assert_eq!(replaced.metadata.revision, 3);
    assert!(!journal_path.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn artifact_create_recovery_completes_file_and_manifest_together() {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("datavisool-artifact-create-recovery-{}-{sequence}", std::process::id()));
    let manager = WorkspaceManager::new(&root);
    let workspace = manager.create("workspace-1").unwrap();
    let artifact_id = ArtifactId::new("result").unwrap();
    let content = r#"{"value":1}"#.to_string();
    let old_manifest = read_manifest(&workspace.layout).unwrap();
    let timestamp = now_ms();
    let metadata = ArtifactMetadata {
        artifact_id: artifact_id.clone(),
        revision: 1,
        content_format: ArtifactContentFormat::Json,
        content_sha256: content_sha256(content.as_bytes()),
        status: ArtifactStatus::Draft,
        created_at: timestamp,
        updated_at: timestamp,
    };
    let mut new_manifest = old_manifest.clone();
    new_manifest.workspace_revision = next_workspace_revision(&new_manifest).unwrap();
    new_manifest.updated_at = timestamp;
    let workspace_revision = new_manifest.workspace_revision;
    upsert_file_metadata(&mut new_manifest, artifact_workspace_path(&artifact_id).unwrap(), content.as_bytes(), workspace_revision);
    new_manifest.artifacts.push(metadata);
    let journal = ArtifactCreateJournal {
        schema_version: ARTIFACT_COMMIT_JOURNAL_SCHEMA_VERSION,
        workspace_id: workspace.layout.workspace_id().to_string(),
        artifact_id: artifact_id.clone(),
        old_manifest,
        new_manifest,
        new_content: content.clone(),
    };
    validate_artifact_create_journal(&workspace.layout, &journal).unwrap();
    write_artifact_create_journal(&workspace.layout, &journal).unwrap();
    write_bytes_atomically(&workspace.layout.artifact_file(&artifact_id), content.as_bytes()).unwrap();

    let reopened = manager.open("workspace-1").expect("open should forward the interrupted create");
    let snapshot = ArtifactStore::new(&reopened).read(&artifact_id).unwrap();
    assert_eq!(snapshot.content, content);
    assert_eq!(snapshot.metadata.revision, 1);
    assert!(!artifact_create_journal_path(&workspace.layout).exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn artifact_commit_recovery_keeps_old_state_when_only_journal_was_written() {
    let (root, _manager, workspace, store, artifact_id) = test_workspace("keep-old");
    let journal = prepare_journal(&workspace, &artifact_id, r#"{"revision":2}"#);
    write_artifact_commit_journal(&workspace.layout, &journal).expect("journal should be durable");

    let snapshot = store
        .read(&artifact_id)
        .expect("read should recover and keep the complete old state");
    assert_eq!(snapshot.metadata.revision, 1);
    assert_eq!(snapshot.content, r#"{"revision":1}"#);
    assert!(!artifact_commit_journal_path(&workspace.layout).exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn artifact_commit_recovery_accepts_already_completed_state() {
    let (root, manager, workspace, _store, artifact_id) = test_workspace("already-new");
    let journal = stage_interrupted_commit(&workspace, &artifact_id, r#"{"revision":2}"#);
    write_manifest(&workspace.layout, &journal.new_manifest).expect("new manifest should be published");

    let reopened = manager
        .open("workspace-1")
        .expect("open should only clean the completed journal");
    let snapshot = ArtifactStore::new(&reopened).read(&artifact_id).unwrap();
    assert_eq!(snapshot.metadata.revision, 2);
    assert_eq!(snapshot.content, r#"{"revision":2}"#);
    assert!(!artifact_commit_journal_path(&workspace.layout).exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn artifact_commit_recovery_rolls_back_indistinguishable_same_content_then_allows_replace() {
    let (root, manager, workspace, _store, artifact_id) = test_workspace("same-content");
    let unchanged_content = r#"{"revision":1}"#;
    stage_interrupted_commit(&workspace, &artifact_id, unchanged_content);

    let reopened = manager
        .open("workspace-1")
        .expect("same-content crash should preserve old state");
    let store = ArtifactStore::new(&reopened);
    let recovered = store.read(&artifact_id).unwrap();
    assert_eq!(recovered.metadata.revision, 1);
    assert_eq!(recovered.content, unchanged_content);
    assert!(!artifact_commit_journal_path(&workspace.layout).exists());

    let replaced = store
        .replace(&artifact_id, 1, unchanged_content)
        .expect("normal same-content replace should commit a revision");
    assert_eq!(replaced.metadata.revision, 2);
    assert!(!artifact_commit_journal_path(&workspace.layout).exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn status_update_recovers_pending_content_before_revision_check() {
    let (root, _manager, workspace, store, artifact_id) = test_workspace("status-after-recovery");
    stage_interrupted_commit(&workspace, &artifact_id, r#"{"revision":2}"#);

    let error = store
        .update_status(&artifact_id, 1, ArtifactStatus::Valid)
        .expect_err("old revision must be rejected after recovery");
    assert!(matches!(error, WorkspaceError::RevisionConflict { actual: 2, .. }));
    assert!(!artifact_commit_journal_path(&workspace.layout).exists());

    let metadata = store
        .update_status(&artifact_id, 2, ArtifactStatus::Valid)
        .expect("status update should continue from recovered revision");
    assert_eq!(metadata.revision, 3);
    assert_eq!(metadata.status, ArtifactStatus::Valid);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn artifact_commit_journal_does_not_overwrite_residual_transaction() {
    let (root, _manager, workspace, _store, artifact_id) = test_workspace("residual-journal");
    let first = prepare_journal(&workspace, &artifact_id, r#"{"revision":2}"#);
    write_artifact_commit_journal(&workspace.layout, &first).expect("first journal should be durable");
    let first_bytes = fs::read(artifact_commit_journal_path(&workspace.layout)).unwrap();

    let second = prepare_journal(&workspace, &artifact_id, r#"{"revision":200}"#);
    let error = write_artifact_commit_journal(&workspace.layout, &second).expect_err("residual journal must not be overwritten");
    assert!(matches!(error, WorkspaceError::InvalidManifest(message) if message.contains("journal already exists")));
    assert_eq!(fs::read(artifact_commit_journal_path(&workspace.layout)).unwrap(), first_bytes);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn artifact_commit_recovery_rejects_corrupt_journal_without_deleting_it() {
    let (root, manager, workspace, _store, _artifact_id) = test_workspace("corrupt");
    let journal_path = artifact_commit_journal_path(&workspace.layout);
    write_bytes_atomically(&journal_path, br#"{"schemaVersion":1"#).expect("corrupt journal fixture should be written");

    let error = manager.open("workspace-1").expect_err("corrupt journal must block opening");
    assert!(matches!(error, WorkspaceError::InvalidManifest(message) if message.contains("artifact commit journal")));
    assert!(journal_path.is_file());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn artifact_commit_recovery_rejects_journal_symlink() {
    use std::os::unix::fs::symlink;

    let (root, manager, workspace, _store, _artifact_id) = test_workspace("symlink");
    let outside_path = root.join("outside-journal.json");
    fs::write(&outside_path, "{}").expect("outside journal should be written");
    let journal_path = artifact_commit_journal_path(&workspace.layout);
    symlink(&outside_path, &journal_path).expect("journal symlink should be created");

    let error = manager.open("workspace-1").expect_err("journal symlink must block opening");
    assert!(matches!(error, WorkspaceError::InvalidManifest(message) if message.contains("not a regular file")));
    assert!(journal_path.symlink_metadata().unwrap().file_type().is_symlink());
    let _ = fs::remove_dir_all(root);
}
