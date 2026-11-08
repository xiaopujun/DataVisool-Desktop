/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Rig Agent 的统一构建计划与任务级 Tool 集合。

use rig_core::tool::ToolDyn;

use crate::backend::ai_driven::memory::ConversationMemoryAdapter;
use crate::backend::ai_driven::provider::BuiltRigModel;
use crate::backend::ai_driven::tools::canvas::{
    CreateCanvasComponentsTool, DeleteCanvasComponentsTool, GetCanvasOperationStatusTool, GetCurrentCanvasSelectionTool, PatchCanvasComponentsTool,
    CREATE_CANVAS_COMPONENTS_TOOL, DELETE_CANVAS_COMPONENTS_TOOL, GET_CANVAS_OPERATION_STATUS_TOOL, GET_CURRENT_CANVAS_SELECTION_TOOL,
    PATCH_CANVAS_COMPONENTS_TOOL,
};
use crate::backend::ai_driven::tools::component_catalog::{GetComponentCatalogTool, GET_COMPONENT_CATALOG_TOOL};
use crate::backend::ai_driven::tools::component_data_contract::{GetComponentDataContractTool, GET_COMPONENT_DATA_CONTRACT_TOOL};
use crate::backend::ai_driven::tools::component_resolution::{ResolveComponentSpecsTool, RESOLVE_COMPONENT_SPECS_TOOL};
use crate::backend::ai_driven::tools::file::{
    ApplyWorkspacePatchTool, DeleteWorkspacePathTool, ListWorkspaceFilesTool, MoveWorkspacePathTool, ReadWorkspaceFileTool, SearchWorkspaceFilesTool,
    APPLY_WORKSPACE_PATCH_TOOL, DELETE_WORKSPACE_PATH_TOOL, LIST_WORKSPACE_FILES_TOOL, MOVE_WORKSPACE_PATH_TOOL, READ_WORKSPACE_FILE_TOOL,
    SEARCH_WORKSPACE_FILES_TOOL,
};
use crate::backend::ai_driven::tools::interaction::{
    InspectCurrentPageTool, RequestUserInputTool, INSPECT_CURRENT_PAGE_TOOL, REQUEST_USER_INPUT_TOOL,
};
use crate::backend::ai_driven::tools::runtime::AgentToolRuntime;
use crate::backend::ai_driven::tools::selection::{RequestComponentSelectionTool, REQUEST_COMPONENT_SELECTION_TOOL};
use crate::backend::ai_driven::tools::task_intent::{UpdatePlanTool, UPDATE_PLAN_TOOL};

use super::{AgentContextLimits, AgentToolRoute};

/// 工厂输入。模型配置与 API Key 已在 Provider 工厂边界内消费。
pub(crate) struct AgentBuildRequest {
    pub model: BuiltRigModel,
    pub memory: ConversationMemoryAdapter,
    pub tool_runtime: AgentToolRuntime,
    pub tool_route: AgentToolRoute,
    pub enable_workspace_file_tools: bool,
    pub max_model_calls: usize,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub context_window_tokens: Option<u64>,
    pub context_limits: AgentContextLimits,
}

/// 可直接交给 Runner 的完整、但尚未进入模型循环的 Agent 计划。
pub(crate) struct BuiltAgent {
    pub(crate) model: BuiltRigModel,
    pub(crate) memory: ConversationMemoryAdapter,
    pub(crate) tools: Vec<Box<dyn ToolDyn>>,
    pub(crate) preamble: String,
    pub(crate) max_model_calls: usize,
    pub(crate) temperature: Option<f64>,
    pub(crate) max_tokens: Option<u64>,
    pub(crate) context_window_tokens: Option<u64>,
    pub(crate) context_limits: AgentContextLimits,
    pub(crate) tool_route: AgentToolRoute,
}

/// 唯一 Agent 工厂；业务模块不能自行拼装 Provider、Memory 或 Tool。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RigAgentFactory;

impl RigAgentFactory {
    pub(crate) fn build(&self, request: AgentBuildRequest) -> BuiltAgent {
        let tools = build_tools(request.enable_workspace_file_tools, request.tool_runtime);
        let preamble = request.tool_route.current_plan().system_prompt();
        BuiltAgent {
            model: request.model,
            memory: request.memory,
            tools,
            preamble,
            max_model_calls: request.max_model_calls,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            context_window_tokens: request.context_window_tokens,
            context_limits: request.context_limits,
            tool_route: request.tool_route,
        }
    }

    /// 服务端权限策略与实际 Tool 注册共用此函数，防止只靠 Prompt 授权。
    pub(crate) fn allowed_tool_names(&self, enable_workspace_file_tools: bool) -> Vec<&'static str> {
        let mut names = vec![
            REQUEST_USER_INPUT_TOOL,
            UPDATE_PLAN_TOOL,
            INSPECT_CURRENT_PAGE_TOOL,
            GET_COMPONENT_CATALOG_TOOL,
            RESOLVE_COMPONENT_SPECS_TOOL,
            GET_COMPONENT_DATA_CONTRACT_TOOL,
            CREATE_CANVAS_COMPONENTS_TOOL,
            PATCH_CANVAS_COMPONENTS_TOOL,
            DELETE_CANVAS_COMPONENTS_TOOL,
            GET_CANVAS_OPERATION_STATUS_TOOL,
            GET_CURRENT_CANVAS_SELECTION_TOOL,
            REQUEST_COMPONENT_SELECTION_TOOL,
        ];
        if enable_workspace_file_tools {
            names.extend([
                LIST_WORKSPACE_FILES_TOOL,
                SEARCH_WORKSPACE_FILES_TOOL,
                READ_WORKSPACE_FILE_TOOL,
                APPLY_WORKSPACE_PATCH_TOOL,
                MOVE_WORKSPACE_PATH_TOOL,
                DELETE_WORKSPACE_PATH_TOOL,
            ]);
        }
        names
    }
}

fn build_tools(enable_workspace_file_tools: bool, runtime: AgentToolRuntime) -> Vec<Box<dyn ToolDyn>> {
    let mut tools: Vec<Box<dyn ToolDyn>> = vec![
        Box::new(RequestUserInputTool(runtime.clone())),
        Box::new(UpdatePlanTool(runtime.clone())),
        Box::new(InspectCurrentPageTool(runtime.clone())),
        Box::new(GetComponentCatalogTool(runtime.clone())),
        Box::new(ResolveComponentSpecsTool(runtime.clone())),
        Box::new(GetComponentDataContractTool(runtime.clone())),
        Box::new(CreateCanvasComponentsTool(runtime.clone())),
        Box::new(PatchCanvasComponentsTool(runtime.clone())),
        Box::new(DeleteCanvasComponentsTool(runtime.clone())),
        Box::new(GetCanvasOperationStatusTool(runtime.clone())),
        Box::new(GetCurrentCanvasSelectionTool(runtime.clone())),
        Box::new(RequestComponentSelectionTool(runtime.clone())),
    ];
    if enable_workspace_file_tools {
        tools.extend([
            Box::new(ListWorkspaceFilesTool(runtime.clone())) as Box<dyn ToolDyn>,
            Box::new(SearchWorkspaceFilesTool(runtime.clone())),
            Box::new(ReadWorkspaceFileTool(runtime.clone())),
            Box::new(ApplyWorkspacePatchTool(runtime.clone())),
            Box::new(MoveWorkspacePathTool(runtime.clone())),
            Box::new(DeleteWorkspacePathTool(runtime.clone())),
        ]);
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_agent_exposes_capability_tools_without_shell() {
        let factory = RigAgentFactory;
        let tools = factory.allowed_tool_names(true);
        assert!(tools.contains(&CREATE_CANVAS_COMPONENTS_TOOL));
        assert!(tools.contains(&PATCH_CANVAS_COMPONENTS_TOOL));
        assert!(tools.contains(&DELETE_CANVAS_COMPONENTS_TOOL));
        assert!(tools.contains(&GET_CANVAS_OPERATION_STATUS_TOOL));
        assert!(tools.contains(&GET_CURRENT_CANVAS_SELECTION_TOOL));
        assert!(tools.contains(&UPDATE_PLAN_TOOL));
        assert!(tools.contains(&GET_COMPONENT_CATALOG_TOOL));
        assert!(!tools.contains(&"get_component_document"));
        assert!(!tools.contains(&"search_component_documents"));
        assert!(!tools.contains(&"search_available_components"));
        assert!(tools.contains(&LIST_WORKSPACE_FILES_TOOL));
        assert!(tools.contains(&READ_WORKSPACE_FILE_TOOL));
        assert!(tools.contains(&APPLY_WORKSPACE_PATCH_TOOL));
        assert!(!tools.contains(&"submit_task_completion"));
        assert!(!tools.iter().any(|name| name.contains("shell")));
        assert!(!factory.allowed_tool_names(false).contains(&APPLY_WORKSPACE_PATCH_TOOL));
        assert!(!tools.contains(&"submit_result"));
    }

    #[test]
    fn prompt_requires_direct_canvas_actions() {
        let mut input = crate::backend::ai_driven::agent_context::input::AgentRunInput::new("创建页面组件");
        input.canvas_snapshot = Some(crate::backend::ai_driven::agent_context::input::AgentCanvasSnapshot {
            width: 1200.0,
            height: 800.0,
            manager_id: Some("manager-1".to_string()),
        });
        let route = crate::backend::ai_driven::agent::AgentToolRoute::new(input, true);
        let ready_prompt = route.current_plan().system_prompt();
        assert!(ready_prompt.contains("update_plan"));
        assert!(ready_prompt.contains("create_canvas_components"));
        let prompt = route.current_plan().system_prompt();
        assert!(prompt.contains("create_canvas_components"));
        assert!(prompt.contains("Applied 回执"));
        assert!(prompt.contains("operationId"));
        assert!(!prompt.contains("submit_result"));
        assert!(!prompt.contains("ai-agent-result/v1"));
        assert!(!prompt.contains("submit_task_completion"));
        assert!(prompt.contains("你负责判断任务是否完成"));
    }
}
