/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Project 级唯一 Workspace 解析。

use std::path::{Path, PathBuf};

use crate::backend::ai_driven::ai_task_execution::workspace::{WorkspaceHandle, WorkspaceManager};

/// 打开或创建当前项目唯一的持久 Workspace。
pub(super) fn open_project_workspace(storage_root: &Path, project_id: i64) -> Result<WorkspaceHandle, String> {
    if project_id <= 0 {
        return Err("projectId 必须为正整数".to_string());
    }
    let workspace_id = format!("project-{project_id}");
    WorkspaceManager::new(projects_root(storage_root))
        .open_or_create(&workspace_id)
        .map_err(|error| error.to_string())
}

/// 输入快照保留 Turn 标识，便于宿主追溯。
pub(super) fn turn_input_snapshot_name(turn_id: &str) -> String {
    format!("request-{turn_id}")
}

fn projects_root(storage_root: &Path) -> PathBuf {
    storage_root.join("agent").join("projects")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::backend::ai_driven::ai_task_execution::workspace::{ArtifactContentFormat, ArtifactId, ArtifactStore};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "datavisool-project-workspace-{name}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn same_project_reuses_workspace_across_threads_and_turns() {
        let root = root("reuse");
        let first = open_project_workspace(&root, 1).unwrap();
        let result_id = ArtifactId::new("result").unwrap();
        ArtifactStore::new(&first)
            .create(result_id.clone(), ArtifactContentFormat::Json, r#"{"value":1}"#)
            .unwrap();

        let second = open_project_workspace(&root, 1).unwrap();
        let inherited = ArtifactStore::new(&second).read(&result_id).unwrap();

        assert_eq!(first.layout.workspace_dir(), second.layout.workspace_dir());
        assert_eq!(inherited.content, r#"{"value":1}"#);
        assert!(root.join("agent/projects/project-1/workspace/INDEX.json").is_file());
        assert!(root.join("agent/projects/project-1/runtime/manifest.json").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn different_projects_are_isolated() {
        let root = root("isolation");
        let first = open_project_workspace(&root, 1).unwrap();
        let second = open_project_workspace(&root, 2).unwrap();
        assert_ne!(first.layout.workspace_dir(), second.layout.workspace_dir());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_invalid_project_id() {
        let error = open_project_workspace(&root("invalid"), 0).unwrap_err();
        assert!(error.contains("正整数"));
    }
}
