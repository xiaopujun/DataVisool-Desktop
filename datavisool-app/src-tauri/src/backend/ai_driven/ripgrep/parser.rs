/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! `rg --files` 与 `rg --json` 的逐行解析器。

use std::collections::BTreeSet;

use base64::Engine;
use regex::{Regex, RegexBuilder};
use serde::Deserialize;

use super::types::{RipgrepSearchMatch, SearchCaseMode, SearchMatchKind, SearchMode, SearchSubmatch};
use super::{RipgrepError, RipgrepResult};

pub(super) trait RipgrepLineParser {
    /// 返回 `true` 表示候选结果已足够，Runner 应立即停止 Sidecar。
    fn push_line(&mut self, line: &[u8]) -> RipgrepResult<bool>;
}

#[derive(Debug)]
pub(super) struct WorkspaceFileSetParser {
    matcher: Option<Regex>,
    candidate_limit: usize,
    paths: BTreeSet<String>,
    matches: Vec<RipgrepSearchMatch>,
}

impl WorkspaceFileSetParser {
    pub(super) fn new(
        query: &str,
        mode: SearchMode,
        case_mode: SearchCaseMode,
        candidate_limit: usize,
        collect_path_matches: bool,
    ) -> RipgrepResult<Self> {
        Ok(Self {
            matcher: collect_path_matches
                .then(|| build_matcher(query, mode, case_mode))
                .transpose()?,
            candidate_limit,
            paths: BTreeSet::new(),
            matches: Vec::new(),
        })
    }

    pub(super) fn into_parts(self) -> (BTreeSet<String>, Vec<RipgrepSearchMatch>) {
        (self.paths, self.matches)
    }
}

impl RipgrepLineParser for WorkspaceFileSetParser {
    fn push_line(&mut self, line: &[u8]) -> RipgrepResult<bool> {
        let path = parse_path_line(line)?;
        self.paths.insert(path.clone());
        let Some(matcher) = &self.matcher else {
            return Ok(false);
        };
        if let Some(matched) = path_match(matcher, path) {
            self.matches.push(matched);
        }
        Ok(self.matches.len() >= self.candidate_limit)
    }
}

#[derive(Debug)]
pub(super) struct PathLineParser<'a> {
    matcher: Regex,
    candidate_limit: usize,
    allowed_paths: &'a BTreeSet<String>,
    matches: Vec<RipgrepSearchMatch>,
}

impl<'a> PathLineParser<'a> {
    pub(super) fn new(
        query: &str,
        mode: SearchMode,
        case_mode: SearchCaseMode,
        candidate_limit: usize,
        allowed_paths: &'a BTreeSet<String>,
    ) -> RipgrepResult<Self> {
        Ok(Self { matcher: build_matcher(query, mode, case_mode)?, candidate_limit, allowed_paths, matches: Vec::new() })
    }

    pub(super) fn into_matches(self) -> Vec<RipgrepSearchMatch> {
        self.matches
    }
}

impl RipgrepLineParser for PathLineParser<'_> {
    fn push_line(&mut self, line: &[u8]) -> RipgrepResult<bool> {
        let path = parse_path_line(line)?;
        if !self.allowed_paths.contains(&path) {
            return Ok(false);
        }
        if let Some(matched) = path_match(&self.matcher, path) {
            self.matches.push(matched);
        }
        Ok(self.matches.len() >= self.candidate_limit)
    }
}

pub(super) struct JsonMatchLineParser<'a> {
    candidate_limit: usize,
    max_text_bytes: usize,
    allowed_paths: Option<&'a BTreeSet<String>>,
    matches: Vec<RipgrepSearchMatch>,
}

impl<'a> JsonMatchLineParser<'a> {
    pub(super) fn new(candidate_limit: usize, max_text_bytes: usize) -> Self {
        Self { candidate_limit, max_text_bytes, allowed_paths: None, matches: Vec::new() }
    }

    pub(super) fn with_allowed_paths(candidate_limit: usize, max_text_bytes: usize, allowed_paths: &'a BTreeSet<String>) -> Self {
        Self { candidate_limit, max_text_bytes, allowed_paths: Some(allowed_paths), matches: Vec::new() }
    }

    pub(super) fn into_matches(self) -> Vec<RipgrepSearchMatch> {
        self.matches
    }
}

impl RipgrepLineParser for JsonMatchLineParser<'_> {
    fn push_line(&mut self, line: &[u8]) -> RipgrepResult<bool> {
        let message: RgMessage =
            serde_json::from_slice(line).map_err(|error| RipgrepError::output_invalid(format!("rg --json 输出无法解析：{error}")))?;
        if message.kind != "match" {
            return Ok(false);
        }
        let data: RgMatchData =
            serde_json::from_value(message.data).map_err(|error| RipgrepError::output_invalid(format!("rg match 输出缺少字段：{error}")))?;
        let path = decode_utf8(data.path, "路径")?;
        let path = normalize_relative_path(&path)?;
        if self.allowed_paths.is_some_and(|allowed_paths| !allowed_paths.contains(&path)) {
            return Ok(false);
        }
        let (mut text, byte_offsets_valid) = decode_text(data.lines)?;
        trim_line_ending(&mut text);
        let original_len = text.len();
        truncate_utf8(&mut text, self.max_text_bytes);
        let submatches = if byte_offsets_valid {
            data.submatches
                .into_iter()
                .filter(|matched| matched.start <= matched.end && matched.end <= text.len() && matched.end <= original_len)
                .map(|matched| SearchSubmatch { start: matched.start, end: matched.end })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let column = submatches
            .first()
            .and_then(|matched| u64::try_from(matched.start.saturating_add(1)).ok());
        self.matches
            .push(RipgrepSearchMatch { path, line: data.line_number, column, text, match_kind: SearchMatchKind::Content, submatches });
        Ok(self.matches.len() >= self.candidate_limit)
    }
}

fn parse_path_line(line: &[u8]) -> RipgrepResult<String> {
    let path = std::str::from_utf8(line).map_err(|_| RipgrepError::output_invalid("rg --files 返回了非 UTF-8 路径"))?;
    normalize_relative_path(path)
}

fn path_match(matcher: &Regex, path: String) -> Option<RipgrepSearchMatch> {
    let submatches = matcher
        .find_iter(&path)
        .map(|matched| SearchSubmatch { start: matched.start(), end: matched.end() })
        .collect::<Vec<_>>();
    if submatches.is_empty() {
        return None;
    }
    let column = submatches
        .first()
        .and_then(|matched| u64::try_from(matched.start.saturating_add(1)).ok());
    Some(RipgrepSearchMatch { path: path.clone(), line: None, column, text: path, match_kind: SearchMatchKind::Path, submatches })
}

fn build_matcher(query: &str, mode: SearchMode, case_mode: SearchCaseMode) -> RipgrepResult<Regex> {
    let pattern = match mode {
        SearchMode::Literal => regex::escape(query),
        SearchMode::Regex => query.to_string(),
    };
    let case_insensitive = match case_mode {
        SearchCaseMode::Sensitive => false,
        SearchCaseMode::Insensitive => true,
        SearchCaseMode::Smart => !query.chars().any(char::is_uppercase),
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|error| RipgrepError::invalid_pattern(error.to_string()))
}

fn normalize_relative_path(path: &str) -> RipgrepResult<String> {
    let path = path.strip_prefix("./").unwrap_or(path);
    if path.is_empty()
        || path.starts_with(['/', '\\'])
        || path.contains('\\')
        || path.as_bytes().iter().any(|byte| byte.is_ascii_control())
        || path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(RipgrepError::output_invalid("rg 返回了范围外路径"));
    }
    Ok(path.to_string())
}

fn decode_utf8(value: RgText, field: &str) -> RipgrepResult<String> {
    if let Some(text) = value.text {
        return Ok(text);
    }
    let bytes = value
        .bytes
        .ok_or_else(|| RipgrepError::output_invalid(format!("rg {field}缺少 text/bytes")))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes)
        .map_err(|_| RipgrepError::output_invalid(format!("rg {field}包含无效 base64")))?;
    String::from_utf8(bytes).map_err(|_| RipgrepError::output_invalid(format!("rg {field}不是 UTF-8")))
}

fn decode_text(value: RgText) -> RipgrepResult<(String, bool)> {
    if let Some(text) = value.text {
        return Ok((text, true));
    }
    let bytes = value
        .bytes
        .ok_or_else(|| RipgrepError::output_invalid("rg 行内容缺少 text/bytes"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes)
        .map_err(|_| RipgrepError::output_invalid("rg 行内容包含无效 base64"))?;
    match String::from_utf8(bytes) {
        Ok(text) => Ok((text, true)),
        Err(error) => Ok((String::from_utf8_lossy(error.as_bytes()).into_owned(), false)),
    }
}

fn trim_line_ending(text: &mut String) {
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    }
}

fn truncate_utf8(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    text.truncate(boundary);
}

#[derive(Deserialize)]
struct RgMessage {
    #[serde(rename = "type")]
    kind: String,
    data: serde_json::Value,
}

#[derive(Deserialize)]
struct RgMatchData {
    path: RgText,
    lines: RgText,
    line_number: Option<u64>,
    #[serde(default)]
    submatches: Vec<RgSubmatch>,
}

#[derive(Deserialize)]
struct RgText {
    text: Option<String>,
    bytes: Option<String>,
}

#[derive(Deserialize)]
struct RgSubmatch {
    start: usize,
    end: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_parser_applies_literal_smart_case_and_normalizes_paths() {
        let allowed_paths = BTreeSet::from(["src/AgentRuntime.rs".to_string()]);
        let mut parser = PathLineParser::new("agent", SearchMode::Literal, SearchCaseMode::Smart, 10, &allowed_paths).unwrap();
        assert!(!parser.push_line(b"./src/AgentRuntime.rs").unwrap());
        let matches = parser.into_matches();
        assert_eq!(matches[0].path, "src/AgentRuntime.rs");
        assert_eq!(matches[0].submatches, vec![SearchSubmatch { start: 4, end: 9 }]);
    }

    #[test]
    fn json_parser_decodes_text_and_clips_submatches_after_truncation() {
        let mut parser = JsonMatchLineParser::new(10, 5);
        parser
            .push_line(
                br#"{"type":"match","data":{"path":{"text":"./a.txt"},"lines":{"text":"abcdef\n"},"line_number":2,"submatches":[{"start":1,"end":3},{"start":4,"end":6}]}}"#,
            )
            .unwrap();
        let matches = parser.into_matches();
        assert_eq!(matches[0].text, "abcde");
        assert_eq!(matches[0].submatches, vec![SearchSubmatch { start: 1, end: 3 }]);
        assert_eq!(matches[0].column, Some(2));
    }

    #[test]
    fn json_parser_decodes_base64_utf8_and_windows_line_endings() {
        let path = base64::engine::general_purpose::STANDARD.encode("./中文.txt");
        let line = base64::engine::general_purpose::STANDARD.encode("针线\r\n");
        let message = format!(
            r#"{{"type":"match","data":{{"path":{{"bytes":"{path}"}},"lines":{{"bytes":"{line}"}},"line_number":3,"submatches":[{{"start":0,"end":6}}]}}}}"#
        );
        let mut parser = JsonMatchLineParser::new(10, 100);
        parser.push_line(message.as_bytes()).unwrap();
        let matches = parser.into_matches();
        assert_eq!(matches[0].path, "中文.txt");
        assert_eq!(matches[0].text, "针线");
        assert_eq!(matches[0].submatches, vec![SearchSubmatch { start: 0, end: 6 }]);
    }

    #[test]
    fn invalid_regex_is_reported_before_process_start() {
        let error = PathLineParser::new("(", SearchMode::Regex, SearchCaseMode::Sensitive, 10, &BTreeSet::new()).unwrap_err();
        assert_eq!(error.code, "agent.workspace.search_invalid_pattern");
    }

    #[test]
    fn json_parser_rejects_paths_outside_host_file_set() {
        let allowed_paths = BTreeSet::from(["visible.rs".to_string()]);
        let mut parser = JsonMatchLineParser::with_allowed_paths(10, 100, &allowed_paths);
        parser
            .push_line(
                br#"{"type":"match","data":{"path":{"text":"./ignored.rs"},"lines":{"text":"needle\n"},"line_number":1,"submatches":[{"start":0,"end":6}]}}"#,
            )
            .unwrap();
        assert!(parser.into_matches().is_empty());
    }
}
