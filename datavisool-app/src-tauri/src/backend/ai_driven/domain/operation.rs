/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 直接写入设计器后的受控业务操作回执。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CanvasOperationKind {
    CreateComponents,
    PatchComponents,
    DeleteComponents,
}

impl CanvasOperationKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::CreateComponents => "createComponents",
            Self::PatchComponents => "patchComponents",
            Self::DeleteComponents => "deleteComponents",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CanvasOperationStatus {
    Prepared,
    Applying,
    Applied,
    Rejected,
    Failed,
}

impl CanvasOperationStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Applying => "applying",
            Self::Applied => "applied",
            Self::Rejected => "rejected",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Applied | Self::Rejected | Self::Failed)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CanvasOperationError {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) retryable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CanvasOperationReceipt {
    pub(crate) operation_id: String,
    pub(crate) tool_call_id: String,
    pub(crate) kind: CanvasOperationKind,
    pub(crate) status: CanvasOperationStatus,
    pub(crate) project_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) manager_id: Option<String>,
    #[serde(default)]
    pub(crate) component_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) component_bindings: BTreeMap<String, String>,
    /// Host-validated patch fields persisted with the operation so status
    /// recovery never has to trust the status query arguments.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) component_fields: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub(crate) configured_component_count: usize,
    #[serde(default)]
    pub(crate) default_config_component_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) base_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) applied_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<CanvasOperationError>,
}

impl CanvasOperationReceipt {
    pub(crate) fn prepared(
        operation_id: String,
        tool_call_id: String,
        kind: CanvasOperationKind,
        project_id: String,
        manager_id: Option<String>,
        component_ids: Vec<String>,
    ) -> Self {
        Self {
            operation_id,
            tool_call_id,
            kind,
            status: CanvasOperationStatus::Prepared,
            project_id,
            manager_id,
            component_ids,
            component_bindings: BTreeMap::new(),
            component_fields: BTreeMap::new(),
            configured_component_count: 0,
            default_config_component_count: 0,
            base_revision: None,
            result_revision: None,
            applied_at: None,
            error: None,
        }
    }
}
