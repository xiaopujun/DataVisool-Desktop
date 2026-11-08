/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
/*
 * Copyright © 2023-2025 puyinzhen
 * All rights reserved.
 */

import {cloneDeep, isEqual, merge} from "lodash";
import {FrontendEffectClientError} from "@/frontend-effect/FrontendEffectClientError.ts";
import {
    findInvalidExpressionInPatch,
    findSensitiveFieldInPatch,
    getDataPatchUserInputRequirement,
} from "@/util/DataSourceAiPolicy.ts";
import {normalizeDataConfigAsync} from "@/util/DataSourceConfigUtil.ts";
import {
    COMPONENT_DATA_FIELD_MAPPING_KEYS,
    validateComponentFieldMappings,
} from "@/util/DataSourceContractUtil.ts";
import componentDataGvParser from "@/designer/left/global-variable/parse/ComponentDataGvParser.ts";
import {canvasRevision, runIdempotentCanvasOperation} from "@/frontend-effect/CanvasOperationJournal.ts";
import {toSerializableSnapshot} from "@/frontend-effect/SerializableSnapshot.ts";

interface ComponentPatchItem {
    componentId: string;
    componentType: string;
    stylePatch?: {style: Record<string, unknown>};
    dataPatch?: Record<string, unknown>;
    layerPatch?: {
        x?: number;
        y?: number;
        width?: number;
        height?: number;
        name?: string;
    };
    filterPatch?: {
        enable?: boolean;
        blur?: number;
        brightness?: number;
        contrast?: number;
        opacity?: number;
        saturate?: number;
        hueRotate?: number;
    };
    reason?: string;
}

interface ExpectedComponentSnapshot {
    componentId: string;
    componentType: string;
    style?: unknown;
    data?: unknown;
    layer?: unknown;
    filter?: unknown;
}

interface ApplyComponentPatchPayload {
    patches: ComponentPatchItem[];
    expectedSnapshots: ExpectedComponentSnapshot[];
}

interface ComponentPatchFailure {
    componentId: string;
    componentType?: string;
    code: string;
    message: string;
}

interface ComponentPatchResult {
    applied: Array<{componentId: string; componentType: string}>;
    failed: ComponentPatchFailure[];
    baseRevision?: string;
    resultRevision?: string;
}

/** 对已有组件执行一批同时包含 style/data 的原子配置调整。 */
export async function runApplyComponentPatchTool(payload: unknown): Promise<ComponentPatchResult> {
    requireApprovalTicketForHighRiskPatch(payload);
    return runIdempotentCanvasOperation(payload, () => applyComponentPatch(payload), (result) => result.failed.length === 0);
}

function requireApprovalTicketForHighRiskPatch(payload: unknown): void {
    if (!isRecord(payload) || !Array.isArray(payload.patches))
        return;
    const requiresApproval = payload.patches.some((patch) => isRecord(patch) && (
        patch.dataPatch !== undefined
        || patch.layerPatch !== undefined
        || patch.filterPatch !== undefined
        || (isRecord(patch.stylePatch) && isRecord(patch.stylePatch.style)
            && Object.keys(patch.stylePatch.style).some((key) => COMPONENT_DATA_FIELD_MAPPING_KEYS.includes(
                key as (typeof COMPONENT_DATA_FIELD_MAPPING_KEYS)[number],
            )))
    ));
    if (!requiresApproval)
        return;
    const ticket = payload.approvalTicket;
    if (!isRecord(ticket) || !text(ticket.approvalId) || !text(ticket.nonce))
        throw new FrontendEffectClientError("approval_ticket_missing", "高风险组件修改必须携带服务端签发的审批票据");
}

async function applyComponentPatch(payload: unknown): Promise<ComponentPatchResult> {
    const {patches, expectedSnapshots} = normalizePayload(payload);
    const baseRevision = canvasRevision(expectedSnapshots);
    const layerManager = window.DATAVISOOL_ENV?.currentManager?.layerManager;
    const expectedById = new Map(expectedSnapshots.map((snapshot) => [snapshot.componentId, snapshot]));
    const prepared: Array<{
        patch: ComponentPatchItem;
        controller: any;
        previousStyle: unknown;
        previousData: unknown;
        previousLayer: any;
        previousFilter: unknown;
        nextStyle?: unknown;
        nextData?: Awaited<ReturnType<typeof normalizeDataConfigAsync>>;
        nextLayer?: any;
        nextFilter?: Record<string, unknown>;
    }> = [];
    const failures: ComponentPatchFailure[] = [];

    patches.forEach((patch) => {
        const controller = layerManager?.controllers?.[patch.componentId];
        const layer = layerManager?.layerConfigs?.[patch.componentId];
        const currentConfig = controller?.getConfig?.();
        const currentType = currentConfig?.base?.type ?? layer?.type;
        if (!controller) {
            failures.push(failure(patch, "controller_not_found", `未找到组件控制器：${patch.componentId}`));
            return;
        }
        if (currentType !== patch.componentType) {
            failures.push(failure(
                patch,
                "component_type_mismatch",
                `组件类型不匹配：期望 ${patch.componentType}，实际 ${currentType || "unknown"}`,
            ));
            return;
        }
        const expected = expectedById.get(patch.componentId)!;
        if ((patch.stylePatch && !matchesSnapshot(currentConfig?.style ?? null, expected.style))
            || (patch.dataPatch && !matchesSnapshot(currentConfig?.data ?? null, expected.data))
            || (patch.layerPatch && !matchesSnapshot(layer ?? null, expected.layer))
            || (patch.filterPatch && !matchesSnapshot(currentConfig?.filter ?? null, expected.filter))) {
            failures.push(failure(patch, "component_revision_conflict", `组件读取后已发生变化：${patch.componentId}`));
            return;
        }
        const currentData = isRecord(currentConfig?.data) ? currentConfig.data : {};
        if (patch.dataPatch) {
            const userInput = getDataPatchUserInputRequirement(currentData, patch.dataPatch);
            if (userInput.required) {
                failures.push(failure(patch, "requires_user_input", userInput.message ?? "数据源配置缺少用户需要补充的信息"));
                return;
            }
        }
        prepared.push({
            patch,
            controller,
            previousStyle: cloneDeep(currentConfig?.style),
            previousData: cloneDeep(currentConfig?.data),
            previousLayer: cloneDeep(layer),
            previousFilter: cloneDeep(currentConfig?.filter),
            nextStyle: patch.stylePatch
                ? merge({}, cloneDeep(currentConfig?.style ?? {}), cloneDeep(patch.stylePatch.style))
                : undefined,
            nextData: undefined,
            nextLayer: patch.layerPatch
                ? merge({}, cloneDeep(layer ?? {}), cloneDeep(patch.layerPatch), {id: patch.componentId})
                : undefined,
            nextFilter: patch.filterPatch
                ? merge({}, defaultFilter(), cloneDeep(currentConfig?.filter ?? {}), cloneDeep(patch.filterPatch))
                : undefined,
        });
    });

    if (failures.length > 0)
        return {...rejectBatch(patches, failures), baseRevision, resultRevision: baseRevision};

    for (const item of prepared) {
        if (item.patch.dataPatch)
            item.nextData = await normalizeDataConfigAsync(isRecord(item.previousData) ? item.previousData : {}, item.patch.dataPatch);
    }
    const mappingFailures = prepared.flatMap((item) => {
        const stylePatch = item.patch.stylePatch?.style;
        const mappingChanged = stylePatch
            && COMPONENT_DATA_FIELD_MAPPING_KEYS.some((key) => Object.prototype.hasOwnProperty.call(stylePatch, key));
        const staticDataChanged = item.patch.dataPatch?.sourceType === "static";
        if (!mappingChanged && !staticDataChanged)
            return [];
        if (item.patch.dataPatch && !staticDataChanged)
            return [];
        const finalStyle = item.nextStyle ?? item.previousStyle;
        const finalData = item.nextData ?? item.previousData;
        const issues = validateComponentFieldMappings(finalStyle, isRecord(finalData) ? finalData.value : undefined);
        return issues.length > 0
            ? [failure(item.patch, issues[0].code, issues[0].message)]
            : [];
    });
    if (mappingFailures.length > 0)
        return {...rejectBatch(patches, mappingFailures), baseRevision, resultRevision: baseRevision};
    if (window.DATAVISOOL_ENV?.currentManager?.layerManager !== layerManager)
        throw new FrontendEffectClientError("agent_scope_changed_before_apply", "数据归一化期间画布已切换，整批补丁未应用");
    const conflicts = prepared.filter(({patch, controller}) => {
        const currentConfig = controller.getConfig?.();
        const currentLayer = layerManager?.layerConfigs?.[patch.componentId];
        const expected = expectedById.get(patch.componentId)!;
        return (patch.stylePatch && !matchesSnapshot(currentConfig?.style ?? null, expected.style))
            || (patch.dataPatch && !matchesSnapshot(currentConfig?.data ?? null, expected.data))
            || (patch.layerPatch && !matchesSnapshot(currentLayer ?? null, expected.layer))
            || (patch.filterPatch && !matchesSnapshot(currentConfig?.filter ?? null, expected.filter));
    });
    if (conflicts.length > 0) {
        return {
            ...rejectBatch(patches, conflicts.map(({patch}) =>
                failure(patch, "component_revision_conflict", `组件读取后已发生变化：${patch.componentId}`))),
            baseRevision,
            resultRevision: baseRevision,
        };
    }

    const previousLayerData = layerManager?.getData?.();
    const started: typeof prepared = [];
    try {
        prepared.forEach((item) => {
            const {patch, controller, nextStyle, nextData, nextLayer, nextFilter} = item;
            started.push(item);
            const update: Record<string, unknown> = {};
            if (nextStyle !== undefined)
                update.style = nextStyle;
            if (nextData !== undefined)
                update.data = nextData;
            if (nextFilter !== undefined)
                update.filter = nextFilter;
            if (nextLayer !== undefined)
                layerManager?.updateLayer?.([nextLayer], true);
            controller.update(update, {reRender: true});
            if (nextFilter !== undefined)
                controller.updateFilter?.(nextFilter);
            if (nextData !== undefined)
                syncGlobalVariableBinding(patch.componentId, nextData);
            const actual = controller.getConfig?.();
            const actualLayer = layerManager?.layerConfigs?.[patch.componentId];
            if ((patch.stylePatch && !containsPatch(actual?.style, patch.stylePatch.style))
                || (patch.dataPatch && !containsPatch(actual?.data, patch.dataPatch))
                || (patch.layerPatch && !containsPatch(actualLayer, patch.layerPatch))
                || (patch.filterPatch && !containsPatch(actual?.filter, patch.filterPatch))) {
                throw new Error(`组件补丁未实际生效：${patch.componentId}`);
            }
        });
    } catch (error) {
        const rollbackFailures: string[] = [];
        [...started].reverse().forEach(({patch, controller, previousStyle, previousData, previousLayer, previousFilter}) => {
            const current = controller.getConfig?.();
            const restoreStyle = Boolean(patch.stylePatch) && !isEqual(current?.style, previousStyle);
            const restoreData = Boolean(patch.dataPatch) && !isEqual(current?.data, previousData);
            const restoreLayer = Boolean(patch.layerPatch) && !isEqual(layerManager?.layerConfigs?.[patch.componentId], previousLayer);
            const restoreFilter = Boolean(patch.filterPatch) && !isEqual(current?.filter, previousFilter);
            if (!restoreStyle && !restoreData && !restoreLayer && !restoreFilter)
                return;
            try {
                const rollback: Record<string, unknown> = {};
                if (restoreStyle)
                    rollback.style = cloneDeep(previousStyle);
                if (restoreData)
                    rollback.data = cloneDeep(previousData);
                if (restoreFilter)
                    rollback.filter = cloneDeep(previousFilter);
                if (restoreLayer)
                    layerManager?.updateLayer?.([cloneDeep(previousLayer)], true);
                controller.update(rollback, {reRender: true});
                if (restoreFilter)
                    controller.updateFilter?.(cloneDeep(previousFilter));
                if (restoreData)
                    syncGlobalVariableBinding(patch.componentId, previousData);
                const restored = controller.getConfig?.();
                if ((restoreStyle && !isEqual(restored?.style, previousStyle))
                    || (restoreData && !isEqual(restored?.data, previousData))
                    || (restoreLayer && !isEqual(layerManager?.layerConfigs?.[patch.componentId], previousLayer))
                    || (restoreFilter && !isEqual(restored?.filter, previousFilter))) {
                    throw new Error("rollback verification failed");
                }
            } catch {
                rollbackFailures.push(patch.componentId);
            }
        });
        if (previousLayerData && layerManager?.init) {
            try {
                layerManager.init(cloneDeep(previousLayerData));
            } catch {
                rollbackFailures.push("layerManager");
            }
        }
        const message = error instanceof Error ? error.message : String(error);
        const rollbackComplete = rollbackFailures.length === 0;
        return {
            applied: [],
            failed: patches.map((patch) => failure(
                patch,
                rollbackComplete ? "batch_apply_rolled_back" : "batch_rollback_failed",
                rollbackComplete
                    ? `组件补丁整批应用失败并已回滚：${message}`
                    : `组件补丁应用失败，且以下目标未能完整回滚：${rollbackFailures.join("、")}；${message}`,
            )),
            baseRevision,
            resultRevision: baseRevision,
        };
    }

    const actualSnapshots = patches.map((patch) => {
        const config = layerManager?.controllers?.[patch.componentId]?.getConfig?.();
        return {
            componentId: patch.componentId,
            componentType: patch.componentType,
            style: patch.stylePatch ? cloneDeep(config?.style) : undefined,
            data: patch.dataPatch ? cloneDeep(config?.data) : undefined,
            layer: patch.layerPatch ? cloneDeep(layerManager?.layerConfigs?.[patch.componentId]) : undefined,
            filter: patch.filterPatch ? cloneDeep(config?.filter) : undefined,
        };
    });
    return {
        applied: patches.map((patch) => ({componentId: patch.componentId, componentType: patch.componentType})),
        failed: [],
        baseRevision,
        resultRevision: canvasRevision(actualSnapshots),
    };
}

function syncGlobalVariableBinding(componentId: string, data: any): void {
    if (!window.DATAVISOOL_ENV?.currentManager?.globalVariableManager)
        return;
    if (data?.sourceType === "globalVariable") {
        const gvId = isRecord(data.globalVariable) && typeof data.globalVariable.gvId === "string"
            ? data.globalVariable.gvId
            : "";
        componentDataGvParser.directBindByCompIdAndGvId(componentId, gvId);
        return;
    }
    componentDataGvParser.bindConfigToGvById([componentId]);
}

function normalizePayload(payload: unknown): ApplyComponentPatchPayload {
    if (!isRecord(payload))
        throw new FrontendEffectClientError("invalid_payload", "组件补丁入参必须是对象");
    if (!Array.isArray(payload.patches) || payload.patches.length === 0)
        throw new FrontendEffectClientError("invalid_payload", "组件补丁列表不能为空");
    const patches = payload.patches.map(normalizePatch);
    if (!Array.isArray(payload.expectedSnapshots) || payload.expectedSnapshots.length !== patches.length)
        throw new FrontendEffectClientError("invalid_payload", "expectedSnapshots 必须与组件补丁一一对应");
    const expectedSnapshots = payload.expectedSnapshots.map((value, index) => normalizeExpectedSnapshot(value, patches[index], index));
    const targetIds = new Set(patches.map((patch) => patch.componentId));
    const expectedIds = new Set(expectedSnapshots.map((snapshot) => snapshot.componentId));
    if (targetIds.size !== patches.length || expectedIds.size !== expectedSnapshots.length
        || [...targetIds].some((componentId) => !expectedIds.has(componentId))) {
        throw new FrontendEffectClientError("invalid_payload", "组件补丁目标和 expectedSnapshots 必须唯一且一致");
    }
    return {patches, expectedSnapshots};
}

function normalizePatch(value: unknown, index: number): ComponentPatchItem {
    if (!isRecord(value))
        throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个组件补丁必须是对象`);
    const componentId = text(value.componentId);
    const componentType = text(value.componentType);
    if (!componentId || !componentType)
        throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个组件补丁缺少 componentId 或 componentType`);
    const stylePatch = value.stylePatch === undefined ? undefined : normalizeStylePatch(value.stylePatch, index);
    const dataPatch = value.dataPatch === undefined ? undefined : normalizeDataPatch(value.dataPatch, index);
    const layerPatch = value.layerPatch === undefined ? undefined : normalizeLayerPatch(value.layerPatch, index);
    const filterPatch = value.filterPatch === undefined ? undefined : normalizeFilterPatch(value.filterPatch, index);
    if (!stylePatch && !dataPatch && !layerPatch && !filterPatch)
        throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个组件补丁不能为空`);
    return {componentId, componentType, stylePatch, dataPatch, layerPatch, filterPatch, reason: text(value.reason) || undefined};
}

function normalizeStylePatch(value: unknown, index: number): {style: Record<string, unknown>} {
    if (!isRecord(value) || Object.keys(value).length !== 1 || !isRecord(value.style) || Object.keys(value.style).length === 0)
        throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个 stylePatch 必须只包含非空 style`);
    return {style: value.style};
}

function normalizeDataPatch(value: unknown, index: number): Record<string, unknown> {
    if (!isRecord(value) || Object.keys(value).length === 0)
        throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个 dataPatch 必须是非空对象`);
    const sensitivePath = findSensitiveFieldInPatch(value);
    if (sensitivePath)
        throw new FrontendEffectClientError("sensitive_field_rejected", `组件补丁不能包含敏感字段：${sensitivePath}`);
    const invalidExpressionPath = findInvalidExpressionInPatch(value);
    if (invalidExpressionPath)
        throw new FrontendEffectClientError("invalid_expression_rejected", `组件补丁不能包含函数、表达式或非法值：${invalidExpressionPath}`);
    return value;
}

function normalizeLayerPatch(value: unknown, index: number): NonNullable<ComponentPatchItem["layerPatch"]> {
    if (!isRecord(value) || Object.keys(value).length === 0)
        throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个 layerPatch 必须是非空对象`);
    const allowed = new Set(["x", "y", "width", "height", "name"]);
    if (Object.keys(value).some((key) => !allowed.has(key)))
        throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个 layerPatch 包含未开放字段`);
    const result: NonNullable<ComponentPatchItem["layerPatch"]> = {};
    for (const key of ["x", "y", "width", "height"] as const) {
        if (value[key] === undefined)
            continue;
        const number = value[key];
        if (typeof number !== "number" || !Number.isFinite(number) || number < 0 || ((key === "width" || key === "height") && number === 0))
            throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个 layerPatch.${key} 无效`);
        result[key] = number;
    }
    if (value.name !== undefined) {
        const name = text(value.name);
        if (!name || name.length > 200)
            throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个 layerPatch.name 无效`);
        result.name = name;
    }
    return result;
}

function normalizeFilterPatch(value: unknown, index: number): NonNullable<ComponentPatchItem["filterPatch"]> {
    if (!isRecord(value) || Object.keys(value).length === 0)
        throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个 filterPatch 必须是非空对象`);
    const ranges: Record<string, [number, number]> = {
        blur: [0, 100], brightness: [0, 10], contrast: [0, 10], opacity: [0, 1], saturate: [0, 10], hueRotate: [-360, 360],
    };
    const allowed = new Set(["enable", ...Object.keys(ranges)]);
    if (Object.keys(value).some((key) => !allowed.has(key)) || (value.enable !== undefined && typeof value.enable !== "boolean"))
        throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个 filterPatch 包含无效字段`);
    Object.entries(ranges).forEach(([key, [min, max]]) => {
        const number = value[key];
        if (number !== undefined && (typeof number !== "number" || !Number.isFinite(number) || number < min || number > max))
            throw new FrontendEffectClientError("invalid_patch", `第 ${index + 1} 个 filterPatch.${key} 超出范围`);
    });
    return value as NonNullable<ComponentPatchItem["filterPatch"]>;
}

function normalizeExpectedSnapshot(value: unknown, patch: ComponentPatchItem, index: number): ExpectedComponentSnapshot {
    if (!isRecord(value) || text(value.componentId) !== patch.componentId || text(value.componentType) !== patch.componentType)
        throw new FrontendEffectClientError("invalid_payload", `第 ${index + 1} 个组件基线身份不匹配`);
    if ((patch.stylePatch && !Object.prototype.hasOwnProperty.call(value, "style"))
        || (patch.dataPatch && !Object.prototype.hasOwnProperty.call(value, "data"))
        || (patch.layerPatch && !Object.prototype.hasOwnProperty.call(value, "layer"))
        || (patch.filterPatch && !Object.prototype.hasOwnProperty.call(value, "filter"))) {
        throw new FrontendEffectClientError("invalid_payload", `第 ${index + 1} 个组件基线字段不完整`);
    }
    return {
        componentId: patch.componentId,
        componentType: patch.componentType,
        style: cloneDeep(value.style),
        data: cloneDeep(value.data),
        layer: cloneDeep(value.layer),
        filter: cloneDeep(value.filter),
    };
}

function rejectBatch(patches: ComponentPatchItem[], failures: ComponentPatchFailure[]): ComponentPatchResult {
    const failureById = new Map(failures.map((item) => [item.componentId, item]));
    return {
        applied: [],
        failed: patches.map((patch) => failureById.get(patch.componentId)
            ?? failure(patch, "batch_preflight_aborted", "同批次存在无效组件补丁，整批未应用")),
    };
}

function failure(patch: ComponentPatchItem, code: string, message: string): ComponentPatchFailure {
    return {componentId: patch.componentId, componentType: patch.componentType, code, message};
}

function containsPatch(actual: unknown, patch: unknown): boolean {
    if (!isRecord(patch))
        return isEqual(actual, patch);
    if (!isRecord(actual))
        return false;
    return Object.entries(patch).every(([key, value]) => containsPatch(actual[key], value));
}

function matchesSnapshot(actual: unknown, expected: unknown): boolean {
    return isEqual(toSerializableSnapshot(actual), toSerializableSnapshot(expected));
}

function defaultFilter(): Record<string, unknown> {
    return {enable: false, blur: 0, brightness: 1, contrast: 1, opacity: 1, saturate: 1, hueRotate: 0};
}

function text(value: unknown): string {
    return typeof value === "string" ? value.trim() : "";
}

function isRecord(value: unknown): value is Record<string, any> {
    return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}
