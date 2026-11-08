/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {invoke} from "@tauri-apps/api/core";
import {listen} from "@tauri-apps/api/event";
import {FrontendEffectClientError} from "@/frontend-effect/FrontendEffectClientError.ts";
import {runApplyComponentPatchTool} from "@/frontend-effect/tools/ApplyComponentPatchTool.ts";
import {runApplyComponentStylePatchTool} from "@/frontend-effect/tools/ApplyComponentStylePatchTool.ts";
import {runFrontendEffectLinkTestTool} from "@/frontend-effect/tools/EchoTool.ts";
import {runGetComponentCatalogTool} from "@/frontend-effect/tools/GetComponentCatalogTool.ts";
import {runGetComponentSnapshotsTool} from "@/frontend-effect/tools/GetComponentSnapshotsTool.ts";
import {runGetComponentDataContractTool} from "@/frontend-effect/tools/GetComponentDataContractTool.ts";
import {runPushAiDrivenProgressLogTool} from "@/frontend-effect/tools/PushAiDrivenProgressLogTool.ts";
import {
    runRequestComponentSelectionTool,
    runShowPreviewTool,
} from "@/frontend-effect/tools/PreviewInteractionTools.ts";
import {runRenderDraftTool} from "@/frontend-effect/tools/RenderDraftTool.ts";
import {runUserInputTool} from "@/frontend-effect/tools/UserInputTool.ts";
import {runGetCanvasOperationStatusTool} from "@/frontend-effect/CanvasOperationJournal.ts";
import {runGetCurrentCanvasSelectionTool} from "@/frontend-effect/tools/GetCurrentCanvasSelectionTool.ts";
import {
    runConfirmCanvasDeletionTool,
    runDeleteCanvasComponentsTool,
} from "@/frontend-effect/tools/DeleteCanvasComponentsTool.ts";
import {runConfirmCanvasPatchTool} from "@/frontend-effect/tools/ConfirmCanvasPatchTool.ts";
import type {
    FrontendEffectCancelRequest,
    FrontendEffectErrorPayload,
    FrontendEffectRequest,
    FrontendEffectResponse,
    FrontendEffectToolHandler,
} from "@/frontend-effect/FrontendEffectTypes.ts";

const FRONTEND_EFFECT_LINK_TEST_TOOL_NAME = "echo";
const FRONTEND_EFFECT_USER_INPUT_TOOL_NAME = "userInput";
const FRONTEND_EFFECT_APPLY_COMPONENT_STYLE_PATCH_TOOL_NAME = "applyStylePatch";
const FRONTEND_EFFECT_APPLY_COMPONENT_PATCH_TOOL_NAME = "applyComponentPatch";
const FRONTEND_EFFECT_GET_COMPONENT_CATALOG_TOOL_NAME = "getComponentCatalog";
const FRONTEND_EFFECT_GET_COMPONENT_SNAPSHOTS_TOOL_NAME = "getComponentSnapshots";
const FRONTEND_EFFECT_GET_COMPONENT_DATA_CONTRACT_TOOL_NAME = "getComponentDataContract";
const FRONTEND_EFFECT_PUSH_AI_DRIVEN_PROGRESS_LOG_TOOL_NAME = "logProgress";
const FRONTEND_EFFECT_RENDER_DRAFT_TOOL_NAME = "renderDraft";
const FRONTEND_EFFECT_SHOW_PREVIEW_TOOL_NAME = "showPreview";
const FRONTEND_EFFECT_REQUEST_COMPONENT_SELECTION_TOOL_NAME = "requestComponentSelection";
const FRONTEND_EFFECT_GET_CANVAS_OPERATION_STATUS_TOOL_NAME = "getCanvasOperationStatus";
const FRONTEND_EFFECT_GET_CURRENT_CANVAS_SELECTION_TOOL_NAME = "getCurrentCanvasSelection";
const FRONTEND_EFFECT_CONFIRM_CANVAS_DELETION_TOOL_NAME = "confirmCanvasDeletion";
const FRONTEND_EFFECT_CONFIRM_CANVAS_PATCH_TOOL_NAME = "confirmCanvasPatch";
const FRONTEND_EFFECT_DELETE_CANVAS_COMPONENTS_TOOL_NAME = "deleteCanvasComponents";
const frontendEffectTools = new Map<string, FrontendEffectToolHandler>();
const activeFrontendEffects = new Map<string, AbortController>();
let frontendEffectUnlisten: (() => void) | null = null;

/**
 * 注册一个可被 Rust 调用的前端工具。
 *
 * Rust 只会通过工具名发起调用；这里把工具名映射到前端函数。
 * 返回的清理函数用于后续扩展动态工具时解除注册。
 */
export const registerFrontendEffectTool = (
    name: string,
    handler: FrontendEffectToolHandler,
): (() => void) => {
    frontendEffectTools.set(name, handler);
    return () => {
        // 只删除当前 handler，避免旧清理函数误删同名新工具。
        if (frontendEffectTools.get(name) === handler)
            frontendEffectTools.delete(name);
    };
};

/**
 * 注册 Tauri 前端交互事件监听器。
 *
 * 整体链路：
 * 1. Rust emit `datavisool:frontend-effect-request`。
 * 2. 前端根据 kind 执行工具或弹窗。
 * 3. 前端 invoke `datavisool_complete_frontend_effect` 回传结果。
 */
export async function registerFrontendEffectListener(): Promise<() => void> {
    registerFrontendEffectTools();
    if (frontendEffectUnlisten)
        return frontendEffectUnlisten;

    const [unlistenRequest, unlistenCancel] = await Promise.all([
        listen<FrontendEffectRequest>("datavisool:frontend-effect-request", async (event) => {
            await handleFrontendEffectRequest(event.payload);
        }),
        listen<FrontendEffectCancelRequest>("datavisool:frontend-effect-cancel", (event) => {
            activeFrontendEffects.get(event.payload.effectId)?.abort(event.payload.reason);
        }),
    ]);
    frontendEffectUnlisten = () => {
        unlistenRequest();
        unlistenCancel();
        activeFrontendEffects.forEach((controller) => controller.abort("listener_disposed"));
        activeFrontendEffects.clear();
        frontendEffectUnlisten = null;
    };
    return frontendEffectUnlisten;
}

/**
 * 分发单次 Rust 前端交互请求。
 *
 * 无论成功还是失败，都必须回调 Rust；否则 Rust 侧 oneshot receiver 会一直 await。
 */
async function handleFrontendEffectRequest(request: FrontendEffectRequest): Promise<void> {
    const controller = new AbortController();
    activeFrontendEffects.set(request.effectId, controller);
    let response: FrontendEffectResponse;
    try {
        const agentScope = validateAgentScope(request.payload);
        // toolCall 走工具注册表，userInput 走用户补充输入工具。
        const data = request.kind === "toolCall"
            ? await runFrontendTool(request.name, request.payload, controller.signal)
            : await runUserInputTool(request.name, request.payload, controller.signal);
        if (controller.signal.aborted)
            return;
        // 异步用户交互返回时再次检查，避免等待期间切换项目或画布后继续旧任务。
        validateCurrentAgentScope(agentScope);
        response = {
            effectId: request.effectId,
            binding: request.binding,
            ok: true,
            data,
        };
    } catch (error) {
        if (controller.signal.aborted)
            return;
        response = {
            effectId: request.effectId,
            binding: request.binding,
            ok: false,
            error: normalizeFrontendEffectError(error),
        };
    } finally {
        activeFrontendEffects.delete(request.effectId);
    }
    if (!controller.signal.aborted)
        await completeFrontendEffect(response);
}

interface FrontendAgentScope {
    projectId: string;
    managerId?: string;
}

/** Rust Agent 请求携带服务端任务作用域；旧的非 Agent Effect 没有该字段，保持兼容。
 * TODO(legacy-compat): 确认非 Agent 调用方已迁移后，移除缺失作用域的兼容分支。
 */
function validateAgentScope(payload: unknown): FrontendAgentScope | undefined {
    if (!payload || typeof payload !== "object" || Array.isArray(payload))
        return undefined;
    const rawScope = (payload as Record<string, unknown>).agentScope;
    if (rawScope === undefined)
        return undefined;
    if (!rawScope || typeof rawScope !== "object" || Array.isArray(rawScope))
        throw new FrontendEffectClientError("agent_scope_invalid", "Agent 前端请求缺少有效作用域");
    const projectId = typeof (rawScope as Record<string, unknown>).projectId === "string"
        ? ((rawScope as Record<string, unknown>).projectId as string).trim()
        : "";
    const managerIdValue = (rawScope as Record<string, unknown>).managerId;
    const managerId = typeof managerIdValue === "string" ? managerIdValue.trim() : undefined;
    if (!projectId)
        throw new FrontendEffectClientError("agent_scope_invalid", "Agent 前端请求缺少 projectId");
    const scope = {projectId, managerId: managerId || undefined};
    validateCurrentAgentScope(scope);
    return scope;
}

function validateCurrentAgentScope(scope: FrontendAgentScope | undefined): void {
    if (!scope)
        return;
    const currentProjectId = window.DATAVISOOL_ENV?.projectId?.trim() ?? "";
    if (currentProjectId !== scope.projectId) {
        throw new FrontendEffectClientError(
            "agent_scope_project_changed",
            `AI 任务所属项目已切换：期望 ${scope.projectId}，当前 ${currentProjectId || "unknown"}`,
        );
    }
    if (!scope.managerId)
        return;
    const currentManagerId = window.DATAVISOOL_ENV?.currentManager?.id?.trim() ?? "";
    if (currentManagerId !== scope.managerId) {
        throw new FrontendEffectClientError(
            "agent_scope_manager_changed",
            `AI 任务所属画布已切换：期望 ${scope.managerId}，当前 ${currentManagerId || "unknown"}`,
        );
    }
}

/**
 * 调用 Rust command 完成一次前端交互。
 *
 * response.effectId 会让 Rust 从 registry 中取出对应 sender 并唤醒 await。
 */
async function completeFrontendEffect(response: FrontendEffectResponse): Promise<void> {
    await invoke("datavisool_complete_frontend_effect", {response});
}

/**
 * 执行指定前端工具。
 *
 * 工具不存在时返回明确错误码，Rust 侧会把它转为 FrontendFailed。
 */
async function runFrontendTool(name: string, payload: unknown, signal?: AbortSignal): Promise<unknown> {
    const handler = frontendEffectTools.get(name);
    if (!handler)
        throw new FrontendEffectClientError("tool_not_found", `未找到前端工具：${name}`);
    return await handler(payload, signal);
}

/**
 * 注册前端工具。
 *
 * 内置工具在监听器初始化时注册，确保 Rust 侧可直接按工具名调用。
 */
function registerFrontendEffectTools(): void {
    if (!frontendEffectTools.has(FRONTEND_EFFECT_LINK_TEST_TOOL_NAME))
        registerFrontendEffectTool(FRONTEND_EFFECT_LINK_TEST_TOOL_NAME, runFrontendEffectLinkTestTool);
    if (!frontendEffectTools.has(FRONTEND_EFFECT_USER_INPUT_TOOL_NAME))
        registerFrontendEffectTool(FRONTEND_EFFECT_USER_INPUT_TOOL_NAME, runUserInputFrontendTool);
    if (!frontendEffectTools.has(FRONTEND_EFFECT_APPLY_COMPONENT_STYLE_PATCH_TOOL_NAME))
        registerFrontendEffectTool(FRONTEND_EFFECT_APPLY_COMPONENT_STYLE_PATCH_TOOL_NAME, runApplyComponentStylePatchTool);
    if (!frontendEffectTools.has(FRONTEND_EFFECT_APPLY_COMPONENT_PATCH_TOOL_NAME))
        registerFrontendEffectTool(FRONTEND_EFFECT_APPLY_COMPONENT_PATCH_TOOL_NAME, runApplyComponentPatchTool);
    if (!frontendEffectTools.has(FRONTEND_EFFECT_GET_COMPONENT_CATALOG_TOOL_NAME))
        registerFrontendEffectTool(
            FRONTEND_EFFECT_GET_COMPONENT_CATALOG_TOOL_NAME,
            runGetComponentCatalogTool,
        );
    if (!frontendEffectTools.has(FRONTEND_EFFECT_GET_COMPONENT_SNAPSHOTS_TOOL_NAME))
        registerFrontendEffectTool(
            FRONTEND_EFFECT_GET_COMPONENT_SNAPSHOTS_TOOL_NAME,
            runGetComponentSnapshotsTool,
        );
    if (!frontendEffectTools.has(FRONTEND_EFFECT_GET_COMPONENT_DATA_CONTRACT_TOOL_NAME))
        registerFrontendEffectTool(
            FRONTEND_EFFECT_GET_COMPONENT_DATA_CONTRACT_TOOL_NAME,
            runGetComponentDataContractTool,
        );
    if (!frontendEffectTools.has(FRONTEND_EFFECT_PUSH_AI_DRIVEN_PROGRESS_LOG_TOOL_NAME))
        registerFrontendEffectTool(
            FRONTEND_EFFECT_PUSH_AI_DRIVEN_PROGRESS_LOG_TOOL_NAME,
            runPushAiDrivenProgressLogTool,
        );
    if (!frontendEffectTools.has(FRONTEND_EFFECT_RENDER_DRAFT_TOOL_NAME))
        registerFrontendEffectTool(
            FRONTEND_EFFECT_RENDER_DRAFT_TOOL_NAME,
            runRenderDraftTool,
        );
    if (!frontendEffectTools.has(FRONTEND_EFFECT_SHOW_PREVIEW_TOOL_NAME))
        registerFrontendEffectTool(
            FRONTEND_EFFECT_SHOW_PREVIEW_TOOL_NAME,
            runShowPreviewTool,
        );
    if (!frontendEffectTools.has(FRONTEND_EFFECT_REQUEST_COMPONENT_SELECTION_TOOL_NAME))
        registerFrontendEffectTool(
            FRONTEND_EFFECT_REQUEST_COMPONENT_SELECTION_TOOL_NAME,
            runRequestComponentSelectionTool,
        );
    if (!frontendEffectTools.has(FRONTEND_EFFECT_GET_CANVAS_OPERATION_STATUS_TOOL_NAME))
        registerFrontendEffectTool(
            FRONTEND_EFFECT_GET_CANVAS_OPERATION_STATUS_TOOL_NAME,
            runGetCanvasOperationStatusTool,
        );
    if (!frontendEffectTools.has(FRONTEND_EFFECT_GET_CURRENT_CANVAS_SELECTION_TOOL_NAME))
        registerFrontendEffectTool(
            FRONTEND_EFFECT_GET_CURRENT_CANVAS_SELECTION_TOOL_NAME,
            runGetCurrentCanvasSelectionTool,
        );
    if (!frontendEffectTools.has(FRONTEND_EFFECT_CONFIRM_CANVAS_DELETION_TOOL_NAME))
        registerFrontendEffectTool(
            FRONTEND_EFFECT_CONFIRM_CANVAS_DELETION_TOOL_NAME,
            runConfirmCanvasDeletionTool,
        );
    if (!frontendEffectTools.has(FRONTEND_EFFECT_CONFIRM_CANVAS_PATCH_TOOL_NAME))
        registerFrontendEffectTool(
            FRONTEND_EFFECT_CONFIRM_CANVAS_PATCH_TOOL_NAME,
            runConfirmCanvasPatchTool,
        );
    if (!frontendEffectTools.has(FRONTEND_EFFECT_DELETE_CANVAS_COMPONENTS_TOOL_NAME))
        registerFrontendEffectTool(
            FRONTEND_EFFECT_DELETE_CANVAS_COMPONENTS_TOOL_NAME,
            runDeleteCanvasComponentsTool,
        );
}

/**
 * 执行用户补充输入前端工具。
 *
 * 工具注册表只传 payload，这里补齐固定弹框标题后复用现有用户输入交互。
 */
function runUserInputFrontendTool(payload: unknown, signal?: AbortSignal): Promise<unknown> {
    return runUserInputTool("用户补充输入", payload, signal);
}

/**
 * 把未知异常归一化为前端 effect 错误载荷。
 *
 * Rust 侧只理解 code/message，所以这里把 Error、字符串和自定义错误统一收敛。
 */
function normalizeFrontendEffectError(error: unknown): FrontendEffectErrorPayload {
    if (error instanceof FrontendEffectClientError) {
        return {
            code: error.code,
            message: error.message,
        };
    }
    if (error instanceof Error) {
        return {
            code: "frontend_effect_failed",
            message: error.message,
        };
    }
    return {
        code: "frontend_effect_failed",
        message: String(error || "前端交互失败"),
    };
}
