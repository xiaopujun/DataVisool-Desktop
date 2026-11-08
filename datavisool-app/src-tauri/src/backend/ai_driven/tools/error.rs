/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Rig Tool 的项目稳定错误边界。

use std::fmt::{Display, Formatter};

use rig_core::tool::ToolFailure;

use crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceError;
use crate::backend::ai_driven::ai_task_execution::workspace::WorkspaceSearchError;
use crate::backend::ai_driven::component_docs::{ComponentDocumentSearchError, ComponentDocumentSearchErrorKind};
use crate::backend::ai_driven::ripgrep::{RipgrepError, RipgrepErrorKind};
use crate::backend::ai_driven::security::SecurityDecisionError;
use crate::backend::ai_driven::task::CanvasOperationJournalError;
use crate::backend::FrontendEffectError;

use super::context::AgentToolContextError;

/// Tool 失败分类。该分类用于 Rig Hook，错误码用于产品协议。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentToolErrorKind {
    InvalidArgs,
    Cancelled,
    NotFound,
    PermissionDenied,
    Timeout,
    Conflict,
    Frontend,
    Storage,
    Validation,
    Internal,
}

/// Tool 对模型和任务层暴露的稳定错误。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentToolError {
    pub(crate) kind: AgentToolErrorKind,
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) retryable: bool,
    pub(crate) field_path: String,
}

impl AgentToolError {
    pub(crate) fn new(kind: AgentToolErrorKind, code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self { kind, code: code.into(), message: message.into(), retryable, field_path: String::new() }
    }

    pub(crate) fn invalid(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(AgentToolErrorKind::InvalidArgs, code, message, false)
    }

    pub(crate) fn validation(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(AgentToolErrorKind::Validation, code, message, true)
    }

    pub(crate) fn internal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(AgentToolErrorKind::Internal, code, message, false)
    }

    pub(crate) fn with_field_path(mut self, path: impl Into<String>) -> Self {
        self.field_path = path.into();
        self
    }

    pub(crate) fn recovery(&self) -> &'static str {
        if self.code.contains("snapshot") || self.code.contains("revision_conflict") || self.code.contains("version_conflict") {
            return "read_current_state_then_recompute";
        }
        match self.kind {
            AgentToolErrorKind::InvalidArgs | AgentToolErrorKind::Validation => "correct_arguments",
            AgentToolErrorKind::Conflict => "read_current_state_then_recompute",
            AgentToolErrorKind::PermissionDenied => "do_not_retry_without_authorization_change",
            AgentToolErrorKind::Cancelled => "stop_or_wait_for_user",
            AgentToolErrorKind::Timeout | AgentToolErrorKind::Frontend => "recover_original_operation_before_retry",
            AgentToolErrorKind::Storage | AgentToolErrorKind::Internal => "stop_and_preserve_applied_results",
            AgentToolErrorKind::NotFound => "check_target_or_choose_alternative",
        }
    }

    pub(crate) fn to_tool_failure(&self) -> ToolFailure {
        let failure = match self.kind {
            AgentToolErrorKind::InvalidArgs | AgentToolErrorKind::Validation | AgentToolErrorKind::Conflict => {
                ToolFailure::invalid_args(self.message.clone())
            }
            AgentToolErrorKind::Cancelled => ToolFailure::cancelled(self.message.clone()),
            AgentToolErrorKind::NotFound => ToolFailure::not_found(self.message.clone()),
            AgentToolErrorKind::PermissionDenied => ToolFailure::permission_denied(self.message.clone()),
            AgentToolErrorKind::Timeout => ToolFailure::timeout(self.message.clone()),
            AgentToolErrorKind::Frontend => ToolFailure::provider(self.message.clone()),
            AgentToolErrorKind::Storage | AgentToolErrorKind::Internal => ToolFailure::other(self.message.clone()),
        };
        failure.with_code(self.code.clone()).with_retryable(self.retryable)
    }
}

impl Display for AgentToolError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let details = serde_json::from_str::<serde_json::Value>(&self.message).ok();
        let field_path = if self.field_path.is_empty() {
            details
                .as_ref()
                .and_then(|value| value.get("fieldPath").or_else(|| value.get("instancePath")))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        } else {
            &self.field_path
        };
        write!(
            formatter,
            "{}",
            serde_json::json!({
                "code": self.code, "message": self.message, "fieldPath": field_path,
                "retryable": self.retryable,
                "argumentsCorrectable": matches!(self.kind, AgentToolErrorKind::InvalidArgs | AgentToolErrorKind::Validation | AgentToolErrorKind::Conflict),
                "recovery": self.recovery(), "details": details,
            })
        )
    }
}

impl std::error::Error for AgentToolError {}

impl From<AgentToolContextError> for AgentToolError {
    fn from(error: AgentToolContextError) -> Self {
        let kind = match &error {
            AgentToolContextError::Cancelled => AgentToolErrorKind::Cancelled,
            AgentToolContextError::Policy(_) => AgentToolErrorKind::PermissionDenied,
            AgentToolContextError::InvalidScope { .. } => AgentToolErrorKind::InvalidArgs,
        };
        Self::new(kind, error.code(), error.to_string(), false)
    }
}

impl From<SecurityDecisionError> for AgentToolError {
    fn from(error: SecurityDecisionError) -> Self {
        if error.code() == "agent.security.target_count_exceeded" {
            return Self::invalid(error.code(), error.to_string()).with_field_path("components");
        }
        Self::new(AgentToolErrorKind::PermissionDenied, error.code(), error.to_string(), false)
    }
}

impl From<WorkspaceError> for AgentToolError {
    fn from(error: WorkspaceError) -> Self {
        let (kind, code, retryable) = match &error {
            WorkspaceError::ArtifactNotFound(_) | WorkspaceError::WorkspaceNotFound(_) => {
                (AgentToolErrorKind::NotFound, "agent.workspace.not_found", false)
            }
            WorkspaceError::RevisionConflict { .. } => (AgentToolErrorKind::Conflict, "agent.workspace.revision_conflict", true),
            WorkspaceError::PathOutOfScope(_) => (AgentToolErrorKind::PermissionDenied, "agent.workspace.path_out_of_scope", false),
            WorkspaceError::InvalidIndex(_) | WorkspaceError::InvalidFilePatch(_) | WorkspaceError::ResourceLimit(_) => {
                (AgentToolErrorKind::InvalidArgs, "agent.workspace.invalid_request", true)
            }
            WorkspaceError::InvalidToolCall { .. }
            | WorkspaceError::InvalidArtifactId(_)
            | WorkspaceError::InvalidWorkspaceId(_)
            | WorkspaceError::InvalidJson { .. }
            | WorkspaceError::JsonPointerNotFound { .. }
            | WorkspaceError::JsonPatchFailed { .. }
            | WorkspaceError::TextPatchNotAllowed(_)
            | WorkspaceError::TextPatchContentMismatch(_)
            | WorkspaceError::InvalidLineRange { .. } => (AgentToolErrorKind::InvalidArgs, "agent.workspace.invalid_request", true),
            _ => (AgentToolErrorKind::Storage, "agent.workspace.storage_failed", false),
        };
        Self::new(kind, code, error.to_string(), retryable)
    }
}

impl From<WorkspaceSearchError> for AgentToolError {
    fn from(error: WorkspaceSearchError) -> Self {
        match error {
            WorkspaceSearchError::InvalidPath(message) => {
                Self::new(AgentToolErrorKind::PermissionDenied, "agent.workspace.path_out_of_scope", message, false)
            }
            WorkspaceSearchError::WorkspaceState => {
                Self::new(AgentToolErrorKind::Storage, "agent.workspace.search_state_failed", "无法读取 Workspace 搜索状态", true)
            }
            WorkspaceSearchError::Ripgrep(error) => error.into(),
        }
    }
}

impl From<RipgrepError> for AgentToolError {
    fn from(error: RipgrepError) -> Self {
        let kind = match error.kind {
            RipgrepErrorKind::InvalidPattern => AgentToolErrorKind::InvalidArgs,
            RipgrepErrorKind::MissingBinary | RipgrepErrorKind::Spawn | RipgrepErrorKind::Output | RipgrepErrorKind::Process => {
                AgentToolErrorKind::Internal
            }
            RipgrepErrorKind::Timeout => AgentToolErrorKind::Timeout,
            RipgrepErrorKind::Cancelled => AgentToolErrorKind::Cancelled,
        };
        Self::new(kind, error.code, error.message, false)
    }
}

impl From<ComponentDocumentSearchError> for AgentToolError {
    fn from(error: ComponentDocumentSearchError) -> Self {
        if let Some(ripgrep) = error.ripgrep {
            return ripgrep.into();
        }
        let kind = match error.kind {
            ComponentDocumentSearchErrorKind::InvalidRequest => AgentToolErrorKind::InvalidArgs,
            ComponentDocumentSearchErrorKind::ResourceMissing => AgentToolErrorKind::NotFound,
            ComponentDocumentSearchErrorKind::ResourceMismatch => AgentToolErrorKind::Validation,
            ComponentDocumentSearchErrorKind::InvalidResult | ComponentDocumentSearchErrorKind::Ripgrep => AgentToolErrorKind::Internal,
        };
        Self::new(kind, error.code, error.message, false)
    }
}

impl From<FrontendEffectError> for AgentToolError {
    fn from(error: FrontendEffectError) -> Self {
        if let FrontendEffectError::FrontendFailed(Some(payload)) = &error {
            let kind = if payload.code.contains("revision_conflict") || payload.code.contains("snapshot_conflict") {
                AgentToolErrorKind::Conflict
            } else if payload.code.contains("out_of_scope") || payload.code.contains("scope_mismatch") || payload.code.contains("forbidden") {
                AgentToolErrorKind::PermissionDenied
            } else if payload.code.contains("cancelled") || payload.code.contains("user_rejected") {
                AgentToolErrorKind::Cancelled
            } else {
                AgentToolErrorKind::Frontend
            };
            return Self::new(
                kind,
                &payload.code,
                &payload.message,
                !matches!(kind, AgentToolErrorKind::PermissionDenied | AgentToolErrorKind::Cancelled),
            );
        }
        let (kind, code, retryable) = match &error {
            FrontendEffectError::BindingMismatch(_) => (AgentToolErrorKind::PermissionDenied, "agent.frontend.binding_mismatch", false),
            FrontendEffectError::EncodeRequest(_) | FrontendEffectError::Unavailable(_) => {
                (AgentToolErrorKind::Internal, "agent.frontend.internal_failed", false)
            }
            FrontendEffectError::Cancelled(_) => (AgentToolErrorKind::Cancelled, "agent.frontend.cancelled", false),
            FrontendEffectError::Timeout => (AgentToolErrorKind::Timeout, "agent.frontend.timeout", true),
            _ => (AgentToolErrorKind::Frontend, "agent.frontend.failed", true),
        };
        Self::new(kind, code, error.to_string(), retryable)
    }
}

impl From<CanvasOperationJournalError> for AgentToolError {
    fn from(error: CanvasOperationJournalError) -> Self {
        let kind = if error.code == "agent.operation.conflict" {
            AgentToolErrorKind::Conflict
        } else {
            AgentToolErrorKind::Storage
        };
        let retryable = kind == AgentToolErrorKind::Conflict;
        Self::new(kind, error.code, error.message, retryable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_parameters_can_be_corrected_without_blind_replay() {
        let error = AgentToolError::invalid("invalid", "width must be positive").with_field_path("components[0].width");
        let value: serde_json::Value = serde_json::from_str(&error.to_string()).unwrap();
        assert_eq!(value["fieldPath"], "components[0].width");
        assert_eq!(value["argumentsCorrectable"], true);
        assert_eq!(value["retryable"], false);
        assert_eq!(value["recovery"], "correct_arguments");
    }
    #[test]
    fn permission_and_conflict_require_different_recovery() {
        let denied = AgentToolError::new(AgentToolErrorKind::PermissionDenied, "denied", "outside scope", false);
        let conflict = AgentToolError::new(AgentToolErrorKind::Conflict, "version_conflict", "read again", true);
        assert_eq!(denied.recovery(), "do_not_retry_without_authorization_change");
        assert_eq!(conflict.recovery(), "read_current_state_then_recompute");
    }
}
