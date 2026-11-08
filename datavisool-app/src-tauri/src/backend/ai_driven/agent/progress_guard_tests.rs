/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use super::*;

#[test]
fn regrouped_specs_do_not_reset_the_actual_guard() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    for (index, style, expected) in [(0, json!({"color": {}}), true), (1, json!({"font": {}}), false), (2, json!({"font": {}, "color": {}}), false)] {
        let args = json!({"componentTypes":["Text"],"intent":"create"}).to_string();
        let call = format!("atomic-{index}");
        guard
            .before_call(&call, RESOLVE_COMPONENT_SPECS_TOOL, &args, ToolCallKind::Read, true)
            .unwrap();
        let result =
            json!({"corpusVersion":"v1","specs":[{"componentType":"Text","configurable":{"style":style},"example":{"config":{"style":{}}}}]})
                .to_string();
        assert_eq!(
            guard
                .finish_call(&call, RESOLVE_COMPONENT_SPECS_TOOL, ToolCallKind::Read, &result, true)
                .produced_new_fact,
            expected
        );
    }
}
use crate::backend::ai_driven::agent::AgentToolRoute;

#[test]
fn json_property_order_does_not_change_fingerprint() {
    assert_eq!(fingerprint("read", r#"{"a":1,"b":2}"#, Some("v1")), fingerprint("read", r#"{"b":2,"a":1}"#, Some("v1")));
}

#[test]
fn component_spec_component_order_uses_the_same_call_key() {
    let first = canonical_call_key(RESOLVE_COMPONENT_SPECS_TOOL, r#"{"componentTypes":["B","A"],"intent":"create"}"#);
    let second = canonical_call_key(RESOLVE_COMPONENT_SPECS_TOOL, r#"{"intent":"create","componentTypes":["A","B"]}"#);
    assert_eq!(first, second);
}

#[test]
fn volatile_timing_does_not_create_a_new_fact() {
    assert_eq!(
        semantic_fact_signature("", r#"{"matches":[],"elapsedMs":1}"#),
        semantic_fact_signature("", r#"{"elapsedMs":99,"matches":[]}"#)
    );
}

#[test]
fn timestamps_and_resolution_metadata_do_not_create_a_new_fact() {
    assert_eq!(
        semantic_fact_signature("", r#"{"matches":["BaseText"],"timestamp":1,"elapsedMs":1}"#,),
        semantic_fact_signature("", r#"{"matches":["BaseText"],"timestamp":2,"elapsedMs":99}"#,),
    );
}

#[test]
fn component_spec_facts_use_version_and_complete_shape_instead_of_request_shape() {
    let first = semantic_fact_signature(
        RESOLVE_COMPONENT_SPECS_TOOL,
        r#"{"corpusVersion":"docs-v1","specs":[{"componentType":"BaseText","configurable":{"style":{"color":"red"}},"example":{"config":{"style":{"color":"red"}}}}]}"#,
    );
    let same_fact = semantic_fact_signature(
        RESOLVE_COMPONENT_SPECS_TOOL,
        r#"{"corpusVersion":"docs-v1","specs":[{"componentType":"BaseText","configurable":{"style":{"font":"12px"}},"example":{"config":{"style":{"font":"12px"}}}}]}"#,
    );
    let new_path = semantic_fact_signature(
        RESOLVE_COMPONENT_SPECS_TOOL,
        r#"{"corpusVersion":"docs-v1","specs":[{"componentType":"BaseText","configurable":{"style":{"color":"blue"}},"example":{"config":{"style":{"color":"blue"}}}}]}"#,
    );
    assert_eq!(first, same_fact);
    assert_eq!(first, new_path);
}

#[test]
fn versioned_component_specs_are_reexecuted_to_observe_new_corpus_version() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    let args = r#"{"componentTypes":["BaseText"],"intent":"create"}"#;
    let first = guard
        .before_call("1", RESOLVE_COMPONENT_SPECS_TOOL, args, ToolCallKind::Read, true)
        .unwrap();
    assert!(matches!(first, ProgressGuardAction::Execute(_)));
    guard.finish_call(
        "1",
        RESOLVE_COMPONENT_SPECS_TOOL,
        ToolCallKind::Read,
        r#"{"corpusVersion":"docs-v1","specs":[{"componentType":"BaseText","configurable":{"style":{"color":"red"}},"example":{"config":{"style":{"color":"red"}}}}]}"#,
        true,
    );

    let second = guard
        .before_call("2", RESOLVE_COMPONENT_SPECS_TOOL, args, ToolCallKind::Read, true)
        .unwrap();
    assert!(matches!(second, ProgressGuardAction::Execute(_)));
    let metadata = guard.finish_call(
        "2",
        RESOLVE_COMPONENT_SPECS_TOOL,
        ToolCallKind::Read,
        r#"{"corpusVersion":"docs-v2","specs":[{"componentType":"BaseText","configurable":{"style":{"color":"red"}},"example":{"config":{"style":{"color":"red"}}}}]}"#,
        true,
    );
    assert!(metadata.produced_new_fact);
    assert_eq!(metadata.source_version.as_deref(), Some("docs-v2"));
}

#[test]
fn alternating_reads_stop_after_no_progress_threshold() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    let args_a = r#"{"componentTypes":["BaseText"],"intent":"create"}"#;
    let args_b = r#"{"componentTypes":["Button"],"intent":"create"}"#;
    let result = r#"{"corpusVersion":"docs-v1","specs":[{"componentType":"BaseText","configurable":{"style":{"color":"red"}},"example":{"config":{"style":{"color":"red"}}}}]}"#;

    guard
        .before_call("a1", RESOLVE_COMPONENT_SPECS_TOOL, args_a, ToolCallKind::Read, true)
        .unwrap();
    guard.finish_call("a1", RESOLVE_COMPONENT_SPECS_TOOL, ToolCallKind::Read, result, true);
    guard
        .before_call("b1", RESOLVE_COMPONENT_SPECS_TOOL, args_b, ToolCallKind::Read, true)
        .unwrap();
    guard.finish_call("b1", RESOLVE_COMPONENT_SPECS_TOOL, ToolCallKind::Read, result, true);
    guard
        .before_call("a2", RESOLVE_COMPONENT_SPECS_TOOL, args_a, ToolCallKind::Read, true)
        .unwrap();
    guard.finish_call("a2", RESOLVE_COMPONENT_SPECS_TOOL, ToolCallKind::Read, result, true);
    guard
        .before_call("b2", RESOLVE_COMPONENT_SPECS_TOOL, args_b, ToolCallKind::Read, true)
        .unwrap();
    guard.finish_call("b2", RESOLVE_COMPONENT_SPECS_TOOL, ToolCallKind::Read, result, true);

    let violation = guard
        .before_call("a3", RESOLVE_COMPONENT_SPECS_TOOL, args_a, ToolCallKind::Read, true)
        .unwrap_err();
    assert_eq!(violation.code, AgentExecutionErrorCode::NoProgressLimit);
    assert_eq!(violation.reads_without_business_progress, 3);
}

#[test]
fn operation_status_without_changes_eventually_stops() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    let args = r#"{"operationId":"op-1"}"#;
    let result = r#"{"operationId":"op-1","status":"applying","resultRevision":1,"componentIds":["cmp-1"]}"#;

    for call_id in ["1", "2", "3", "4"] {
        guard
            .before_call(call_id, GET_CANVAS_OPERATION_STATUS_TOOL, args, ToolCallKind::Read, true)
            .unwrap();
        guard.finish_call(call_id, GET_CANVAS_OPERATION_STATUS_TOOL, ToolCallKind::Read, result, true);
    }

    let violation = guard
        .before_call("5", GET_CANVAS_OPERATION_STATUS_TOOL, args, ToolCallKind::Read, true)
        .unwrap_err();
    assert_eq!(violation.code, AgentExecutionErrorCode::NoProgressLimit);
    assert_eq!(violation.repeat_count, 4);
}

#[test]
fn paginated_new_components_reset_no_progress_between_pages() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    for (call_id, offset, component_id) in [("1", 0, "cmp-1"), ("2", 2, "cmp-2"), ("3", 4, "cmp-3")] {
        let args = format!(r#"{{"offset":{offset},"limit":2}}"#);
        let result = format!(r#"{{"components":[{{"id":"{component_id}"}}],"offset":{offset},"nextOffset":{next}}}"#, next = offset + 2);
        guard
            .before_call(call_id, INSPECT_CURRENT_PAGE_TOOL, &args, ToolCallKind::FrontendInteraction, true)
            .unwrap();
        let metadata = guard.finish_call(call_id, INSPECT_CURRENT_PAGE_TOOL, ToolCallKind::FrontendInteraction, &result, true);
        assert!(metadata.produced_new_fact, "page {offset} should be a new fact");
    }
}

#[test]
fn pagination_cursor_without_new_component_ids_is_not_progress() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    guard
        .before_call(
            "1",
            INSPECT_CURRENT_PAGE_TOOL,
            r#"{"allPage":true,"offset":0,"limit":2}"#,
            ToolCallKind::FrontendInteraction,
            true,
        )
        .unwrap();
    let first = guard.finish_call(
        "1",
        INSPECT_CURRENT_PAGE_TOOL,
        ToolCallKind::FrontendInteraction,
        r#"{"components":[{"id":"cmp-1"}],"offset":0,"nextOffset":2}"#,
        true,
    );
    assert!(first.produced_new_fact);

    guard
        .before_call(
            "2",
            INSPECT_CURRENT_PAGE_TOOL,
            r#"{"allPage":true,"offset":2,"limit":2}"#,
            ToolCallKind::FrontendInteraction,
            true,
        )
        .unwrap();
    let repeated_ids = guard.finish_call(
        "2",
        INSPECT_CURRENT_PAGE_TOOL,
        ToolCallKind::FrontendInteraction,
        r#"{"components":[{"id":"cmp-1"}],"offset":2,"nextOffset":4}"#,
        true,
    );
    assert!(!repeated_ids.produced_new_fact);

    guard
        .before_call(
            "3",
            INSPECT_CURRENT_PAGE_TOOL,
            r#"{"allPage":true,"offset":4,"limit":2}"#,
            ToolCallKind::FrontendInteraction,
            true,
        )
        .unwrap();
    let empty_page = guard.finish_call(
        "3",
        INSPECT_CURRENT_PAGE_TOOL,
        ToolCallKind::FrontendInteraction,
        r#"{"components":[],"offset":4,"nextOffset":6}"#,
        true,
    );
    assert!(!empty_page.produced_new_fact);
}

#[test]
fn unversioned_component_specs_are_not_cached() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    let args = r#"{"componentTypes":["BaseText"],"intent":"create"}"#;
    guard
        .before_call("1", RESOLVE_COMPONENT_SPECS_TOOL, args, ToolCallKind::Read, true)
        .unwrap();
    guard.finish_call(
        "1",
        RESOLVE_COMPONENT_SPECS_TOOL,
        ToolCallKind::Read,
        r#"{"specs":[{"componentType":"BaseText","configurable":{"style":{"color":"red"}},"example":{"config":{"style":{"color":"red"}}}}]}"#,
        true,
    );

    let second = guard
        .before_call("2", RESOLVE_COMPONENT_SPECS_TOOL, args, ToolCallKind::Read, true)
        .unwrap();
    assert!(matches!(second, ProgressGuardAction::Execute(_)));
    let metadata = guard.finish_call(
        "2",
        RESOLVE_COMPONENT_SPECS_TOOL,
        ToolCallKind::Read,
        r#"{"specs":[{"componentType":"Button","configurable":{"style":{"color":"blue"}},"example":{"config":{"style":{"color":"blue"}}}}]}"#,
        true,
    );
    assert!(!metadata.produced_new_fact);
}

#[test]
fn component_spec_corpus_version_invalidation_is_a_new_fact() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    let args_v1 = r#"{"componentTypes":["BaseText"],"intent":"create"}"#;
    let args_v2 = r#"{"componentTypes":["BaseText"],"intent":"create"}"#;
    guard
        .before_call("1", RESOLVE_COMPONENT_SPECS_TOOL, args_v1, ToolCallKind::Read, true)
        .unwrap();
    guard.finish_call(
        "1",
        RESOLVE_COMPONENT_SPECS_TOOL,
        ToolCallKind::Read,
        r#"{"corpusVersion":"docs-v1","specs":[{"componentType":"BaseText","configurable":{"style":{"color":"red"}},"example":{"config":{"style":{"color":"red"}}}}]}"#,
        true,
    );
    guard
        .before_call("2", RESOLVE_COMPONENT_SPECS_TOOL, args_v2, ToolCallKind::Read, true)
        .unwrap();
    let metadata = guard.finish_call(
        "2",
        RESOLVE_COMPONENT_SPECS_TOOL,
        ToolCallKind::Read,
        r#"{"corpusVersion":"docs-v2","specs":[{"componentType":"BaseText","configurable":{"style":{"color":"red"}},"example":{"config":{"style":{"color":"red"}}}}]}"#,
        true,
    );
    assert!(metadata.produced_new_fact);
    assert_eq!(metadata.source_version.as_deref(), Some("docs-v2"));
}

#[test]
fn new_read_fact_resets_progress_counter() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    guard
        .before_call("1", "inspect_current_page", "{}", ToolCallKind::FrontendInteraction, true)
        .unwrap();
    let first = guard.finish_call(
        "1",
        "inspect_current_page",
        ToolCallKind::FrontendInteraction,
        r#"{"components":[{"id":"cmp-1"}],"revision":"1"}"#,
        true,
    );
    assert!(first.produced_new_fact);
    guard
        .before_call("2", "inspect_current_page", "{}", ToolCallKind::FrontendInteraction, true)
        .unwrap();
    guard.finish_call(
        "2",
        "inspect_current_page",
        ToolCallKind::FrontendInteraction,
        r#"{"components":[{"id":"cmp-1"}],"revision":"1"}"#,
        true,
    );
    assert!(guard
        .before_call("3", "inspect_current_page", "{}", ToolCallKind::FrontendInteraction, true)
        .is_ok());

    guard
        .before_call("4", "create_canvas_components", "{}", ToolCallKind::WriteFrontendInteraction, true)
        .unwrap();
    let write = guard.finish_call("4", "create_canvas_components", ToolCallKind::WriteFrontendInteraction, "{}", true);
    assert!(write.business_write);
    assert!(guard
        .before_call("5", "inspect_current_page", "{}", ToolCallKind::FrontendInteraction, true)
        .is_ok());
}

#[test]
fn stops_after_three_unproductive_component_resolutions() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    for index in 1..=3 {
        let call_id = index.to_string();
        guard
            .before_call(
                &call_id,
                RESOLVE_COMPONENT_SPECS_TOOL,
                &format!(r#"{{"componentTypes":["Missing{index}"],"intent":"create"}}"#),
                ToolCallKind::Read,
                true,
            )
            .unwrap();
        guard.finish_call(
            &call_id,
            RESOLVE_COMPONENT_SPECS_TOOL,
            ToolCallKind::Read,
            r#"{"catalogVersion":"v1","totalMatches":0,"specs":[]}"#,
            true,
        );
    }

    let violation = guard
        .before_call(
            "4",
            RESOLVE_COMPONENT_SPECS_TOOL,
            r#"{"componentTypes":["Another"],"intent":"create"}"#,
            ToolCallKind::Read,
            true,
        )
        .unwrap_err();

    assert_eq!(violation.code, AgentExecutionErrorCode::NoProgressLimit);
    assert_eq!(violation.repeat_count, 4);
}

#[test]
fn successful_component_resolution_resets_empty_counter() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    for (call_id, result) in [
        ("1", r#"{"totalMatches":0,"specs":[]}"#),
        ("2", r#"{"totalMatches":0,"specs":[]}"#),
        (
            "3",
            r#"{"corpusVersion":"docs-v1","specs":[{"componentType":"BaseText","configurable":{"style":{"color":"red"}},"example":{"config":{"style":{"color":"red"}}}}]}"#,
        ),
    ] {
        guard
            .before_call(
                call_id,
                RESOLVE_COMPONENT_SPECS_TOOL,
                &format!(r#"{{"componentTypes":["Type{call_id}"],"intent":"create"}}"#),
                ToolCallKind::Read,
                true,
            )
            .unwrap();
        guard.finish_call(call_id, RESOLVE_COMPONENT_SPECS_TOOL, ToolCallKind::Read, result, true);
    }

    assert!(guard
        .before_call(
            "4",
            RESOLVE_COMPONENT_SPECS_TOOL,
            r#"{"componentTypes":["Next"],"intent":"create"}"#,
            ToolCallKind::Read,
            true,
        )
        .is_ok());
}

#[test]
fn unchanged_plan_does_not_fake_business_progress() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    let plan = r#"{"plan":[{"step":"create","status":"in_progress"}]}"#;

    guard
        .before_call("1", UPDATE_PLAN_TOOL, plan, ToolCallKind::Read, true)
        .unwrap();
    let first = guard.finish_call("1", UPDATE_PLAN_TOOL, ToolCallKind::Read, plan, true);
    assert!(!first.produced_new_fact);
    guard
        .before_call("2", UPDATE_PLAN_TOOL, plan, ToolCallKind::Read, true)
        .unwrap();
    guard.finish_call("2", UPDATE_PLAN_TOOL, ToolCallKind::Read, plan, true);
    let violation = guard
        .before_call("3", UPDATE_PLAN_TOOL, plan, ToolCallKind::Read, true)
        .unwrap_err();

    assert_eq!(violation.code, AgentExecutionErrorCode::NoProgressLimit);
}

#[test]
fn alternating_plan_text_does_not_mask_repeated_reads() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    let read_args = r#"{"componentTypes":["BaseText"],"intent":"create"}"#;
    for (index, status) in [(1, "in_progress"), (2, "completed"), (3, "in_progress")] {
        let plan_id = format!("plan-{index}");
        let plan = format!(r#"{{"plan":[{{"step":"create","status":"{status}"}}]}}"#);
        guard
            .before_call(&plan_id, UPDATE_PLAN_TOOL, &plan, ToolCallKind::Read, true)
            .unwrap();
        guard.finish_call(&plan_id, UPDATE_PLAN_TOOL, ToolCallKind::Read, &plan, true);

        let read_id = format!("read-{index}");
        guard
            .before_call(&read_id, RESOLVE_COMPONENT_SPECS_TOOL, read_args, ToolCallKind::Read, true)
            .unwrap();
        guard.finish_call(
            &read_id,
            RESOLVE_COMPONENT_SPECS_TOOL,
            ToolCallKind::Read,
            r#"{"corpusVersion":"docs-v1","specs":[{"componentType":"BaseText","configurable":{"style":{"color":"red"}},"example":{"config":{"style":{"color":"red"}}}}]}"#,
            true,
        );
    }

    let plan = r#"{"plan":[{"step":"create","status":"completed"}]}"#;
    guard
        .before_call("plan-4", UPDATE_PLAN_TOOL, plan, ToolCallKind::Read, true)
        .unwrap();
    guard.finish_call("plan-4", UPDATE_PLAN_TOOL, ToolCallKind::Read, plan, true);
    guard
        .before_call("read-4", RESOLVE_COMPONENT_SPECS_TOOL, read_args, ToolCallKind::Read, true)
        .unwrap();
    guard.finish_call(
        "read-4",
        RESOLVE_COMPONENT_SPECS_TOOL,
        ToolCallKind::Read,
        r#"{"corpusVersion":"docs-v1","specs":[{"componentType":"BaseText","configurable":{"style":{"color":"red"}},"example":{"config":{"style":{"color":"red"}}}}]}"#,
        true,
    );
    let violation = guard
        .before_call("read-5", RESOLVE_COMPONENT_SPECS_TOOL, read_args, ToolCallKind::Read, true)
        .unwrap_err();

    assert_eq!(violation.code, AgentExecutionErrorCode::NoProgressLimit);
    assert_eq!(violation.repeat_count, 4);
}

#[test]
fn routed_guard_rejects_unadvertised_tools_but_leaves_resource_arguments_to_tools() {
    let route = AgentToolRoute::new(crate::backend::ai_driven::agent_context::input::AgentRunInput::new("创建组件"), false);
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    let planning = route.snapshot_for_step(0);

    let violation = guard
        .before_routed_call("1", "unregistered_write_tool", "{}", ToolCallKind::WriteFrontendInteraction, true, &planning)
        .unwrap_err();
    assert_eq!(violation.code, AgentExecutionErrorCode::ToolNotAdvertisedInStep);
    assert!(guard
        .before_routed_call(
            "2",
            UPDATE_PLAN_TOOL,
            r#"{"plan":[{"step":"创建组件","status":"in_progress"}]}"#,
            ToolCallKind::Read,
            true,
            &planning
        )
        .is_ok());
    guard.finish_call("2", UPDATE_PLAN_TOOL, ToolCallKind::Read, r#"{"plan":[{"step":"创建组件","status":"in_progress"}]}"#, true);
    route.observe_operation_result(&crate::backend::ai_driven::domain::operation::CanvasOperationReceipt {
        operation_id: "op-1".to_string(),
        tool_call_id: "call-1".to_string(),
        kind: crate::backend::ai_driven::domain::operation::CanvasOperationKind::CreateComponents,
        status: crate::backend::ai_driven::domain::operation::CanvasOperationStatus::Applying,
        project_id: "1".to_string(),
        manager_id: None,
        component_ids: vec![],
        component_bindings: Default::default(),
        component_fields: Default::default(),
        configured_component_count: 0,
        default_config_component_count: 0,
        base_revision: None,
        result_revision: None,
        applied_at: None,
        error: None,
    });
    let applying = route.snapshot_for_step(1);
    let violation = guard
        .before_routed_call("3", GET_CANVAS_OPERATION_STATUS_TOOL, r#"{"operationId":"other"}"#, ToolCallKind::Read, true, &applying)
        .unwrap_err();
    assert_eq!(violation.code, AgentExecutionErrorCode::ToolNotAdvertisedInStep);
    assert!(guard
        .before_routed_call("3", GET_CANVAS_OPERATION_STATUS_TOOL, r#"{"operationId":"op-1"}"#, ToolCallKind::Read, true, &applying)
        .is_ok());
}

#[test]
fn deferred_call_metadata_does_not_increment_no_progress_state() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());

    let deferred = guard.deferred_call_metadata("patch_canvas_components", "{}");
    assert_eq!(deferred.reads_without_business_progress, 0);
    assert!(!deferred.business_write);

    assert!(guard
        .before_call(
            "read",
            RESOLVE_COMPONENT_SPECS_TOOL,
            r#"{"componentTypes":["BaseText"],"intent":"create"}"#,
            ToolCallKind::Read,
            true
        )
        .is_ok());
}

#[test]
fn component_catalog_executes_once_and_reuses_the_result_after_writes() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    let route = AgentToolRoute::new(crate::backend::ai_driven::agent_context::input::AgentRunInput::new("创建组件"), false);
    let ready = route.snapshot_for_step(0);

    let first = guard
        .before_routed_call("1", GET_COMPONENT_CATALOG_TOOL, "{}", ToolCallKind::FrontendInteraction, false, &ready)
        .unwrap();
    let first_metadata = match first {
        ProgressGuardAction::Execute(metadata) => metadata,
        ProgressGuardAction::Cached { .. } => panic!("first component catalog read should execute"),
    };
    assert_eq!(first_metadata.reads_without_business_progress, 1);

    let in_flight = guard
        .before_routed_call("2", GET_COMPONENT_CATALOG_TOOL, "{}", ToolCallKind::FrontendInteraction, false, &ready)
        .unwrap_err();
    assert_eq!(in_flight.code, AgentExecutionErrorCode::RepeatedReadLimit);

    guard.finish_call(
        "1",
        GET_COMPONENT_CATALOG_TOOL,
        ToolCallKind::FrontendInteraction,
        r#"{"catalogVersion":"component-catalog/v3:51","components":[]}"#,
        true,
    );

    let second = guard
        .before_routed_call("3", GET_COMPONENT_CATALOG_TOOL, "{}", ToolCallKind::FrontendInteraction, false, &ready)
        .unwrap();
    let second_metadata = match second {
        ProgressGuardAction::Cached { metadata, .. } => metadata,
        ProgressGuardAction::Execute(_) => panic!("component catalog should be cached after the first successful read"),
    };
    assert!(second_metadata.cache_hit);
    guard.finish_call("3", GET_COMPONENT_CATALOG_TOOL, ToolCallKind::FrontendInteraction, "{}", true);

    guard
        .before_call("4", "create_canvas_components", "{}", ToolCallKind::WriteFrontendInteraction, false)
        .unwrap();
    guard.finish_call("4", "create_canvas_components", ToolCallKind::WriteFrontendInteraction, "{}", true);

    let after_write = guard
        .before_routed_call("5", GET_COMPONENT_CATALOG_TOOL, "{}", ToolCallKind::FrontendInteraction, false, &ready)
        .unwrap();
    assert!(matches!(after_write, ProgressGuardAction::Cached { .. }));
}

#[test]
fn restored_component_catalog_reuses_the_checkpointed_result() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    guard.restore_component_catalog(r#"{"catalogVersion":"component-catalog/v3:51","components":[]}"#.to_string());

    let result = guard
        .before_call("1", GET_COMPONENT_CATALOG_TOOL, "{}", ToolCallKind::FrontendInteraction, false)
        .unwrap();
    let metadata = match result {
        ProgressGuardAction::Cached { metadata, .. } => metadata,
        ProgressGuardAction::Execute(_) => panic!("checkpointed component catalog should be cached"),
    };
    assert!(metadata.cache_hit);
    assert_eq!(metadata.actual_execution_count, 1);
}

#[test]
fn failed_component_catalog_read_can_retry() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());
    assert!(matches!(
        guard
            .before_call("1", GET_COMPONENT_CATALOG_TOOL, "{}", ToolCallKind::FrontendInteraction, false)
            .unwrap(),
        ProgressGuardAction::Execute(_)
    ));
    guard.finish_call("1", GET_COMPONENT_CATALOG_TOOL, ToolCallKind::FrontendInteraction, "frontend unavailable", false);

    assert!(matches!(
        guard
            .before_call("2", GET_COMPONENT_CATALOG_TOOL, "{}", ToolCallKind::FrontendInteraction, false)
            .unwrap(),
        ProgressGuardAction::Execute(_)
    ));
}

#[test]
fn changed_read_results_reset_no_progress_counter() {
    let guard = ProgressGuard::new(&AgentExecutionLimits::default());

    for (call_id, query) in [("1", "first"), ("2", "second")] {
        guard
            .before_call(call_id, "inspect_current_page", &format!(r#"{{"query":"{query}"}}"#), ToolCallKind::FrontendInteraction, true)
            .unwrap();
        guard.finish_call(
            call_id,
            "inspect_current_page",
            ToolCallKind::FrontendInteraction,
            &format!(r#"{{"matches":["{query}"]}}"#),
            true,
        );
    }

    assert!(guard
        .before_call("3", "inspect_current_page", r#"{"query":"third"}"#, ToolCallKind::FrontendInteraction, true)
        .is_ok());
}
