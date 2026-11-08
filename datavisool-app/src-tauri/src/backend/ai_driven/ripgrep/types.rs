/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 受控 ripgrep 搜索的通用协议类型。

use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SearchMode {
    #[default]
    Literal,
    Regex,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SearchCaseMode {
    #[default]
    Sensitive,
    Insensitive,
    Smart,
}

#[derive(Clone, Debug)]
pub(crate) struct RipgrepSearchOptions {
    pub query: String,
    pub mode: SearchMode,
    pub case_mode: SearchCaseMode,
    pub globs: Vec<String>,
    pub include_hidden: bool,
    pub respect_ignore_files: bool,
    pub max_results: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SearchMatchKind {
    Path,
    Content,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchSubmatch {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RipgrepSearchMatch {
    pub path: String,
    pub line: Option<u64>,
    pub column: Option<u64>,
    pub text: String,
    pub match_kind: SearchMatchKind,
    pub submatches: Vec<SearchSubmatch>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::enum_variant_names)] // 协议明确使用 resultLimit/rawOutputLimit/responseLimit/lineLimit。
pub(crate) enum SearchTruncationReason {
    ResultLimit,
    RawOutputLimit,
    ResponseLimit,
    LineLimit,
}

#[derive(Clone, Debug)]
pub(crate) struct RipgrepSearchOutput {
    pub matches: Vec<RipgrepSearchMatch>,
    pub truncation_reason: Option<SearchTruncationReason>,
    pub matched_files: usize,
    pub matched_lines: usize,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct RipgrepLimits {
    pub max_query_bytes: usize,
    pub max_globs: usize,
    pub max_glob_bytes: usize,
    pub max_results: usize,
    pub max_line_bytes: usize,
    pub max_match_text_bytes: usize,
    pub max_raw_output_bytes: usize,
    pub max_stderr_bytes: usize,
    pub timeout: Duration,
}

impl Default for RipgrepLimits {
    fn default() -> Self {
        Self {
            max_query_bytes: 1_024,
            max_globs: 20,
            max_glob_bytes: 256,
            max_results: 100,
            max_line_bytes: 256 * 1_024,
            max_match_text_bytes: 8 * 1_024,
            max_raw_output_bytes: 8 * 1_024 * 1_024,
            max_stderr_bytes: 32 * 1_024,
            timeout: Duration::from_secs(30),
        }
    }
}
