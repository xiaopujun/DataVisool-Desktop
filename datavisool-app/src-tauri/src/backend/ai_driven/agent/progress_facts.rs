/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Read progress is a union of facts, never a hash of a newly grouped response.
use super::*;

#[derive(Debug, Default)]
pub(super) struct ProgressFacts {
    seen: HashSet<String>,
}

impl ProgressFacts {
    pub(super) fn record(&mut self, tool: &str, result: &str) -> bool {
        self.record_with_args(tool, None, result)
    }

    pub(super) fn record_with_args(&mut self, tool: &str, args: Option<&str>, result: &str) -> bool {
        let Ok(mut value) = serde_json::from_str::<Value>(result) else {
            return false;
        };
        strip_volatile_fact_fields(&mut value);
        match tool {
            RESOLVE_COMPONENT_SPECS_TOOL => self.specs(args, &value),
            INSPECT_CURRENT_PAGE_TOOL => self.canvas(&value),
            _ => semantic_fact_signature(tool, result).is_some_and(|key| self.seen.insert(key)),
        }
    }

    fn insert(&mut self, value: Value) -> bool {
        self.seen.insert(canonical_json(&value).to_string())
    }

    fn specs(&mut self, args: Option<&str>, value: &Value) -> bool {
        let Some(specs) = value.get("specs").and_then(Value::as_array) else {
            return false;
        };
        let intent = args
            .and_then(|args| serde_json::from_str::<Value>(args).ok())
            .and_then(|args| args.get("intent").and_then(Value::as_str).map(str::trim).map(str::to_string))
            .or_else(|| value.get("intent").and_then(Value::as_str).map(str::trim).map(str::to_string));
        let Some(intent) = intent.filter(|intent| !intent.is_empty()) else {
            return false;
        };
        let Some(version) = value
            .get("corpusVersion")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|version| !version.is_empty())
        else {
            return false;
        };
        let mut changed = false;
        for spec in specs {
            let Some(component) = complete_component_type(spec) else {
                continue;
            };
            changed |= self.insert(json!(["spec", component, intent, version]));
        }
        changed
    }

    fn canvas(&mut self, value: &Value) -> bool {
        let version = value.get("revision").or_else(|| value.get("workspaceRevision"));
        let mut changed = false;
        for component in value.get("components").and_then(Value::as_array).into_iter().flatten() {
            if component.get("id").and_then(Value::as_str).is_some_and(|id| !id.is_empty()) {
                changed |= self.insert(json!(["canvas", version, component["id"]]));
            }
        }
        for missing in text_values(value.get("missingComponentIds")) {
            changed |= self.insert(json!(["missing_component", version, missing]));
        }
        changed
    }
}

pub(super) fn complete_component_type(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    let component = object
        .get("componentType")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|component| !component.is_empty())?;
    let configurable = object
        .get("configurable")
        .and_then(Value::as_object)
        .filter(|value| !value.is_empty())?;
    let example = object
        .get("example")
        .and_then(Value::as_object)
        .filter(|value| !value.is_empty())?;
    if object.get("dataRequirements").is_some_and(|value| !value.is_string())
        || object.get("constraints").is_some_and(|value| !value.is_array())
        || configurable.is_empty()
        || example.is_empty()
    {
        return None;
    }
    Some(component)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_spec_read_is_progress_once_per_component_intent_and_version() {
        let mut facts = ProgressFacts::default();
        let args = r#"{"componentTypes":["Text"],"intent":"create"}"#;
        let result = |style: Value| {
            json!({"corpusVersion":"v1", "specs":[
                {"componentType":"Text", "configurable":{"style":style}, "example":{"config":{"style":{}}}}
            ]})
            .to_string()
        };
        assert!(facts.record_with_args(RESOLVE_COMPONENT_SPECS_TOOL, Some(args), &result(json!({"color":{}}))));
        assert!(!facts.record_with_args(RESOLVE_COMPONENT_SPECS_TOOL, Some(args), &result(json!({"font":{}}))));
        assert!(!facts.record_with_args(RESOLVE_COMPONENT_SPECS_TOOL, Some(args), &result(json!({"font":{"size":12}}))));
        assert!(facts.record_with_args(
            RESOLVE_COMPONENT_SPECS_TOOL,
            Some(r#"{"componentTypes":["Text"],"intent":"patch"}"#),
            &result(json!({"font":{}})),
        ));
        assert!(facts.record_with_args(
            RESOLVE_COMPONENT_SPECS_TOOL,
            Some(r#"{"componentTypes":["Text"],"intent":"create"}"#),
            &result(json!({"font":{},"version":2})).replace("\"v1\"", "\"v2\""),
        ));
    }

    #[test]
    fn regrouping_canvas_ids_and_empty_new_cursor_are_not_progress() {
        let mut facts = ProgressFacts::default();
        for id in ["one", "two"] {
            assert!(facts.record(INSPECT_CURRENT_PAGE_TOOL, &json!({"revision":1,"components":[{"id":id}]}).to_string()));
        }
        assert!(!facts.record(INSPECT_CURRENT_PAGE_TOOL, r#"{"revision":1,"offset":42,"components":[{"id":"two"},{"id":"one"}]}"#));
        assert!(!facts.record(INSPECT_CURRENT_PAGE_TOOL, r#"{"revision":2,"offset":99,"components":[]}"#));
    }

    #[test]
    fn incomplete_spec_does_not_count_as_progress() {
        let mut facts = ProgressFacts::default();
        assert!(!facts.record_with_args(
            RESOLVE_COMPONENT_SPECS_TOOL,
            Some(r#"{"componentTypes":["Text"],"intent":"create"}"#),
            r#"{"corpusVersion":"v1","specs":[{"componentType":"Text","configurable":{},"example":{}}]}"#,
        ));
    }
}
