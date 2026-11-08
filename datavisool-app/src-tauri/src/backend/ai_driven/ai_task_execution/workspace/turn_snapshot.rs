/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 项目 Workspace 内按 Turn 保存的不可变执行快照。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use super::artifact_store::ArtifactStore;
use super::file_edit::WorkspaceFileTraceSnapshot;
use super::layout::WorkspaceLayout;
use super::manager::{read_json_if_exists, read_manifest, write_bytes_create_once, WorkspaceHandle};
use super::types::{content_sha256, is_safe_name, ArtifactId, ArtifactSnapshot, WorkspaceError, WorkspaceFileMetadata, WorkspaceResult};
use crate::backend::ai_driven::time::now_ms;

const TURN_SNAPSHOT_SCHEMA_VERSION: u32 = 2;
const RESULT_ARTIFACT_ID: &str = "result";

/// 单次 Run 在 Thread/Turn 下的稳定身份。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnSnapshotIdentity {
    pub project_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub task_id: String,
    pub run_id: String,
}

impl TurnSnapshotIdentity {
    pub(crate) fn new(
        project_id: i64,
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        task_id: impl Into<String>,
        run_id: impl Into<String>,
    ) -> WorkspaceResult<Self> {
        let identity = Self {
            project_id,
            thread_id: thread_id.into(),
            turn_id: turn_id.into(),
            task_id: task_id.into(),
            run_id: run_id.into(),
        };
        validate_identity(&identity)?;
        Ok(identity)
    }
}

/// 一个 Turn 首次开始执行时捕获的请求和 working head。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnBaseSnapshot {
    pub schema_version: u32,
    pub identity: TurnSnapshotIdentity,
    pub request: Value,
    #[serde(default)]
    pub artifact: Option<ArtifactSnapshot>,
    pub workspace_base_revision: u64,
    pub workspace_files: Vec<WorkspaceFileMetadata>,
    pub index_sha256: Option<String>,
    pub workspace_snapshot_ref: String,
    pub captured_at: i64,
}

/// 单次 Run 写入 Turn outcome 时使用的稳定状态。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TurnSnapshotOutcomeStatus {
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

/// 单次 Run 结束时捕获的 working head。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnOutcomeSnapshot {
    pub schema_version: u32,
    pub identity: TurnSnapshotIdentity,
    pub status: TurnSnapshotOutcomeStatus,
    pub artifact: Option<ArtifactSnapshot>,
    pub workspace_base_revision: u64,
    pub workspace_result_revision: u64,
    pub index_sha256: Option<String>,
    pub workspace_snapshot_ref: String,
    pub file_trace: WorkspaceFileTraceSnapshot,
    pub captured_at: i64,
}

/// 绑定一个项目 Workspace 的 Turn 快照仓库。
#[derive(Clone, Debug)]
pub(crate) struct TurnSnapshotStore {
    layout: WorkspaceLayout,
}

impl TurnSnapshotStore {
    pub(crate) fn new(workspace: &WorkspaceHandle) -> Self {
        Self { layout: workspace.layout.clone() }
    }

    /// create-once 保存请求与 Turn 起点，并返回可克隆的 Run journal。
    pub(crate) fn begin_turn(&self, identity: TurnSnapshotIdentity, request: Value, base: ArtifactSnapshot) -> WorkspaceResult<TurnSnapshotJournal> {
        self.begin_turn_optional(identity, request, Some(base))
    }

    /// 新架构只记录 Workspace 基线，不创建强制 Result Artifact。
    pub(crate) fn begin_turn_without_artifact(&self, identity: TurnSnapshotIdentity, request: Value) -> WorkspaceResult<TurnSnapshotJournal> {
        self.begin_turn_optional(identity, request, None)
    }

    fn begin_turn_optional(
        &self,
        identity: TurnSnapshotIdentity,
        request: Value,
        base: Option<ArtifactSnapshot>,
    ) -> WorkspaceResult<TurnSnapshotJournal> {
        validate_identity(&identity)?;
        let directory = self.ensure_turn_directory(&identity.turn_id)?;
        let path = directory.join("base.json");
        let existing = if let Some(existing) = read_record::<TurnBaseSnapshot>(&path, "readTurnBaseSnapshot")? {
            existing
        } else {
            let workspace_manifest = read_manifest(&self.layout)?;
            let proposed = TurnBaseSnapshot {
                schema_version: TURN_SNAPSHOT_SCHEMA_VERSION,
                identity: identity.clone(),
                request: request.clone(),
                artifact: base,
                workspace_base_revision: workspace_manifest.workspace_revision,
                workspace_files: workspace_manifest.files,
                index_sha256: current_index_sha256(&self.layout)?,
                workspace_snapshot_ref: snapshot_ref(&identity.turn_id, "base.json"),
                captured_at: now_ms(),
            };
            if write_record_create_once(&path, &proposed, "serializeTurnBaseSnapshot")? {
                proposed
            } else {
                read_record(&path, "readTurnBaseSnapshotAfterRace")?
                    .ok_or_else(|| WorkspaceError::InvalidManifest(format!("Turn base snapshot disappeared after concurrent creation: {path:?}")))?
            }
        };
        validate_existing_base(&existing, &identity, &request)?;
        Ok(TurnSnapshotJournal {
            inner: Arc::new(TurnSnapshotJournalInner { store: self.clone(), identity, base: existing, finished: AtomicBool::new(false) }),
        })
    }

    pub(crate) fn read_base(&self, turn_id: &str) -> WorkspaceResult<Option<TurnBaseSnapshot>> {
        validate_snapshot_id("turnId", turn_id)?;
        read_record(&self.turn_directory(turn_id).join("base.json"), "readTurnBaseSnapshot")
    }

    pub(crate) fn read_outcome(&self, turn_id: &str, run_id: &str) -> WorkspaceResult<Option<TurnOutcomeSnapshot>> {
        validate_snapshot_id("turnId", turn_id)?;
        validate_snapshot_id("runId", run_id)?;
        read_record(&self.turn_directory(turn_id).join(format!("outcome-{run_id}.json")), "readTurnOutcomeSnapshot")
    }

    /// 将指定 Turn 的 `result` 基线内容恢复为 working head 的新版本。
    ///
    /// 该操作复用 ArtifactStore 的版本检查：成功时 revision 只增不减且状态回到 Draft；
    /// 传入过期 revision 时不会写入任何内容。
    pub(crate) fn restore_result_as_new_revision(
        &self,
        turn_id: &str,
        store: &ArtifactStore,
        expected_current_revision: u64,
    ) -> WorkspaceResult<ArtifactSnapshot> {
        let base = self
            .read_base(turn_id)?
            .ok_or_else(|| WorkspaceError::InvalidManifest(format!("Turn base snapshot does not exist: turnId={turn_id}")))?;
        let result_id = ArtifactId::new(RESULT_ARTIFACT_ID).expect("fixed result artifactId must be valid");
        let base_artifact = base
            .artifact
            .ok_or_else(|| WorkspaceError::InvalidManifest(format!("Turn base has no legacy result artifact: turnId={turn_id}")))?;
        if base_artifact.metadata.artifact_id != result_id {
            return Err(WorkspaceError::InvalidManifest(format!(
                "Turn base artifactId mismatch: turnId={turn_id}, expected={result_id}, actual={}",
                base_artifact.metadata.artifact_id
            )));
        }
        if content_sha256(base_artifact.content.as_bytes()) != base_artifact.metadata.content_sha256 {
            return Err(WorkspaceError::InvalidManifest(format!(
                "Turn base content hash mismatch: turnId={turn_id}, artifactId={result_id}"
            )));
        }
        let current = store.read(&result_id)?;
        if current.metadata.content_format != base_artifact.metadata.content_format {
            return Err(WorkspaceError::InvalidManifest(format!(
                "Turn base contentFormat mismatch: turnId={turn_id}, artifactId={result_id}"
            )));
        }
        store.replace(&result_id, expected_current_revision, base_artifact.content)
    }

    fn write_outcome(
        &self,
        identity: &TurnSnapshotIdentity,
        status: TurnSnapshotOutcomeStatus,
        artifact: Option<ArtifactSnapshot>,
        file_trace: WorkspaceFileTraceSnapshot,
    ) -> WorkspaceResult<TurnOutcomeSnapshot> {
        validate_identity(identity)?;
        let directory = self.ensure_turn_directory(&identity.turn_id)?;
        let path = directory.join(format!("outcome-{}.json", identity.run_id));
        let current_manifest = read_manifest(&self.layout)?;
        let workspace_result_revision = current_manifest.workspace_revision;
        let file_name = format!("outcome-{}.json", identity.run_id);
        let base = self
            .read_base(&identity.turn_id)?
            .ok_or_else(|| WorkspaceError::InvalidManifest(format!("Turn base snapshot does not exist: turnId={}", identity.turn_id)))?;
        let file_trace = merge_manifest_changes(file_trace, &base.workspace_files, &current_manifest.files);
        let proposed = TurnOutcomeSnapshot {
            schema_version: TURN_SNAPSHOT_SCHEMA_VERSION,
            identity: identity.clone(),
            status,
            artifact,
            workspace_base_revision: base.workspace_base_revision,
            workspace_result_revision,
            index_sha256: current_index_sha256(&self.layout)?,
            workspace_snapshot_ref: snapshot_ref(&identity.turn_id, &file_name),
            file_trace,
            captured_at: now_ms(),
        };
        if write_record_create_once(&path, &proposed, "serializeTurnOutcomeSnapshot")? {
            return Ok(proposed);
        }
        let existing = read_record::<TurnOutcomeSnapshot>(&path, "readTurnOutcomeSnapshotAfterRace")?
            .ok_or_else(|| WorkspaceError::InvalidManifest(format!("Turn outcome snapshot disappeared after concurrent creation: {path:?}")))?;
        if existing.schema_version != TURN_SNAPSHOT_SCHEMA_VERSION
            || existing.identity != *identity
            || existing.status != status
            || existing.artifact != proposed.artifact
            || existing.workspace_base_revision != proposed.workspace_base_revision
            || existing.workspace_result_revision != proposed.workspace_result_revision
            || existing.index_sha256 != proposed.index_sha256
            || existing.workspace_snapshot_ref != proposed.workspace_snapshot_ref
            || existing.file_trace != proposed.file_trace
        {
            return Err(WorkspaceError::InvalidManifest(format!(
                "Turn outcome snapshot conflicts with existing run outcome: turnId={}, runId={}",
                identity.turn_id, identity.run_id
            )));
        }
        Ok(existing)
    }

    fn ensure_turn_directory(&self, turn_id: &str) -> WorkspaceResult<PathBuf> {
        validate_snapshot_id("turnId", turn_id)?;
        ensure_regular_directory(&self.layout.turn_snapshots_dir(), "createTurnSnapshotsDirectory")?;
        let directory = self.turn_directory(turn_id);
        ensure_regular_directory(&directory, "createTurnSnapshotDirectory")?;
        Ok(directory)
    }

    fn turn_directory(&self, turn_id: &str) -> PathBuf {
        self.layout.turn_snapshots_dir().join(turn_id)
    }
}

/// `begin_turn` 返回的单次 Run 快照句柄。
#[derive(Clone, Debug)]
pub(crate) struct TurnSnapshotJournal {
    inner: Arc<TurnSnapshotJournalInner>,
}

#[derive(Debug)]
struct TurnSnapshotJournalInner {
    store: TurnSnapshotStore,
    identity: TurnSnapshotIdentity,
    base: TurnBaseSnapshot,
    finished: AtomicBool,
}

impl TurnSnapshotJournal {
    pub(crate) fn base(&self) -> &TurnBaseSnapshot {
        &self.inner.base
    }

    /// create-once 保存当前 Run 的最终 working head。
    pub(crate) fn finish(&self, status: TurnSnapshotOutcomeStatus, current: Option<ArtifactSnapshot>) -> WorkspaceResult<TurnOutcomeSnapshot> {
        self.finish_with_workspace_trace(status, current, WorkspaceFileTraceSnapshot::default())
    }

    pub(crate) fn finish_with_workspace_trace(
        &self,
        status: TurnSnapshotOutcomeStatus,
        current: Option<ArtifactSnapshot>,
        file_trace: WorkspaceFileTraceSnapshot,
    ) -> WorkspaceResult<TurnOutcomeSnapshot> {
        // 一旦调用方已选择明确终态，即使后续 I/O 失败也不能在 Drop 时把它误记为 Interrupted。
        self.inner.finished.store(true, Ordering::Release);
        self.inner
            .store
            .write_outcome(&self.inner.identity, status, current, file_trace)
    }
}

impl Drop for TurnSnapshotJournalInner {
    fn drop(&mut self) {
        if self.finished.load(Ordering::Acquire) {
            return;
        }
        if let Err(error) =
            self.store
                .write_outcome(&self.identity, TurnSnapshotOutcomeStatus::Interrupted, None, WorkspaceFileTraceSnapshot::default())
        {
            tracing::error!(
                error = %error,
                turn_id = %self.identity.turn_id,
                run_id = %self.identity.run_id,
                "failed to persist interrupted Turn workspace outcome on journal drop"
            );
        }
    }
}

fn validate_identity(identity: &TurnSnapshotIdentity) -> WorkspaceResult<()> {
    if identity.project_id <= 0 {
        return Err(WorkspaceError::InvalidWorkspaceId(format!("projectId={}", identity.project_id)));
    }
    for (field, value) in [
        ("threadId", identity.thread_id.as_str()),
        ("turnId", identity.turn_id.as_str()),
        ("taskId", identity.task_id.as_str()),
        ("runId", identity.run_id.as_str()),
    ] {
        validate_snapshot_id(field, value)?;
    }
    Ok(())
}

fn validate_snapshot_id(field: &str, value: &str) -> WorkspaceResult<()> {
    if !is_safe_name(value, 160) {
        return Err(WorkspaceError::InvalidWorkspaceId(format!("{field}={value}")));
    }
    Ok(())
}

fn validate_existing_base(existing: &TurnBaseSnapshot, identity: &TurnSnapshotIdentity, request: &Value) -> WorkspaceResult<()> {
    let same_turn = existing.schema_version == TURN_SNAPSHOT_SCHEMA_VERSION
        && existing.identity.project_id == identity.project_id
        && existing.identity.thread_id == identity.thread_id
        && existing.identity.turn_id == identity.turn_id;
    if !same_turn || existing.request != *request {
        return Err(WorkspaceError::InvalidManifest(format!(
            "Turn base snapshot conflicts with persisted request: turnId={}",
            identity.turn_id
        )));
    }
    Ok(())
}

fn current_index_sha256(layout: &WorkspaceLayout) -> WorkspaceResult<Option<String>> {
    let path = layout.index_file();
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(WorkspaceError::Storage { operation: "inspectTurnWorkspaceIndex", path: path.clone(), message: error.to_string() }),
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(WorkspaceError::InvalidIndex("INDEX.json must be a regular file".to_string()));
    }
    let bytes = fs::read(&path).map_err(|error| WorkspaceError::Storage { operation: "readTurnWorkspaceIndex", path, message: error.to_string() })?;
    Ok(Some(content_sha256(&bytes)))
}

fn snapshot_ref(turn_id: &str, file_name: &str) -> String {
    format!("runtime/turn-snapshots/{turn_id}/{file_name}")
}

fn merge_manifest_changes(
    mut trace: WorkspaceFileTraceSnapshot,
    base_files: &[WorkspaceFileMetadata],
    current_files: &[WorkspaceFileMetadata],
) -> WorkspaceFileTraceSnapshot {
    let base = base_files
        .iter()
        .map(|file| (file.path.clone(), file))
        .collect::<BTreeMap<_, _>>();
    let current = current_files
        .iter()
        .map(|file| (file.path.clone(), file))
        .collect::<BTreeMap<_, _>>();
    let mut added = trace.added_files.into_iter().collect::<BTreeSet<_>>();
    let mut modified = trace.modified_files.into_iter().collect::<BTreeSet<_>>();
    let mut deleted = trace.deleted_files.into_iter().collect::<BTreeSet<_>>();
    for (path, current_file) in &current {
        match base.get(path) {
            None => {
                added.insert(path.clone());
            }
            Some(base_file) if base_file.content_sha256 != current_file.content_sha256 => {
                modified.insert(path.clone());
            }
            _ => {}
        }
    }
    for path in base.keys() {
        if !current.contains_key(path) {
            deleted.insert(path.clone());
        }
    }
    trace.added_files = added.into_iter().collect();
    trace.modified_files = modified.into_iter().collect();
    trace.deleted_files = deleted.into_iter().collect();
    trace
}

fn ensure_regular_directory(path: &Path, operation: &'static str) -> WorkspaceResult<()> {
    if !path.exists() {
        fs::create_dir_all(path).map_err(|error| WorkspaceError::Storage { operation, path: path.to_path_buf(), message: error.to_string() })?;
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|error| WorkspaceError::Storage { operation, path: path.to_path_buf(), message: error.to_string() })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(WorkspaceError::InvalidManifest(format!("Turn snapshot path is not a regular directory: {path:?}")));
    }
    Ok(())
}

fn write_record_create_once<T: Serialize>(path: &Path, value: &T, operation: &'static str) -> WorkspaceResult<bool> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| WorkspaceError::Storage {
        operation,
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    write_bytes_create_once(path, &bytes)
}

fn read_record<T: DeserializeOwned>(path: &Path, operation: &'static str) -> WorkspaceResult<Option<T>> {
    read_json_if_exists(path, operation)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::backend::ai_driven::ai_task_execution::workspace::{
        ArtifactContentFormat, ArtifactId, ArtifactStatus, ArtifactStore, WorkspaceManager,
    };

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn test_workspace(name: &str) -> (PathBuf, WorkspaceHandle, ArtifactId) {
        let root =
            std::env::temp_dir().join(format!("datavisool-turn-snapshot-{name}-{}-{}", std::process::id(), TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)));
        let workspace = WorkspaceManager::new(&root).create("workspace").unwrap();
        let artifact_id = ArtifactId::new("result").unwrap();
        ArtifactStore::new(&workspace)
            .create(artifact_id.clone(), ArtifactContentFormat::Json, r#"{"value":1}"#)
            .unwrap();
        (root, workspace, artifact_id)
    }

    fn identity(turn_id: &str, run_id: &str) -> TurnSnapshotIdentity {
        TurnSnapshotIdentity::new(1, "thread-1", turn_id, format!("task-{run_id}"), run_id).unwrap()
    }

    #[test]
    fn turn_base_is_create_once_and_outcomes_are_isolated_by_run() {
        let (root, workspace, artifact_id) = test_workspace("lifecycle");
        let store = ArtifactStore::new(&workspace);
        let snapshots = TurnSnapshotStore::new(&workspace);
        let base = store.read(&artifact_id).unwrap();
        let first = snapshots
            .begin_turn(identity("turn-1", "run-1"), serde_json::json!({ "requirement": "first" }), base.clone())
            .unwrap();
        let current = store.replace(&artifact_id, base.metadata.revision, r#"{"value":2}"#).unwrap();

        let reopened = snapshots
            .begin_turn(identity("turn-1", "run-2"), serde_json::json!({ "requirement": "first" }), current.clone())
            .unwrap();
        let completed = first
            .finish(TurnSnapshotOutcomeStatus::Completed, Some(current.clone()))
            .unwrap();
        let failed = reopened
            .finish(TurnSnapshotOutcomeStatus::Failed, Some(current.clone()))
            .unwrap();

        assert_eq!(first.base().artifact.as_ref().unwrap().metadata.revision, 1);
        assert_eq!(reopened.base().artifact.as_ref().unwrap().metadata.revision, 1);
        assert_eq!(completed.identity.run_id, "run-1");
        assert_eq!(completed.workspace_base_revision, 1);
        assert_eq!(completed.workspace_result_revision, 2);
        assert!(completed
            .file_trace
            .modified_files
            .iter()
            .any(|path| path.as_str() == "artifacts/result.json"));
        assert!(completed.index_sha256.is_some());
        assert_eq!(completed.workspace_snapshot_ref, "runtime/turn-snapshots/turn-1/outcome-run-1.json");
        assert_eq!(failed.identity.run_id, "run-2");
        assert_eq!(snapshots.read_base("turn-1").unwrap().unwrap().artifact, Some(base));
        assert_eq!(snapshots.read_outcome("turn-1", "run-1").unwrap().unwrap(), completed);
        assert_eq!(snapshots.read_outcome("turn-1", "run-2").unwrap().unwrap(), failed);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn different_turns_keep_independent_requests_bases_and_outcomes() {
        let (root, workspace, artifact_id) = test_workspace("turn-isolation");
        let store = ArtifactStore::new(&workspace);
        let snapshots = TurnSnapshotStore::new(&workspace);
        let first_base = store.read(&artifact_id).unwrap();
        let first = snapshots
            .begin_turn(identity("turn-1", "run-1"), serde_json::json!({ "turn": 1 }), first_base)
            .unwrap();
        let second_base = store.replace(&artifact_id, 1, r#"{"value":2}"#).unwrap();
        let second = snapshots
            .begin_turn(identity("turn-2", "run-2"), serde_json::json!({ "turn": 2 }), second_base)
            .unwrap();
        first.finish(TurnSnapshotOutcomeStatus::Completed, None).unwrap();
        second.finish(TurnSnapshotOutcomeStatus::Cancelled, None).unwrap();

        assert_eq!(snapshots.read_base("turn-1").unwrap().unwrap().request["turn"], 1);
        assert_eq!(snapshots.read_base("turn-2").unwrap().unwrap().request["turn"], 2);
        assert_eq!(snapshots.read_outcome("turn-1", "run-1").unwrap().unwrap().status, TurnSnapshotOutcomeStatus::Completed);
        assert_eq!(snapshots.read_outcome("turn-2", "run-2").unwrap().unwrap().status, TurnSnapshotOutcomeStatus::Cancelled);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_conflicting_request_and_unsafe_ids() {
        let (root, workspace, artifact_id) = test_workspace("validation");
        let snapshots = TurnSnapshotStore::new(&workspace);
        let base = ArtifactStore::new(&workspace).read(&artifact_id).unwrap();
        snapshots
            .begin_turn(identity("turn-1", "run-1"), serde_json::json!({ "value": 1 }), base.clone())
            .unwrap();
        let error = snapshots
            .begin_turn(identity("turn-1", "run-2"), serde_json::json!({ "value": 2 }), base)
            .unwrap_err();

        assert!(matches!(error, WorkspaceError::InvalidManifest(_)));
        assert!(TurnSnapshotIdentity::new(1, "thread-1", "../turn", "task-1", "run-1").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn restore_creates_a_new_draft_revision_and_stale_revisions_always_conflict() {
        let (root, workspace, artifact_id) = test_workspace("restore");
        let store = ArtifactStore::new(&workspace);
        let snapshots = TurnSnapshotStore::new(&workspace);
        let base = store.read(&artifact_id).unwrap();
        snapshots
            .begin_turn(identity("turn-1", "run-1"), serde_json::json!({ "value": 1 }), base.clone())
            .unwrap();
        let changed = store.replace(&artifact_id, base.metadata.revision, r#"{"value":2}"#).unwrap();

        let stale = snapshots
            .restore_result_as_new_revision("turn-1", &store, base.metadata.revision)
            .unwrap_err();
        assert!(matches!(stale, WorkspaceError::RevisionConflict { expected: 1, actual: 2, .. }));
        assert_eq!(store.read(&artifact_id).unwrap(), changed);

        let restored = snapshots
            .restore_result_as_new_revision("turn-1", &store, changed.metadata.revision)
            .unwrap();
        assert_eq!(restored.metadata.revision, 3);
        assert_eq!(restored.metadata.status, ArtifactStatus::Draft);
        assert_eq!(restored.content, base.content);

        let stale_again = snapshots
            .restore_result_as_new_revision("turn-1", &store, changed.metadata.revision)
            .unwrap_err();
        assert!(matches!(stale_again, WorkspaceError::RevisionConflict { expected: 2, actual: 3, .. }));
        assert_eq!(store.read(&artifact_id).unwrap(), restored);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn dropping_the_last_unfinished_journal_records_interrupted_once() {
        let (root, workspace, artifact_id) = test_workspace("drop-interrupted");
        let snapshots = TurnSnapshotStore::new(&workspace);
        let base = ArtifactStore::new(&workspace).read(&artifact_id).unwrap();
        let journal = snapshots
            .begin_turn(identity("turn-1", "run-1"), serde_json::json!({ "value": 1 }), base)
            .unwrap();
        let clone = journal.clone();

        drop(journal);
        assert!(snapshots.read_outcome("turn-1", "run-1").unwrap().is_none());
        drop(clone);

        let outcome = snapshots.read_outcome("turn-1", "run-1").unwrap().unwrap();
        assert_eq!(outcome.status, TurnSnapshotOutcomeStatus::Interrupted);
        assert!(outcome.artifact.is_none());
        let _ = fs::remove_dir_all(root);
    }
}
