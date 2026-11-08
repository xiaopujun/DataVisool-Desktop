/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI Agent 稳定任务状态与运行事件存储。

mod completion;
mod completion_requirements;
mod consistency;
mod events;
mod identity;
mod objective;
mod operation_journal;
mod persistence;
mod recovery_context;
mod registry;
mod run_budget;
pub(crate) use run_budget::BudgetExtension;
mod service;
mod state;
mod validation_preflight;
mod working_checkpoint;
mod workspace;

pub(crate) use completion::TaskCompletionGate;
pub(crate) use completion_requirements::{CompletionRequirementsSnapshot, CompletionRequirementsState, RequiredCanvasEffect};
pub(crate) use consistency::{scoped_task_status, ScopedTaskStatusError};
pub(crate) use events::{AgentTaskEventPublisher, AiDrivenTaskEventEnvelope};
pub(crate) use objective::ObjectiveSpec;
pub(crate) use operation_journal::{CanvasOperationJournal, CanvasOperationJournalError, CanvasOperationRecord};
pub(crate) use persistence::{AgentTaskRepository, SqliteAgentTaskRepository, TaskRepositoryError};
pub(crate) use registry::{
    ai_driven_task_registry, AgentSteerInput, AgentSteeringMailbox, AiDrivenTaskCancelError, AiDrivenTaskIdentity, AiDrivenTaskRegistrationError,
    AiDrivenTaskRegistry, AiDrivenTaskRunSnapshot, AiDrivenTaskSteerError,
};
pub(crate) use service::{AgentTaskRequest, AgentTaskService};
pub(crate) use state::{
    AgentArtifactRef, AgentExecutionLimitsSnapshot, AgentPendingKind, AgentPendingSnapshot, AgentTaskCheckpointResult, AgentTaskError,
    AgentTaskEvent, AgentTaskRun, AgentTaskStatus, ListTaskEvents, NewAgentTaskEvent, NewAgentTaskRun, SaveAgentTaskCheckpoint,
    AGENT_RESTART_INTERRUPTED,
};
pub(crate) use working_checkpoint::{AgentWorkingCheckpoint, AgentWorkingCheckpointState, AgentWorkingPlanStep, SaveAgentWorkingCheckpoint};
