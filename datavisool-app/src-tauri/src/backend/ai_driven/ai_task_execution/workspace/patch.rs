/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 任务产物的结构化和局部文本补丁服务。
//!
//! 有效 JSON 使用 RFC 6902 JSON Patch；语法无效的 JSON 使用精确行范围文本补丁。
//! 两种补丁都要求调用方携带 `baseRevision`，并仅在检查成功后提交。

use super::artifact_store::resolve_line_range;
use super::artifact_store::ArtifactStore;
use super::types::{
    ArtifactContentFormat, ArtifactExcerpt, ArtifactId, JsonPatchRequest, PatchResult, TextPatchRequest, WorkspaceError, WorkspaceResult,
};

/// 受控产物补丁服务。
#[derive(Clone, Debug)]
pub(crate) struct ArtifactPatchService {
    store: ArtifactStore,
}

impl ArtifactPatchService {
    /// 基于工作区产物仓库创建补丁服务。
    pub(crate) fn new(store: ArtifactStore) -> Self {
        Self { store }
    }

    /// 读取目标产物的有限行范围，供无效 JSON 的局部修复定位使用。
    pub(crate) fn read_lines(&self, artifact_id: &ArtifactId, start_line: usize, end_line: usize) -> WorkspaceResult<ArtifactExcerpt> {
        self.store.read_lines(artifact_id, start_line, end_line)
    }

    /// 对有效 JSON 产物应用 RFC 6902 结构化补丁。
    pub(crate) fn apply_json_patch(&self, request: JsonPatchRequest) -> WorkspaceResult<PatchResult> {
        let _guard = self.store.lock_workspace()?;
        let snapshot = self.store.read_locked(&request.artifact_id)?;
        ensure_base_revision(&snapshot.metadata.artifact_id.to_string(), snapshot.metadata.revision, request.base_revision)?;
        if request.operations.0.is_empty() {
            return Ok(PatchResult { artifact: snapshot });
        }

        let mut value = serde_json::from_str(&snapshot.content).map_err(|error| WorkspaceError::InvalidJson {
            artifact_id: request.artifact_id.to_string(),
            message: format!("{} at line {}, column {}", error, error.line(), error.column()),
        })?;
        json_patch::patch(&mut value, &request.operations.0)
            .map_err(|error| WorkspaceError::JsonPatchFailed { artifact_id: request.artifact_id.to_string(), message: error.to_string() })?;
        let content = serde_json::to_string_pretty(&value)
            .map_err(|error| WorkspaceError::JsonPatchFailed { artifact_id: request.artifact_id.to_string(), message: error.to_string() })?;
        let artifact = self
            .store
            .commit_content_locked(&request.artifact_id, request.base_revision, content)?;
        Ok(PatchResult { artifact })
    }

    /// 对无效 JSON 或普通文本应用精确行范围局部补丁。
    ///
    /// JSON 类型产物允许经过多次文本补丁逐步恢复语法；每次修改仍会生成新版本，
    /// 上层校验器负责定位下一处错误，解析成功后切换回 JSON Patch。
    pub(crate) fn apply_text_patch(&self, request: TextPatchRequest) -> WorkspaceResult<PatchResult> {
        let _guard = self.store.lock_workspace()?;
        let snapshot = self.store.read_locked(&request.artifact_id)?;
        ensure_base_revision(&snapshot.metadata.artifact_id.to_string(), snapshot.metadata.revision, request.base_revision)?;
        if snapshot.metadata.content_format == ArtifactContentFormat::Json && serde_json::from_str::<serde_json::Value>(&snapshot.content).is_ok() {
            return Err(WorkspaceError::TextPatchNotAllowed(request.artifact_id.to_string()));
        }

        let range = resolve_line_range(&snapshot.content, request.start_line, request.end_line)?;
        let current_text = &snapshot.content[range.start_byte..range.end_byte];
        if current_text != request.expected_text {
            return Err(WorkspaceError::TextPatchContentMismatch(request.artifact_id.to_string()));
        }
        let (prefix_end, suffix_start) = replacement_bounds(&snapshot.content, range.start_byte, range.end_byte, request.new_text.is_empty());
        let mut content = String::with_capacity(snapshot.content.len() - current_text.len() + request.new_text.len());
        content.push_str(&snapshot.content[..prefix_end]);
        content.push_str(&request.new_text);
        content.push_str(&snapshot.content[suffix_start..]);
        let artifact = self
            .store
            .commit_content_locked(&request.artifact_id, request.base_revision, content)?;
        Ok(PatchResult { artifact })
    }
}

/// 删除整行时一并删除相邻换行符，避免首行、中间行或末行留下空白行。
fn replacement_bounds(content: &str, start: usize, end: usize, deleting: bool) -> (usize, usize) {
    if !deleting {
        return (start, end);
    }
    if content[end..].starts_with("\r\n") {
        return (start, end + 2);
    }
    if content[end..].starts_with('\n') {
        return (start, end + 1);
    }
    if content[..start].ends_with("\r\n") {
        return (start - 2, end);
    }
    if content[..start].ends_with('\n') {
        return (start - 1, end);
    }
    (start, end)
}

/// 在执行补丁前检查调用方版本。
fn ensure_base_revision(artifact_id: &str, actual: u64, expected: u64) -> WorkspaceResult<()> {
    if actual != expected {
        return Err(WorkspaceError::RevisionConflict { artifact_id: artifact_id.to_string(), expected, actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::json;

    use super::*;
    use crate::backend::ai_driven::ai_task_execution::workspace::{ArtifactId, TextPatchRequest, WorkspaceManager};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn test_service(name: &str) -> (PathBuf, ArtifactStore, ArtifactPatchService) {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("datavisool-artifact-patch-{name}-{}-{sequence}", std::process::id()));
        let workspace = WorkspaceManager::new(&root)
            .create("run-1")
            .expect("workspace should be created");
        let store = ArtifactStore::new(&workspace);
        let service = ArtifactPatchService::new(store.clone());
        (root, store, service)
    }

    #[test]
    fn applies_json_patch_and_increments_revision() {
        let (root, store, service) = test_service("json");
        let artifact_id = ArtifactId::new("result").expect("artifactId should be valid");
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Json, r#"{"title":"旧标题"}"#)
            .expect("artifact should be created");
        let operations = serde_json::from_value(json!([{ "op": "replace", "path": "/title", "value": "新标题" }])).expect("patch should deserialize");
        let result = service
            .apply_json_patch(JsonPatchRequest { artifact_id: artifact_id.clone(), base_revision: 1, operations })
            .expect("patch should succeed");
        assert_eq!(result.artifact.metadata.revision, 2);
        assert_eq!(store.read_json_pointer(&artifact_id, "/title").expect("title should exist").1, json!("新标题"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn repairs_invalid_json_with_unique_text_patch() {
        let (root, store, service) = test_service("text");
        let artifact_id = ArtifactId::new("result").expect("artifactId should be valid");
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Json, "{\n  \"name\": \"示例\"\n  \"enabled\": true\n}")
            .expect("invalid JSON draft should be stored");
        let result = service
            .apply_text_patch(TextPatchRequest {
                artifact_id: artifact_id.clone(),
                base_revision: 1,
                start_line: 2,
                end_line: 2,
                expected_text: "  \"name\": \"示例\"".to_string(),
                new_text: "  \"name\": \"示例\",".to_string(),
            })
            .expect("text patch should repair JSON");
        assert_eq!(result.artifact.metadata.revision, 2);
        assert!(store.read_json(&artifact_id).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn allows_text_patch_to_fix_multiple_json_errors_in_steps() {
        let (root, store, service) = test_service("multi-step-text");
        let artifact_id = ArtifactId::new("broken").expect("artifactId should be valid");
        store
            .create(
                artifact_id.clone(),
                ArtifactContentFormat::Json,
                "{\n  \"name\": \"示例\"\n  \"enabled\": true\n  \"count\": 1\n}",
            )
            .expect("invalid JSON draft should be stored");

        let first = service
            .apply_text_patch(TextPatchRequest {
                artifact_id: artifact_id.clone(),
                base_revision: 1,
                start_line: 2,
                end_line: 2,
                expected_text: "  \"name\": \"示例\"".to_string(),
                new_text: "  \"name\": \"示例\",".to_string(),
            })
            .expect("first partial repair should be committed");
        assert_eq!(first.artifact.metadata.revision, 2);
        assert!(store.read_json(&artifact_id).is_err());

        service
            .apply_text_patch(TextPatchRequest {
                artifact_id: artifact_id.clone(),
                base_revision: 2,
                start_line: 3,
                end_line: 3,
                expected_text: "  \"enabled\": true".to_string(),
                new_text: "  \"enabled\": true,".to_string(),
            })
            .expect("second repair should be committed");
        assert!(store.read_json(&artifact_id).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn patches_only_the_requested_duplicate_line() {
        let (root, store, service) = test_service("duplicate-line");
        let artifact_id = ArtifactId::new("text").expect("artifactId should be valid");
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Text, "same\nsame")
            .expect("artifact should be created");
        let result = service
            .apply_text_patch(TextPatchRequest {
                artifact_id: artifact_id.clone(),
                base_revision: 1,
                start_line: 2,
                end_line: 2,
                expected_text: "same".to_string(),
                new_text: "changed".to_string(),
            })
            .expect("line range should disambiguate duplicate text");
        assert_eq!(result.artifact.content, "same\nchanged");
        assert_eq!(result.artifact.metadata.revision, 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_changed_expected_line_without_writing() {
        let (root, store, service) = test_service("expected-text");
        let artifact_id = ArtifactId::new("text").expect("artifactId should be valid");
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Text, "first\nsecond")
            .expect("artifact should be created");
        let error = service
            .apply_text_patch(TextPatchRequest {
                artifact_id: artifact_id.clone(),
                base_revision: 1,
                start_line: 2,
                end_line: 2,
                expected_text: "stale".to_string(),
                new_text: "changed".to_string(),
            })
            .expect_err("stale expected text must be rejected");
        assert!(matches!(error, WorkspaceError::TextPatchContentMismatch(_)));
        assert_eq!(store.read(&artifact_id).unwrap().metadata.revision, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn empty_replacement_deletes_the_selected_line() {
        let (root, store, service) = test_service("delete-line");
        let artifact_id = ArtifactId::new("text").unwrap();
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Text, "first\nremove\nlast")
            .unwrap();
        let result = service
            .apply_text_patch(TextPatchRequest {
                artifact_id,
                base_revision: 1,
                start_line: 2,
                end_line: 2,
                expected_text: "remove".to_string(),
                new_text: String::new(),
            })
            .unwrap();
        assert_eq!(result.artifact.content, "first\nlast");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn empty_replacement_deletes_the_final_line_delimiter() {
        let (root, store, service) = test_service("delete-final-line");
        let artifact_id = ArtifactId::new("text").unwrap();
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Text, "first\r\nlast")
            .unwrap();
        let result = service
            .apply_text_patch(TextPatchRequest {
                artifact_id,
                base_revision: 1,
                start_line: 2,
                end_line: 2,
                expected_text: "last".to_string(),
                new_text: String::new(),
            })
            .unwrap();
        assert_eq!(result.artifact.content, "first");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_stale_json_patch() {
        let (root, store, service) = test_service("revision");
        let artifact_id = ArtifactId::new("sample").expect("artifactId should be valid");
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Json, r#"{"value":1}"#)
            .expect("artifact should be created");
        let error = service
            .apply_json_patch(JsonPatchRequest { artifact_id, base_revision: 0, operations: json_patch::Patch(Vec::new()) })
            .expect_err("stale patch must be rejected");
        assert!(matches!(error, WorkspaceError::RevisionConflict { actual: 1, .. }));
        let _ = fs::remove_dir_all(root);
    }
}
