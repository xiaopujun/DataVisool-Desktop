/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 将常见的工具参数形状错误转换为简短、可执行的模型反馈。

use serde_json::{json, Value};

use crate::backend::ai_driven::tools::canvas::CREATE_CANVAS_COMPONENTS_TOOL;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ArgumentShapeIssue {
    pub(super) field_path: &'static str,
    pub(super) expected: &'static str,
    pub(super) actual: &'static str,
}

impl ArgumentShapeIssue {
    pub(super) fn model_result(self) -> String {
        json!({
            "code": "agent.tool.invalid_argument_type",
            "message": "components 必须直接传 JSON 数组，不能传序列化后的字符串",
            "fieldPath": self.field_path,
            "expected": self.expected,
            "actual": self.actual,
            "retryable": true,
            "argumentsCorrectable": true,
            "recovery": "regenerate_arguments_with_native_array",
            "incorrectExample": { "components": "[{\"id\":\"component_id\"}]" },
            "correctExample": {
                "components": [{
                    "id": "component_id",
                    "type": "BaseText",
                    "x": 0,
                    "y": 0,
                    "width": 100,
                    "height": 30,
                    "useDefaultConfig": true
                }]
            }
        })
        .to_string()
    }
}

/// 仅识别问题，不解析、替换或修复原始参数。
pub(super) fn detect_argument_shape_issue(tool_name: &str, args: &str) -> Option<ArgumentShapeIssue> {
    if tool_name != CREATE_CANVAS_COMPONENTS_TOOL {
        return None;
    }
    let value: Value = serde_json::from_str(args).ok()?;
    value
        .get("components")
        .is_some_and(Value::is_string)
        .then_some(ArgumentShapeIssue { field_path: "components", expected: "array", actual: "string" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_stringified_components_without_unpacking_them() {
        let args = r#"{"components":"[{\"id\":\"component-1\"}]"}"#;
        let issue = detect_argument_shape_issue(CREATE_CANVAS_COMPONENTS_TOOL, args).unwrap();

        assert_eq!(issue.field_path, "components");
        assert_eq!(issue.expected, "array");
        assert_eq!(issue.actual, "string");
        assert_eq!(serde_json::from_str::<Value>(args).unwrap()["components"].as_str(), Some(r#"[{"id":"component-1"}]"#));
    }

    #[test]
    fn accepts_native_component_arrays() {
        let args = r#"{"components":[{"id":"component-1"}]}"#;
        assert_eq!(detect_argument_shape_issue(CREATE_CANVAS_COMPONENTS_TOOL, args), None);
    }

    #[test]
    fn compact_feedback_does_not_echo_the_large_invalid_payload() {
        let invalid = "x".repeat(9_000);
        let args = json!({ "components": invalid }).to_string();
        let feedback = detect_argument_shape_issue(CREATE_CANVAS_COMPONENTS_TOOL, &args)
            .unwrap()
            .model_result();
        let value: Value = serde_json::from_str(&feedback).unwrap();

        assert_eq!(value["fieldPath"], "components");
        assert_eq!(value["expected"], "array");
        assert_eq!(value["actual"], "string");
        assert!(feedback.len() < 1_000);
        assert!(!feedback.contains(&"x".repeat(100)));
    }
}
