/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {FrontendEffectClientError} from "@/frontend-effect/FrontendEffectClientError.ts";
import {
    aiLayerDraftRuntime,
    type AiLayerDraftComponent,
} from "@/designer/ai-driven/AiLayerDraftRuntime.ts";
import {
    canvasRevision,
    hasCanvasOperationIdentity,
    runIdempotentCanvasOperation,
} from "@/frontend-effect/CanvasOperationJournal.ts";

interface RenderDraftPayload {
    action?: "render" | "accept" | "clear";
    incremental?: boolean;
    interactive?: boolean;
    draftId?: string;
    components?: AiLayerDraftComponent[];
    requestedComponentIds?: string[];
    reservedComponentIds?: Record<string, string>;
    approvalTicket?: unknown;
}

interface NormalizedRenderDraftPayload {
    action: "render" | "accept" | "clear";
    incremental: boolean;
    interactive: boolean;
    draftId: string;
    components: AiLayerDraftComponent[];
    requestedComponentIds?: string[];
    reservedComponentIds?: Record<string, string>;
}

export type RenderDraftToolResult =
    | {
        status: "draft-rendered";
        draftId: string;
        layerCount: number;
    }
    | {
        status: "draft-cleared";
        layerCount: 0;
    }
    | {
        status: "draft-accepted";
        draftId: string;
        layerCount: number;
        componentBindings: Record<string, string>;
        appliedLayerIds: string[];
        resultRevision?: string;
    };

/** 创建、接受或清空画布上的临时草稿。 */
export function runRenderDraftTool(payload: unknown): RenderDraftToolResult | Promise<RenderDraftToolResult> {
    const input = normalizePayload(payload);
    if (input.action === "clear") {
        aiLayerDraftRuntime.clear();
        return {
            status: "draft-cleared",
            layerCount: 0,
        };
    }
    if (input.action === "accept") {
        requireApprovalTicket(payload);
        if (!hasCanvasOperationIdentity(payload))
            return acceptDraft(input.draftId, input.requestedComponentIds, input.reservedComponentIds);
        return runIdempotentCanvasOperation(payload, () => acceptDraft(input.draftId, input.requestedComponentIds, input.reservedComponentIds));
    }

    try {
        return aiLayerDraftRuntime.render(input.components, input.incremental, input.interactive);
    } catch (error) {
        const message = error instanceof Error ? error.message : String(error ?? "未知错误");
        throw new FrontendEffectClientError("draft_render_failed", message);
    }
}

function requireApprovalTicket(payload: unknown): void {
    const ticket = payload && typeof payload === "object" && !Array.isArray(payload)
        ? (payload as Record<string, unknown>).approvalTicket
        : undefined;
    if (!ticket || typeof ticket !== "object" || Array.isArray(ticket)
        || typeof (ticket as Record<string, unknown>).approvalId !== "string"
        || typeof (ticket as Record<string, unknown>).nonce !== "string") {
        throw new FrontendEffectClientError("approval_ticket_missing", "接受草稿必须携带服务端签发的审批票据");
    }
}

function acceptDraft(
    draftId: string,
    requestedComponentIds?: string[],
    reservedComponentIds?: Record<string, string>,
): RenderDraftToolResult {
    const activeDraft = aiLayerDraftRuntime.activeDraft;
    if (!activeDraft || activeDraft.draftId !== draftId)
        throw new FrontendEffectClientError("draft_not_found", "已确认的页面草稿不存在或已发生变化");

    const layerCount = activeDraft.layerCount;
    const accepted = aiLayerDraftRuntime.acceptDraft(draftId, requestedComponentIds, reservedComponentIds);
    if (!accepted || accepted.appliedLayerIds.length < layerCount)
        throw new FrontendEffectClientError("draft_accept_failed", "页面草稿未能完整写入画布");
    const layerManager = window.DATAVISOOL_ENV?.currentManager?.layerManager;
    const applied = accepted.appliedLayerIds.map((componentId) => ({
        componentId,
        config: layerManager?.controllers?.[componentId]?.getConfig?.()
            ?? layerManager?.elemConfigs?.[componentId]
            ?? null,
    }));
    return {
        status: "draft-accepted",
        draftId,
        layerCount: accepted.appliedLayerIds.length,
        componentBindings: accepted.componentBindings,
        appliedLayerIds: accepted.appliedLayerIds,
        resultRevision: canvasRevision(applied),
    };
}

function normalizePayload(payload: unknown): NormalizedRenderDraftPayload {
    if (!payload || typeof payload !== "object" || Array.isArray(payload))
        throw new FrontendEffectClientError("invalid_payload", "草稿渲染工具入参必须是对象");

    const input = payload as RenderDraftPayload;
    const action = input.action ?? "render";
    if (action !== "render" && action !== "accept" && action !== "clear")
        throw new FrontendEffectClientError("invalid_payload", `不支持的草稿操作：${String(action)}`);
    if (input.incremental != null && typeof input.incremental !== "boolean")
        throw new FrontendEffectClientError("invalid_payload", "草稿增量渲染参数 incremental 必须是布尔值");
    if (input.interactive != null && typeof input.interactive !== "boolean")
        throw new FrontendEffectClientError("invalid_payload", "草稿交互参数 interactive 必须是布尔值");

    if (action === "clear")
        return {action, incremental: false, interactive: false, draftId: "", components: []};
    if (action === "accept") {
        const draftId = typeof input.draftId === "string" ? input.draftId.trim() : "";
        if (!draftId)
            throw new FrontendEffectClientError("invalid_payload", "接受页面草稿时必须提供 draftId");
        let normalizedRequestedComponentIds: string[] | undefined;
        if (input.requestedComponentIds !== undefined) {
            if (!Array.isArray(input.requestedComponentIds)
                || input.requestedComponentIds.some((id) => typeof id !== "string" || !id.trim())) {
                throw new FrontendEffectClientError("invalid_payload", "requestedComponentIds 必须是非空字符串数组");
            }
            normalizedRequestedComponentIds = input.requestedComponentIds.map((id) => id.trim());
        }
        let normalizedReservedComponentIds: Record<string, string> | undefined;
        if (input.reservedComponentIds !== undefined) {
            if (!input.reservedComponentIds || typeof input.reservedComponentIds !== "object" || Array.isArray(input.reservedComponentIds))
                throw new FrontendEffectClientError("invalid_payload", "reservedComponentIds 必须是对象");
            const entries = Object.entries(input.reservedComponentIds);
            if (entries.some(([sourceId, layerId]) => !sourceId.trim() || typeof layerId !== "string" || !layerId.trim()))
                throw new FrontendEffectClientError("invalid_payload", "reservedComponentIds 的键和值必须是非空字符串");
            normalizedReservedComponentIds = Object.fromEntries(entries.map(([sourceId, layerId]) => [sourceId.trim(), layerId.trim()]));
        }
        return {
            action,
            incremental: false,
            interactive: false,
            draftId,
            components: [],
            requestedComponentIds: normalizedRequestedComponentIds,
            reservedComponentIds: normalizedReservedComponentIds,
        };
    }
    if (!Array.isArray(input.components) || input.components.length === 0)
        throw new FrontendEffectClientError("invalid_payload", "草稿组件列表不能为空");

    const incremental = input.incremental ?? false;
    const interactive = input.interactive ?? true;
    const components = input.components.map(normalizeComponent);
    if (incremental && components.some((component) => !component.id))
        throw new FrontendEffectClientError("invalid_component", "增量渲染的每个草稿组件都必须提供 id");

    return {
        action,
        incremental,
        interactive,
        draftId: "",
        components,
    };
}

function normalizeComponent(component: AiLayerDraftComponent, index: number): AiLayerDraftComponent {
    if (!component || typeof component !== "object" || Array.isArray(component))
        throw new FrontendEffectClientError("invalid_component", `第 ${index + 1} 个草稿组件必须是对象`);

    const type = typeof component.type === "string" ? component.type.trim() : "";
    if (!type)
        throw new FrontendEffectClientError("invalid_component", `第 ${index + 1} 个草稿组件缺少 type`);

    const x = requireFiniteNumber(component.x, "x", index);
    const y = requireFiniteNumber(component.y, "y", index);
    const width = requirePositiveNumber(component.width, "width", index);
    const height = requirePositiveNumber(component.height, "height", index);
    const order = component.order == null
        ? undefined
        : requireFiniteNumber(component.order, "order", index);

    if (component.config != null && (
        typeof component.config !== "object"
        || Array.isArray(component.config)
    )) {
        throw new FrontendEffectClientError("invalid_component", `第 ${index + 1} 个草稿组件 config 必须是对象`);
    }

    return {
        id: typeof component.id === "string" ? component.id.trim() || undefined : undefined,
        type,
        name: typeof component.name === "string" ? component.name.trim() || undefined : undefined,
        x,
        y,
        width,
        height,
        order,
        config: component.config,
    };
}

function requireFiniteNumber(value: unknown, field: string, index: number): number {
    if (typeof value !== "number" || !Number.isFinite(value))
        throw new FrontendEffectClientError("invalid_component", `第 ${index + 1} 个草稿组件 ${field} 必须是有限数字`);
    return value;
}

function requirePositiveNumber(value: unknown, field: string, index: number): number {
    const number = requireFiniteNumber(value, field, index);
    if (number <= 0)
        throw new FrontendEffectClientError("invalid_component", `第 ${index + 1} 个草稿组件 ${field} 必须大于 0`);
    return number;
}
