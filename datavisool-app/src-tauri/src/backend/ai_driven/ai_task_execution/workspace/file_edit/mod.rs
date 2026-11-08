/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Agent Workspace 通用文件读取与事务 Patch。

mod parser;
mod path;
mod reconcile;
mod service;
mod transaction;

#[cfg(test)]
mod service_tests;

pub(super) use reconcile::reconcile_manifest_files;
pub(crate) use service::{
    WorkspaceApplyPatchResult, WorkspaceCapturedFile, WorkspaceFileExcerpt, WorkspaceFileLimits, WorkspaceFileListEntry, WorkspaceFileService,
    WorkspaceFileTrace, WorkspaceFileTraceSnapshot, WorkspaceMovedFile,
};
pub(super) use transaction::recover_pending_file_commit;
