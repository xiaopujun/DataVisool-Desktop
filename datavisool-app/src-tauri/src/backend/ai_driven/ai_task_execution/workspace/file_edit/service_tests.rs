/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{WorkspaceFileLimits, WorkspaceFileService};
use crate::backend::ai_driven::ai_task_execution::workspace::{
    read_and_validate_index, WorkspaceError, WorkspaceIndexValidationMode, WorkspaceManager,
};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "datavisool-workspace-file-service-{name}-{}-{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn applies_multi_file_patch_and_exposes_list_and_read() {
    let root = root("lifecycle");
    let workspace = WorkspaceManager::new(&root).create("project-1").unwrap();
    let service = WorkspaceFileService::new(workspace.layout.clone(), WorkspaceFileLimits::default());
    let patch = r#"*** Begin Patch
*** Add File: docs/a.md
+hello
*** Update File: INDEX.json
@@
-  "projectSummary": "",
-  "entries": [],
+  "projectSummary": "project",
+  "entries": [
+    {
+      "path": "docs/a.md",
+      "role": "draft",
+      "summary": "draft",
+      "updatedByTurnId": "turn-1"
+    }
+  ],
*** End Patch"#;

    let result = service.apply_patch(patch, 0).unwrap();
    assert_eq!(result.workspace_revision, 1);
    assert_eq!(result.added[0].as_str(), "docs/a.md");
    read_and_validate_index(workspace.layout.workspace_dir(), WorkspaceIndexValidationMode::Terminal).unwrap();

    let listed = service.list(None, 3).unwrap();
    assert!(listed.iter().any(|entry| entry.path == "docs"));
    assert!(listed
        .iter()
        .any(|entry| entry.path == "docs/a.md" && entry.modified_revision == 1));
    let excerpt = service.read("docs/a.md", 1, 10).unwrap();
    assert_eq!(excerpt.content, "hello");
    assert_eq!(excerpt.workspace_revision, 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejects_invalid_json_and_allows_artifact_paths_without_partial_write() {
    let root = root("preflight");
    let workspace = WorkspaceManager::new(&root).create("project-1").unwrap();
    let service = WorkspaceFileService::new(workspace.layout.clone(), WorkspaceFileLimits::default());

    let invalid_json = "*** Begin Patch\n*** Add File: bad.json\n+{bad}\n*** End Patch";
    assert!(matches!(service.apply_patch(invalid_json, 0), Err(WorkspaceError::InvalidFilePatch(_))));
    assert!(!workspace.layout.workspace_dir().join("bad.json").exists());

    let artifact = "*** Begin Patch\n*** Add File: artifacts/result.json\n+{}\n*** End Patch";
    assert_eq!(service.apply_patch(artifact, 0).unwrap().workspace_revision, 1);
    let reopened = WorkspaceManager::new(&root).open("project-1").unwrap();
    assert_eq!(reopened.manifest.workspace_revision, 1);
    assert!(reopened
        .manifest
        .artifacts
        .iter()
        .any(|artifact| artifact.artifact_id.name() == "result"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn index_is_an_optional_movable_and_deletable_file() {
    let root = root("index");
    let workspace = WorkspaceManager::new(&root).create("project-1").unwrap();
    let service = WorkspaceFileService::new(workspace.layout.clone(), WorkspaceFileLimits::default());
    let update = "*** Begin Patch\n*** Update File: INDEX.json\n@@\n-  \"projectSummary\": \"\",\n+  \"projectSummary\": \"updated\",\n*** End Patch";
    assert_eq!(service.apply_patch(update, 0).unwrap().workspace_revision, 1);

    let move_index = "*** Begin Patch\n*** Update File: INDEX.json\n@@\n-  \"projectSummary\": \"updated\",\n+  \"projectSummary\": \"moved\",\n*** Move to: OTHER.json\n*** End Patch";
    assert_eq!(service.apply_patch(move_index, 1).unwrap().workspace_revision, 2);
    let delete = "*** Begin Patch\n*** Delete File: OTHER.json\n*** End Patch";
    assert_eq!(service.apply_patch(delete, 2).unwrap().workspace_revision, 3);
    assert_eq!(
        WorkspaceManager::new(&root)
            .open("project-1")
            .unwrap()
            .manifest
            .workspace_revision,
        3
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn moves_and_deletes_directory_trees_in_single_revisions() {
    let root = root("directory-operations");
    let workspace = WorkspaceManager::new(&root).create("project-1").unwrap();
    let service = WorkspaceFileService::new(workspace.layout.clone(), WorkspaceFileLimits::default());
    let added = service
        .apply_patch("*** Begin Patch\n*** Add File: source/a.txt\n+a\n*** Add File: source/nested/b.txt\n+b\n*** End Patch", 0)
        .unwrap();
    let moved = service.move_path("source", "target", added.workspace_revision).unwrap();

    assert_eq!(moved.workspace_revision, added.workspace_revision + 1);
    assert_eq!(moved.moved.len(), 2);
    assert!(service.read("source/a.txt", 1, 1).is_err());
    assert_eq!(service.read("target/nested/b.txt", 1, 1).unwrap().content, "b");

    let deleted = service.delete_path("target", moved.workspace_revision).unwrap();
    assert_eq!(deleted.workspace_revision, moved.workspace_revision + 1);
    assert_eq!(deleted.deleted.len(), 2);
    assert!(service.list(Some("target"), 3).unwrap().is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejects_moving_a_file_onto_its_ancestor_directory() {
    let root = root("ancestor-target");
    let workspace = WorkspaceManager::new(&root).create("project-1").unwrap();
    let service = WorkspaceFileService::new(workspace.layout.clone(), WorkspaceFileLimits::default());
    let added = service
        .apply_patch("*** Begin Patch\n*** Add File: drafts/result.json\n+{}\n*** End Patch", workspace.manifest.workspace_revision)
        .unwrap();

    let error = service
        .move_path("drafts/result.json", "drafts", added.workspace_revision)
        .expect_err("an ancestor directory cannot be a file target");

    assert!(error.to_string().contains("ancestor"));
    assert_eq!(service.read("drafts/result.json", 1, 1).unwrap().content, "{}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn generic_file_moves_keep_artifact_metadata_consistent() {
    let root = root("artifact-move");
    let workspace = WorkspaceManager::new(&root).create("project-1").unwrap();
    let service = WorkspaceFileService::new(workspace.layout.clone(), WorkspaceFileLimits::default());
    let added = service
        .apply_patch("*** Begin Patch\n*** Add File: artifacts/result.json\n+{}\n*** End Patch", 0)
        .unwrap();
    let moved = service
        .move_path("artifacts/result.json", "drafts/result.json", added.workspace_revision)
        .unwrap();
    let manifest = WorkspaceManager::new(&root).open("project-1").unwrap().manifest;
    assert!(manifest.artifacts.is_empty());

    service
        .move_path("drafts/result.json", "artifacts/final.json", moved.workspace_revision)
        .unwrap();
    let manifest = WorkspaceManager::new(&root).open("project-1").unwrap().manifest;
    assert_eq!(manifest.artifacts.len(), 1);
    assert_eq!(manifest.artifacts[0].artifact_id.name(), "final");
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn rejects_hard_links_before_reading_workspace_content() {
    let root = root("hard-link");
    let workspace = WorkspaceManager::new(&root).create("project-1").unwrap();
    let service = WorkspaceFileService::new(workspace.layout.clone(), WorkspaceFileLimits::default());
    let index = workspace.layout.index_file();
    let outside = root.join("outside.json");
    fs::write(&outside, fs::read(&index).unwrap()).unwrap();
    fs::remove_file(&index).unwrap();
    fs::hard_link(&outside, &index).unwrap();

    assert!(matches!(service.read("INDEX.json", 1, 1), Err(WorkspaceError::InvalidManifest(_))));
    let _ = fs::remove_dir_all(root);
}
