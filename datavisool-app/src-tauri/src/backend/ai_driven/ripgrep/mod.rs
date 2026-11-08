/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 仅允许固定语料、固定程序和白名单参数的 ripgrep 搜索服务。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use tokio::sync::watch;

mod parser;
mod runner;
mod types;

use parser::{JsonMatchLineParser, PathLineParser, WorkspaceFileSetParser};
use runner::{RipgrepCommandKind, RipgrepRunRequest, RipgrepRunner};
pub(crate) use types::{
    RipgrepLimits, RipgrepSearchMatch, RipgrepSearchOptions, RipgrepSearchOutput, SearchCaseMode, SearchMatchKind, SearchMode, SearchSubmatch,
    SearchTruncationReason,
};

pub(crate) type RipgrepResult<T> = Result<T, RipgrepError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RipgrepErrorKind {
    InvalidPattern,
    MissingBinary,
    Spawn,
    Output,
    Timeout,
    Cancelled,
    Process,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RipgrepError {
    pub(crate) kind: RipgrepErrorKind,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl RipgrepError {
    fn new(kind: RipgrepErrorKind, code: &'static str, message: impl Into<String>) -> Self {
        Self { kind, code, message: message.into() }
    }

    fn invalid_pattern(message: impl Into<String>) -> Self {
        Self::new(RipgrepErrorKind::InvalidPattern, "agent.workspace.search_invalid_pattern", message)
    }

    fn binary_missing() -> Self {
        Self::new(RipgrepErrorKind::MissingBinary, "agent.workspace.search_binary_missing", "安装包缺少 rg Sidecar")
    }

    fn spawn_failed(message: impl Into<String>) -> Self {
        Self::new(RipgrepErrorKind::Spawn, "agent.workspace.search_spawn_failed", message)
    }

    fn output_invalid(message: impl Into<String>) -> Self {
        Self::new(RipgrepErrorKind::Output, "agent.workspace.search_output_invalid", message)
    }

    fn timeout() -> Self {
        Self::new(RipgrepErrorKind::Timeout, "agent.workspace.search_timeout", "文件内容搜索超时，rg Sidecar 已终止")
    }

    fn cancelled() -> Self {
        Self::new(RipgrepErrorKind::Cancelled, "agent.workspace.search_cancelled", "文件内容搜索已取消，rg Sidecar 已终止")
    }

    fn process_failed(message: impl Into<String>) -> Self {
        Self::new(RipgrepErrorKind::Process, "agent.workspace.search_process_failed", message)
    }
}

impl std::fmt::Display for RipgrepError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RipgrepError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SearchCorpusKind {
    Workspace,
    ComponentDocuments,
}

/// 只能由宿主适配器构造，不能从 Agent 参数反序列化。
pub(crate) struct SearchCorpus {
    kind: SearchCorpusKind,
    root: PathBuf,
}

impl SearchCorpus {
    pub(crate) fn workspace(root: PathBuf) -> Self {
        Self { kind: SearchCorpusKind::Workspace, root }
    }

    pub(crate) fn component_documents(root: PathBuf) -> Self {
        Self { kind: SearchCorpusKind::ComponentDocuments, root }
    }

    fn root(&self) -> &Path {
        &self.root
    }
}

#[derive(Clone)]
pub(crate) struct RipgrepSearchService {
    runner: RipgrepRunner,
    limits: RipgrepLimits,
}

impl RipgrepSearchService {
    pub(crate) fn new(app_handle: tauri::AppHandle) -> Self {
        Self::with_limits(app_handle, RipgrepLimits::default())
    }

    pub(crate) fn with_limits(app_handle: tauri::AppHandle, limits: RipgrepLimits) -> Self {
        Self { runner: RipgrepRunner::new(app_handle, limits.clone()), limits }
    }

    #[cfg(test)]
    pub(crate) fn for_test<R: tauri::Runtime>(app_handle: tauri::AppHandle<R>, binary: PathBuf, limits: RipgrepLimits) -> Self {
        Self { runner: RipgrepRunner::direct(app_handle, binary, limits.clone()), limits }
    }

    pub(crate) async fn search_workspace(
        &self,
        corpus: SearchCorpus,
        options: RipgrepSearchOptions,
        cancel_rx: watch::Receiver<bool>,
    ) -> RipgrepResult<RipgrepSearchOutput> {
        debug_assert_eq!(corpus.kind, SearchCorpusKind::Workspace);
        self.validate_options(&options)?;
        let started_at = Instant::now();
        let candidate_limit = options.max_results.saturating_add(1);
        // ripgrep 的显式 --glob 会覆盖隐藏文件和 ignore 规则。先用不带
        // Agent Glob 的 --files 建立宿主允许集合，后续命中只能落在集合内。
        let mut file_set_parser =
            WorkspaceFileSetParser::new(&options.query, options.mode, options.case_mode, candidate_limit, options.globs.is_empty())?;
        let file_set_run = self
            .runner
            .run(
                &corpus,
                RipgrepRunRequest {
                    kind: RipgrepCommandKind::Files,
                    globs: &[],
                    include_hidden: options.include_hidden,
                    respect_ignore_files: options.respect_ignore_files,
                    raw_output_budget: self.limits.max_raw_output_bytes,
                },
                cancel_rx.clone(),
                &mut file_set_parser,
            )
            .await?;
        let (allowed_paths, mut matches) = file_set_parser.into_parts();
        let mut raw_output_bytes = file_set_run.raw_output_bytes;
        let mut truncation_reason = file_set_run.truncation_reason;

        let remaining_raw = self.limits.max_raw_output_bytes.saturating_sub(raw_output_bytes);
        if truncation_reason.is_none() && remaining_raw > 0 && !options.globs.is_empty() && matches.len() < candidate_limit {
            let mut path_parser = PathLineParser::new(&options.query, options.mode, options.case_mode, candidate_limit, &allowed_paths)?;
            let paths = self
                .runner
                .run(
                    &corpus,
                    RipgrepRunRequest {
                        kind: RipgrepCommandKind::Files,
                        globs: &options.globs,
                        include_hidden: options.include_hidden,
                        respect_ignore_files: options.respect_ignore_files,
                        raw_output_budget: remaining_raw,
                    },
                    cancel_rx.clone(),
                    &mut path_parser,
                )
                .await?;
            raw_output_bytes = raw_output_bytes.saturating_add(paths.raw_output_bytes);
            matches = path_parser.into_matches();
            truncation_reason = paths.truncation_reason;
        }

        let remaining_raw = self.limits.max_raw_output_bytes.saturating_sub(raw_output_bytes);
        if truncation_reason.is_none() && remaining_raw > 0 && !allowed_paths.is_empty() && matches.len() < candidate_limit {
            let remaining_candidates = candidate_limit.saturating_sub(matches.len());
            let mut content_parser = JsonMatchLineParser::with_allowed_paths(remaining_candidates, self.limits.max_match_text_bytes, &allowed_paths);
            let content = self
                .runner
                .run(
                    &corpus,
                    RipgrepRunRequest {
                        kind: RipgrepCommandKind::Content(&options),
                        globs: &options.globs,
                        include_hidden: options.include_hidden,
                        respect_ignore_files: options.respect_ignore_files,
                        raw_output_budget: remaining_raw,
                    },
                    cancel_rx,
                    &mut content_parser,
                )
                .await?;
            matches.extend(content_parser.into_matches());
            truncation_reason = content.truncation_reason;
        }
        sort_matches(&mut matches);
        if matches.len() > options.max_results {
            matches.truncate(options.max_results);
            truncation_reason.get_or_insert(SearchTruncationReason::ResultLimit);
        }
        Ok(search_output(matches, truncation_reason, started_at))
    }

    pub(crate) async fn search_component_documents(
        &self,
        corpus: SearchCorpus,
        options: RipgrepSearchOptions,
        cancel_rx: watch::Receiver<bool>,
    ) -> RipgrepResult<RipgrepSearchOutput> {
        debug_assert_eq!(corpus.kind, SearchCorpusKind::ComponentDocuments);
        self.validate_options(&options)?;
        let started_at = Instant::now();
        let candidate_limit = options.max_results.saturating_add(1);
        let mut parser = JsonMatchLineParser::new(candidate_limit, self.limits.max_match_text_bytes);
        let run = self
            .runner
            .run(
                &corpus,
                RipgrepRunRequest {
                    kind: RipgrepCommandKind::Content(&options),
                    globs: &options.globs,
                    include_hidden: false,
                    respect_ignore_files: true,
                    raw_output_budget: self.limits.max_raw_output_bytes,
                },
                cancel_rx,
                &mut parser,
            )
            .await?;
        let mut matches = parser.into_matches();
        sort_matches(&mut matches);
        let mut truncation_reason = run.truncation_reason;
        if matches.len() > options.max_results {
            matches.truncate(options.max_results);
            truncation_reason.get_or_insert(SearchTruncationReason::ResultLimit);
        }
        Ok(search_output(matches, truncation_reason, started_at))
    }

    fn validate_options(&self, options: &RipgrepSearchOptions) -> RipgrepResult<()> {
        if options.query.is_empty() || options.query.len() > self.limits.max_query_bytes || options.query.contains('\0') {
            return Err(RipgrepError::new(
                RipgrepErrorKind::InvalidPattern,
                "agent.workspace.search_invalid_pattern",
                format!("query 必须为 1 到 {} 字节且不能包含 NUL", self.limits.max_query_bytes),
            ));
        }
        if options.globs.len() > self.limits.max_globs
            || options
                .globs
                .iter()
                .any(|glob| glob.is_empty() || glob.len() > self.limits.max_glob_bytes || glob.contains('\0'))
        {
            return Err(RipgrepError::new(
                RipgrepErrorKind::InvalidPattern,
                "agent.workspace.search_invalid_pattern",
                format!("globs 最多 {} 条，每条为 1 到 {} 字节", self.limits.max_globs, self.limits.max_glob_bytes),
            ));
        }
        if !(1..=self.limits.max_results).contains(&options.max_results) {
            return Err(RipgrepError::new(
                RipgrepErrorKind::InvalidPattern,
                "agent.workspace.search_invalid_pattern",
                format!("maxResults 必须在 1 到 {} 之间", self.limits.max_results),
            ));
        }
        if options.mode == SearchMode::Regex {
            let _ = PathLineParser::new(&options.query, options.mode, options.case_mode, 1, &BTreeSet::new())?;
        }
        Ok(())
    }
}

fn sort_matches(matches: &mut [RipgrepSearchMatch]) {
    matches.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.line.unwrap_or(0).cmp(&right.line.unwrap_or(0)))
            .then_with(|| left.column.unwrap_or(0).cmp(&right.column.unwrap_or(0)))
            .then_with(|| match_rank(left.match_kind).cmp(&match_rank(right.match_kind)))
    });
}

fn match_rank(kind: SearchMatchKind) -> u8 {
    match kind {
        SearchMatchKind::Path => 0,
        SearchMatchKind::Content => 1,
    }
}

fn search_output(matches: Vec<RipgrepSearchMatch>, truncation_reason: Option<SearchTruncationReason>, started_at: Instant) -> RipgrepSearchOutput {
    let matched_files = matches
        .iter()
        .map(|matched| matched.path.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let matched_lines = matches
        .iter()
        .filter(|matched| matched.match_kind == SearchMatchKind::Content)
        .count();
    RipgrepSearchOutput {
        matches,
        truncation_reason,
        matched_files,
        matched_lines,
        elapsed_ms: started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    }
}

#[cfg(test)]
pub(crate) fn bundled_test_binary() -> PathBuf {
    let binary = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "rg-aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "rg-x86_64-apple-darwin"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "rg-x86_64-pc-windows-msvc.exe"
    } else {
        panic!("当前测试平台没有登记 ripgrep Sidecar")
    };
    Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries").join(binary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_options(query: &str, max_results: usize) -> RipgrepSearchOptions {
        RipgrepSearchOptions {
            query: query.to_string(),
            mode: SearchMode::Literal,
            case_mode: SearchCaseMode::Sensitive,
            globs: Vec::new(),
            include_hidden: false,
            respect_ignore_files: true,
            max_results,
        }
    }

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("datavisool-ripgrep-{name}-{}", std::process::id()))
    }

    #[test]
    fn sorting_is_stable_across_path_and_content_matches() {
        let mut matches = vec![
            RipgrepSearchMatch {
                path: "b.txt".to_string(),
                line: Some(2),
                column: Some(1),
                text: "x".to_string(),
                match_kind: SearchMatchKind::Content,
                submatches: vec![],
            },
            RipgrepSearchMatch {
                path: "a.txt".to_string(),
                line: None,
                column: Some(1),
                text: "a.txt".to_string(),
                match_kind: SearchMatchKind::Path,
                submatches: vec![],
            },
        ];
        sort_matches(&mut matches);
        assert_eq!(matches.iter().map(|matched| matched.path.as_str()).collect::<Vec<_>>(), vec!["a.txt", "b.txt"]);
    }

    #[tokio::test]
    async fn missing_sidecar_fails_closed_without_path_fallback() {
        let root = test_root("missing-binary");
        std::fs::create_dir_all(&root).unwrap();
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let missing = root.join("missing-rg");
        let service = RipgrepSearchService::for_test(app.handle().clone(), missing, RipgrepLimits::default());
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let error = service
            .search_component_documents(SearchCorpus::component_documents(root.clone()), test_options("needle", 10), cancel_rx)
            .await
            .unwrap_err();
        assert_eq!(error.kind, RipgrepErrorKind::MissingBinary);
        assert_eq!(error.code, "agent.workspace.search_binary_missing");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn pre_cancelled_search_does_not_start_sidecar() {
        let root = test_root("cancelled");
        std::fs::create_dir_all(&root).unwrap();
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let service = RipgrepSearchService::for_test(app.handle().clone(), bundled_test_binary(), RipgrepLimits::default());
        let (_cancel_tx, cancel_rx) = watch::channel(true);
        let error = service
            .search_component_documents(SearchCorpus::component_documents(root.clone()), test_options("needle", 10), cancel_rx)
            .await
            .unwrap_err();
        assert_eq!(error.kind, RipgrepErrorKind::Cancelled);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn raw_output_and_result_limits_are_reported() {
        let root = test_root("limits");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("needle-a.txt"), "needle\n").unwrap();
        std::fs::write(root.join("needle-b.txt"), "needle\n").unwrap();
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();

        let result_service = RipgrepSearchService::for_test(app.handle().clone(), bundled_test_binary(), RipgrepLimits::default());
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let limited = result_service
            .search_workspace(SearchCorpus::workspace(root.clone()), test_options("needle", 1), cancel_rx)
            .await
            .unwrap();
        assert_eq!(limited.matches.len(), 1);
        assert_eq!(limited.truncation_reason, Some(SearchTruncationReason::ResultLimit));

        let raw_limits = RipgrepLimits { max_raw_output_bytes: 1, ..RipgrepLimits::default() };
        let raw_service = RipgrepSearchService::for_test(app.handle().clone(), bundled_test_binary(), raw_limits);
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let limited = raw_service
            .search_workspace(SearchCorpus::workspace(root.clone()), test_options("needle", 10), cancel_rx)
            .await
            .unwrap();
        assert!(limited.matches.is_empty());
        assert_eq!(limited.truncation_reason, Some(SearchTruncationReason::RawOutputLimit));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn search_timeout_returns_stable_error() {
        let root = test_root("timeout");
        std::fs::create_dir_all(&root).unwrap();
        for index in 0..200 {
            std::fs::write(root.join(format!("file-{index:03}.txt")), "content\n").unwrap();
        }
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let limits = RipgrepLimits { timeout: std::time::Duration::ZERO, ..RipgrepLimits::default() };
        let service = RipgrepSearchService::for_test(app.handle().clone(), bundled_test_binary(), limits);
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let error = service
            .search_workspace(SearchCorpus::workspace(root.clone()), test_options("missing", 10), cancel_rx)
            .await
            .unwrap_err();
        assert_eq!(error.kind, RipgrepErrorKind::Timeout);
        assert_eq!(error.code, "agent.workspace.search_timeout");
        let _ = std::fs::remove_dir_all(root);
    }
}
