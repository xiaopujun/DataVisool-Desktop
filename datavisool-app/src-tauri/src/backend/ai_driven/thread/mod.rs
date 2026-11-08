/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Thread、Turn、Item 的稳定领域模型与 SQLite 持久化边界。

mod consistency;
mod error;
mod id;
mod lifecycle;
mod persistence;
mod service;
mod state;
mod timeline_writer;
mod usage;

#[cfg(test)]
mod tests;

pub(crate) use error::ThreadRepositoryError;
pub(crate) use id::new_item_id;
pub(crate) use persistence::SqliteThreadRepository;
pub(crate) use service::ThreadService;
pub(crate) use state::{
    AcceptTurnInput, AgentItem, AgentTaskRunRef, AgentThread, AgentTimelineEvent, AgentTurnInput,
    AppendTimelineEventInput, ApplyItemEventInput, AttachmentClaim, ClaimTurnRunningInput, CompleteTurnInput,
    ConsumeTurnInput, ConsumedTurnInput, CreateResumeAttemptInput, CreateThreadInput, CreateTurnInput, CreateTurnResult,
    ItemStatus, ListItemsQuery, ListRunEventsQuery, ListThreadEventsQuery, ListThreadsQuery, ListTurnsQuery, MarkTurnWaitingInput, ResumeTurnInput,
    SetThreadArchivedInput, StartItemInput, ThreadCursor, ThreadStatus, TimelineWriteResult,
    TransitionTurnExecutionStatusInput, TurnInputStatus, TurnStatus, UpdateThreadTitleInput,
};
pub(crate) use timeline_writer::TimelineWriter;
pub(crate) use usage::{thread_usage_snapshot, ThreadTokenUsage};
