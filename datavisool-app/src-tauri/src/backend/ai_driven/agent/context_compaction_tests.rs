/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use rig_core::message::{ToolResultContent, UserContent};
use rig_core::test_utils::{MockCompletionModel, MockTurn};
use rig_core::OneOrMany;
use serde_json::json;

use super::*;

#[test]
fn applied_compaction_preserves_specs_failed_actions_and_multiple_tool_pairs() {
    let history = vec![
        Message::user("创建后按需要微调"),
        Message::Assistant {
            id: None,
            content: OneOrMany::many([
                AssistantContent::tool_call("spec", "resolve_component_specs", json!({"componentTypes":["BaseColorBlock"],"intent":"create"})),
                AssistantContent::tool_call("read", "read_workspace_file", json!({"path":"theme.json"})),
            ])
            .unwrap(),
        },
        Message::tool_result(
            "spec",
            r##"{"corpusVersion":"docs-v1","specs":[{"componentType":"BaseColorBlock","configurable":{"style":{"color":"#123456"}},"example":{"config":{"style":{"color":"#123456"}}}}]}"##,
        ),
        Message::tool_result("read", r#"{"code":"permission_denied","recovery":"do_not_retry"}"#),
        Message::assistant("过时分析".repeat(300)),
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call("write", CREATE_CANVAS_COMPONENTS_TOOL, json!({"components":[]}))),
        },
    ];
    let prompt = Message::tool_result("write", r#"{"status":"applied","operationId":"op"}"#);
    let mut compacted = build_post_action_history(&history, &prompt, PostActionCompaction::AppliedWrite, &ExecutionUsageSnapshot::default());
    compacted.push(prompt);
    assert_eq!(tool_call_ids(&compacted), tool_result_ids(&compacted));
    let text = serde_json::to_string(&compacted).unwrap();
    assert!(text.contains("#123456"));
    assert!(text.contains("permission_denied"));
    assert!(!text.contains("过时分析"));
    let essential = essential_tool_history(&history[..3]);
    assert_eq!(essential.len(), 2);
}

#[test]
fn essential_history_keeps_a_compact_successful_workspace_read_receipt() {
    let history = vec![
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call(
                "read-receipt",
                "read_workspace_file",
                json!({ "path": "workspace/data.json", "page": 2, "startOffset": 20, "nextStartOffset": 40 }),
            )),
        },
        Message::tool_result(
            "read-receipt",
            json!({
                "path": "workspace/data.json",
                "sourceVersion": "v7",
                "page": 2,
                "startOffset": 20,
                "nextStartOffset": 40,
                "content": "x".repeat(8_000)
            })
            .to_string(),
        ),
    ];

    let essential = essential_tool_history(&history);
    let encoded = serde_json::to_string(&essential).unwrap();
    assert!(encoded.contains("read-receipt"));
    assert!(encoded.contains("recoveryReceipt"));
    assert!(encoded.contains("original_tool_result"));
    assert!(encoded.contains("sourceVersion"));
    assert!(encoded.contains("nextStartOffset"));
    assert!(!encoded.contains(&"x".repeat(8_000)));
    assert_eq!(tool_call_ids(&essential), tool_result_ids(&essential));
}

#[test]
fn essential_history_keeps_complete_spec_identity_for_legal_reread() {
    let history = vec![
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call(
                "spec-receipt",
                "resolve_component_specs",
                json!({ "componentTypes": ["DigitalFlipper"], "intent": "create" }),
            )),
        },
        Message::tool_result(
            "spec-receipt",
            json!({
                "corpusVersion": "docs-v1",
                "specs": [{
                    "componentType": "DigitalFlipper",
                    "configurable": { "style": { "fontSize": { "type": "number" } } },
                    "example": { "config": { "style": { "fontSize": 16 } } }
                }]
            })
            .to_string(),
        ),
    ];

    let essential = essential_tool_history(&history);
    let encoded = serde_json::to_string(&essential).unwrap();
    assert!(encoded.contains("recoveryReceipt"));
    assert!(encoded.contains("DigitalFlipper"));
    assert!(encoded.contains("docs-v1"));
    assert!(encoded.contains("complete"));
}

#[test]
fn reserves_output_and_next_turn_capacity() {
    let limits = AgentContextLimits::default();
    assert_eq!(context_input_limit(1_000_000, Some(8_000), &limits), 128_000);
    assert_eq!(context_input_limit(16_000, Some(8_000), &limits), 9_000);
}

#[test]
fn skips_summary_when_expected_savings_do_not_cover_the_minimum() {
    assert!(!should_compact_history(20_000, 12_000, 4_096));
    assert!(!should_compact_history(10_000, 1_000, 4_096));
    assert!(should_compact_history(64_000, 8_000, 4_096));
}

#[test]
fn only_reaches_compaction_at_the_token_threshold() {
    assert!(!reached_compaction_threshold(30_000, 32_000, 100_000));
    assert!(reached_compaction_threshold(100_000, 32_000, 100_000));
    assert!(reached_compaction_threshold(30_000, 100_000, 100_000));
}

#[test]
fn provider_usage_correction_includes_new_messages_since_last_request() {
    // The previous provider usage can exceed the local estimate. Preserve only
    // that excess while adding the current request estimate, so new tool
    // results cannot be hidden by the stale provider value.
    assert_eq!(corrected_context_tokens(50_000, 40_000, 42_000), 52_000);
    assert!(reached_compaction_threshold_with_correction(50_000, 42_000, 51_000, 40_000));
    assert!(!reached_compaction_threshold_with_correction(50_000, 39_000, 51_000, 40_000));
}

#[test]
fn essential_history_keeps_applied_results_for_registered_or_custom_writes() {
    let history = vec![
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call("create-call", "create_batch", json!({ "batch": 0 }))),
        },
        Message::tool_result("create-call", json!({ "status": "applied", "operationId": "operation-0" }).to_string()),
    ];
    let projected = essential_tool_history(&history);
    let encoded = serde_json::to_string(&projected).unwrap();
    assert!(encoded.contains("create_batch"));
    assert!(encoded.contains("operation-0"));
}

#[test]
fn compacted_suffix_preserves_matching_tool_call() {
    let mut history = vec![Message::user("原始用户要求")];
    history.extend((0..12).map(|index| Message::assistant(format!("old-{index}"))));
    history.push(Message::Assistant {
        id: None,
        content: OneOrMany::one(AssistantContent::tool_call("call-1", "read_artifact", json!({ "artifactId": "result" }))),
    });
    history.push(Message::User {
        content: OneOrMany::one(UserContent::tool_result("call-1", OneOrMany::one(ToolResultContent::text("revision 4")))),
    });

    let compacted = build_compacted_history_with_limits(&history, "summary", &ExecutionUsageSnapshot::default(), 16_384, 10_000, 16_000, 4_096, None);

    assert!(tool_result_ids(&compacted).is_subset(&tool_call_ids(&compacted)));
    assert!(compacted.len() < history.len());
}

#[test]
fn compacted_suffix_requires_both_call_and_result_directions() {
    let mut history = vec![Message::user("原始用户要求")];
    history.extend((0..10).map(|index| Message::assistant(format!("old-{index}"))));
    history.push(Message::Assistant { id: None, content: OneOrMany::one(AssistantContent::tool_call("call-only", "tool", json!({}))) });

    let start = recent_complete_suffix_start(&history);

    assert_eq!(start, 0);
}

#[tokio::test]
async fn model_compaction_uses_a_bounded_request_and_returns_usage() {
    let model = MockCompletionModel::new([MockTurn::text("保留 revision 4，并继续 validate_artifact")]);

    let (summary, _) = summarize_history(&model, &[Message::user("生成页面")], 4_096).await.unwrap();

    assert!(summary.contains("revision 4"));
    let requests = model.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].max_tokens, Some(4_096));
    assert!(requests[0].tools.is_empty());
}

#[test]
fn fallback_summary_excludes_tool_arguments_and_results() {
    let history = vec![
        Message::user("用户要求"),
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call("call-secret", "tool", json!({ "apiKey": "secret" }))),
        },
        Message::User {
            content: OneOrMany::one(UserContent::tool_result("call-secret", OneOrMany::one(ToolResultContent::text("secret-result")))),
        },
    ];

    let summary = fallback_summary(&history);

    assert!(summary.contains("用户要求"));
    assert!(!summary.contains("secret"));
}

#[test]
fn applied_write_compaction_keeps_current_pair_and_drops_old_exploration() {
    let history = vec![
        Message::user("创建 KPI 指标卡片"),
        Message::assistant("旧规格与推理".repeat(2_000)),
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call(
                "write",
                CREATE_CANVAS_COMPONENTS_TOOL,
                json!({ "components": [{"id": "card-1", "type": "BaseColorBlock"}] }),
            )),
        },
    ];
    let prompt = Message::tool_result("write", json!({ "status": "applied", "operationId": "op-1", "componentIds": ["card-1"] }).to_string());

    assert!(matches!(
        post_action_compaction(&history, &prompt, AgentRoutePhase::Executing),
        Some(PostActionCompaction::AppliedWrite)
    ));
    let compacted = build_post_action_history(&history, &prompt, PostActionCompaction::AppliedWrite, &ExecutionUsageSnapshot::default());
    let encoded = serde_json::to_string(&compacted).unwrap();

    assert!(encoded.contains("创建 KPI 指标卡片"));
    assert!(serde_json::to_string(&prompt).unwrap().contains("op-1"));
    assert!(encoded.contains("card-1"));
    assert!(!encoded.contains(&"旧规格与推理".repeat(2_000)));
    assert!(current_prompt_tool_result_id(&prompt).is_some());
    assert!(encoded.len() < serde_json::to_string(&history).unwrap().len());
}

#[test]
fn verification_compaction_rejects_failed_reads_and_compacts_successful_snapshots() {
    let history = vec![
        Message::user("创建 KPI 指标卡片"),
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call(
                "inspect",
                INSPECT_CURRENT_PAGE_TOOL,
                json!({ "componentIds": ["card-1"], "fields": ["layer"] }),
            )),
        },
    ];
    let success = Message::tool_result(
        "inspect",
        json!({ "components": [{"id": "card-1", "type": "BaseColorBlock"}], "missingComponentIds": [] }).to_string(),
    );
    let failed = Message::tool_result("inspect", json!({ "status": "error", "code": "snapshot_failed" }).to_string());

    assert!(matches!(
        post_action_compaction(&history, &success, AgentRoutePhase::Verifying),
        Some(PostActionCompaction::Verification)
    ));
    assert!(post_action_compaction(&history, &failed, AgentRoutePhase::Verifying).is_none());
    assert!(post_action_compaction(&history, &success, AgentRoutePhase::Ready).is_none());
}

#[test]
fn summary_history_is_bounded_before_summary_model_request() {
    let mut history = vec![Message::user("保留用户目标")];
    history.extend((0..12).map(|index| Message::assistant(format!("old-{index} ").repeat(2_000))));
    let prompt = Message::user("继续");

    let bounded = fit_summary_history(&history, &prompt, 1_024, 128, 10_000, 16_000).unwrap();

    assert!(summary_request_tokens(&bounded, 128) <= 1_024);
    assert!(serde_json::to_string(&bounded).unwrap().contains("保留用户目标"));
    assert!(!serde_json::to_string(&bounded).unwrap().contains("old-0"));
}

#[test]
fn summary_history_rejects_an_unfit_current_tool_pair() {
    let history = vec![
        Message::user("创建组件"),
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call("current", "inspect_current_page", json!({ "blob": "x".repeat(20_000) }))),
        },
    ];
    let prompt = Message::tool_result("current", "{}");

    assert!(fit_summary_history(&history, &prompt, 128, 32, 10_000, 16_000).is_none());
}

#[test]
fn compacted_history_keeps_the_current_tool_call_when_dropping_old_text() {
    let input = crate::backend::ai_driven::agent_context::input::AgentRunInput::new("创建组件");
    let plan = AgentContextPlan::for_route(&input, false, AgentRoutePhase::Executing, vec![]);
    let history = vec![
        Message::user("创建组件"),
        Message::user("[Agent context summary]\n已创建的操作须继续核验"),
        Message::assistant("old-analysis ".repeat(8_000)),
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call("current", "inspect_current_page", json!({}))),
        },
    ];
    let prompt = Message::tool_result("current", r#"{"status":"ok"}"#);

    let compacted = fit_compacted_history(&prompt, history.clone(), &history[..3], &plan, &HashMap::new(), 2_000, 10_000, 16_000).unwrap();

    assert!(tool_call_ids(&compacted).contains("current"));
    assert!(!serde_json::to_string(&compacted).unwrap().contains("old-analysis"));
}

#[test]
fn compacted_request_keeps_the_current_complete_spec_prompt_unmodified() {
    let input = crate::backend::ai_driven::agent_context::input::AgentRunInput::new("创建图表");
    let plan = AgentContextPlan::for_route(&input, false, AgentRoutePhase::Executing, vec![]);
    let marker = "complete-spec-marker".repeat(1_000);
    let history = vec![
        Message::user("创建图表"),
        Message::assistant("old-analysis ".repeat(8_000)),
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call(
                "current-spec",
                "resolve_component_specs",
                json!({ "componentTypes": ["AntdBaseArea"], "intent": "create" }),
            )),
        },
    ];
    let prompt = Message::tool_result(
        "current-spec",
        json!({
            "corpusVersion": "docs-v1",
            "specs": [{
                "componentType": "AntdBaseArea",
                "configurable": { "style": { "description": marker } },
                "example": { "config": { "style": { "xField": "name", "yField": "value" } } }
            }]
        })
        .to_string(),
    );
    let compacted_candidate = build_compacted_history_with_limits(
        &history,
        "已保留用户目标",
        &ExecutionUsageSnapshot::default(),
        4_096,
        10_000,
        16_000,
        4_096,
        Some(&prompt),
    );

    let compacted = fit_compacted_history(&prompt, compacted_candidate, &history, &plan, &HashMap::new(), 16_000, 10_000, 16_000).unwrap();

    let dispatched = serde_json::to_string(&(compacted, prompt)).unwrap();
    assert!(dispatched.contains("complete-spec-marker"));
    assert!(!dispatched.contains("old-analysis"));
}
