/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use serde_json::json;

use super::*;

const BASE_TEXT_DOC: &str = include_str!("../../../../resources/ai/component-optimization/BaseText.md");

#[test]
fn records_the_current_component_markdown_schema() {
    let evidence = AgentValidationEvidence::default();
    evidence
        .record_component_documents(&json!({
            "docs": [{
                "componentType": "BaseText",
                "content": BASE_TEXT_DOC,
                "dataFormatRequirements": "该组件支持通过 `dataPatch` 修改数据。",
                "supportsDataInput": true
            }],
            "missingComponentTypes": []
        }))
        .unwrap();
    let style = serde_json::from_value(json!({"fontSize": 16, "textAlign": "center"})).unwrap();

    evidence.validate_style_patch("BaseText", &style).unwrap();
    let function_style = serde_json::from_value(json!({"cursor": "() => 'pointer'"})).unwrap();
    assert!(evidence.validate_style_patch("BaseText", &function_style).is_err());
}

#[test]
fn style_patch_requires_recorded_doc_and_obeys_path_type_and_range() {
    let evidence = AgentValidationEvidence::default();
    let style = serde_json::from_value(json!({"color": "#fff"})).unwrap();
    assert_eq!(evidence.validate_style_patch("BaseText", &style).unwrap_err().code, "agent.operation.style_schema_required");
    evidence
        .record_component_documents(&json!({
            "docs": [{
                "componentType": "BaseText",
                "content": "```json\n{\"schemaVersion\":\"component-style-doc/v1\",\"componentType\":\"BaseText\",\"allowedStylePaths\":[{\"path\":\"style.color\",\"type\":\"color\"},{\"path\":\"style.fontSize\",\"type\":\"number\",\"min\":1,\"max\":200},{\"path\":\"style.columns\",\"type\":\"array\"},{\"path\":\"style.columns[].key\",\"type\":\"string\"}]}\n```",
                "dataFormatRequirements": "该组件支持通过 `dataPatch` 修改数据。",
                "supportsDataInput": true
            }],
            "missingComponentTypes": []
        }))
        .unwrap();

    evidence.validate_style_patch("BaseText", &style).unwrap();
    let invalid = serde_json::from_value(json!({"fontSize": 500, "data": []})).unwrap();
    let error = evidence.validate_style_patch("BaseText", &invalid).unwrap_err();
    assert_eq!(error.code, "agent.operation.style_patch_not_allowed");

    let invalid_array_item = serde_json::from_value(json!({"columns": [{"key": "name", "rogue": true}]})).unwrap();
    assert_eq!(
        evidence
            .validate_style_patch("BaseText", &invalid_array_item)
            .unwrap_err()
            .path
            .as_deref(),
        Some("style.columns[0].rogue")
    );
}

#[test]
fn component_data_change_requires_an_explicit_contract_query() {
    let evidence = AgentValidationEvidence::default();
    let patch = serde_json::from_value(json!({
        "sourceType": "static",
        "staticData": {"value": "新标题"}
    }))
    .unwrap();
    assert_eq!(
        evidence
            .validate_component_data_change("text-1", "BaseText", Some(&patch), None)
            .unwrap_err()
            .code,
        "agent.operation.component_data_contract_required"
    );

    evidence
        .record_component_data_contract(
            &json!({
                "schemaVersion": "component-data-contract/v1",
                "sourceContract": {
                    "sourceType": "static",
                    "branch": "staticData",
                    "structure": {"sourceType": "static", "staticData": {"value": "string"}}
                },
                "components": [{
                    "componentId": "text-1",
                    "componentType": "BaseText",
                    "supportsDataInput": true,
                    "dataFormatRequirements": "该组件支持通过 `dataPatch` 修改数据。",
                    "currentData": {"valueExample": "旧标题"},
                    "fieldMapping": {}
                }]
            }),
            "static",
        )
        .unwrap();

    evidence
        .validate_component_data_change("text-1", "BaseText", Some(&patch), None)
        .unwrap();
}

#[test]
fn component_creation_data_uses_document_capability_and_new_structure() {
    let evidence = AgentValidationEvidence::default();
    evidence
        .record_component_documents(&json!({
            "docs": [{
                "componentType": "BaseText",
                "content": BASE_TEXT_DOC,
                "dataFormatRequirements": "该组件支持通过 `dataPatch` 修改数据。",
                "supportsDataInput": true
            }],
            "missingComponentTypes": []
        }))
        .unwrap();

    evidence
        .validate_page_component_data("BaseText", &json!({"sourceType": "static", "staticData": {"value": "标题"}}))
        .unwrap();
    assert!(evidence
        .validate_page_component_data("BaseText", &json!({"sourceType": "static", "value": "旧结构"}))
        .is_err());
}

#[test]
fn page_component_must_come_from_recorded_catalog() {
    let evidence = AgentValidationEvidence::default();
    assert_eq!(
        evidence.validate_page_component_type("BaseText").unwrap_err().code,
        "agent.operation.component_catalog_required"
    );
    evidence
        .record_component_catalog(&json!({
            "catalogVersion": "component-catalog/v3:1",
            "components": [{"componentType": "BaseText"}]
        }))
        .unwrap();
    evidence.validate_page_component_type("BaseText").unwrap();
    assert_eq!(
        evidence.validate_page_component_type("Unknown").unwrap_err().code,
        "agent.operation.page_component_not_in_catalog"
    );
}

#[test]
fn later_host_query_refreshes_the_trusted_component_catalog() {
    let evidence = AgentValidationEvidence::default();
    evidence
        .record_component_catalog(&json!({
            "catalogVersion": "component-catalog/v3:host",
            "components": [{"componentType": "BaseText"}]
        }))
        .unwrap();
    evidence
        .record_component_catalog(&json!({
            "catalogVersion": "component-catalog/v3:later-query",
            "components": [{"componentType": "Unknown"}]
        }))
        .unwrap();

    assert!(evidence.validate_page_component_type("BaseText").is_err());
    evidence.validate_page_component_type("Unknown").unwrap();
}

#[test]
fn component_specs_use_the_preloaded_catalog_and_track_complete_reads() {
    let evidence = AgentValidationEvidence::default();
    let catalog = json!({
        "catalogVersion": "component-catalog/v3:1",
        "components": [{"componentType": "AntdBaseColumn", "componentName": "柱状图"}]
    });
    evidence.record_component_catalog(&catalog).unwrap();
    assert_eq!(evidence.component_catalog_for_specs().unwrap()["catalogVersion"], "component-catalog/v3:1");
    let version = crate::backend::ai_driven::component_docs::component_docs_corpus_version();
    assert!(!evidence.has_complete_component_spec("AntdBaseColumn", "create", version));
    evidence.record_complete_component_spec("AntdBaseColumn", "create", version);
    assert!(evidence.has_complete_component_spec("AntdBaseColumn", "create", version));
    assert!(!evidence.has_complete_component_spec("AntdBaseColumn", "patch", version));
}

#[test]
fn component_specs_reject_missing_host_catalog() {
    let evidence = AgentValidationEvidence::default();
    assert_eq!(evidence.component_catalog_for_specs().unwrap_err().code, "agent.evidence.component_catalog_unavailable");
}

#[test]
fn restored_catalog_view_does_not_replace_current_trusted_catalog() {
    let evidence = AgentValidationEvidence::default();
    evidence
        .record_component_catalog(&json!({
            "catalogVersion": "component-catalog/v3:1",
            "components": [{"componentType": "AntdBaseArea"}]
        }))
        .unwrap();
    AgentValidationEvidence::validate_restored_component_catalog(&json!({
        "catalogVersion": "component-catalog/v3:1",
        "components": [{"componentType": "BaseText"}]
    }))
    .unwrap();

    assert_eq!(evidence.component_catalog_for_specs().unwrap()["components"][0]["componentType"], "AntdBaseArea");
    assert!(evidence.validate_page_component_type("AntdBaseArea").is_ok());
    assert!(evidence.validate_page_component_type("BaseText").is_err());
}

#[test]
fn create_data_components_require_a_current_complete_create_spec() {
    let evidence = AgentValidationEvidence::default();
    evidence
        .record_component_documents(&json!({
            "docs": [{
                "componentType": "BaseText",
                "content": BASE_TEXT_DOC,
                "dataFormatRequirements": "该组件支持通过 `dataPatch` 修改数据。",
                "supportsDataInput": true
            }],
            "missingComponentTypes": []
        }))
        .unwrap();
    assert_eq!(
        evidence.validate_component_spec_for_create("BaseText").unwrap_err().code,
        "agent.operation.component_spec_required"
    );

    evidence.record_complete_component_spec("BaseText", "create", "docs-v1");
    assert_eq!(
        evidence.validate_component_spec_for_create("BaseText").unwrap_err().code,
        "agent.operation.component_spec_incomplete"
    );

    let current_version = crate::backend::ai_driven::component_docs::component_docs_corpus_version();
    evidence.record_complete_component_spec("BaseText", "create", current_version);
    evidence.validate_component_spec_for_create("BaseText").unwrap();
}

#[test]
fn complete_spec_reads_are_idempotent_and_isolated_by_intent_and_version() {
    let evidence = AgentValidationEvidence::default();
    evidence.record_complete_component_spec("BaseText", "create", "docs-v1");
    evidence.record_complete_component_spec("BaseText", "create", "docs-v1");
    assert!(evidence.has_complete_component_spec("BaseText", "create", "docs-v1"));
    assert!(!evidence.has_complete_component_spec("BaseText", "patch", "docs-v1"));
    assert!(!evidence.has_complete_component_spec("BaseText", "create", "docs-v2"));
    assert_eq!(evidence.component_spec_corpus_version("BaseText", "create"), None);

    let current_version = crate::backend::ai_driven::component_docs::component_docs_corpus_version();
    evidence.record_complete_component_spec("BaseText", "create", current_version);
    assert_eq!(evidence.component_spec_corpus_version("BaseText", "create").as_deref(), Some(current_version));
}
