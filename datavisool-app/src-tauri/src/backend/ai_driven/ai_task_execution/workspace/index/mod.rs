/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 项目 Workspace 的可选兼容 INDEX.json 协议。
//! TODO(legacy-compat): 迁移当前工作区调用方后，移除旧 INDEX.json 兼容协议。

mod schema;
mod validator;

pub(crate) use schema::{
    WorkspaceIndex, WorkspaceIndexEntry, WorkspaceIndexRole, WorkspaceIndexTurn, WorkspaceIndexTurnStatus, WORKSPACE_INDEX_SCHEMA_VERSION,
};
pub(crate) use validator::{
    read_and_validate_index, scan_workspace_files, serialize_initial_index, validate_index, validate_index_against_paths,
    WorkspaceIndexValidationMode, WORKSPACE_INDEX_FILE,
};
