/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use rig_core::completion::AssistantContent;

use super::*;

const TEST_MAX_TOOL_RESULT_TOKENS: u64 = 10_000;

#[test]
fn preserves_all_bounded_results_and_truncates_only_oversized_items() {
    let mut history = Vec::new();
    for index in 0..4 {
        let id = format!("call-{index}");
        history.push(Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call(&id, "inspect_current_page", json!({ "componentIds": [format!("c-{index}")] }))),
        });
        history.push(Message::User {
            content: OneOrMany::one(UserContent::tool_result(
                &id,
                OneOrMany::one(ToolResultContent::text(
                    json!({ "revision": index, "components": [{"componentId": format!("c-{index}"), "data": "x".repeat(3_000)}] }).to_string(),
                )),
            )),
        });
    }

    let (projected, stats) = project_tool_history(&history, TEST_MAX_TOOL_RESULT_TOKENS);

    assert_eq!(stats.projected_results, 0);
    let encoded = serde_json::to_string(&projected).unwrap();
    assert!(!encoded.contains("\"truncated\":true"));
    assert!(encoded.contains("c-0"));
    assert!(encoded.contains(&"x".repeat(3_000)));

    let max_bytes = usize::try_from(TEST_MAX_TOOL_RESULT_TOKENS * 4).unwrap();
    let oversized = vec![Message::tool_result("large", "x".repeat(max_bytes + 1))];
    let (projected, stats) = project_tool_history(&oversized, TEST_MAX_TOOL_RESULT_TOKENS);
    assert_eq!(stats.projected_results, 1);
    let encoded = serde_json::to_string(&projected).unwrap();
    assert!(encoded.contains("\\\"truncated\\\":true"));
    assert!(encoded.contains("original_tool_result"));
    assert!(encoded.contains("large"));
}

#[test]
fn state_digest_keeps_early_operations_and_versioned_reads() {
    let mut history = Vec::new();
    for (id, tool, arguments, result) in [
        (
            "create",
            "create_canvas_components",
            json!({ "components": [{"id": "area"}] }),
            json!({
                "operationId": "op-1",
                "status": "applied",
                "resultRevision": "9",
                "managerId": "manager-1",
                "componentBindings": {"area": "canvas-area"},
                "configuredComponentCount": 1,
                "defaultConfigComponentCount": 0
            }),
        ),
        (
            "specs",
            "resolve_component_specs",
            json!({ "componentTypes": ["AntdBaseArea"], "intent": "create" }),
            json!({
                "corpusVersion": "v3",
                "specs": [{
                    "componentType": "AntdBaseArea",
                    "configurable": {"style": {"xField": "name"}},
                    "dataRequirements": "数组数据，使用 xField 映射名称",
                    "constraints": ["禁止函数值"],
                    "example": {"config": {"style": {"xField": "name"}}}
                }]
            }),
        ),
    ] {
        history.push(Message::Assistant { id: None, content: OneOrMany::one(AssistantContent::tool_call(id, tool, arguments)) });
        history.push(Message::tool_result(id, result.to_string()));
    }
    history.extend((0..20).map(|index| Message::assistant(format!("later-{index}"))));

    let digest = state_digest_with_limits(&history, TEST_MAX_TOOL_RESULT_TOKENS, 16_000, 4_096);

    assert!(digest.contains("op-1"));
    assert!(digest.contains("configuredComponentCount"));
    assert!(digest.contains("canvas-area"));
    assert!(digest.contains("AntdBaseArea"));
    assert!(digest.contains("v3"));
    assert!(digest.contains("\"complete\":true"));
}

#[test]
fn prompt_digest_keeps_inspection_facts_without_snapshot_payload() {
    let history = vec![Message::Assistant {
        id: None,
        content: OneOrMany::one(AssistantContent::tool_call(
            "inspect",
            "inspect_current_page",
            json!({ "componentIds": ["card-1", "card-2"], "fields": ["style", "data"] }),
        )),
    }];
    let prompt = Message::tool_result(
        "inspect",
        json!({
            "components": [
                {"id": "card-1", "type": "KpiCard", "style": {"huge": "x".repeat(20_000)}},
                {"id": "card-2", "type": "KpiCard", "data": {"huge": "x".repeat(20_000)}}
            ],
            "missingComponentIds": [],
            "hasMore": false,
            "offset": 0
        })
        .to_string(),
    );

    let mut complete_history = history;
    complete_history.push(prompt);
    let digest = state_digest_with_limits(&complete_history, TEST_MAX_TOOL_RESULT_TOKENS, 16_000, 4_096);

    assert!(digest.contains("requestedComponentIds"));
    assert!(digest.contains("actualComponentIds"));
    assert!(digest.contains("KpiCard"));
    assert!(digest.contains("actualComponentCount"));
    assert!(!digest.contains(&"x".repeat(20_000)));
}

#[test]
fn rolls_multiple_medium_results_into_the_total_budget() {
    let mut history = Vec::new();
    for index in 0..20 {
        let id = format!("call-{index}");
        history.push(Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call(&id, "inspect_current_page", json!({}))),
        });
        history.push(Message::tool_result(id, "x".repeat(12_000)));
    }

    let (projected, stats) = project_tool_history_with_budget(&history, None, 10_000, 16_000);

    assert!(stats.budget_projected_results > 0);
    assert!(stats.tool_result_bytes_after <= 16_000 * 4);
    assert_eq!(stats.tool_result_bytes_after, projected.iter().map(message_tool_result_bytes).sum::<usize>());
    let encoded = serde_json::to_string(&projected).unwrap();
    assert!(encoded.contains("original_tool_result"));
    assert!(encoded.contains("call-19"));
}

#[test]
fn normalizes_call_only_and_result_only_history() {
    let history = vec![
        Message::Assistant { id: None, content: OneOrMany::one(AssistantContent::tool_call("call-1", "tool", json!({}))) },
        Message::tool_result("orphan", "ignored"),
    ];

    let (projected, stats) = project_tool_history_with_budget(&history, None, 10_000, 16_000);
    let calls = tool_call_names(&projected).into_keys().collect::<HashSet<_>>();
    let results = projected
        .iter()
        .filter_map(|message| match message {
            Message::User { content } => Some(content),
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|item| match item {
            UserContent::ToolResult(result) => Some(result.id.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>();

    assert_eq!(stats.orphan_results_removed, 1);
    assert_eq!(stats.synthetic_results, 1);
    assert_eq!(calls, results);
}

#[test]
fn does_not_synthesize_result_present_in_current_prompt() {
    let history = vec![Message::Assistant { id: None, content: OneOrMany::one(AssistantContent::tool_call("call-1", "update_plan", json!({}))) }];
    let prompt = Message::tool_result("call-1", json!({ "status": "ok" }).to_string());

    let (mut projected, stats) = project_tool_history_with_budget(&history, Some(&prompt), 10_000, 16_000);
    projected.push(prompt);
    let result_count = projected
        .iter()
        .filter_map(|message| match message {
            Message::User { content } => Some(content),
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter(|item| matches!(item, UserContent::ToolResult(result) if result.id == "call-1"))
        .count();

    assert_eq!(stats.synthetic_results, 0);
    assert_eq!(result_count, 1);
}

#[test]
fn bounds_state_digest_even_when_many_facts_are_present() {
    let mut history = Vec::new();
    for index in 0..100 {
        let id = format!("call-{index}");
        history.push(Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call(&id, "create_canvas_components", json!({}))),
        });
        history.push(Message::tool_result(
            id,
            json!({ "operationId": format!("operation-{index}"), "status": "applied", "componentBindings": { "x": "y" } }).to_string(),
        ));
    }

    let digest = state_digest_with_limits(&history, 10_000, 16_000, 512);

    assert!(digest.len() <= 512 * 4);
    assert!(digest.contains("operation-0"));
    assert!(digest.contains("operation-99"));
}

#[test]
fn state_digest_keeps_all_operation_references_and_pending_ids() {
    let mut history = Vec::new();
    for index in 0..40 {
        let id = format!("call-{index}");
        history.push(Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call(id.clone(), "create_canvas_components", json!({}))),
        });
        history.push(Message::tool_result(
            id,
            json!({ "operationId": format!("operation-{index}"), "status": if index == 39 { "applying" } else { "applied" } }).to_string(),
        ));
    }

    let digest = state_digest_with_limits(&history, 10_000, 16_000, 4_096);

    assert!(digest.contains("\"operationIds\""));
    assert!(digest.contains("operation-0"));
    assert!(digest.contains("operation-39"));
    assert!(digest.contains("\"pendingOperationIds\":[\"operation-39\"]"));
}

#[test]
fn state_digest_reads_operation_ids_before_result_projection() {
    let history = vec![
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call("call", "create_canvas_components", json!({}))),
        },
        Message::tool_result(
            "call",
            json!({ "operationId": "op-original", "status": "applying", "payload": "x".repeat(20_000) }).to_string(),
        ),
    ];

    let digest = state_digest_with_limits(&history, 1, 1, 512);

    assert!(digest.contains("op-original"));
    assert!(digest.contains("pendingOperationIds"));
}

#[test]
fn read_projection_preserves_pagination_and_recovery_arguments() {
    let history = vec![
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call(
                "read-page",
                "read_workspace_file",
                json!({ "path": "workspace/data.json", "page": 3, "startOffset": 40, "nextStartOffset": 60 }),
            )),
        },
        Message::tool_result(
            "read-page",
            json!({ "path": "workspace/data.json", "page": 3, "startOffset": 40, "nextStartOffset": 60, "sourceVersion": "v7", "content": "x" })
                .to_string(),
        ),
    ];

    let digest = state_digest_with_limits(&history, 10_000, 16_000, 4_096);
    assert!(digest.contains("startOffset"));
    assert!(digest.contains("nextStartOffset"));
    assert!(digest.contains("\"page\":3"));

    let (projected, _) = project_tool_history(&history, 1);
    let encoded = serde_json::to_string(&projected).unwrap();
    assert!(encoded.contains("startOffset"));
    assert!(encoded.contains("nextStartOffset"));
}

#[test]
fn state_digest_carries_operation_ids_across_compactions() {
    let prior = Message::user(
        "[Agent context summary]\n已有摘要\n\n[Agent state digest]\n{\"operationIds\":[\"operation-0\"],\"pendingOperationIds\":[\"operation-0\"],\"operations\":[{\"operationId\":\"operation-0\",\"status\":\"applying\"}]}\n\n[Execution budget]\n{}",
    );
    let current = vec![
        prior,
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call("create-1", "create_batch", json!({}))),
        },
        Message::tool_result("create-1", json!({ "operationId": "operation-1", "status": "applied" }).to_string()),
    ];
    let digest = state_digest_with_limits(&current, 10_000, 16_000, 4_096);
    assert!(digest.contains("operation-0"));
    assert!(digest.contains("operation-1"));
    assert!(digest.contains("\"pendingOperationIds\":[\"operation-0\"]"));
}

#[test]
fn oversized_operation_reference_digest_is_marked_unrecoverable() {
    let mut history = Vec::new();
    for index in 0..100 {
        let id = format!("call-{index}");
        history.push(Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call(id.clone(), "create_canvas_components", json!({}))),
        });
        history.push(Message::tool_result(
            id,
            json!({ "operationId": format!("operation-{index}-with-a-long-reference"), "status": "applied" }).to_string(),
        ));
    }
    let digest = state_digest_with_limits(&history, 10_000, 16_000, 8);
    assert!(digest.contains("\"unrecoverable\":true"));
}
