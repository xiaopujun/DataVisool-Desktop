/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
/*
 * Copyright © 2023-2025 puyinzhen
 * All rights reserved.
 */

export interface IAiDrivenToolPresentation {
    title: string;
    detail: string;
}

const TOOL_PRESENTATIONS: Record<string, IAiDrivenToolPresentation> = {
    update_plan: {title: "更新任务计划", detail: "调整后续执行步骤"},
    declare_execution_scope: {title: "声明执行范围", detail: "确认本轮允许的写入动作"},
    inspect_current_page: {title: "读取当前页面", detail: "了解画布结构和现有内容"},
    get_component_document: {title: "读取组件说明", detail: "确认组件能力和配置方式"},
    get_component_catalog: {title: "读取组件目录", detail: "确认可用组件类型和名称"},
    get_component_data_contract: {title: "读取数据说明书", detail: "确认数据源结构与组件数据格式"},
    create_canvas_components: {title: "创建画布组件", detail: "预览确认后写入真实画布"},
    patch_canvas_components: {title: "修改画布组件", detail: "校验快照后原子更新真实画布"},
    delete_canvas_components: {title: "删除画布组件", detail: "确认并校验快照后删除真实画布组件"},
    get_current_canvas_selection: {title: "读取当前选区", detail: "获取调用时刻的真实画布选区"},
    get_canvas_operation_status: {title: "查询画布动作", detail: "校准尚未确认的动作结果"},
    request_component_selection: {title: "等待组件选择", detail: "需要选择本次使用的组件"},
    request_user_input: {title: "等待补充信息", detail: "需要更多信息后继续"},
    ask_user: {title: "等待补充信息", detail: "需要更多信息后继续"},
    list_workspace_files: {title: "查看工作区文件", detail: "了解已有草稿和文档"},
    search_workspace_files: {title: "搜索工作区文件", detail: "查找相关内容"},
    search_component_documents: {title: "搜索组件说明", detail: "查找组件能力与配置规则"},
    read_workspace_file: {title: "读取工作区文件", detail: "检查文件内容"},
    apply_workspace_patch: {title: "修改工作区文件", detail: "写入文件变更"},
    apply_patch: {title: "修改工作区文件", detail: "写入文件变更"},
    report_progress: {title: "更新任务状态", detail: "同步当前执行进度"},
};

export const getAiDrivenToolPresentation = (toolName: string): IAiDrivenToolPresentation => {
    return TOOL_PRESENTATIONS[toolName] ?? {
        title: toolName ? `执行 ${toolName}` : "执行下一步",
        detail: "Agent 正在处理当前任务",
    };
};

export const hiddenLiveToolNames = new Set(["report_progress", "update_plan", "declare_execution_scope"]);
