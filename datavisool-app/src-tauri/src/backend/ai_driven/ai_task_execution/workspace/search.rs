/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Workspace 根目录适配、相对子目录校验和搜索版本语义。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio::sync::watch;

use super::{WorkspaceFileService, WorkspaceLayout, WorkspaceRelativePath};
use crate::backend::ai_driven::ripgrep::{
    RipgrepError, RipgrepSearchMatch, RipgrepSearchOptions, RipgrepSearchService, SearchCorpus, SearchMatchKind, SearchTruncationReason,
};

#[derive(Clone)]
pub(crate) struct WorkspaceSearchService {
    layout: WorkspaceLayout,
    files: WorkspaceFileService,
    ripgrep: RipgrepSearchService,
    max_response_bytes: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceSearchRequest {
    pub path: String,
    pub options: RipgrepSearchOptions,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceSearchResponse {
    pub searched_revision: u64,
    pub matches: Vec<RipgrepSearchMatch>,
    pub truncated: bool,
    pub truncation_reason: Option<SearchTruncationReason>,
    pub matched_files: usize,
    pub matched_lines: usize,
    pub elapsed_ms: u64,
}

#[derive(Debug)]
pub(crate) enum WorkspaceSearchError {
    InvalidPath(String),
    WorkspaceState,
    Ripgrep(RipgrepError),
}

impl std::fmt::Display for WorkspaceSearchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath(path) => write!(formatter, "Workspace 搜索目录无效：{path}"),
            Self::WorkspaceState => formatter.write_str("无法读取 Workspace 搜索版本"),
            Self::Ripgrep(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for WorkspaceSearchError {}

impl From<RipgrepError> for WorkspaceSearchError {
    fn from(error: RipgrepError) -> Self {
        Self::Ripgrep(error)
    }
}

impl WorkspaceSearchService {
    pub(crate) fn new(layout: WorkspaceLayout, files: WorkspaceFileService, ripgrep: RipgrepSearchService, max_response_bytes: usize) -> Self {
        Self { layout, files, ripgrep, max_response_bytes }
    }

    pub(crate) async fn search(
        &self,
        request: WorkspaceSearchRequest,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<WorkspaceSearchResponse, WorkspaceSearchError> {
        let searched_revision = self
            .files
            .workspace_revision()
            .map_err(|_| WorkspaceSearchError::WorkspaceState)?;
        let (root, prefix) = resolve_search_root(&self.layout, &request.path)?;
        let mut output = self
            .ripgrep
            .search_workspace(SearchCorpus::workspace(root), request.options, cancel_rx)
            .await?;
        for matched in &mut output.matches {
            let logical_path = if let Some(prefix) = &prefix {
                format!("{prefix}/{}", matched.path)
            } else {
                matched.path.clone()
            };
            matched.path = WorkspaceRelativePath::new(&logical_path)
                .map_err(|_| WorkspaceSearchError::InvalidPath(logical_path))?
                .to_string();
        }
        let mut response = WorkspaceSearchResponse {
            searched_revision,
            matches: output.matches,
            truncated: output.truncation_reason.is_some(),
            truncation_reason: output.truncation_reason,
            matched_files: output.matched_files,
            matched_lines: output.matched_lines,
            elapsed_ms: output.elapsed_ms,
        };
        fit_response(&mut response, self.max_response_bytes)?;
        Ok(response)
    }
}

fn resolve_search_root(layout: &WorkspaceLayout, requested_path: &str) -> Result<(PathBuf, Option<String>), WorkspaceSearchError> {
    let workspace_root = layout.workspace_dir();
    ensure_directory(workspace_root, "<workspace-root>")?;
    let canonical_root = fs::canonicalize(workspace_root).map_err(|_| WorkspaceSearchError::WorkspaceState)?;
    if requested_path.is_empty() {
        return Ok((canonical_root, None));
    }
    let relative = WorkspaceRelativePath::new(requested_path).map_err(|_| WorkspaceSearchError::InvalidPath(requested_path.to_string()))?;
    let mut current = workspace_root.to_path_buf();
    for component in relative.as_str().split('/') {
        current.push(component);
        ensure_directory(&current, relative.as_str())?;
    }
    let canonical = fs::canonicalize(&current).map_err(|_| WorkspaceSearchError::InvalidPath(relative.to_string()))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(WorkspaceSearchError::InvalidPath(relative.to_string()));
    }
    Ok((canonical, Some(relative.to_string())))
}

fn ensure_directory(path: &Path, logical_path: &str) -> Result<(), WorkspaceSearchError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| WorkspaceSearchError::InvalidPath(logical_path.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(WorkspaceSearchError::InvalidPath(logical_path.to_string()));
    }
    Ok(())
}

fn fit_response(response: &mut WorkspaceSearchResponse, max_bytes: usize) -> Result<(), WorkspaceSearchError> {
    while serde_json::to_vec(response)
        .map_err(|_| WorkspaceSearchError::WorkspaceState)?
        .len()
        > max_bytes
    {
        if response.matches.pop().is_none() {
            return Err(WorkspaceSearchError::WorkspaceState);
        }
        response.truncated = true;
        response.truncation_reason = Some(SearchTruncationReason::ResponseLimit);
    }
    response.matched_files = response
        .matches
        .iter()
        .map(|matched| matched.path.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    response.matched_lines = response
        .matches
        .iter()
        .filter(|matched| matched.match_kind == SearchMatchKind::Content)
        .count();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::ai_task_execution::workspace::{WorkspaceFileLimits, WorkspaceManager};
    use crate::backend::ai_driven::ripgrep::{bundled_test_binary, RipgrepLimits, SearchCaseMode, SearchMode};
    use tauri::Manager;

    fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("datavisool-workspace-search-{name}-{}", std::process::id()))
    }

    #[test]
    fn accepts_only_regular_relative_directories() {
        let root = root("paths");
        let workspace = WorkspaceManager::new(&root).create("project-1").unwrap();
        fs::create_dir_all(workspace.layout.workspace_dir().join("src/nested")).unwrap();
        assert_eq!(resolve_search_root(&workspace.layout, "src/nested").unwrap().1.as_deref(), Some("src/nested"));
        assert!(resolve_search_root(&workspace.layout, "../outside").is_err());
        assert!(resolve_search_root(&workspace.layout, "src\\nested").is_err());
        assert!(resolve_search_root(&workspace.layout, "missing").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_in_directory_chain() {
        use std::os::unix::fs::symlink;

        let root = root("symlink");
        let workspace = WorkspaceManager::new(&root).create("project-1").unwrap();
        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, workspace.layout.workspace_dir().join("linked")).unwrap();
        assert!(resolve_search_root(&workspace.layout, "linked").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn sidecar_search_supports_regex_case_globs_ignore_and_hidden_rules() {
        let root = root("features");
        let workspace = WorkspaceManager::new(&root).create("project-1").unwrap();
        let workspace_root = workspace.layout.workspace_dir();
        fs::write(workspace_root.join("visible.rs"), "AgentToolRuntime\n").unwrap();
        fs::write(workspace_root.join("ignored.rs"), "AgentToolRuntime\n").unwrap();
        fs::write(workspace_root.join(".hidden.rs"), "AgentToolRuntime\n").unwrap();
        fs::write(workspace_root.join(".gitignore"), "ignored.rs\n").unwrap();
        let limits = WorkspaceFileLimits::default();
        let files = WorkspaceFileService::new(workspace.layout.clone(), limits.clone());
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let service = WorkspaceSearchService::new(
            workspace.layout.clone(),
            files.clone(),
            RipgrepSearchService::for_test(app.handle().clone(), bundled_test_binary(), RipgrepLimits::default()),
            limits.max_tool_output_bytes,
        );
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let response = service
            .search(
                WorkspaceSearchRequest {
                    path: String::new(),
                    options: RipgrepSearchOptions {
                        query: "agent(tool)?runtime".to_string(),
                        mode: SearchMode::Regex,
                        case_mode: SearchCaseMode::Insensitive,
                        globs: vec!["*.rs".to_string()],
                        include_hidden: false,
                        respect_ignore_files: true,
                        max_results: 20,
                    },
                },
                cancel_rx,
            )
            .await
            .unwrap();

        assert!(response.matches.iter().any(|matched| matched.path == "visible.rs"));
        assert!(!response
            .matches
            .iter()
            .any(|matched| matched.path == "ignored.rs" || matched.path == ".hidden.rs"));
        assert_eq!(response.searched_revision, files.workspace_revision().unwrap());

        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let unfiltered = service
            .search(
                WorkspaceSearchRequest {
                    path: String::new(),
                    options: RipgrepSearchOptions {
                        query: "AgentToolRuntime".to_string(),
                        mode: SearchMode::Literal,
                        case_mode: SearchCaseMode::Sensitive,
                        globs: vec!["*.rs".to_string(), "!ignored.rs".to_string()],
                        include_hidden: true,
                        respect_ignore_files: false,
                        max_results: 20,
                    },
                },
                cancel_rx,
            )
            .await
            .unwrap();
        assert!(unfiltered.matches.iter().any(|matched| matched.path == ".hidden.rs"));
        assert!(!unfiltered.matches.iter().any(|matched| matched.path == "ignored.rs"));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn sidecar_search_merges_unicode_path_and_crlf_content_matches() {
        let root = root("unicode-crlf");
        let workspace = WorkspaceManager::new(&root).create("project-1").unwrap();
        fs::create_dir_all(workspace.layout.workspace_dir().join("中文目录")).unwrap();
        fs::write(workspace.layout.workspace_dir().join("中文目录/目标文件.txt"), "第一行\r\n目标内容\r\n").unwrap();
        let limits = WorkspaceFileLimits::default();
        let files = WorkspaceFileService::new(workspace.layout.clone(), limits.clone());
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let service = WorkspaceSearchService::new(
            workspace.layout.clone(),
            files,
            RipgrepSearchService::for_test(app.handle().clone(), bundled_test_binary(), RipgrepLimits::default()),
            limits.max_tool_output_bytes,
        );
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let response = service
            .search(
                WorkspaceSearchRequest {
                    path: "中文目录".to_string(),
                    options: RipgrepSearchOptions {
                        query: "目标".to_string(),
                        mode: SearchMode::Literal,
                        case_mode: SearchCaseMode::Sensitive,
                        globs: Vec::new(),
                        include_hidden: false,
                        respect_ignore_files: true,
                        max_results: 10,
                    },
                },
                cancel_rx,
            )
            .await
            .unwrap();

        assert!(response
            .matches
            .iter()
            .any(|matched| matched.match_kind == SearchMatchKind::Path && matched.path == "中文目录/目标文件.txt"));
        assert!(response.matches.iter().any(|matched| {
            matched.match_kind == SearchMatchKind::Content
                && matched.path == "中文目录/目标文件.txt"
                && matched.line == Some(2)
                && matched.text == "目标内容"
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn sidecar_search_finds_content_after_old_sixteen_mib_boundary() {
        let root = root("beyond-old-limit");
        let workspace = WorkspaceManager::new(&root).create("project-1").unwrap();
        for index in 0..9 {
            fs::write(workspace.layout.workspace_dir().join(format!("a{index:02}.txt")), vec![b'x'; 2 * 1024 * 1024]).unwrap();
        }
        fs::write(workspace.layout.workspace_dir().join("z-target.txt"), "needle-after-sixteen-mib\n").unwrap();
        let limits = WorkspaceFileLimits::default();
        let files = WorkspaceFileService::new(workspace.layout.clone(), limits.clone());
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let service = WorkspaceSearchService::new(
            workspace.layout.clone(),
            files,
            RipgrepSearchService::for_test(app.handle().clone(), bundled_test_binary(), RipgrepLimits::default()),
            limits.max_tool_output_bytes,
        );
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let response = service
            .search(
                WorkspaceSearchRequest {
                    path: String::new(),
                    options: RipgrepSearchOptions {
                        query: "needle-after-sixteen-mib".to_string(),
                        mode: SearchMode::Literal,
                        case_mode: SearchCaseMode::Sensitive,
                        globs: Vec::new(),
                        include_hidden: false,
                        respect_ignore_files: true,
                        max_results: 10,
                    },
                },
                cancel_rx,
            )
            .await
            .unwrap();

        assert!(response
            .matches
            .iter()
            .any(|matched| matched.path == "z-target.txt" && matched.line == Some(1)));
        let _ = fs::remove_dir_all(root);
    }
}
