/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use super::*;
use crate::backend::ai_driven::component_docs::full_spec::full_component_spec_types;

const TEN_TYPES: [&str; 10] = [
    "AntdBaseArea",
    "AntdBaseColumn",
    "AntdBaseLine",
    "BaseColorBlock",
    "BaseImage",
    "BaseText",
    "DigitalFlipper",
    "FourAngleGlowBorder",
    "DateTime",
    "TextScroller",
];
const COMMUNITY_SCREEN_TYPES: [&str; 9] = [
    "BaseText",
    "BaseImage",
    "BaseButton",
    "BaseColorBlock",
    "AntdRingProgress",
    "AntdMultiLine",
    "AntdBaseLine",
    "AntdBaseColumn",
    "AntdPercentColumn",
];
const AREA_FRAGMENTS: [&str; 13] = [
    "<g2plot-canvas>",
    "<g2plot-area-plot>",
    "<g2plot-smooth>",
    "<g2plot-pattern>",
    "<g2plot-area-line>",
    "<g2plot-area-style>",
    "<g2plot-area-point>",
    "<g2plot-label-area>",
    "<g2plot-axis>",
    "<g2plot-legend>",
    "<g2plot-tooltip>",
    "<g2plot-annotations>",
    "<g2plot-slider>",
];
const LINE_FRAGMENTS: [&str; 12] = [
    "<g2plot-canvas>",
    "<g2plot-line-plot>",
    "<g2plot-smooth>",
    "<g2plot-line-style>",
    "<g2plot-line-point>",
    "<g2plot-label-line>",
    "<g2plot-axis-extended>",
    "<g2plot-legend>",
    "<g2plot-tooltip>",
    "<g2plot-annotations>",
    "<g2plot-slider>",
    "<g2plot-scrollbar>",
];
const GROUP_COLUMN_FRAGMENTS: [&str; 11] = [
    "<g2plot-canvas>",
    "<g2plot-column-layout>",
    "<g2plot-column-style>",
    "<g2plot-column-background>",
    "<g2plot-label-column>",
    "<g2plot-axis>",
    "<g2plot-legend>",
    "<g2plot-tooltip>",
    "<g2plot-annotations>",
    "<g2plot-slider>",
    "<g2plot-scrollbar>",
];

fn evidence() -> AgentValidationEvidence {
    let evidence = AgentValidationEvidence::default();
    let components = full_component_spec_types()
        .into_iter()
        .map(|component_type| json!({ "componentType": component_type, "componentName": component_type }))
        .collect::<Vec<_>>();
    evidence
        .record_component_catalog(&json!({
            "catalogVersion": format!("component-catalog/v3:{}", components.len()),
            "components": components,
        }))
        .unwrap();
    evidence
}

fn args(types: &[&str], intent: &str) -> ResolveComponentSpecsArgs {
    serde_json::from_value(json!({ "componentTypes": types, "intent": intent })).unwrap()
}

fn fragment_keys(result: &Value) -> std::collections::BTreeSet<String> {
    result["commonFragments"]
        .as_object()
        .expect("commonFragments must be an object")
        .keys()
        .cloned()
        .collect()
}

fn key_set(keys: &[&str]) -> std::collections::BTreeSet<String> {
    keys.iter().map(|key| (*key).to_string()).collect()
}

fn assert_fragments_are_serialized_in_sorted_order(result: &Value) {
    let fragments = &result["commonFragments"];
    let serialized = serde_json::to_string(fragments).unwrap();
    let mut markers = fragments
        .as_object()
        .unwrap()
        .keys()
        .map(|key| format!("\"{key}\":"))
        .collect::<Vec<_>>();
    markers.sort();
    let positions = markers
        .iter()
        .map(|marker| serialized.find(marker).unwrap_or_else(|| panic!("missing serialized key {marker}")))
        .collect::<Vec<_>>();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn parameters_publish_only_full_spec_contract() {
    let parameters = parameters_schema();
    assert_eq!(parameters["required"], json!(["componentTypes", "intent"]));
    let properties = parameters["properties"].as_object().unwrap();
    assert_eq!(properties.len(), 2);
    assert!(properties.contains_key("componentTypes"));
    assert!(properties.contains_key("intent"));
}

#[test]
fn deserialization_rejects_legacy_projection_arguments_and_query() {
    assert!(serde_json::from_value::<ResolveComponentSpecsArgs>(json!({
        "componentTypes": ["BaseText"],
        "intent": "create",
        "fields": ["config"],
        "page": { "field": "color" },
        "corpusVersion": "old"
    }))
    .is_err());
    assert!(serde_json::from_value::<ResolveComponentSpecsArgs>(json!({
        "componentTypes": ["BaseText"], "intent": "create", "query": "text"
    }))
    .is_err());
}

#[test]
fn duplicate_types_are_trimmed_and_deduplicated_before_limit_check() {
    let mut types = vec![" BaseText ".to_string(), "BaseText".to_string(), "".to_string()];
    normalize_component_types(&mut types).unwrap();
    assert_eq!(types, ["BaseText"]);

    let mut too_many = (0..=MAX_COMPONENT_TYPES)
        .map(|index| format!("Type{index}"))
        .collect::<Vec<_>>();
    let error = normalize_component_types(&mut too_many).unwrap_err();
    assert_eq!(error.code, "agent.component_specs.component_types_limit_exceeded");
    assert_eq!(serde_json::from_str::<Value>(&error.message).unwrap()["actualItems"], MAX_COMPONENT_TYPES + 1);
}

#[test]
fn selectors_require_exact_trusted_catalog_types() {
    let catalog = vec![ComponentCatalogItem { component_type: "BaseText".to_string() }];
    assert_eq!(select_components(&catalog, &["BaseText".to_string()]).unwrap().len(), 1);
    assert_eq!(
        select_components(&catalog, &["basetext".to_string()]).unwrap_err().code,
        "agent.component_specs.component_unavailable"
    );
}

#[test]
fn output_contains_full_specs_and_only_referenced_common_fragments() {
    let evidence = evidence();
    let result = resolve_specs(&evidence, args(&["AntdBaseArea"], "create")).unwrap();
    assert_eq!(result.to_string().matches("corpusVersion").count(), 1);
    assert_eq!(
        result.as_object().unwrap().keys().cloned().collect::<std::collections::BTreeSet<_>>(),
        key_set(&["corpusVersion", "specs", "commonFragments"])
    );
    let spec = &result["specs"][0];
    assert_eq!(
        spec.as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        ["componentType", "configurable", "constraints", "dataRequirements", "example"]
            .iter()
            .map(|key| key.to_string())
            .collect::<std::collections::BTreeSet<_>>()
    );
    for removed in ["actionGuide", "returnedFields", "newFields", "requiredFields", "omittedFields", "cacheHit", "supplementRequests"] {
        assert!(spec.get(removed).is_none());
    }
    assert_eq!(spec["configurable"]["style"]["xAxis"], "<g2plot-axis>");
    assert_eq!(spec["configurable"]["style"]["yAxis"], "<g2plot-axis>");
    assert!(result["commonFragments"]["<g2plot-axis>"]["label"]["style"]["fontSize"].is_string());
    assert_eq!(fragment_keys(&result), key_set(&AREA_FRAGMENTS));
    assert_eq!(result["commonFragments"].as_object().unwrap().len(), AREA_FRAGMENTS.len());
    assert!(spec["configurable"]["style"]["xField"].is_string());
    assert!(spec["configurable"]["style"]["yField"].is_string());
    assert!(evidence.has_complete_component_spec("AntdBaseArea", "create", component_docs_corpus_version()));

    let repeated = resolve_specs(&evidence, args(&["AntdBaseArea"], "create")).unwrap();
    assert_eq!(result, repeated);
}

#[test]
fn ordinary_components_have_no_fragments_and_mixed_requests_add_only_chart_fragments() {
    let evidence = evidence();
    let ordinary = resolve_specs(&evidence, args(&["BaseText"], "create")).unwrap();
    assert_eq!(ordinary["commonFragments"], json!({}));

    let chart = resolve_specs(&evidence, args(&["AntdBaseLine"], "create")).unwrap();
    let mixed = resolve_specs(&evidence, args(&["BaseText", "AntdBaseLine"], "create")).unwrap();
    assert_eq!(fragment_keys(&chart), key_set(&LINE_FRAGMENTS));
    assert_eq!(mixed["commonFragments"], chart["commonFragments"]);
    assert_eq!(mixed["specs"].as_array().unwrap().len(), 2);
}

#[test]
fn multi_chart_fragments_are_unioned_deduplicated_and_sorted_independently_of_request_order() {
    let evidence = evidence();
    let result = resolve_specs(&evidence, args(&["AntdBaseLine", "AntdGroupColumn", "BaseText"], "create")).unwrap();
    let reversed = resolve_specs(&evidence, args(&["BaseText", "AntdGroupColumn", "AntdBaseLine"], "create")).unwrap();
    let expected = LINE_FRAGMENTS
        .into_iter()
        .chain(GROUP_COLUMN_FRAGMENTS)
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(fragment_keys(&result), key_set(&expected.iter().copied().collect::<Vec<_>>()));
    assert_eq!(result["commonFragments"], reversed["commonFragments"]);
    assert_eq!(result["commonFragments"].as_object().unwrap().len(), expected.len());
    assert_fragments_are_serialized_in_sorted_order(&result);
}

#[test]
fn group_column_keeps_legend_exclusions_and_validator_rejects_them() {
    let evidence = evidence();
    let result = resolve_specs(&evidence, args(&["AntdGroupColumn"], "create")).unwrap();
    let legend_fragment = &result["commonFragments"]["<g2plot-legend>"];
    assert!(legend_fragment.get("title").is_some());
    assert!(legend_fragment.get("itemValue").is_some());
    assert!(legend_fragment.get("background").is_some());

    let constraints = result["specs"][0]["constraints"].as_array().unwrap();
    assert!(constraints.iter().any(|constraint| {
        let constraint = constraint.as_str().unwrap_or_default();
        constraint.contains("不包含")
            && constraint.contains("title")
            && constraint.contains("itemValue")
            && constraint.contains("background")
    }));

    for invalid_style in [
        json!({ "legend": { "title": "unsupported" } }),
        json!({ "legend": { "itemValue": { "alignRight": true } } }),
        json!({ "legend": { "background": { "padding": 8 } } }),
    ] {
        assert!(evidence.validate_style_patch("AntdGroupColumn", invalid_style.as_object().unwrap()).is_err());
    }
}

#[test]
fn resolving_specs_requires_a_trusted_host_catalog() {
    let evidence = AgentValidationEvidence::default();
    let error = resolve_specs(&evidence, args(&["BaseText"], "create")).unwrap_err();
    assert_eq!(error.code, "agent.evidence.component_catalog_unavailable");
}

#[test]
fn resolving_specs_uses_preloaded_catalog_without_agent_catalog_read() {
    let evidence = evidence();
    let result = resolve_specs(&evidence, args(&["AntdBaseArea"], "patch")).unwrap();
    assert_eq!(result["specs"][0]["componentType"], "AntdBaseArea");
    assert!(result["specs"][0]["configurable"].is_object());

    let error = resolve_specs(&evidence, args(&["UnknownComponent"], "patch")).unwrap_err();
    assert_eq!(error.code, "agent.component_specs.component_unavailable");
}

#[test]
fn critical_components_return_all_documented_fields() {
    let evidence = evidence();
    let result = resolve_specs(&evidence, args(&["DigitalFlipper", "FourAngleGlowBorder"], "create")).unwrap();
    assert_eq!(result["specs"][0]["configurable"]["style"].as_object().unwrap().len(), 16);
    assert_eq!(result["specs"][1]["configurable"]["style"].as_object().unwrap().len(), 4);
}

#[test]
fn ten_distinct_components_fit_without_truncation() {
    let evidence = evidence();
    let result = resolve_specs(&evidence, args(&TEN_TYPES, "create")).unwrap();
    assert_eq!(result["specs"].as_array().unwrap().len(), MAX_COMPONENT_TYPES);
    assert!(serialized_len(&result).unwrap() <= COMPONENT_SPEC_OUTPUT_BUDGET_BYTES);
    assert!(!result.to_string().contains("truncated"));
}

#[test]
fn community_screen_mixed_spec_requests_return_complete_results() {
    let evidence = evidence();
    let nine = resolve_specs(&evidence, args(&COMMUNITY_SCREEN_TYPES, "create")).unwrap();
    assert_eq!(nine["specs"].as_array().unwrap().len(), COMMUNITY_SCREEN_TYPES.len());
    assert!(serialized_len(&nine).unwrap() <= COMPONENT_SPEC_OUTPUT_BUDGET_BYTES);
    assert!(!nine.to_string().contains("truncated"));

    let five = resolve_specs(&evidence, args(&COMMUNITY_SCREEN_TYPES[..5], "create")).unwrap();
    assert_eq!(five["specs"].as_array().unwrap().len(), 5);
    assert!(!five.to_string().contains("truncated"));
}

#[test]
fn largest_ten_serialized_specs_fit_the_dedicated_budget() {
    let mut specs = full_component_spec_types()
        .into_iter()
        .map(|component_type| {
            let spec = full_component_spec(component_type, "create").unwrap();
            let value = serde_json::to_value(spec).unwrap();
            (serialized_len(&value).unwrap(), component_type)
        })
        .collect::<Vec<_>>();
    specs.sort_unstable_by(|left, right| right.0.cmp(&left.0));
    let largest = specs
        .into_iter()
        .take(MAX_COMPONENT_TYPES)
        .map(|(_, component_type)| component_type)
        .collect::<Vec<_>>();
    let output = resolve_specs(&evidence(), args(&largest, "create")).unwrap();
    assert_eq!(output["specs"].as_array().unwrap().len(), MAX_COMPONENT_TYPES);
    assert!(!output["commonFragments"].as_object().unwrap().is_empty());
    assert!(serialized_len(&output).unwrap() <= COMPONENT_SPEC_OUTPUT_BUDGET_BYTES);
}

#[test]
fn output_limit_failure_does_not_record_complete_read() {
    let evidence = evidence();
    let oversized = json!({
        "corpusVersion": component_docs_corpus_version(),
        "specs": ["complete spec"],
        "commonFragments": { "<g2plot-canvas>": "x".repeat(COMPONENT_SPEC_OUTPUT_BUDGET_BYTES) },
    });
    let error = finalize_output(&evidence, oversized, "create", ["BaseText"]).unwrap_err();
    assert_eq!(error.code, "agent.component_specs.output_limit_exceeded");
    assert!(!evidence.has_complete_component_spec("BaseText", "create", component_docs_corpus_version()));
}

#[test]
fn patch_example_uses_patch_shape_and_create_data_passes_validation() {
    let evidence = evidence();
    let created = resolve_specs(&evidence, args(&["DigitalFlipper"], "create")).unwrap();
    evidence
        .validate_page_component_data("DigitalFlipper", &created["specs"][0]["example"]["config"]["data"])
        .unwrap();
    let patched = resolve_specs(&evidence, args(&["AntdBaseArea"], "patch")).unwrap();
    assert!(patched["specs"][0]["example"].get("config").is_none());
    assert!(patched["specs"][0]["example"]["stylePatch"].is_object());
    assert!(patched["specs"][0]["example"]["dataPatch"].is_object());
}
