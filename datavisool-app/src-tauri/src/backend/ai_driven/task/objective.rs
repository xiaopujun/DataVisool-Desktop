/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 根据结构化入口生成不可扩张的任务事实。

use serde::Serialize;

use crate::backend::ai_driven::agent_context::input::AgentRunInput;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObjectiveSpec {
    pub requirement: String,
    pub target_component_ids: Vec<String>,
}

impl ObjectiveSpec {
    pub(crate) fn build(input: &AgentRunInput) -> Self {
        Self { requirement: input.original_requirement_text(), target_component_ids: normalized_targets(input) }
    }
}

fn normalized_targets(input: &AgentRunInput) -> Vec<String> {
    let mut targets = input
        .selection
        .as_ref()
        .map(|selection| selection.target_ids.clone())
        .unwrap_or_default();
    targets.extend(input.selected_components().iter().map(|component| component.id.clone()));
    targets = targets
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    targets.sort_unstable();
    targets.dedup();
    targets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::agent_context::input::AgentSelectionContext;

    #[test]
    fn entry_targets_are_normalized_without_classifying_intent() {
        let mut input = AgentRunInput::new("把选中的组件主题颜色改为夕阳红");
        input.selection =
            Some(AgentSelectionContext { target_ids: vec![" c2 ".to_string(), "c1".to_string(), "c1".to_string()], ..Default::default() });

        let objective = ObjectiveSpec::build(&input);

        assert_eq!(objective.requirement, "把选中的组件主题颜色改为夕阳红");
        assert_eq!(objective.target_component_ids, vec!["c1", "c2"]);
    }
}
