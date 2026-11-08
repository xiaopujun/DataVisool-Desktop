/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {cloneDeep, isEqual} from "lodash";
import {FrontendEffectClientError} from "@/frontend-effect/FrontendEffectClientError.ts";

/** 统一样式补丁协议：顶层只能包含非空 style。 */
interface ComponentStylePatch {
    style: Record<string, unknown>;
}

/**
 * 单个组件样式补丁请求。
 */
interface ApplyComponentStylePatchItem {
    /** 目标组件 id。 */
    componentId: string;
    /** 目标组件类型，必须与当前画布组件类型一致。 */
    componentType: string;
    /** 只包含 style 根字段的完整组件配置补丁。 */
    stylePatch: ComponentStylePatch;
    /** AI 生成该补丁的简短原因。 */
    reason?: string;
}

/**
 * 批量组件样式补丁请求。
 */
interface ApplyComponentStylePatchPayload {
    /** 本次工具调用要应用的补丁列表。 */
    patches: ApplyComponentStylePatchItem[];
    /** 服务端从 inspect_current_page 结果保存的只读并发基线。 */
    expectedSnapshots?: ExpectedComponentStyleSnapshot[];
}

interface ExpectedComponentStyleSnapshot {
    componentId: string;
    componentType: string;
    style: unknown;
}

/**
 * 单个组件样式补丁应用成功结果。
 */
interface ApplyComponentStylePatchSuccess {
    /** 目标组件 id。 */
    componentId: string;
    /** 目标组件类型。 */
    componentType: string;
    /** 应用后的组件 style 快照。 */
    style: unknown;
}

/**
 * 单个组件样式补丁应用失败结果。
 */
interface ApplyComponentStylePatchFailure {
    /** 目标组件 id。 */
    componentId: string;
    /** 目标组件类型。 */
    componentType?: string;
    /** 稳定错误码。 */
    code: string;
    /** 错误说明。 */
    message: string;
}

/**
 * 批量组件样式补丁应用结果。
 */
interface ApplyComponentStylePatchResult {
    /** 成功应用的组件结果。 */
    applied: ApplyComponentStylePatchSuccess[];
    /** 应用失败的组件结果。 */
    failed: ApplyComponentStylePatchFailure[];
}

/**
 * 执行组件样式补丁应用工具。
 */
export function runApplyComponentStylePatchTool(payload: unknown): ApplyComponentStylePatchResult {
    const {patches, expectedSnapshots} = normalizePayload(payload);
    const failed: ApplyComponentStylePatchFailure[] = [];
    const layerManager = window.DATAVISOOL_ENV?.currentManager?.layerManager;

    if (expectedSnapshots) {
        const conflicts = findStyleRevisionConflicts(patches, expectedSnapshots, layerManager);
        if (conflicts.length > 0)
            return rejectStyleBatch(patches, conflicts);
    }

    const prepared = patches.flatMap((patch) => {
        const controller = layerManager?.controllers?.[patch.componentId];
        const layer = layerManager?.layerConfigs?.[patch.componentId];
        const currentConfig = controller?.getConfig?.();
        const currentType = currentConfig?.base?.type ?? layer?.type;

        if (!controller) {
            failed.push({
                componentId: patch.componentId,
                componentType: patch.componentType,
                code: "controller_not_found",
                message: `未找到组件控制器：${patch.componentId}`,
            });
            return [];
        }
        if (currentType !== patch.componentType) {
            failed.push({
                componentId: patch.componentId,
                componentType: patch.componentType,
                code: "component_type_mismatch",
                message: `组件类型不匹配：期望 ${patch.componentType}，实际 ${currentType || "unknown"}`,
            });
            return [];
        }
        return [{patch, controller, previousStyle: cloneDeep(currentConfig?.style)}];
    });

    if (failed.length > 0)
        return rejectStylePreflightBatch(patches, failed);

    const applied: ApplyComponentStylePatchSuccess[] = [];
    try {
        prepared.forEach(({patch, controller}) => {
            // stylePatch 已是完整组件配置补丁，不再额外包装 style。
            controller.update(cloneDeep(patch.stylePatch) as any, {reRender: true});
            const appliedStyle = controller.getConfig?.()?.style;
            if (!containsPatch(appliedStyle, patch.stylePatch.style))
                throw new Error(`组件样式补丁未实际生效：${patch.componentId}`);
            applied.push({
                componentId: patch.componentId,
                componentType: patch.componentType,
                style: cloneDeep(appliedStyle),
            });
        });
    } catch (error) {
        prepared.forEach(({controller, previousStyle}) => {
            controller.update({style: cloneDeep(previousStyle)} as any, {reRender: true});
        });
        const message = error instanceof Error ? error.message : String(error);
        return {
            applied: [],
            failed: patches.map((patch) => ({
                componentId: patch.componentId,
                componentType: patch.componentType,
                code: "batch_apply_rolled_back",
                message: `样式补丁整批应用失败并已回滚：${message}`,
            })),
        };
    }

    return {applied, failed};
}

function rejectStylePreflightBatch(
    patches: ApplyComponentStylePatchItem[],
    failures: ApplyComponentStylePatchFailure[],
): ApplyComponentStylePatchResult {
    const failureById = new Map(failures.map((failure) => [failure.componentId, failure]));
    return {
        applied: [],
        failed: patches.map((patch) => failureById.get(patch.componentId) ?? ({
            componentId: patch.componentId,
            componentType: patch.componentType,
            code: "batch_preflight_aborted",
            message: "同批次存在无效样式补丁，整批未应用",
        })),
    };
}

/**
 * 归一化并校验工具入参。
 */
function normalizePayload(payload: unknown): {
    patches: ApplyComponentStylePatchItem[];
    expectedSnapshots?: ExpectedComponentStyleSnapshot[];
} {
    if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
        throw new FrontendEffectClientError("invalid_payload", "样式补丁工具入参必须是对象");
    }

    const rawPayload = payload as ApplyComponentStylePatchPayload;
    if (!Array.isArray(rawPayload.patches) || rawPayload.patches.length === 0) {
        throw new FrontendEffectClientError("invalid_payload", "样式补丁列表不能为空");
    }

    const patches = rawPayload.patches.map((patch, index) => {
        if (!patch || typeof patch !== "object" || Array.isArray(patch)) {
            throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个样式补丁必须是对象`);
        }
        if (!patch.componentId?.trim()) {
            throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个样式补丁缺少 componentId`);
        }
        if (!patch.componentType?.trim()) {
            throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个样式补丁缺少 componentType`);
        }
        return {
            componentId: patch.componentId.trim(),
            componentType: patch.componentType.trim(),
            stylePatch: normalizeStylePatch(patch.stylePatch, index),
            reason: patch.reason,
        };
    });
    return {
        patches,
        expectedSnapshots: normalizeExpectedSnapshots(rawPayload.expectedSnapshots, patches),
    };
}

function normalizeExpectedSnapshots(
    value: unknown,
    patches: ApplyComponentStylePatchItem[],
): ExpectedComponentStyleSnapshot[] | undefined {
    if (value === undefined)
        return undefined;
    if (!Array.isArray(value) || value.length === 0)
        throw new FrontendEffectClientError("invalid_payload", "expectedSnapshots 必须是非空数组");
    const snapshots = value.map((snapshot, index) => {
        if (!isRecord(snapshot) || typeof snapshot.componentId !== "string" || !snapshot.componentId.trim()
            || typeof snapshot.componentType !== "string" || !snapshot.componentType.trim()
            || !Object.prototype.hasOwnProperty.call(snapshot, "style")) {
            throw new FrontendEffectClientError("invalid_payload", `第 ${index + 1} 个样式基线无效`);
        }
        return {
            componentId: snapshot.componentId.trim(),
            componentType: snapshot.componentType.trim(),
            style: cloneDeep(snapshot.style),
        };
    });
    const targetIds = new Set(patches.map((patch) => patch.componentId));
    const snapshotIds = new Set(snapshots.map((snapshot) => snapshot.componentId));
    if (snapshots.length !== snapshotIds.size || snapshotIds.size !== targetIds.size
        || [...snapshotIds].some((componentId) => !targetIds.has(componentId))) {
        throw new FrontendEffectClientError("invalid_payload", "expectedSnapshots 必须与补丁目标一一对应");
    }
    return snapshots;
}

function findStyleRevisionConflicts(
    patches: ApplyComponentStylePatchItem[],
    snapshots: ExpectedComponentStyleSnapshot[],
    layerManager: any,
): string[] {
    const snapshotById = new Map(snapshots.map((snapshot) => [snapshot.componentId, snapshot]));
    return patches.flatMap((patch) => {
        const expected = snapshotById.get(patch.componentId)!;
        const controller = layerManager?.controllers?.[patch.componentId];
        const layer = layerManager?.layerConfigs?.[patch.componentId];
        const currentConfig = controller?.getConfig?.();
        const currentType = currentConfig?.base?.type ?? layer?.type;
        if (!controller)
            return [`${patch.componentId}:组件已不存在`];
        if (expected.componentType !== patch.componentType || currentType !== expected.componentType)
            return [`${patch.componentId}:组件类型已变化`];
        if (!isEqual(currentConfig?.style ?? null, expected.style))
            return [`${patch.componentId}:style 已变化`];
        return [];
    });
}

function rejectStyleBatch(
    patches: ApplyComponentStylePatchItem[],
    conflicts: string[],
): ApplyComponentStylePatchResult {
    const message = `画布组件在读取后发生变化，整批补丁未应用：${conflicts.join("；")}`;
    return {
        applied: [],
        failed: patches.map((patch) => ({
            componentId: patch.componentId,
            componentType: patch.componentType,
            code: "component_revision_conflict",
            message,
        })),
    };
}

/** 校验统一样式补丁协议，不兼容旧的扁平格式。 */
function normalizeStylePatch(value: unknown, index: number): ComponentStylePatch {
    if (!isRecord(value))
        throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个 stylePatch 必须是对象`);
    const rootKeys = Object.keys(value);
    if (rootKeys.length !== 1 || rootKeys[0] !== "style")
        throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个 stylePatch 顶层只能包含 style`);
    if (!isRecord(value.style) || Object.keys(value.style).length === 0)
        throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个 stylePatch.style 必须是非空对象`);
    return {style: value.style};
}

/** 判断应用后的真实 style 是否完整包含补丁内容。 */
function containsPatch(actual: unknown, patch: unknown): boolean {
    if (!isRecord(patch))
        return isEqual(actual, patch);
    if (!isRecord(actual))
        return false;
    return Object.entries(patch).every(([key, value]) => containsPatch(actual[key], value));
}

function isRecord(value: unknown): value is Record<string, any> {
    return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}
