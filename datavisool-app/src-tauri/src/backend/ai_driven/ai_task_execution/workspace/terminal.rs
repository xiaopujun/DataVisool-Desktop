/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 工作区产物的唯一终态提交服务。
//!
//! 模型不能直接推进产物状态。调用方必须指定读取过的 revision，服务对该快照执行
//! 注入的完整校验，并依靠 ArtifactStore 的 revision 检查原子推进到 `Accepted`。
//! 最终发布只接收本服务签发且仍对应当前 Accepted 版本的凭证。

use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

use super::artifact_store::ArtifactStore;
use super::manager::WorkspaceHandle;
use super::types::{ArtifactContentFormat, ArtifactId, ArtifactSnapshot, ArtifactStatus, WorkspaceError};

/// 完整校验返回的结构化问题。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceTerminalValidationIssue {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl WorkspaceTerminalValidationIssue {
    pub(crate) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), path: None }
    }

    pub(crate) fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

/// 业务模块注入的最终完整校验器；不能用模型自报完成替代。
pub(crate) trait WorkspaceTerminalValidator: Send + Sync {
    /// 返回全部语法、Schema 和业务问题；空列表是唯一通过条件。
    fn validate_complete(&self, artifact: &ArtifactSnapshot) -> Vec<WorkspaceTerminalValidationIssue>;
}

impl<F> WorkspaceTerminalValidator for F
where
    F: Fn(&ArtifactSnapshot) -> Vec<WorkspaceTerminalValidationIssue> + Send + Sync,
{
    fn validate_complete(&self, artifact: &ArtifactSnapshot) -> Vec<WorkspaceTerminalValidationIssue> {
        self(artifact)
    }
}

/// 工作区终态服务的稳定错误码。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum WorkspaceTerminalErrorCode {
    #[serde(rename = "workspace_terminal.artifact_not_found")]
    ArtifactNotFound,
    #[serde(rename = "workspace_terminal.revision_conflict")]
    RevisionConflict,
    #[serde(rename = "workspace_terminal.validation_failed")]
    ValidationFailed,
    #[serde(rename = "workspace_terminal.artifact_not_accepted")]
    ArtifactNotAccepted,
    #[serde(rename = "workspace_terminal.invalid_artifact_state")]
    InvalidArtifactState,
    #[serde(rename = "workspace_terminal.workspace_failure")]
    WorkspaceFailure,
}

impl WorkspaceTerminalErrorCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ArtifactNotFound => "workspace_terminal.artifact_not_found",
            Self::RevisionConflict => "workspace_terminal.revision_conflict",
            Self::ValidationFailed => "workspace_terminal.validation_failed",
            Self::ArtifactNotAccepted => "workspace_terminal.artifact_not_accepted",
            Self::InvalidArtifactState => "workspace_terminal.invalid_artifact_state",
            Self::WorkspaceFailure => "workspace_terminal.workspace_failure",
        }
    }
}

impl Display for WorkspaceTerminalErrorCode {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 提交或结束任务失败时返回的结构化错误。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceTerminalError {
    pub code: WorkspaceTerminalErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<WorkspaceTerminalValidationIssue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_revision: Option<u64>,
}

impl Display for WorkspaceTerminalError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for WorkspaceTerminalError {}

impl WorkspaceTerminalError {
    fn validation_failed(artifact_id: &ArtifactId, revision: u64, issues: Vec<WorkspaceTerminalValidationIssue>) -> Self {
        Self {
            code: WorkspaceTerminalErrorCode::ValidationFailed,
            message: format!("产物完整校验失败：artifactId={artifact_id}, revision={revision}"),
            issues,
            expected_revision: Some(revision),
            actual_revision: Some(revision),
        }
    }

    fn not_accepted(artifact_id: &ArtifactId, revision: u64, actual_status: ArtifactStatus) -> Self {
        Self {
            code: WorkspaceTerminalErrorCode::ArtifactNotAccepted,
            message: format!("最终发布只接受 Accepted 产物：artifactId={artifact_id}, revision={revision}, status={actual_status:?}"),
            issues: Vec::new(),
            expected_revision: Some(revision),
            actual_revision: Some(revision),
        }
    }

    fn from_workspace(error: WorkspaceError) -> Self {
        let message = error.to_string();
        match error {
            WorkspaceError::ArtifactNotFound(_) => Self {
                code: WorkspaceTerminalErrorCode::ArtifactNotFound,
                message,
                issues: Vec::new(),
                expected_revision: None,
                actual_revision: None,
            },
            WorkspaceError::RevisionConflict { expected, actual, .. } => Self {
                code: WorkspaceTerminalErrorCode::RevisionConflict,
                message,
                issues: Vec::new(),
                expected_revision: Some(expected),
                actual_revision: Some(actual),
            },
            _ => Self {
                code: WorkspaceTerminalErrorCode::WorkspaceFailure,
                message,
                issues: Vec::new(),
                expected_revision: None,
                actual_revision: None,
            },
        }
    }
}

/// 完整校验通过并成功推进到 Accepted 后签发的不可外部构造凭证。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcceptedArtifact {
    artifact_id: ArtifactId,
    revision: u64,
    content_format: ArtifactContentFormat,
    content_sha256: String,
}

impl AcceptedArtifact {
    pub(crate) fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn content_format(&self) -> ArtifactContentFormat {
        self.content_format
    }

    pub(crate) fn content_sha256(&self) -> &str {
        &self.content_sha256
    }
}

/// 宿主完成发布后可直接交给任务层的唯一成功结果。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FinishTaskResult {
    pub artifact_id: ArtifactId,
    pub revision: u64,
    pub artifact_status: ArtifactStatus,
    pub content_format: ArtifactContentFormat,
    pub content_sha256: String,
}

/// 绑定单个工作区和完整校验器的终态服务。
pub(crate) struct WorkspaceTerminalService<'a> {
    store: ArtifactStore,
    validator: &'a dyn WorkspaceTerminalValidator,
}

impl<'a> WorkspaceTerminalService<'a> {
    pub(crate) fn new(workspace: &WorkspaceHandle, validator: &'a dyn WorkspaceTerminalValidator) -> Self {
        Self::from_store(ArtifactStore::new(workspace), validator)
    }

    pub(crate) fn from_store(store: ArtifactStore, validator: &'a dyn WorkspaceTerminalValidator) -> Self {
        Self { store, validator }
    }

    /// 对指定 revision 执行完整校验，并以同一 revision 原子提交为 Accepted。
    pub(crate) fn commit_artifact(&self, artifact_id: &ArtifactId, expected_revision: u64) -> Result<AcceptedArtifact, WorkspaceTerminalError> {
        let snapshot = self.store.read(artifact_id).map_err(WorkspaceTerminalError::from_workspace)?;
        ensure_revision(&snapshot, expected_revision)?;
        let issues = self.validator.validate_complete(&snapshot);
        if !issues.is_empty() {
            return Err(WorkspaceTerminalError::validation_failed(artifact_id, expected_revision, issues));
        }

        // 项目 Workspace 可能保留上个 Turn 的 Accepted 结果。完整校验仍通过时，
        // 对精确 revision 重新签发内存凭证，不制造无意义的新 revision。
        if snapshot.metadata.status == ArtifactStatus::Accepted {
            let current = self.store.read(artifact_id).map_err(WorkspaceTerminalError::from_workspace)?;
            ensure_revision(&current, expected_revision)?;
            if current.metadata.status != ArtifactStatus::Accepted
                || current.metadata.content_sha256 != snapshot.metadata.content_sha256
                || current.metadata.content_format != snapshot.metadata.content_format
            {
                return Err(WorkspaceTerminalError {
                    code: WorkspaceTerminalErrorCode::InvalidArtifactState,
                    message: format!("Accepted 产物在重新签发凭证前发生变化：artifactId={artifact_id}, revision={expected_revision}"),
                    issues: Vec::new(),
                    expected_revision: Some(expected_revision),
                    actual_revision: Some(current.metadata.revision),
                });
            }
            return Ok(accepted_artifact(current));
        }

        if !matches!(snapshot.metadata.status, ArtifactStatus::Draft | ArtifactStatus::Valid | ArtifactStatus::Invalid) {
            return Err(WorkspaceTerminalError {
                code: WorkspaceTerminalErrorCode::InvalidArtifactState,
                message: format!(
                    "产物状态不能提交：artifactId={artifact_id}, revision={expected_revision}, status={:?}",
                    snapshot.metadata.status
                ),
                issues: Vec::new(),
                expected_revision: Some(expected_revision),
                actual_revision: Some(expected_revision),
            });
        }

        // update_status 自身再次检查 revision，阻止校验期间发生的并发写入被错误接受。
        let metadata = self
            .store
            .update_status(artifact_id, expected_revision, ArtifactStatus::Accepted)
            .map_err(WorkspaceTerminalError::from_workspace)?;
        Ok(AcceptedArtifact {
            artifact_id: metadata.artifact_id,
            revision: metadata.revision,
            content_format: metadata.content_format,
            content_sha256: metadata.content_sha256,
        })
    }

    /// 只有本服务签发且仍对应当前 Accepted revision 的产物才能形成宿主完成结果。
    pub(crate) fn finish_artifact(&self, accepted: &AcceptedArtifact) -> Result<FinishTaskResult, WorkspaceTerminalError> {
        let snapshot = self
            .store
            .read(&accepted.artifact_id)
            .map_err(WorkspaceTerminalError::from_workspace)?;
        ensure_revision(&snapshot, accepted.revision)?;
        if snapshot.metadata.status != ArtifactStatus::Accepted {
            return Err(WorkspaceTerminalError::not_accepted(&accepted.artifact_id, accepted.revision, snapshot.metadata.status));
        }
        if snapshot.metadata.content_sha256 != accepted.content_sha256 || snapshot.metadata.content_format != accepted.content_format {
            return Err(WorkspaceTerminalError {
                code: WorkspaceTerminalErrorCode::InvalidArtifactState,
                message: format!("Accepted 产物凭证与当前元数据不一致：artifactId={}, revision={}", accepted.artifact_id, accepted.revision),
                issues: Vec::new(),
                expected_revision: Some(accepted.revision),
                actual_revision: Some(snapshot.metadata.revision),
            });
        }

        Ok(FinishTaskResult {
            artifact_id: snapshot.metadata.artifact_id,
            revision: snapshot.metadata.revision,
            artifact_status: snapshot.metadata.status,
            content_format: snapshot.metadata.content_format,
            content_sha256: snapshot.metadata.content_sha256,
        })
    }
}

fn accepted_artifact(snapshot: ArtifactSnapshot) -> AcceptedArtifact {
    AcceptedArtifact {
        artifact_id: snapshot.metadata.artifact_id,
        revision: snapshot.metadata.revision,
        content_format: snapshot.metadata.content_format,
        content_sha256: snapshot.metadata.content_sha256,
    }
}

fn ensure_revision(snapshot: &ArtifactSnapshot, expected_revision: u64) -> Result<(), WorkspaceTerminalError> {
    if snapshot.metadata.revision == expected_revision {
        return Ok(());
    }
    Err(WorkspaceTerminalError::from_workspace(WorkspaceError::RevisionConflict {
        artifact_id: snapshot.metadata.artifact_id.to_string(),
        expected: expected_revision,
        actual: snapshot.metadata.revision,
    }))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::backend::ai_driven::ai_task_execution::workspace::manager::WorkspaceManager;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    struct TestWorkspace {
        root: std::path::PathBuf,
        artifact_id: ArtifactId,
        store: ArtifactStore,
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn test_workspace(name: &str) -> TestWorkspace {
        let root = std::env::temp_dir().join(format!(
            "datavisool-workspace-terminal-{name}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = WorkspaceManager::new(&root)
            .create("run-1")
            .expect("workspace should be created");
        let artifact_id = ArtifactId::new("result").expect("artifactId should be valid");
        let store = ArtifactStore::new(&workspace);
        store
            .create(artifact_id.clone(), ArtifactContentFormat::Json, r#"{"value":1}"#)
            .expect("artifact should be created");
        TestWorkspace { root, artifact_id, store }
    }

    struct AcceptingValidator;

    impl WorkspaceTerminalValidator for AcceptingValidator {
        fn validate_complete(&self, _artifact: &ArtifactSnapshot) -> Vec<WorkspaceTerminalValidationIssue> {
            Vec::new()
        }
    }

    struct RejectingValidator;

    impl WorkspaceTerminalValidator for RejectingValidator {
        fn validate_complete(&self, _artifact: &ArtifactSnapshot) -> Vec<WorkspaceTerminalValidationIssue> {
            vec![WorkspaceTerminalValidationIssue::new("page.schema.invalid", "页面结构不完整").with_path("/components")]
        }
    }

    #[test]
    fn commit_accepts_only_the_requested_revision() {
        let workspace = test_workspace("revision");
        let service = WorkspaceTerminalService::from_store(workspace.store.clone(), &AcceptingValidator);

        let error = service.commit_artifact(&workspace.artifact_id, 0).unwrap_err();
        assert_eq!(error.code, WorkspaceTerminalErrorCode::RevisionConflict);
        assert_eq!(error.expected_revision, Some(0));
        assert_eq!(error.actual_revision, Some(1));
        let current = workspace.store.read(&workspace.artifact_id).unwrap();
        assert_eq!(current.metadata.status, ArtifactStatus::Draft);
        assert_eq!(current.metadata.revision, 1);
    }

    #[test]
    fn validation_failure_does_not_accept_or_change_revision() {
        let workspace = test_workspace("validation-failure");
        let service = WorkspaceTerminalService::from_store(workspace.store.clone(), &RejectingValidator);

        let error = service.commit_artifact(&workspace.artifact_id, 1).unwrap_err();
        assert_eq!(error.code, WorkspaceTerminalErrorCode::ValidationFailed);
        assert_eq!(error.issues.len(), 1);
        assert_eq!(error.issues[0].code, "page.schema.invalid");
        let current = workspace.store.read(&workspace.artifact_id).unwrap();
        assert_eq!(current.metadata.status, ArtifactStatus::Draft);
        assert_eq!(current.metadata.revision, 1);
    }

    #[test]
    fn successful_commit_signs_the_accepted_revision() {
        let workspace = test_workspace("commit");
        let service = WorkspaceTerminalService::from_store(workspace.store.clone(), &AcceptingValidator);

        let accepted = service.commit_artifact(&workspace.artifact_id, 1).unwrap();
        assert_eq!(accepted.artifact_id(), &workspace.artifact_id);
        assert_eq!(accepted.revision(), 2);
        let current = workspace.store.read(&workspace.artifact_id).unwrap();
        assert_eq!(current.metadata.status, ArtifactStatus::Accepted);
        assert_eq!(current.metadata.revision, 2);
    }

    #[test]
    fn commit_resigns_an_exact_accepted_revision_without_incrementing() {
        let workspace = test_workspace("resume-accepted");
        let service = WorkspaceTerminalService::from_store(workspace.store.clone(), &AcceptingValidator);
        let first = service.commit_artifact(&workspace.artifact_id, 1).unwrap();

        let resumed = service.commit_artifact(&workspace.artifact_id, first.revision()).unwrap();

        assert_eq!(resumed, first);
        assert_eq!(workspace.store.read(&workspace.artifact_id).unwrap().metadata.revision, 2);
    }

    #[test]
    fn finalization_rejects_an_artifact_that_is_not_accepted() {
        let workspace = test_workspace("finish-before-accept");
        let snapshot = workspace.store.read(&workspace.artifact_id).unwrap();
        let forged_for_precondition_test = AcceptedArtifact {
            artifact_id: snapshot.metadata.artifact_id,
            revision: snapshot.metadata.revision,
            content_format: snapshot.metadata.content_format,
            content_sha256: snapshot.metadata.content_sha256,
        };
        let service = WorkspaceTerminalService::from_store(workspace.store.clone(), &AcceptingValidator);

        let error = service.finish_artifact(&forged_for_precondition_test).unwrap_err();
        assert_eq!(error.code, WorkspaceTerminalErrorCode::ArtifactNotAccepted);
    }

    #[test]
    fn finalization_returns_structured_accepted_artifact() {
        let workspace = test_workspace("finish");
        let service = WorkspaceTerminalService::from_store(workspace.store.clone(), &AcceptingValidator);
        let accepted = service.commit_artifact(&workspace.artifact_id, 1).unwrap();

        let result = service.finish_artifact(&accepted).unwrap();
        assert_eq!(result.artifact_id, workspace.artifact_id);
        assert_eq!(result.revision, 2);
        assert_eq!(result.artifact_status, ArtifactStatus::Accepted);
        assert_eq!(result.content_sha256, accepted.content_sha256());
        assert_eq!(result.content_format, accepted.content_format());
    }

    #[test]
    fn finalization_rejects_an_accepted_revision_that_was_later_changed() {
        let workspace = test_workspace("finish-stale");
        let service = WorkspaceTerminalService::from_store(workspace.store.clone(), &AcceptingValidator);
        let accepted = service.commit_artifact(&workspace.artifact_id, 1).unwrap();
        workspace
            .store
            .replace(&workspace.artifact_id, accepted.revision(), r#"{"value":2}"#)
            .unwrap();

        let error = service.finish_artifact(&accepted).unwrap_err();
        assert_eq!(error.code, WorkspaceTerminalErrorCode::RevisionConflict);
        assert_eq!(error.expected_revision, Some(2));
        assert_eq!(error.actual_revision, Some(3));
    }

    struct RacingValidator {
        store: ArtifactStore,
        artifact_id: ArtifactId,
    }

    impl WorkspaceTerminalValidator for RacingValidator {
        fn validate_complete(&self, artifact: &ArtifactSnapshot) -> Vec<WorkspaceTerminalValidationIssue> {
            self.store
                .replace(&self.artifact_id, artifact.metadata.revision, r#"{"value":2}"#)
                .expect("test should update revision during validation");
            Vec::new()
        }
    }

    #[test]
    fn concurrent_write_during_validation_cannot_be_accepted() {
        let workspace = test_workspace("validation-race");
        let validator = RacingValidator { store: workspace.store.clone(), artifact_id: workspace.artifact_id.clone() };
        let service = WorkspaceTerminalService::from_store(workspace.store.clone(), &validator);

        let error = service.commit_artifact(&workspace.artifact_id, 1).unwrap_err();
        assert_eq!(error.code, WorkspaceTerminalErrorCode::RevisionConflict);
        assert_eq!(error.actual_revision, Some(2));
        let current = workspace.store.read(&workspace.artifact_id).unwrap();
        assert_eq!(current.metadata.status, ArtifactStatus::Draft);
    }

    #[test]
    fn terminal_error_codes_have_stable_wire_values() {
        assert_eq!(WorkspaceTerminalErrorCode::ArtifactNotFound.as_str(), "workspace_terminal.artifact_not_found");
        assert_eq!(WorkspaceTerminalErrorCode::RevisionConflict.as_str(), "workspace_terminal.revision_conflict");
        assert_eq!(WorkspaceTerminalErrorCode::ValidationFailed.as_str(), "workspace_terminal.validation_failed");
        assert_eq!(WorkspaceTerminalErrorCode::ArtifactNotAccepted.as_str(), "workspace_terminal.artifact_not_accepted");
        assert_eq!(WorkspaceTerminalErrorCode::InvalidArtifactState.as_str(), "workspace_terminal.invalid_artifact_state");
        assert_eq!(WorkspaceTerminalErrorCode::WorkspaceFailure.as_str(), "workspace_terminal.workspace_failure");
    }
}
