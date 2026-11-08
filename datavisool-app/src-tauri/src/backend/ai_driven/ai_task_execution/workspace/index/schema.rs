/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Agent 维护的项目 Workspace 索引协议。

use serde::{Deserialize, Serialize};

use crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceRelativePath;

pub(crate) const WORKSPACE_INDEX_SCHEMA_VERSION: &str = "datavisool-workspace-index/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkspaceIndex {
    pub schema_version: String,
    #[serde(default)]
    pub project_summary: String,
    #[serde(default)]
    pub entries: Vec<WorkspaceIndexEntry>,
    #[serde(default)]
    pub turns: Vec<WorkspaceIndexTurn>,
}

impl Default for WorkspaceIndex {
    fn default() -> Self {
        Self {
            schema_version: WORKSPACE_INDEX_SCHEMA_VERSION.to_string(),
            project_summary: String::new(),
            entries: Vec::new(),
            turns: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkspaceIndexEntry {
    pub path: WorkspaceRelativePath,
    pub role: WorkspaceIndexRole,
    pub summary: String,
    pub updated_by_turn_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WorkspaceIndexRole {
    Reference,
    Draft,
    Intermediate,
    Final,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkspaceIndexTurn {
    pub turn_id: String,
    pub requirement: String,
    #[serde(default)]
    pub inputs: Vec<WorkspaceRelativePath>,
    #[serde(default)]
    pub outputs: Vec<WorkspaceRelativePath>,
    pub status: WorkspaceIndexTurnStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WorkspaceIndexTurnStatus {
    Completed,
}
