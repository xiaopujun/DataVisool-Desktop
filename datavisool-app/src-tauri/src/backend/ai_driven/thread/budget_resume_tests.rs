/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use super::*;
use crate::backend::ai_driven::task::{AgentExecutionLimitsSnapshot, AgentWorkingCheckpointState, BudgetExtension};

#[tokio::test]
async fn budget_stop_requires_extension_preserves_usage_and_replays_once() {
    let (service, pool) = service_with_pool().await;
    sqlx::raw_sql(include_str!("../../../../migrations_desktop/1016_desktop_ai_turn_verifying.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../../../../migrations_desktop/1019_desktop_ai_budget_exhausted.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let thread = thread(&service, "budget", 10).await;
    let mut input = turn_input(&thread.thread_id, "budget", 20);
    let limits = AgentExecutionLimitsSnapshot {
        token_budget_enabled: true,
        call_budget_enabled: true,
        duration_budget_enabled: true,
        max_total_tokens: 100_000,
        max_model_calls: 120,
        max_tool_calls: 320,
        max_write_calls: 120,
        max_frontend_interactions: 40,
        max_duration_ms: 4_500_000,
        max_tool_duration_ms: 600_000,
        max_consecutive_failures: 25,
        max_same_read_repeats: 3,
        max_completion_continuations: 12,
        ..Default::default()
    };
    input.limits = serde_json::to_value(&limits).unwrap();
    let created = service.create_turn(input).await.unwrap();
    let found = service
        .find_turn(1, &thread.thread_id, &created.turn.turn_id)
        .await
        .unwrap()
        .unwrap();
    let found_limits = found.limits.as_ref().expect("legacy limits in turn read");
    assert_eq!(found_limits["tokenBudgetEnabled"], true);
    assert_eq!(found_limits["callBudgetEnabled"], true);
    assert_eq!(found_limits["durationBudgetEnabled"], true);
    assert_eq!(found_limits["maxTotalTokens"], 100_000);
    assert_eq!(found_limits["maxModelCalls"], 120);
    let listed = service
        .list_turns(ListTurnsQuery { project_id: 1, thread_id: thread.thread_id.clone(), after_turn_sequence: 0, limit: 10 })
        .await
        .unwrap();
    let listed_limits = listed.data[0].limits.as_ref().expect("limits in turn list");
    assert_eq!(listed_limits["tokenBudgetEnabled"], true);
    assert_eq!(listed_limits["maxTotalTokens"], 100_000);
    service
        .claim_turn_running(ClaimTurnRunningInput {
            project_id: 1,
            thread_id: thread.thread_id.clone(),
            turn_id: created.turn.turn_id.clone(),
            run_id: created.run.run_id.clone(),
            started_at: 30,
        })
        .await
        .unwrap();
    let mut state = AgentWorkingCheckpointState::initial("创建 60 个组件，尚有一批待完成", 35);
    state.operation_ids = vec!["applied-first".into(), "applying-second".into()];
    state.run_recovery_state = json!({"usage":{"totalTokens":90000,"modelCalls":20,"elapsedMs":5000,"budgetWarningIssued":true}});
    sqlx::query("INSERT INTO ai_agent_working_checkpoint VALUES (?1,?2,?3,1,?4,35)")
        .bind(&created.run.run_id)
        .bind(&created.run.task_id)
        .bind(&created.turn.turn_id)
        .bind(serde_json::to_string(&state).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    service
        .complete_turn(CompleteTurnInput {
            project_id: 1,
            thread_id: thread.thread_id.clone(),
            turn_id: created.turn.turn_id.clone(),
            run_id: created.run.run_id.clone(),
            status: TurnStatus::BudgetExhausted,
            usage: Some(json!({"totalTokens":110000,"modelCalls":21,"elapsedMs":8000})),
            last_artifact_id: None,
            last_artifact_revision: None,
            error_code: Some("agent_execution.token_limit".into()),
            error_message: Some("预算耗尽".into()),
            thread_preview: None,
            ended_at: 40,
        })
        .await
        .unwrap();
    let mut resume = CreateResumeAttemptInput {
        budget_extension: None,
        project_id: 1,
        thread_id: thread.thread_id.clone(),
        turn_id: created.turn.turn_id.clone(),
        previous_run_id: created.run.run_id.clone(),
        expected_checkpoint_version: 1,
        client_session_id: "client".into(),
        created_at: 50,
    };
    assert!(service.repository().create_resume_attempt(resume.clone()).await.is_err());
    resume.budget_extension = Some(BudgetExtension { max_total_tokens: Some(105_000), ..Default::default() });
    assert!(service.repository().create_resume_attempt(resume.clone()).await.is_err());
    resume.budget_extension = Some(BudgetExtension { max_total_tokens: Some(200_000), ..Default::default() });
    let resumed = service.repository().create_resume_attempt(resume.clone()).await.unwrap();
    assert_eq!(resumed.run.limits["maxTotalTokens"], 200_000);
    assert_eq!(resumed.run.usage.as_ref().unwrap()["totalTokens"], 110_000);
    assert_eq!(resumed.run.usage.as_ref().unwrap()["modelCalls"], 21);
    assert!(resumed.run.usage.as_ref().unwrap()["elapsedMs"].as_u64().unwrap() >= 8000);
    assert_eq!(resumed.run.usage.as_ref().unwrap()["budgetWarningIssued"], false);
    assert_eq!(resumed.events[0].payload["budgetExtension"]["maxTotalTokens"], 200_000);

    let history = service
        .list_turns(ListTurnsQuery { project_id: 1, thread_id: thread.thread_id.clone(), after_turn_sequence: 0, limit: 10 })
        .await
        .unwrap();
    assert_eq!(history.data.len(), 1);
    let historical_limits = history.data[0].limits.as_ref().expect("latest run limits in turn history");
    assert_eq!(historical_limits["maxTotalTokens"], 200_000);
    assert_eq!(historical_limits["tokenBudgetEnabled"], true);

    let replay = service.repository().create_resume_attempt(resume).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.run.run_id, resumed.run.run_id);
    let saved: String = sqlx::query_scalar("SELECT state_json FROM ai_agent_working_checkpoint WHERE run_id=?1")
        .bind(&resumed.run.run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let saved = AgentWorkingCheckpointState::from_json(&saved).unwrap();
    assert_eq!(saved.operation_ids, state.operation_ids);
    assert_eq!(saved.objective, state.objective);
    assert_eq!(saved.run_recovery_state["usage"]["totalTokens"], 110_000);
    assert_eq!(service.interrupt_active_turns(60).await.unwrap(), 1);
    let old_status: String = sqlx::query_scalar("SELECT status FROM ai_agent_task_run WHERE run_id=?1")
        .bind(&created.run.run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(old_status, "budgetExhausted");
}
