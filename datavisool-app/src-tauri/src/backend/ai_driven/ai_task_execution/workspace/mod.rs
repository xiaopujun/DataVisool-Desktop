/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 任务执行过程使用的文件存储基础设施。
//!
//! 该模块提供隔离目录、逻辑产物寻址、版本检查、原子写入、局部补丁和诊断记录，
//! 不包含具体业务结构和目录。
#![allow(dead_code, unused_imports)]

pub(crate) const WORKSPACE_TEMP_MARKER: &str = ".datavisool-tmp-";

mod artifact_store;
mod diagnostic;
mod file_edit;
mod index;
mod layout;
mod manager;
mod patch;
mod search;
mod snapshot;
mod terminal;
mod turn_snapshot;
mod types;

pub(crate) use artifact_store::ArtifactStore;
pub(crate) use diagnostic::DiagnosticStore;
pub(crate) use file_edit::{
    WorkspaceApplyPatchResult, WorkspaceCapturedFile, WorkspaceFileExcerpt, WorkspaceFileLimits, WorkspaceFileListEntry, WorkspaceFileService,
    WorkspaceFileTrace, WorkspaceFileTraceSnapshot, WorkspaceMovedFile,
};
pub(crate) use index::{
    read_and_validate_index, scan_workspace_files, validate_index, validate_index_against_paths, WorkspaceIndex, WorkspaceIndexEntry,
    WorkspaceIndexRole, WorkspaceIndexTurn, WorkspaceIndexTurnStatus, WorkspaceIndexValidationMode, WORKSPACE_INDEX_FILE,
    WORKSPACE_INDEX_SCHEMA_VERSION,
};
pub(crate) use layout::WorkspaceLayout;
pub(crate) use manager::{WorkspaceHandle, WorkspaceManager};
pub(crate) use patch::ArtifactPatchService;
pub(crate) use search::{WorkspaceSearchError, WorkspaceSearchRequest, WorkspaceSearchResponse, WorkspaceSearchService};
pub(crate) use snapshot::{WorkspaceContentSnapshot, WorkspaceSnapshotReason, WorkspaceSnapshotStore};
pub(crate) use terminal::{
    AcceptedArtifact, FinishTaskResult, WorkspaceTerminalError, WorkspaceTerminalErrorCode, WorkspaceTerminalService,
    WorkspaceTerminalValidationIssue, WorkspaceTerminalValidator,
};
pub(crate) use turn_snapshot::{
    TurnBaseSnapshot, TurnOutcomeSnapshot, TurnSnapshotIdentity, TurnSnapshotJournal, TurnSnapshotOutcomeStatus, TurnSnapshotStore,
};
pub(crate) use types::{
    ArtifactContentFormat, ArtifactDiagnostic, ArtifactExcerpt, ArtifactId, ArtifactMetadata, ArtifactRepairRecord, ArtifactSnapshot, ArtifactStatus,
    DiagnosticRecord, DiagnosticSeverity, DiagnosticStage, JsonPatchRequest, PatchResult, PublishedArtifactHead, TextPatchRequest, WorkspaceError,
    WorkspaceFileMetadata, WorkspaceManifest, WorkspaceRelativePath, WorkspaceResult,
};
