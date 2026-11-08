/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Thread 领域的单一应用服务入口。

use sqlx::SqlitePool;

use super::error::ThreadRepositoryError;
use super::persistence::SqliteThreadRepository;
use super::state::{
    AcceptTurnInput, AcceptedTurnInput, AgentItem, AgentTaskRunRef, AgentThread, AgentThreadSummary, AgentTimelineEvent, AgentTurn, AgentTurnInput,
    AppendTimelineEventInput, ApplyItemEventInput, ClaimTurnRunningInput, CompleteTurnInput, CompleteTurnResult, ConsistencyIssue, ConsumeTurnInput,
    ConsumedTurnInput, CreateThreadInput, CreateTurnInput, CreateTurnResult, ListItemsQuery, ListRunEventsQuery, ListThreadEventsQuery,
    ListThreadsQuery, ListTurnsQuery, MarkTurnWaitingInput, ResumeTurnInput, SetThreadArchivedInput, StartItemInput, ThreadCursor, ThreadPage,
    TimelineWriteResult, UpdateThreadTitleInput,
};
use super::timeline_writer::TimelineWriter;

#[derive(Clone)]
pub(crate) struct ThreadService {
    repository: SqliteThreadRepository,
    timeline: TimelineWriter,
}

impl ThreadService {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { repository: SqliteThreadRepository::new(pool.clone()), timeline: TimelineWriter::new(pool) }
    }

    pub(crate) fn repository(&self) -> &SqliteThreadRepository {
        &self.repository
    }

    pub(crate) fn timeline_writer(&self) -> &TimelineWriter {
        &self.timeline
    }

    pub(crate) async fn create_thread(&self, input: CreateThreadInput) -> Result<AgentThread, ThreadRepositoryError> {
        self.repository.create_thread(input).await
    }

    pub(crate) async fn find_thread(&self, project_id: i64, thread_id: &str) -> Result<Option<AgentThreadSummary>, ThreadRepositoryError> {
        self.repository.find_thread(project_id, thread_id).await
    }

    pub(crate) async fn list_threads(&self, query: ListThreadsQuery) -> Result<ThreadPage<AgentThreadSummary, ThreadCursor>, ThreadRepositoryError> {
        self.repository.list_threads(query).await
    }

    pub(crate) async fn update_thread_title(&self, input: UpdateThreadTitleInput) -> Result<AgentThread, ThreadRepositoryError> {
        self.repository.update_thread_title(input).await
    }

    pub(crate) async fn set_thread_archived(&self, input: SetThreadArchivedInput) -> Result<AgentThread, ThreadRepositoryError> {
        self.repository.set_thread_archived(input).await
    }

    pub(crate) async fn create_turn(&self, input: CreateTurnInput) -> Result<CreateTurnResult, ThreadRepositoryError> {
        self.repository.create_turn(input).await
    }

    pub(crate) async fn find_turn(&self, project_id: i64, thread_id: &str, turn_id: &str) -> Result<Option<AgentTurn>, ThreadRepositoryError> {
        self.repository.find_turn(project_id, thread_id, turn_id).await
    }

    pub(crate) async fn list_turns(&self, query: ListTurnsQuery) -> Result<ThreadPage<AgentTurn, u64>, ThreadRepositoryError> {
        self.repository.list_turns(query).await
    }

    pub(crate) async fn find_item(
        &self,
        project_id: i64,
        thread_id: &str,
        turn_id: &str,
        item_id: &str,
    ) -> Result<Option<AgentItem>, ThreadRepositoryError> {
        self.repository.find_item(project_id, thread_id, turn_id, item_id).await
    }

    pub(crate) async fn list_items(&self, query: ListItemsQuery) -> Result<ThreadPage<AgentItem, u64>, ThreadRepositoryError> {
        self.repository.list_items(query).await
    }

    pub(crate) async fn find_task_run_by_run_id(&self, run_id: &str) -> Result<Option<AgentTaskRunRef>, ThreadRepositoryError> {
        self.repository.find_task_run_by_run_id(run_id).await
    }

    pub(crate) async fn find_task_run_by_request_id(&self, request_id: &str) -> Result<Option<AgentTaskRunRef>, ThreadRepositoryError> {
        self.repository.find_task_run_by_request_id(request_id).await
    }

    pub(crate) async fn list_thread_events(
        &self,
        query: ListThreadEventsQuery,
    ) -> Result<ThreadPage<AgentTimelineEvent, u64>, ThreadRepositoryError> {
        self.repository.list_thread_events(query).await
    }

    pub(crate) async fn list_run_events(&self, query: ListRunEventsQuery) -> Result<ThreadPage<AgentTimelineEvent, u64>, ThreadRepositoryError> {
        self.repository.list_run_events(query).await
    }

    pub(crate) async fn claim_turn_running(
        &self,
        input: ClaimTurnRunningInput,
    ) -> Result<(AgentTurn, AgentTaskRunRef, AgentTimelineEvent), ThreadRepositoryError> {
        self.repository.claim_turn_running(input).await
    }

    pub(crate) async fn mark_turn_waiting(
        &self,
        input: MarkTurnWaitingInput,
    ) -> Result<(AgentTurn, AgentTaskRunRef, AgentTimelineEvent), ThreadRepositoryError> {
        self.repository.mark_turn_waiting(input).await
    }

    pub(crate) async fn resume_turn(
        &self,
        input: ResumeTurnInput,
    ) -> Result<(AgentTurn, AgentTaskRunRef, AgentTimelineEvent), ThreadRepositoryError> {
        self.repository.resume_turn(input).await
    }

    pub(crate) async fn start_item(&self, input: StartItemInput) -> Result<TimelineWriteResult, ThreadRepositoryError> {
        self.timeline.start_item(input).await
    }

    pub(crate) async fn apply_item_event(&self, input: ApplyItemEventInput) -> Result<TimelineWriteResult, ThreadRepositoryError> {
        self.timeline.apply_item_event(input).await
    }

    pub(crate) async fn append_event(&self, input: AppendTimelineEventInput) -> Result<TimelineWriteResult, ThreadRepositoryError> {
        self.timeline.append_event(input).await
    }

    pub(crate) async fn accept_turn_input(&self, input: AcceptTurnInput) -> Result<AcceptedTurnInput, ThreadRepositoryError> {
        self.repository.accept_turn_input(input).await
    }

    pub(crate) async fn list_accepted_turn_inputs(
        &self,
        project_id: i64,
        thread_id: &str,
        turn_id: &str,
        limit: u32,
    ) -> Result<Vec<AgentTurnInput>, ThreadRepositoryError> {
        self.repository
            .list_accepted_turn_inputs(project_id, thread_id, turn_id, limit)
            .await
    }

    pub(crate) async fn find_turn_input(
        &self,
        project_id: i64,
        thread_id: &str,
        turn_id: &str,
        input_id: &str,
    ) -> Result<Option<AgentTurnInput>, ThreadRepositoryError> {
        self.repository.find_turn_input(project_id, thread_id, turn_id, input_id).await
    }

    pub(crate) async fn consume_turn_input(&self, input: ConsumeTurnInput) -> Result<ConsumedTurnInput, ThreadRepositoryError> {
        self.repository.consume_turn_input(input).await
    }

    pub(crate) async fn complete_turn(&self, input: CompleteTurnInput) -> Result<CompleteTurnResult, ThreadRepositoryError> {
        self.repository.complete_turn(input).await
    }

    pub(crate) async fn interrupt_active_turns(&self, interrupted_at: i64) -> Result<u64, ThreadRepositoryError> {
        self.repository.interrupt_active_turns(interrupted_at).await
    }

    pub(crate) async fn audit_consistency(&self) -> Result<Vec<ConsistencyIssue>, ThreadRepositoryError> {
        self.repository.audit_consistency().await
    }
}
