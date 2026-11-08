/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {createElement} from "react";
import {cloneDeep, isEqual} from "lodash";
import {globalModal} from "@datavisool/ui";
import {FrontendEffectClientError} from "@/frontend-effect/FrontendEffectClientError.ts";
import {canvasRevision, runIdempotentCanvasOperation} from "@/frontend-effect/CanvasOperationJournal.ts";
import {deleteLayersByIds} from "@/designer/operate-provider/selection/SelectionCommands.ts";

interface DeleteTarget {
    componentId: string;
    componentType: string;
}

interface ExpectedSnapshot extends DeleteTarget {
    layer: unknown;
}

interface DeletePayload {
    targets: DeleteTarget[];
    expectedSnapshots: ExpectedSnapshot[];
}

interface DeleteResult {
    deletedComponentIds: string[];
    failed: Array<{componentId: string; code: string; message: string}>;
    baseRevision: string;
    resultRevision: string;
}

/** 删除前的强制用户确认。 */
export function runConfirmCanvasDeletionTool(payload: unknown, signal?: AbortSignal): Promise<{confirmed: boolean}> {
    const targets = normalizeTargets(payload);
    const layers = window.DATAVISOOL_ENV?.currentManager?.layerManager?.layerConfigs ?? {};
    const targetLabels = targets.map((target) => {
        const name = typeof layers[target.componentId]?.name === "string" ? layers[target.componentId]?.name?.trim() : "";
        return `${name || target.componentId}（${target.componentType}）`;
    });
    return new Promise((resolve, reject) => {
        const modalApi = globalModal.modalApi;
        if (!modalApi) {
            reject(new FrontendEffectClientError("modal_unavailable", "全局弹窗尚未初始化"));
            return;
        }
        let settled = false;
        const finish = (result: {confirmed: boolean} | FrontendEffectClientError, isError = false) => {
            if (settled)
                return;
            settled = true;
            isError ? reject(result) : resolve(result as {confirmed: boolean});
        };
        const modalId = modalApi.confirm({
            title: "确认删除画布组件",
            content: createElement(
                "div",
                {style: {lineHeight: "22px"}},
                `将删除 ${targets.length} 个组件：${targetLabels.join("、")}`,
            ),
            okText: "确认删除",
            cancelText: "取消",
            okType: "danger",
            maskClosable: false,
            onOk: async () => {
                finish({confirmed: true});
                return true;
            },
            onCancel: () => finish({confirmed: false}),
            afterClose: () => finish({confirmed: false}),
        });
        const abort = () => {
            finish(new FrontendEffectClientError("effect_cancelled", "任务已停止等待删除确认"), true);
            modalApi.destroy(modalId);
        };
        signal?.addEventListener("abort", abort, {once: true});
        if (signal?.aborted)
            abort();
    });
}

/** 使用读取基线保护并幂等执行删除。 */
export async function runDeleteCanvasComponentsTool(payload: unknown): Promise<DeleteResult> {
    requireApprovalTicket(payload);
    return runIdempotentCanvasOperation(payload, () => deleteComponents(payload), (result) => result.failed.length === 0);
}

function requireApprovalTicket(payload: unknown): void {
    const ticket = isRecord(payload) ? payload.approvalTicket : undefined;
    if (!isRecord(ticket) || !text(ticket.approvalId) || !text(ticket.nonce))
        throw new FrontendEffectClientError("approval_ticket_missing", "删除组件必须携带服务端签发的审批票据");
}

function deleteComponents(payload: unknown): DeleteResult {
    const {targets, expectedSnapshots} = normalizeDeletePayload(payload);
    const layerManager = window.DATAVISOOL_ENV?.currentManager?.layerManager;
    const baseRevision = canvasRevision(expectedSnapshots);
    const failures = targets.flatMap((target, index) => {
        const current = layerManager?.layerConfigs?.[target.componentId];
        if (!current) {
            return [{componentId: target.componentId, code: "component_not_found", message: "目标组件已不存在"}];
        }
        if (current.type !== target.componentType) {
            return [{componentId: target.componentId, code: "component_type_mismatch", message: "目标组件类型已变化"}];
        }
        if (!isEqual(current, expectedSnapshots[index].layer)) {
            return [{componentId: target.componentId, code: "component_revision_conflict", message: "目标组件读取后已发生变化"}];
        }
        return [];
    });
    if (failures.length > 0) {
        return {deletedComponentIds: [], failed: failures, baseRevision, resultRevision: baseRevision};
    }

    const targetIds = targets.map((target) => target.componentId);
    deleteLayersByIds(targetIds);
    const remaining = targetIds.filter((id) => Boolean(layerManager?.layerConfigs?.[id]));
    if (remaining.length > 0) {
        return {
            deletedComponentIds: targetIds.filter((id) => !remaining.includes(id)),
            failed: remaining.map((componentId) => ({
                componentId,
                code: "delete_verification_failed",
                message: "删除后目标组件仍存在",
            })),
            baseRevision,
            resultRevision: canvasRevision(remaining.map((id) => cloneDeep(layerManager?.layerConfigs?.[id]))),
        };
    }
    return {
        deletedComponentIds: targetIds,
        failed: [],
        baseRevision,
        resultRevision: canvasRevision({deletedComponentIds: [...targetIds].sort()}),
    };
}

function normalizeDeletePayload(payload: unknown): DeletePayload {
    const targets = normalizeTargets(payload);
    if (!isRecord(payload) || !Array.isArray(payload.expectedSnapshots) || payload.expectedSnapshots.length !== targets.length) {
        throw new FrontendEffectClientError("invalid_payload", "expectedSnapshots 必须与删除目标一一对应");
    }
    const expectedSnapshots = payload.expectedSnapshots.map((value, index) => {
        const target = targets[index];
        if (!isRecord(value)
            || text(value.componentId) !== target.componentId
            || text(value.componentType) !== target.componentType
            || !Object.prototype.hasOwnProperty.call(value, "layer")) {
            throw new FrontendEffectClientError("invalid_payload", `第 ${index + 1} 个删除基线无效`);
        }
        return {...target, layer: cloneDeep(value.layer)};
    });
    return {targets, expectedSnapshots};
}

function normalizeTargets(payload: unknown): DeleteTarget[] {
    if (!isRecord(payload) || !Array.isArray(payload.targets) || payload.targets.length === 0 || payload.targets.length > 100)
        throw new FrontendEffectClientError("invalid_payload", "targets 数量必须在 1–100");
    const targets = payload.targets.map((value, index) => {
        if (!isRecord(value))
            throw new FrontendEffectClientError("invalid_payload", `第 ${index + 1} 个删除目标无效`);
        const componentId = text(value.componentId);
        const componentType = text(value.componentType);
        if (!componentId || !componentType || componentType === "group")
            throw new FrontendEffectClientError("invalid_payload", `第 ${index + 1} 个删除目标缺少有效组件身份`);
        return {componentId, componentType};
    });
    if (new Set(targets.map((target) => target.componentId)).size !== targets.length)
        throw new FrontendEffectClientError("invalid_payload", "删除目标不能重复");
    return targets;
}

function text(value: unknown): string {
    return typeof value === "string" ? value.trim() : "";
}

function isRecord(value: unknown): value is Record<string, any> {
    return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}
