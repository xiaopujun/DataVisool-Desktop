/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

use super::*;

const MIGRATION: &str = include_str!("../../../../migrations_desktop/1014_desktop_ai_thread_turn_item.sql");
const WORKING_CHECKPOINT_MIGRATION: &str = include_str!("../../../../migrations_desktop/1017_desktop_ai_working_checkpoint.sql");

async fn service() -> ThreadService {
    service_with_pool().await.0
}

async fn service_with_pool() -> (ThreadService, sqlx::SqlitePool) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect memory database");
    sqlx::raw_sql("CREATE TABLE project (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);")
        .execute(&pool)
        .await
        .expect("create project table");
    sqlx::raw_sql(MIGRATION).execute(&pool).await.expect("apply Thread migration");
    sqlx::raw_sql(WORKING_CHECKPOINT_MIGRATION)
        .execute(&pool)
        .await
        .expect("apply working checkpoint migration");
    sqlx::query("INSERT INTO project (id, name, deleted) VALUES (1, 'project', 0)")
        .execute(&pool)
        .await
        .expect("insert project");
    (ThreadService::new(pool.clone()), pool)
}

async fn thread(service: &ThreadService, suffix: &str, created_at: i64) -> AgentThread {
    service
        .create_thread(CreateThreadInput { project_id: 1, create_request_id: format!("create-{suffix}"), title: None, created_at })
        .await
        .expect("create thread")
}

fn turn_input(thread_id: &str, suffix: &str, created_at: i64) -> CreateTurnInput {
    CreateTurnInput {
        project_id: 1,
        thread_id: thread_id.to_string(),
        request_id: format!("request-{suffix}"),
        request: json!({ "prompt": suffix }),
        input_summary: format!("prompt {suffix}"),
        model_config_id: "model-config".to_string(),
        runtime_model_id: "runtime-model".to_string(),
        provider: Some("test".to_string()),
        entry_digest: format!("sha256:{suffix}"),
        client_session_id: "client-session".to_string(),
        task_type: "general".to_string(),
        limits: json!({ "tokenBudgetEnabled": false, "callBudgetEnabled": false, "durationBudgetEnabled": false, "maxModelCalls": 8 }),
        project_snapshot_ref: None,
        rig_version: Some("test-rig".to_string()),
        rig_snapshot_ref: Some("rig:snapshot".to_string()),
        user_item_payload: json!({ "text": suffix }),
        attachments: Vec::new(),
        created_at,
    }
}

#[tokio::test]
async fn creates_opaque_idempotent_thread_and_turn_with_keyset_reads() {
    let service = service().await;
    let created_thread = thread(&service, "one", 10).await;
    assert!(created_thread.thread_id.starts_with("ai-agent-thread-"));
    assert_ne!(created_thread.thread_id, "create-one");
    let empty_thread_page = service
        .list_threads(ListThreadsQuery { project_id: 1, archived: false, cursor: None, limit: 20 })
        .await
        .expect("list Thread before first Turn");
    assert!(empty_thread_page.data.is_empty());

    let created = service
        .create_turn(turn_input(&created_thread.thread_id, "one", 20))
        .await
        .expect("create turn");
    assert!(created.turn.turn_id.starts_with("ai-agent-turn-"));
    assert!(created.run.run_id.starts_with("ai-agent-run-"));
    assert!(created.user_item.item_id.starts_with("ai-agent-item-"));
    assert_eq!(created.events.len(), 3);
    assert_eq!(created.events[1].payload["item"]["itemId"], created.user_item.item_id);
    assert_eq!(created.events[1].payload["item"]["itemType"], "userMessage");
    assert!(!created.replayed);

    let replayed = service
        .create_turn(turn_input(&created_thread.thread_id, "one", 30))
        .await
        .expect("replay turn");
    assert!(replayed.replayed);
    assert_eq!(replayed.turn.turn_id, created.turn.turn_id);
    assert_eq!(replayed.run.run_id, created.run.run_id);

    let mut conflict = turn_input(&created_thread.thread_id, "one", 30);
    conflict.entry_digest = "sha256:changed".to_string();
    let error = service.create_turn(conflict).await.expect_err("reject changed entry");
    assert_eq!(error.stable_code(), "agent.turn.idempotency_conflict");

    let page = service
        .list_threads(ListThreadsQuery { project_id: 1, archived: false, cursor: None, limit: 1 })
        .await
        .expect("list threads");
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].active_turn_id.as_deref(), Some(created.turn.turn_id.as_str()));
    let turns = service
        .list_turns(ListTurnsQuery { project_id: 1, thread_id: created_thread.thread_id, after_turn_sequence: 0, limit: 1 })
        .await
        .expect("list turns");
    assert_eq!(turns.data[0].turn_id, created.turn.turn_id);
}

#[tokio::test]
async fn validates_thread_titles_at_the_repository_boundary() {
    let service = service().await;
    let too_long = "标".repeat(61);
    let error = service
        .create_thread(CreateThreadInput {
            project_id: 1,
            create_request_id: "title-too-long".to_string(),
            title: Some(too_long.clone()),
            created_at: 1,
        })
        .await
        .expect_err("reject long create title");
    assert_eq!(error.stable_code(), "agent.invalid_record");

    let created = thread(&service, "title", 2).await;
    let error = service
        .update_thread_title(UpdateThreadTitleInput {
            project_id: 1,
            thread_id: created.thread_id,
            title: too_long,
            expected_version: created.row_version,
            updated_at: 3,
        })
        .await
        .expect_err("reject long update title");
    assert_eq!(error.stable_code(), "agent.invalid_record");
}

#[tokio::test]
async fn maps_active_turn_unique_constraints_to_stable_conflicts() {
    let service = service().await;
    let first_thread = thread(&service, "active-first", 1).await;
    let second_thread = thread(&service, "active-second", 2).await;
    service
        .create_turn(turn_input(&first_thread.thread_id, "active", 3))
        .await
        .expect("create active turn");

    let same_thread = service
        .create_turn(turn_input(&first_thread.thread_id, "same-thread", 4))
        .await
        .expect_err("same Thread rejects another active Turn");
    assert_eq!(same_thread.stable_code(), "agent.thread.active_turn");

    let same_project = service
        .create_turn(turn_input(&second_thread.thread_id, "same-project", 5))
        .await
        .expect_err("same project rejects another active Turn");
    assert_eq!(same_project.stable_code(), "agent.project.active_turn");
}

#[tokio::test]
async fn soft_deleted_projects_hide_history_without_deleting_it() {
    let (service, pool) = service_with_pool().await;
    let created_thread = thread(&service, "soft-delete", 10).await;
    let created = service
        .create_turn(turn_input(&created_thread.thread_id, "soft-delete", 20))
        .await
        .expect("create turn");
    sqlx::query("UPDATE project SET deleted = 1 WHERE id = 1")
        .execute(&pool)
        .await
        .expect("soft delete project");

    assert!(service
        .find_thread(1, &created_thread.thread_id)
        .await
        .expect("find thread")
        .is_none());
    assert!(service
        .list_threads(ListThreadsQuery { project_id: 1, archived: false, cursor: None, limit: 20 })
        .await
        .expect("list threads")
        .data
        .is_empty());
    assert!(service
        .find_turn(1, &created_thread.thread_id, &created.turn.turn_id)
        .await
        .expect("find turn")
        .is_none());
    assert!(matches!(
        service
            .list_items(ListItemsQuery {
                project_id: 1,
                thread_id: created_thread.thread_id.clone(),
                turn_id: created.turn.turn_id.clone(),
                after_item_sequence: 0,
                limit: 20,
            })
            .await,
        Err(ThreadRepositoryError::NotFound { entity: "turn", .. })
    ));
    assert!(service
        .find_task_run_by_run_id(&created.run.run_id)
        .await
        .expect("find run")
        .is_none());
    assert!(matches!(
        service
            .update_thread_title(UpdateThreadTitleInput {
                project_id: 1,
                thread_id: created_thread.thread_id.clone(),
                title: "hidden".to_string(),
                expected_version: created_thread.row_version,
                updated_at: 30,
            })
            .await,
        Err(ThreadRepositoryError::NotFound { .. })
    ));
    assert!(matches!(
        service
            .create_turn(turn_input(&created_thread.thread_id, "soft-delete-new", 30))
            .await,
        Err(ThreadRepositoryError::NotFound { .. })
    ));
    assert!(service.audit_consistency().await.expect("audit").is_empty());

    let retained: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM ai_agent_turn WHERE turn_id = ?1")
        .bind(&created.turn.turn_id)
        .fetch_one(&pool)
        .await
        .expect("count retained turn");
    assert_eq!(retained, 1);
}

#[tokio::test]
async fn writes_timeline_inbox_and_terminal_state_in_single_ordered_history() {
    let service = service().await;
    let created_thread = thread(&service, "timeline", 10).await;
    let created = service
        .create_turn(turn_input(&created_thread.thread_id, "timeline", 20))
        .await
        .expect("create turn");
    service
        .claim_turn_running(ClaimTurnRunningInput {
            project_id: 1,
            thread_id: created_thread.thread_id.clone(),
            turn_id: created.turn.turn_id.clone(),
            run_id: created.run.run_id.clone(),
            started_at: 30,
        })
        .await
        .expect("claim turn");
    let open_item = service
        .start_item(StartItemInput {
            project_id: 1,
            thread_id: created_thread.thread_id.clone(),
            turn_id: created.turn.turn_id.clone(),
            run_id: created.run.run_id.clone(),
            item_id: None,
            logical_key: Some("assistant:answer".to_string()),
            item_type: "assistantMessage".to_string(),
            status: ItemStatus::InProgress,
            schema_version: 1,
            payload: json!({ "text": "partial" }),
            event_payload: json!({ "text": "partial" }),
            dedupe_key: "assistant:answer:started".to_string(),
            created_at: 40,
        })
        .await
        .expect("start item")
        .item
        .expect("created item");
    let semantic_replay = service
        .start_item(StartItemInput {
            project_id: 1,
            thread_id: created_thread.thread_id.clone(),
            turn_id: created.turn.turn_id.clone(),
            run_id: created.run.run_id.clone(),
            item_id: Some(new_item_id()),
            logical_key: Some("assistant:answer".to_string()),
            item_type: "assistantMessage".to_string(),
            status: ItemStatus::InProgress,
            schema_version: 1,
            payload: json!({ "text": "partial" }),
            event_payload: json!({ "text": "partial" }),
            dedupe_key: "assistant:answer:started:retry".to_string(),
            created_at: 41,
        })
        .await
        .expect("semantic Item replay");
    assert!(semantic_replay.deduplicated);
    assert_eq!(semantic_replay.item.expect("replayed item").item_id, open_item.item_id);
    let semantic_conflict = service
        .start_item(StartItemInput {
            project_id: 1,
            thread_id: created_thread.thread_id.clone(),
            turn_id: created.turn.turn_id.clone(),
            run_id: created.run.run_id.clone(),
            item_id: Some(new_item_id()),
            logical_key: Some("assistant:answer".to_string()),
            item_type: "assistantMessage".to_string(),
            status: ItemStatus::InProgress,
            schema_version: 1,
            payload: json!({ "text": "changed" }),
            event_payload: json!({ "text": "changed" }),
            dedupe_key: "assistant:answer:started:changed".to_string(),
            created_at: 41,
        })
        .await
        .expect_err("semantic Item conflict");
    assert_eq!(semantic_conflict.stable_code(), "agent.item.idempotency_conflict");
    let waiting_item = service
        .start_item(StartItemInput {
            project_id: 1,
            thread_id: created_thread.thread_id.clone(),
            turn_id: created.turn.turn_id.clone(),
            run_id: created.run.run_id.clone(),
            item_id: None,
            logical_key: Some("waiting:user-input".to_string()),
            item_type: "userInput".to_string(),
            status: ItemStatus::Waiting,
            schema_version: 1,
            payload: json!({ "itemType": "userInput", "status": "waiting" }),
            event_payload: json!({ "item": { "itemType": "userInput", "status": "waiting" } }),
            dedupe_key: "waiting:user-input:started".to_string(),
            created_at: 41,
        })
        .await
        .expect("start waiting item")
        .item
        .expect("waiting item");
    let newer_waiting_item = service
        .start_item(StartItemInput {
            project_id: 1,
            thread_id: created_thread.thread_id.clone(),
            turn_id: created.turn.turn_id.clone(),
            run_id: created.run.run_id.clone(),
            item_id: None,
            logical_key: Some("waiting:decoy".to_string()),
            item_type: "userInput".to_string(),
            status: ItemStatus::Waiting,
            schema_version: 1,
            payload: json!({ "itemType": "userInput", "status": "waiting" }),
            event_payload: json!({ "item": { "itemType": "userInput", "status": "waiting" } }),
            dedupe_key: "waiting:decoy:started".to_string(),
            created_at: 41,
        })
        .await
        .expect("start newer waiting item")
        .item
        .expect("newer waiting item");
    let waiting_checkpoint = MarkTurnWaitingInput {
        project_id: 1,
        thread_id: created_thread.thread_id.clone(),
        turn_id: created.turn.turn_id.clone(),
        run_id: created.run.run_id.clone(),
        status: TurnStatus::WaitingUserInput,
        waiting_item_id: waiting_item.item_id.clone(),
        pending: json!({ "kind": "userInput", "waitingItemId": waiting_item.item_id }),
        updated_at: 42,
    };
    service
        .mark_turn_waiting(waiting_checkpoint.clone())
        .await
        .expect("mark waiting");
    service
        .mark_turn_waiting(MarkTurnWaitingInput { updated_at: 43, ..waiting_checkpoint.clone() })
        .await
        .expect("replay exact waiting checkpoint");
    assert!(service.audit_consistency().await.expect("audit waiting association").is_empty());
    let waiting_conflict = service
        .mark_turn_waiting(MarkTurnWaitingInput {
            pending: json!({
                "kind": "userInput",
                "waitingItemId": waiting_item.item_id,
                "requestSummary": { "changed": true }
            }),
            updated_at: 43,
            ..waiting_checkpoint
        })
        .await
        .expect_err("same waiting Item rejects changed checkpoint");
    assert_eq!(waiting_conflict.stable_code(), "agent.turn.waiting_idempotency_conflict");

    let accept = AcceptTurnInput {
        project_id: 1,
        thread_id: created_thread.thread_id.clone(),
        turn_id: created.turn.turn_id.clone(),
        run_id: created.run.run_id.clone(),
        input_id: "input-1".to_string(),
        content_digest: "sha256:steer".to_string(),
        user_item_payload: json!({ "text": "add detail" }),
        schema_version: 1,
        accepted_at: 50,
    };
    let accepted = service.accept_turn_input(accept.clone()).await.expect("accept input");
    assert_eq!(accepted.input.status, TurnInputStatus::Accepted);
    assert_eq!(accepted.user_item.status, ItemStatus::Completed);
    assert!(service.accept_turn_input(accept.clone()).await.expect("replay input").replayed);

    let complete_input = CompleteTurnInput {
        project_id: 1,
        thread_id: created_thread.thread_id.clone(),
        turn_id: created.turn.turn_id.clone(),
        run_id: created.run.run_id.clone(),
        status: TurnStatus::Completed,
        usage: Some(json!({ "modelCalls": 1 })),
        last_artifact_id: None,
        last_artifact_revision: None,
        error_code: None,
        error_message: None,
        thread_preview: Some("done".to_string()),
        ended_at: 70,
    };
    let error = service
        .complete_turn(complete_input.clone())
        .await
        .expect_err("pending input blocks completion");
    assert_eq!(error.stable_code(), "agent.turn.pending_input");
    let consumed = service
        .consume_turn_input(ConsumeTurnInput {
            project_id: 1,
            thread_id: created_thread.thread_id.clone(),
            turn_id: created.turn.turn_id.clone(),
            run_id: created.run.run_id.clone(),
            input_id: "input-1".to_string(),
            expected_row_version: 1,
            consumed_at: 60,
        })
        .await
        .expect("consume input");
    assert_eq!(consumed.input.status, TurnInputStatus::Consumed);
    assert_eq!(consumed.turn.status, TurnStatus::Running);
    assert_eq!(consumed.run.status, "running");
    assert!(consumed.run.pending.is_none());
    assert_eq!(
        consumed.completed_waiting_item.as_ref().map(|item| item.item_id.as_str()),
        Some(waiting_item.item_id.as_str())
    );
    let untouched_newer_waiting = service
        .find_item(1, &created_thread.thread_id, &created.turn.turn_id, &newer_waiting_item.item_id)
        .await
        .expect("find newer waiting item")
        .expect("newer waiting item exists");
    assert_eq!(untouched_newer_waiting.status, ItemStatus::Waiting);
    assert!(consumed.events.iter().any(|event| event.kind == "agent.user_input.consumed"));
    assert!(consumed.events.iter().any(|event| event.kind == "turn.resumed"));

    let completed = service.complete_turn(complete_input).await.expect("complete turn");
    assert_eq!(completed.turn.status, TurnStatus::Completed);
    assert_eq!(completed.run.status, "completed");
    assert_eq!(completed.closed_items[0].item_id, open_item.item_id);
    assert!(completed.events[0].kind.starts_with("item."));
    assert_eq!(completed.events.last().expect("terminal event").kind, "turn.completed");

    let persisted_input = service
        .find_turn_input(1, &created_thread.thread_id, &created.turn.turn_id, "input-1")
        .await
        .expect("find terminal input")
        .expect("terminal input exists");
    assert_eq!(persisted_input.status, TurnInputStatus::Consumed);
    let terminal_replay = service
        .accept_turn_input(accept.clone())
        .await
        .expect("replay input after terminal");
    assert!(terminal_replay.replayed);
    assert_eq!(terminal_replay.input.status, TurnInputStatus::Consumed);
    let terminal_conflict = service
        .accept_turn_input(AcceptTurnInput { content_digest: "sha256:different".to_string(), ..accept })
        .await
        .expect_err("terminal replay with changed content conflicts");
    assert_eq!(terminal_conflict.stable_code(), "agent.input.idempotency_conflict");

    let events = service
        .list_thread_events(ListThreadEventsQuery { project_id: 1, thread_id: created_thread.thread_id, after_thread_sequence: 0, limit: 100 })
        .await
        .expect("list events");
    assert!(events
        .data
        .windows(2)
        .all(|window| window[0].thread_sequence < window[1].thread_sequence));
    assert!(service.audit_consistency().await.expect("audit").is_empty());
}

#[tokio::test]
async fn restart_interrupts_active_graph_and_abandons_unconsumed_input() {
    let service = service().await;
    let created_thread = thread(&service, "restart", 10).await;
    let created = service
        .create_turn(turn_input(&created_thread.thread_id, "restart", 20))
        .await
        .expect("create turn");
    service
        .claim_turn_running(ClaimTurnRunningInput {
            project_id: 1,
            thread_id: created_thread.thread_id.clone(),
            turn_id: created.turn.turn_id.clone(),
            run_id: created.run.run_id.clone(),
            started_at: 30,
        })
        .await
        .expect("claim turn");
    service
        .start_item(StartItemInput {
            project_id: 1,
            thread_id: created_thread.thread_id.clone(),
            turn_id: created.turn.turn_id.clone(),
            run_id: created.run.run_id.clone(),
            item_id: None,
            logical_key: Some("tool:pending".to_string()),
            item_type: "toolCall".to_string(),
            status: ItemStatus::Waiting,
            schema_version: 1,
            payload: json!({ "tool": "ask" }),
            event_payload: json!({ "tool": "ask" }),
            dedupe_key: "tool:pending:started".to_string(),
            created_at: 40,
        })
        .await
        .expect("start waiting item");
    service
        .accept_turn_input(AcceptTurnInput {
            project_id: 1,
            thread_id: created_thread.thread_id.clone(),
            turn_id: created.turn.turn_id.clone(),
            run_id: created.run.run_id.clone(),
            input_id: "input-restart".to_string(),
            content_digest: "sha256:restart-input".to_string(),
            user_item_payload: json!({ "text": "pending" }),
            schema_version: 1,
            accepted_at: 50,
        })
        .await
        .expect("accept input");

    assert_eq!(service.interrupt_active_turns(60).await.expect("interrupt active"), 1);
    assert_eq!(service.interrupt_active_turns(70).await.expect("idempotent restart"), 0);
    let turn = service
        .find_turn(1, &created_thread.thread_id, &created.turn.turn_id)
        .await
        .expect("find turn")
        .expect("turn exists");
    assert_eq!(turn.status, TurnStatus::Interrupted);
    let run = service
        .find_task_run_by_run_id(&created.run.run_id)
        .await
        .expect("find run")
        .expect("run exists");
    assert_eq!(run.status, "failed");
    assert_eq!(run.error_code.as_deref(), Some("agent.restart_interrupted"));
    assert!(service
        .list_accepted_turn_inputs(1, &created_thread.thread_id, &created.turn.turn_id, 10)
        .await
        .expect("list accepted")
        .is_empty());
    assert!(service.audit_consistency().await.expect("audit").is_empty());
}

#[tokio::test]
async fn restart_resume_creates_an_idempotent_new_attempt_from_checkpoint() {
    let (service, pool) = service_with_pool().await;
    let created_thread = thread(&service, "resume", 10).await;
    let created = service
        .create_turn(turn_input(&created_thread.thread_id, "resume", 20))
        .await
        .expect("create turn");
    service
        .claim_turn_running(ClaimTurnRunningInput {
            project_id: 1,
            thread_id: created_thread.thread_id.clone(),
            turn_id: created.turn.turn_id.clone(),
            run_id: created.run.run_id.clone(),
            started_at: 30,
        })
        .await
        .expect("claim turn");
    let checkpoint_state = json!({
        "schemaVersion": 4,
        "objective": "创建图表",
        "plan": [{ "step": "创建组件", "status": "in_progress" }],
        "nextStep": "创建组件",
        "operationIds": [],
        "requiredCanvasEffects": [],
        "workspaceRevision": 3,
        "updatedAt": 35
    });
    sqlx::query(
        "INSERT INTO ai_agent_working_checkpoint \
         (run_id, task_id, turn_id, checkpoint_version, state_json, updated_at) \
         VALUES (?1, ?2, ?3, 1, ?4, 35)",
    )
    .bind(&created.run.run_id)
    .bind(&created.run.task_id)
    .bind(&created.turn.turn_id)
    .bind(serde_json::to_string(&checkpoint_state).unwrap())
    .execute(&pool)
    .await
    .expect("insert checkpoint");
    assert_eq!(service.interrupt_active_turns(40).await.expect("interrupt active"), 1);

    let input = CreateResumeAttemptInput {
        budget_extension: None,
        project_id: 1,
        thread_id: created_thread.thread_id.clone(),
        turn_id: created.turn.turn_id.clone(),
        previous_run_id: created.run.run_id.clone(),
        expected_checkpoint_version: 1,
        client_session_id: "resume-session".to_string(),
        created_at: 50,
    };
    let resumed = service
        .repository()
        .create_resume_attempt(input.clone())
        .await
        .expect("create resume attempt");
    assert!(!resumed.replayed);
    assert_eq!(resumed.run.attempt_no, 2);
    assert_ne!(resumed.run.run_id, created.run.run_id);
    assert_ne!(resumed.run.task_id, created.run.task_id);
    assert_eq!(resumed.events[0].kind, "agent.resume_attempt.created");

    let replayed = service
        .repository()
        .create_resume_attempt(CreateResumeAttemptInput { created_at: 60, ..input })
        .await
        .expect("replay resume attempt");
    assert!(replayed.replayed);
    assert_eq!(replayed.run.run_id, resumed.run.run_id);
    let copied_state: String = sqlx::query_scalar("SELECT state_json FROM ai_agent_working_checkpoint WHERE run_id = ?1")
        .bind(&resumed.run.run_id)
        .fetch_one(&pool)
        .await
        .expect("load copied checkpoint");
    let copied_state: serde_json::Value = serde_json::from_str(&copied_state).unwrap();
    assert_eq!(copied_state["schemaVersion"], 4);
    assert_eq!(copied_state["operationIds"], serde_json::json!([]));
    assert_eq!(copied_state["requiredCanvasEffects"], serde_json::json!([]));
    assert_eq!(copied_state["runRecoveryState"]["usage"]["totalTokens"], 0);
    assert_eq!(copied_state["updatedAt"], 50);
    assert!(copied_state.get("pending").is_none());
}

#[path = "budget_resume_tests.rs"]
mod budget_resume_tests;
