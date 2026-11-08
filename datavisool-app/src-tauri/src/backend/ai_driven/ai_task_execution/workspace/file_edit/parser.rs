/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Codex 风格文本 Patch 的解析和上下文匹配。

use crate::backend::ai_driven::ai_task_execution::workspace::{WorkspaceError, WorkspaceRelativePath, WorkspaceResult};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ParsedFilePatch {
    pub(super) operations: Vec<FilePatchOperation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum FilePatchOperation {
    Add { path: WorkspaceRelativePath, content: String },
    Update { path: WorkspaceRelativePath, move_to: Option<WorkspaceRelativePath>, hunks: Vec<PatchHunk> },
    Delete { path: WorkspaceRelativePath },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PatchHunk {
    lines: Vec<PatchLine>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PatchLine {
    Context(String),
    Remove(String),
    Add(String),
}

pub(super) fn parse_patch(input: &str, max_bytes: usize, max_operations: usize) -> WorkspaceResult<ParsedFilePatch> {
    if input.len() > max_bytes {
        return Err(WorkspaceError::ResourceLimit(format!("patch exceeds {max_bytes} bytes")));
    }
    let lines = input
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect::<Vec<_>>();
    if lines.first() != Some(&"*** Begin Patch") || !lines.iter().any(|line| *line == "*** End Patch") {
        return Err(invalid("patch must start with '*** Begin Patch' and end with '*** End Patch'"));
    }
    let end = lines
        .iter()
        .position(|line| *line == "*** End Patch")
        .expect("checked end marker");
    if lines[end + 1..].iter().any(|line| !line.is_empty()) {
        return Err(invalid("unexpected content after '*** End Patch'"));
    }

    let mut operations = Vec::new();
    let mut cursor = 1usize;
    while cursor < end {
        let line = lines[cursor];
        if line.is_empty() {
            cursor += 1;
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            cursor += 1;
            let mut added = Vec::new();
            while cursor < end && !is_file_header(lines[cursor]) {
                let line = lines[cursor];
                let content = line
                    .strip_prefix('+')
                    .ok_or_else(|| invalid("Add File content lines must start with '+'"))?;
                added.push(content);
                cursor += 1;
            }
            let content = if added.is_empty() {
                String::new()
            } else {
                format!("{}\n", added.join("\n"))
            };
            operations.push(FilePatchOperation::Add { path: WorkspaceRelativePath::new(path)?, content });
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            operations.push(FilePatchOperation::Delete { path: WorkspaceRelativePath::new(path)? });
            cursor += 1;
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            cursor += 1;
            let mut move_to = None;
            let mut hunks = Vec::new();
            while cursor < end && !is_file_header(lines[cursor]) {
                if let Some(path) = lines[cursor].strip_prefix("*** Move to: ") {
                    if move_to.replace(WorkspaceRelativePath::new(path)?).is_some() {
                        return Err(invalid("Update File contains multiple Move to directives"));
                    }
                    cursor += 1;
                    continue;
                }
                if !lines[cursor].starts_with("@@") {
                    return Err(invalid("Update File content must start with a '@@' hunk"));
                }
                cursor += 1;
                let mut hunk_lines = Vec::new();
                while cursor < end
                    && !is_file_header(lines[cursor])
                    && !lines[cursor].starts_with("@@")
                    && !lines[cursor].starts_with("*** Move to: ")
                {
                    let line = lines[cursor];
                    if line == "*** End of File" {
                        cursor += 1;
                        continue;
                    }
                    let (prefix, content) = line
                        .split_at_checked(1)
                        .ok_or_else(|| invalid("empty hunk line is not allowed"))?;
                    let parsed = match prefix {
                        " " => PatchLine::Context(content.to_string()),
                        "-" => PatchLine::Remove(content.to_string()),
                        "+" => PatchLine::Add(content.to_string()),
                        _ => return Err(invalid("hunk lines must start with space, '-' or '+'")),
                    };
                    hunk_lines.push(parsed);
                    cursor += 1;
                }
                if hunk_lines.is_empty()
                    || !hunk_lines
                        .iter()
                        .any(|line| matches!(line, PatchLine::Remove(_) | PatchLine::Add(_)))
                {
                    return Err(invalid("hunk must contain at least one added or removed line"));
                }
                hunks.push(PatchHunk { lines: hunk_lines });
            }
            if hunks.is_empty() {
                return Err(invalid("Update File requires at least one hunk"));
            }
            operations.push(FilePatchOperation::Update { path: WorkspaceRelativePath::new(path)?, move_to, hunks });
        } else {
            return Err(invalid(&format!("unknown patch directive: {line}")));
        }
        if operations.len() > max_operations {
            return Err(WorkspaceError::ResourceLimit(format!("patch exceeds {max_operations} file operations")));
        }
    }
    if operations.is_empty() {
        return Err(invalid("patch contains no file operations"));
    }
    Ok(ParsedFilePatch { operations })
}

pub(super) fn apply_hunks(original: &str, hunks: &[PatchHunk]) -> WorkspaceResult<String> {
    let uses_crlf = original.contains("\r\n");
    let normalized = original.replace("\r\n", "\n");
    let trailing_newline = normalized.ends_with('\n');
    let body = normalized.strip_suffix('\n').unwrap_or(&normalized);
    let mut lines = if body.is_empty() {
        Vec::new()
    } else {
        body.split('\n').map(str::to_string).collect::<Vec<_>>()
    };
    let mut search_start = 0usize;

    for hunk in hunks {
        let old = hunk
            .lines
            .iter()
            .filter_map(|line| match line {
                PatchLine::Context(value) | PatchLine::Remove(value) => Some(value.as_str()),
                PatchLine::Add(_) => None,
            })
            .collect::<Vec<_>>();
        let new = hunk
            .lines
            .iter()
            .filter_map(|line| match line {
                PatchLine::Context(value) | PatchLine::Add(value) => Some(value.clone()),
                PatchLine::Remove(_) => None,
            })
            .collect::<Vec<_>>();
        let position = find_unique_hunk(&lines, &old, search_start)?;
        let old_len = old.len();
        lines.splice(position..position + old_len, new.iter().cloned());
        search_start = position + new.len();
    }

    let mut result = lines.join("\n");
    if trailing_newline && (!result.is_empty() || !lines.is_empty()) {
        result.push('\n');
    }
    if uses_crlf {
        result = result.replace('\n', "\r\n");
    }
    Ok(result)
}

fn find_unique_hunk(lines: &[String], old: &[&str], search_start: usize) -> WorkspaceResult<usize> {
    if old.is_empty() {
        return Ok(lines.len());
    }
    if old.len() > lines.len() {
        return Err(invalid("patch context does not match target file"));
    }
    let matches = (search_start..=lines.len() - old.len())
        .filter(|start| {
            lines[*start..*start + old.len()]
                .iter()
                .map(String::as_str)
                .eq(old.iter().copied())
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [position] => Ok(*position),
        [] => Err(invalid("patch context does not match target file")),
        _ => Err(invalid("patch context is ambiguous in target file")),
    }
}

fn is_file_header(line: &str) -> bool {
    line.starts_with("*** Add File: ") || line.starts_with("*** Update File: ") || line.starts_with("*** Delete File: ")
}

fn invalid(message: &str) -> WorkspaceError {
    WorkspaceError::InvalidFilePatch(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_add_update_move_and_delete() {
        let patch = "*** Begin Patch\n*** Add File: docs/new.md\n+new\n*** Update File: docs/old.md\n@@\n-old\n+updated\n*** Move to: docs/moved.md\n*** Delete File: docs/delete.md\n*** End Patch";
        let parsed = parse_patch(patch, 10_000, 10).unwrap();
        assert_eq!(parsed.operations.len(), 3);
        assert!(matches!(&parsed.operations[0], FilePatchOperation::Add { content, .. } if content == "new\n"));
        assert!(matches!(&parsed.operations[1], FilePatchOperation::Update { move_to: Some(_), .. }));
    }

    #[test]
    fn applies_unique_context_and_preserves_crlf() {
        let patch = parse_patch("*** Begin Patch\n*** Update File: file.txt\n@@\n one\n-two\n+second\n three\n*** End Patch", 10_000, 10).unwrap();
        let FilePatchOperation::Update { hunks, .. } = &patch.operations[0] else {
            panic!("expected update");
        };
        assert_eq!(apply_hunks("one\r\ntwo\r\nthree\r\n", hunks).unwrap(), "one\r\nsecond\r\nthree\r\n");
    }

    #[test]
    fn rejects_mismatch_ambiguity_and_out_of_scope_paths() {
        let ambiguous = parse_patch("*** Begin Patch\n*** Update File: file.txt\n@@\n-same\n+new\n*** End Patch", 10_000, 10).unwrap();
        let FilePatchOperation::Update { hunks, .. } = &ambiguous.operations[0] else {
            panic!("expected update");
        };
        assert!(apply_hunks("same\nsame\n", hunks).is_err());
        assert!(apply_hunks("other\n", hunks).is_err());
        assert!(parse_patch("*** Begin Patch\n*** Add File: ../bad\n+x\n*** End Patch", 10_000, 10).is_err());
    }
}
