/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {cloneDeep} from "lodash";
import {action, makeObservable, observable} from "mobx";
import {ObjectUtil} from "@datavisool/sdk/utils";
import type {ILayerItem, ILayerManager} from "@datavisool/sdk/types";
import {DesignerMode, DesignerScene} from "@datavisool/sdk/types";
import DesignerManager from "@/designer/manager/DesignerManager.ts";
import managerStore from "@/designer/manager/ManagerStore.ts";
import historyRecordOperateProxy from "@/designer/operate-provider/undo-redo/HistoryRecordOperateProxy.ts";
import {normalizeDataConfig} from "@/util/DataSourceConfigUtil.ts";

/** 通用草稿组件描述，组件类型必须已经注册到设计器。 */
export interface AiLayerDraftComponent {
    id?: string;
    type: string;
    name?: string;
    x: number;
    y: number;
    width: number;
    height: number;
    order?: number;
    config?: Record<string, unknown>;
}

/** 单个草稿批次在画布上的覆盖范围。 */
export interface AiLayerDraftBounds {
    x: number;
    y: number;
    width: number;
    height: number;
}

/** 一次 renderDraft 调用生成的独立草稿批次。 */
export interface AiLayerDraftBatch {
    batchId: string;
    sequence: number;
    layerCount: number;
    bounds: AiLayerDraftBounds;
    componentLayerIds: Record<string, string>;
    manager: DesignerManager;
}

/** 当前挂载到画布上的全部临时草稿批次。 */
export interface AiLayerActiveDraft {
    draftId: string;
    revision: number;
    layerCount: number;
    interactive: boolean;
    canvasWidth: number;
    canvasHeight: number;
    batches: AiLayerDraftBatch[];
}

/** 草稿渲染结果。 */
export interface AiLayerDraftRenderResult {
    status: "draft-rendered";
    draftId: string;
    layerCount: number;
}

export interface AiLayerDraftAcceptResult {
    appliedLayerIds: string[];
    componentBindings: Record<string, string>;
}

interface DraftLayerData {
    layerManager: ILayerManager;
    layerCount: number;
    componentLayerIds: Record<string, string>;
}

interface AiLayerDraftBatchSource {
    batchId: string;
    sequence: number;
    components: AiLayerDraftComponent[];
}

const createDraftId = (): string => (
    `ai-draft-${Date.now()}-${Math.random().toString(16).slice(2)}`
);

const normalizeIdPart = (value: string): string => (
    value.replace(/[^A-Za-z0-9_-]/g, "-") || "component"
);

const normalizeRecord = (value: unknown): Record<string, any> | null => {
    if (!value || typeof value !== "object" || Array.isArray(value))
        return null;
    return value as Record<string, any>;
};

const buildDraftBounds = (components: AiLayerDraftComponent[]): AiLayerDraftBounds => {
    const left = Math.min(...components.map((component) => component.x));
    const top = Math.min(...components.map((component) => component.y));
    const right = Math.max(...components.map((component) => component.x + component.width));
    const bottom = Math.max(...components.map((component) => component.y + component.height));
    return {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    };
};

const currentCanvasSize = (manager: any): {width: number; height: number} => {
    const width = Number(manager?.canvasManager?.canvasConfig?.width);
    const height = Number(manager?.canvasManager?.canvasConfig?.height);
    if (!Number.isFinite(width) || width <= 0 || !Number.isFinite(height) || height <= 0)
        throw new Error("当前画布尺寸无效");
    return {width, height};
};

/**
 * 使用组件默认配置构造草稿配置，传入 config 仅作为补丁。
 * 这样所有已经注册 Definition 的组件都能沿用正式画布的创建流程。
 */
const buildComponentConfig = (
    component: AiLayerDraftComponent,
    layerId: string,
): Record<string, any> => {
    const definition = window.DATAVISOOL_ENV?.definitions?.[component.type];
    if (!definition)
        throw new Error(`未找到组件定义：${component.type}`);

    const defaultConfig = normalizeRecord(definition.getInitConfig?.());
    const config = defaultConfig ? cloneDeep(defaultConfig) : {};
    const defaultData = normalizeRecord(config.data);
    if (component.config) {
        ObjectUtil.merge(config, cloneDeep(component.config));
        const dataPatch = normalizeRecord(component.config.data);
        if (dataPatch)
            config.data = normalizeDataConfig(defaultData, dataPatch);
    }

    const base = normalizeRecord(config.base) ?? {};
    config.base = {
        ...base,
        id: layerId,
        type: component.type,
        name: component.name?.trim() || base.name || component.type,
    };
    return config;
};

/** 按 order 建立与正式图层一致的双向链表。 */
const buildDraftLayerData = (
    draftId: string,
    components: AiLayerDraftComponent[],
): DraftLayerData => {
    const orderedComponents = components
        .map((component, index) => ({component, index}))
        .sort((previous, next) => {
            const previousOrder = previous.component.order ?? previous.index + 1;
            const nextOrder = next.component.order ?? next.index + 1;
            return previousOrder - nextOrder || previous.index - next.index;
        });
    const layerConfigs: Record<string, ILayerItem> = {};
    const elemConfigs: Record<string, any> = {};
    const componentLayerIds: Record<string, string> = {};
    let layerHeader: string | undefined;
    let layerTail: string | undefined;

    orderedComponents.forEach(({component, index}) => {
        const sourceId = component.id?.trim() || `component-${index + 1}`;
        const layerId = `${draftId}-${index + 1}-${normalizeIdPart(sourceId)}`;
        const layer: ILayerItem = {
            id: layerId,
            type: component.type,
            name: component.name?.trim() || component.type,
            x: component.x,
            y: component.y,
            width: component.width,
            height: component.height,
            order: component.order ?? index + 1,
        };

        if (layerHeader) {
            layer.next = layerHeader;
            layerConfigs[layerHeader].prev = layerId;
        } else {
            layerTail = layerId;
        }
        layerHeader = layerId;
        layerConfigs[layerId] = layer;
        elemConfigs[layerId] = buildComponentConfig(component, layerId);
        componentLayerIds[sourceId] = layerId;
    });

    return {
        layerCount: orderedComponents.length,
        layerManager: {
            layerConfigs,
            elemConfigs,
            layerHeader,
            layerTail,
        },
        componentLayerIds,
    };
};

/**
 * 管理画布上的多批次临时草稿。
 * 每个待处理批次使用独立 DesignerManager，接受前不会写入正式画布。
 */
export class AiLayerDraftRuntime {
    public activeDraft: AiLayerActiveDraft | null = null;
    private activeBatchSources: AiLayerDraftBatchSource[] = [];
    private draftSessionId: string | null = null;
    private sessionRevision = 0;
    private nextBatchSequence = 1;

    constructor() {
        makeObservable(this, {
            activeDraft: observable.ref,
            render: action,
            acceptDraft: action,
            acceptBatch: action,
            discardBatch: action,
            clear: action,
        });
    }

    public render(components: AiLayerDraftComponent[], incremental = false, interactive = true): AiLayerDraftRenderResult {
        const currentManager = managerStore.currentManager ?? window.DATAVISOOL_ENV?.currentManager;
        if (!currentManager)
            throw new Error("当前设计器 Manager 不可用");
        const {width: canvasWidth, height: canvasHeight} = currentCanvasSize(currentManager);

        if (!incremental || !this.draftSessionId) {
            this.draftSessionId = createDraftId();
            this.sessionRevision = 0;
            this.nextBatchSequence = 1;
        }
        const previousDraft = incremental ? this.activeDraft : null;
        const draftId = this.draftSessionId;
        const revision = ++this.sessionRevision;
        const nextSequence = this.nextBatchSequence++;
        const incomingComponents = cloneDeep(components);
        const incomingIds = new Set(incomingComponents.map((component) => component.id?.trim()).filter(Boolean));
        const retainedSources = previousDraft
            ? this.activeBatchSources
                .map((batch) => ({
                    ...batch,
                    components: batch.components.filter((component) => !incomingIds.has(component.id?.trim())),
                }))
                .filter((batch) => batch.components.length > 0)
            : [];
        const nextSources = [
            ...retainedSources,
            {
                batchId: `${draftId}-batch-${nextSequence}`,
                sequence: nextSequence,
                components: incomingComponents,
            },
        ];
        if (nextSources.some((batch) => batch.components.some((component) => component.x < 0
            || component.y < 0
            || component.x + component.width > canvasWidth
            || component.y + component.height > canvasHeight))) {
            throw new Error("草稿组件超出当前画布边界");
        }
        const batches = nextSources.map((batch) => this.buildBatch(batch, currentManager));

        this.destroyActiveManagers();
        this.activeBatchSources = nextSources;
        this.activeDraft = {
            draftId,
            revision,
            layerCount: batches.reduce((count, batch) => count + batch.layerCount, 0),
            interactive,
            canvasWidth,
            canvasHeight,
            batches,
        };

        return {
            status: "draft-rendered",
            draftId,
            layerCount: this.activeDraft.layerCount,
        };
    }

    /** 将指定的整份草稿写入真实画布。 */
    public acceptDraft(
        draftId: string,
        requestedComponentIds?: string[],
        reservedComponentIds?: Record<string, string>,
    ): AiLayerDraftAcceptResult | null {
        if (!this.activeDraft || this.activeDraft.draftId !== draftId)
            return null;
        const currentManager = window.DATAVISOOL_ENV?.currentManager;
        if (!currentManager)
            return null;
        const currentCanvas = currentCanvasSize(currentManager);
        if (currentCanvas.width !== this.activeDraft.canvasWidth
            || currentCanvas.height !== this.activeDraft.canvasHeight)
            return null;
        const layers: Record<string, ILayerItem> = {};
        const configs: Record<string, any> = {};
        this.activeDraft.batches
            .slice()
            .sort((left, right) => left.sequence - right.sequence)
            .forEach((batch) => {
                const data = batch.manager.layerManager.getData();
                Object.assign(layers, cloneDeep(data.layerConfigs ?? {}));
                Object.assign(configs, cloneDeep(data.elemConfigs ?? {}));
            });
        if (Object.keys(layers).length !== this.activeDraft.layerCount
            || Object.keys(configs).length !== this.activeDraft.layerCount)
            return null;

        const targetLayerManager = window.DATAVISOOL_ENV?.currentManager?.layerManager;
        const existingComponentIds = new Set(Object.keys(targetLayerManager?.layerConfigs ?? {}));
        const componentLayerEntries = this.activeDraft.batches.flatMap((batch) => Object.entries(batch.componentLayerIds));
        let reservedLayerIds: Record<string, string> | undefined;
        if (reservedComponentIds) {
            const reservedValueFor = (componentId: string): string => (
                typeof reservedComponentIds[componentId] === "string" ? reservedComponentIds[componentId].trim() : ""
            );
            const reservedValues = componentLayerEntries.map(([componentId]) => reservedValueFor(componentId));
            const reservedKeys = Object.keys(reservedComponentIds);
            if (reservedKeys.length !== componentLayerEntries.length
                || reservedValues.some((id) => !id)
                || new Set(reservedValues).size !== reservedValues.length
                || reservedValues.some((id) => !id || existingComponentIds.has(id))) {
                return null;
            }
            reservedLayerIds = Object.fromEntries(componentLayerEntries.map(([componentId, layerId]) => [
                layerId,
                reservedValueFor(componentId),
            ]));
        }
        const previousLayerData = targetLayerManager?.getData?.();
        try {
            const result = historyRecordOperateProxy.doUseTemplate({layers, configs}, reservedLayerIds);
            if (result.appliedLayerIds.length !== this.activeDraft.layerCount) {
                previousLayerData && targetLayerManager?.init?.(cloneDeep(previousLayerData));
                return null;
            }
            const componentBindings = this.activeDraft.batches.reduce((bindings, batch) => {
                Object.entries(batch.componentLayerIds).forEach(([componentId, layerId]) => {
                    const canvasId = result.layerIdMap[layerId];
                    if (canvasId)
                        bindings[componentId] = canvasId;
                });
                return bindings;
            }, {} as Record<string, string>);
            const bindingKeys = Object.keys(componentBindings);
            const bindingValues = Object.values(componentBindings);
            const bindingKeySet = new Set(bindingKeys);
            const bindingValueSet = new Set(bindingValues);
            const appliedLayerIds = result.appliedLayerIds.filter((id): id is string => typeof id === "string" && id.trim().length > 0);
            const expectedIds = requestedComponentIds?.map((id) => id.trim()).filter(Boolean);
            if (Object.keys(componentBindings).length !== this.activeDraft.layerCount
                || bindingValues.some((id) => !id.trim())
                || bindingValueSet.size !== bindingValues.length
                || appliedLayerIds.length !== result.appliedLayerIds.length
                || new Set(appliedLayerIds).size !== appliedLayerIds.length
                || appliedLayerIds.some((id) => existingComponentIds.has(id))
                || bindingValueSet.size !== appliedLayerIds.length
                || !appliedLayerIds.every((id) => bindingValueSet.has(id))
                || (reservedComponentIds && (
                    bindingValueSet.size !== Object.keys(reservedComponentIds).length
                    || !bindingValues.every((id) => Object.values(reservedComponentIds).includes(id))
                    || !appliedLayerIds.every((id) => Object.values(reservedComponentIds).includes(id))
                ))
                || (expectedIds && (new Set(expectedIds).size !== expectedIds.length
                    || expectedIds.length !== bindingKeys.length
                    || !expectedIds.every((id) => bindingKeySet.has(id))))) {
                previousLayerData && targetLayerManager?.init?.(cloneDeep(previousLayerData));
                return null;
            }
            this.clear();
            return {appliedLayerIds, componentBindings};
        } catch (error) {
            previousLayerData && targetLayerManager?.init?.(cloneDeep(previousLayerData));
            throw error;
        }
    }

    /** 将单个草稿批次作为一次可撤销操作写入真实画布。 */
    public acceptBatch(batchId: string): string[] {
        const batch = this.activeDraft?.batches.find((item) => item.batchId === batchId);
        if (!batch)
            return [];

        const {layerConfigs, elemConfigs} = batch.manager.layerManager.getData();
        const result = historyRecordOperateProxy.doUseTemplate({
            layers: cloneDeep(layerConfigs ?? {}),
            configs: cloneDeep(elemConfigs ?? {}),
        });
        this.removeBatch(batchId);
        return result.appliedLayerIds;
    }

    /** 丢弃单个草稿批次，不影响其他待处理批次。 */
    public discardBatch(batchId: string): boolean {
        if (!this.activeDraft?.batches.some((item) => item.batchId === batchId))
            return false;
        this.removeBatch(batchId);
        return true;
    }

    public clear(): void {
        this.destroyActiveManagers();
        this.activeBatchSources = [];
        this.activeDraft = null;
        this.draftSessionId = null;
        this.sessionRevision = 0;
        this.nextBatchSequence = 1;
    }

    private buildBatch(source: AiLayerDraftBatchSource, currentManager: DesignerManager): AiLayerDraftBatch {
        const draftLayerData = buildDraftLayerData(source.batchId, source.components);
        const manager = new DesignerManager(currentManager.scene ?? DesignerScene.DEFAULT);
        manager.init({
            ...currentManager.getData(),
            layerManager: draftLayerData.layerManager,
        }, window.DATAVISOOL_ENV?.mode ?? DesignerMode.EDIT);
        return {
            batchId: source.batchId,
            sequence: source.sequence,
            layerCount: draftLayerData.layerCount,
            bounds: buildDraftBounds(source.components),
            componentLayerIds: draftLayerData.componentLayerIds,
            manager,
        };
    }

    private removeBatch(batchId: string): void {
        const activeDraft = this.activeDraft;
        if (!activeDraft)
            return;

        const batch = activeDraft.batches.find((item) => item.batchId === batchId);
        this.destroyManager(batch?.manager);
        const batches = activeDraft.batches.filter((item) => item.batchId !== batchId);
        this.activeBatchSources = this.activeBatchSources.filter((item) => item.batchId !== batchId);
        const revision = ++this.sessionRevision;
        this.activeDraft = batches.length === 0
            ? null
            : {
                ...activeDraft,
                revision,
                layerCount: batches.reduce((count, item) => count + item.layerCount, 0),
                batches,
            };
    }

    private destroyActiveManagers(): void {
        this.activeDraft?.batches.forEach((batch) => this.destroyManager(batch.manager));
    }

    private destroyManager(manager?: DesignerManager): void {
        if (!manager)
            return;

        try {
            manager.destroy();
        } catch (error) {
            console.error("AI 草稿 Manager 销毁失败", error);
        }
    }
}

export const aiLayerDraftRuntime = new AiLayerDraftRuntime();
