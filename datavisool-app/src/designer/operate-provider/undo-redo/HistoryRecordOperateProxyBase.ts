/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */


import {IAddOperateData, IDelOperateData, IGraphicalTransform, IHistoryRecord, IUpdLayerOperateData, OperateType} from "./OperateType";
import {historyOperator} from "./HistoryOperator";
import eventOperateStore from "@/designer/operate-provider/EventOperateStore";
import {runInAction} from "mobx";
import rightStore from "@/designer/right/RightStore";
import {cloneDeep} from "lodash";
import {IdGenerate} from "@datavisool/sdk/utils";
import LayerUtil from "@/designer/left/layer-list/util/LayerUtil";
import LayerRepairUtil from "@/designer/left/layer-list/util/LayerRepairUtil";
import {ILayerItem} from "@datavisool/sdk/types";
import componentDataGvParser from "@/designer/left/global-variable/parse/ComponentDataGvParser.ts";
import baseInfoStore from "@/comps/common-component/base-info/BaseInfoStore.ts";
import {normalizeLayerTransform} from "@/util/LayerTransformUtil.ts";

const hasValidReservedLayerIds = (
    layers: Record<string, ILayerItem>,
    reservedLayerIds: Record<string, string>,
): boolean => {
    const sourceIds = Object.values(layers)
        .map((layer) => layer.id?.trim())
        .filter((id): id is string => Boolean(id));
    const reservedIds = sourceIds.map((sourceId) => reservedLayerIds[sourceId]?.trim());
    return sourceIds.length === Object.keys(layers).length
        && new Set(sourceIds).size === sourceIds.length
        && Object.keys(reservedLayerIds).length === sourceIds.length
        && reservedIds.every((id): id is string => Boolean(id))
        && new Set(reservedIds).size === reservedIds.length
        && reservedIds.every((id) => !new Set(sourceIds).has(id));
};

export interface UseTemplateOperateResult {
    appliedLayerIds: string[];
    layerIdMap: Record<string, string>;
}

export class HistoryRecordOperateProxyBase {
    /**
     * 修复根链表的头尾锚点。
     * 某些历史回滚或异常中断后，layerHeader / layerTail 可能还保留旧 id，
     * 但对应图层已经不在 layerConfigs 中了。这里优先保留有效锚点，
     * 无效时再从当前顶层图层里推导出一个可继续插入的头尾节点。
     */
    private resolveValidRootAnchors(layerManager: any, layerConfigs: Record<string, ILayerItem>) {
        const topLevelLayers = Object.values(layerConfigs ?? {}).filter((layer) => !layer?.pid);
        const currentHeaderId = typeof layerManager?.layerHeader === "string" ? layerManager.layerHeader : undefined;
        const currentTailId = typeof layerManager?.layerTail === "string" ? layerManager.layerTail : undefined;

        // 优先使用没有 prev / next 的真实边界节点；如果链表本身已不完整，则回退到 order 推导。
        const fallbackHeaderId = topLevelLayers.find((layer) => !layer?.prev)?.id
            ?? [...topLevelLayers].sort((prev, next) => (next.order ?? 0) - (prev.order ?? 0))[0]?.id;
        const fallbackTailId = topLevelLayers.find((layer) => !layer?.next)?.id
            ?? [...topLevelLayers].sort((prev, next) => (prev.order ?? 0) - (next.order ?? 0))[0]?.id;

        return {
            headerId: currentHeaderId && layerConfigs[currentHeaderId] ? currentHeaderId : fallbackHeaderId,
            tailId: currentTailId && layerConfigs[currentTailId] ? currentTailId : fallbackTailId,
        };
    }

    private snapshotLayerForRepair(layer: ILayerItem): IUpdLayerOperateData {
        return {
            id: layer.id,
            name: layer.name,
            type: layer.type,
            width: layer.width,
            height: layer.height,
            x: layer.x,
            y: layer.y,
            rotate: layer.rotate,
            transform: cloneDeep(layer.transform),
            hide: layer.hide,
            lock: layer.lock,
            order: layer.order,
            pid: layer.pid,
            childHeader: layer.childHeader,
            childTail: layer.childTail,
            prev: layer.prev,
            next: layer.next,
        };
    }

    public doDrag(latestState: ILayerItem[]): void {
        //构建历史记录数据
        const {layerConfigs, updateLayer} = window.DATAVISOOL_ENV.currentManager.layerManager!;

        //1. 记录变更前数据
        const prev = latestState.map((item) => {
            const {id, x = 0, y = 0, rotate = 0} = layerConfigs[item.id!];
            return {id, x, y, rotate}
        })

        //2. 更新布局数据
        updateLayer(latestState, false);

        //3. 记录变更后数据
        const next = latestState.map((item) => {
            const {id, x = 0, y = 0, rotate = 0} = layerConfigs[item.id!];
            return {id, x, y, rotate}
        })

        //4. 构建历史记录节点
        const data: IHistoryRecord = {
            type: OperateType.DRAG,
            prev: prev!,
            next: next!
        }

        //历史记录入队
        historyOperator.put({actions: [data]});
    }

    public doRotate(latestState: ILayerItem[]): void {
        if (!latestState || latestState.length === 0)
            return;
        const {layerConfigs, updateLayer} = window.DATAVISOOL_ENV.currentManager.layerManager!;

        //1. 记录变更前数据
        const prev = latestState.map((item) => {
            const {id, x = 0, y = 0, rotate = 0} = layerConfigs[item.id!];
            return {id: id!, x: x!, y: y!, rotate: rotate!}
        })

        //2. 执行数据变更
        updateLayer(latestState, false);

        //3. 记录变更后数据
        const next = latestState.map((item) => {
            const {id, x = 0, y = 0, rotate = 0} = layerConfigs[item.id!];
            return {id: id!, x: x!, y: y!, rotate: rotate!}
        })

        //4. 构建历史记录节点
        const data: IHistoryRecord = {
            type: OperateType.ROTATE,
            prev: prev!,
            next: next!
        }

        //5. 历史记录入队
        historyOperator.put({actions: [data]});
    }

    public doTransform(latestState: ILayerItem[]): void {
        if (!latestState || latestState.length === 0)
            return;
        const {layerConfigs, updateLayer} = window.DATAVISOOL_ENV.currentManager.layerManager!;

        const resolvedItems = latestState
            .map((item) => {
                const currentLayer = layerConfigs[item.id!];
                if (!currentLayer)
                    return null;
                return {
                    id: item.id!,
                    currentLayer,
                    nextTransform: normalizeLayerTransform({
                        ...(currentLayer.transform ?? {}),
                        ...(item.transform ?? {}),
                    }),
                };
            })
            .filter((item): item is {
                id: string;
                currentLayer: ILayerItem;
                nextTransform: ReturnType<typeof normalizeLayerTransform>;
            } => !!item);

        if (resolvedItems.length === 0)
            return;

        const prev = resolvedItems.map((item) => ({
            id: item.id,
            transform: normalizeLayerTransform(item.currentLayer.transform),
        }));

        updateLayer(resolvedItems.map((item) => ({
            id: item.id,
            transform: item.nextTransform,
        })), true);

        const next = resolvedItems.map((item) => ({
            id: item.id,
            transform: normalizeLayerTransform(layerConfigs[item.id!]?.transform),
        }));

        next.forEach((item) => {
            const currentLayer = layerConfigs[item.id];
            if (currentLayer)
                baseInfoStore.updateBaseConfig(currentLayer);
        });

        const data: IHistoryRecord = {
            type: OperateType.TRANSFORM,
            prev: prev!,
            next: next!
        };

        historyOperator.put({actions: [data]});
    }

    public doResize(latestState: ILayerItem[]): void {
        //构建历史记录数据
        const {layerConfigs, updateLayer} = window.DATAVISOOL_ENV.currentManager.layerManager!;

        //1. 记录变更前数据
        const prev: IGraphicalTransform[] = latestState.map((item) => {
            const {id, x = 0, y = 0, width = 0, height = 0, rotate = 0} = layerConfigs[item.id!];
            return {id: id!, x, y, width, height, rotate}
        })

        //2. 更新布局数据
        updateLayer(latestState, true);

        //3. 记录变更后数据
        const next: IGraphicalTransform[] = latestState.map((item) => {
            const {id, x = 0, y = 0, width = 0, height = 0, rotate = 0} = layerConfigs[item.id!];
            return {id: id!, x, y, width, height, rotate}
        })

        //4. 构建历史记录节点
        const record: IHistoryRecord = {type: OperateType.RESIZE, prev: prev!, next: next!}

        //5. 历史记录入队
        historyOperator.put({actions: [record]});
    }

    public doAdd(layer: ILayerItem): void {
        const addPrev: IAddOperateData[] = [];
        const addNext: IAddOperateData[] = [];
        const updLayerPrev: IUpdLayerOperateData[] = [];
        const updLayerNext: IUpdLayerOperateData[] = [];
        const {layerManager} = window.DATAVISOOL_ENV.currentManager!;
        const {layerConfigs, updateLayer} = layerManager!;
        const {headerId: currentHeaderId, tailId: currentTailId} = this.resolveValidRootAnchors(layerManager, layerConfigs);

        //头插法建立双向链表
        if (!currentHeaderId) {
            //首次插入
            addPrev.push({layerHeader: currentHeaderId, layerTail: currentTailId})
            layerManager!.layerHeader = layer.id;
            layerManager!.layerTail = layer.id;
        } else {
            //非首次插入
            addPrev.push({layerHeader: currentHeaderId, layerTail: currentTailId})
            const oldHeaderLayer = layerConfigs[currentHeaderId];
            updLayerPrev.push({id: oldHeaderLayer.id, prev: oldHeaderLayer.prev})
            oldHeaderLayer.prev = layer.id;
            layer.next = oldHeaderLayer.id;
            layerManager.layerHeader = layer.id;
            layerManager.layerTail = currentTailId ?? oldHeaderLayer.id;
            updLayerNext.push({id: oldHeaderLayer.id, prev: layer.id})
        }
        addNext.push({
            id: layer.id,
            layerConfig: cloneDeep(layer),
            layerHeader: layerManager?.layerHeader,
            layerTail: layerManager?.layerTail
        });

        const {addRecordCompId, setAddRecordCompId} = eventOperateStore;
        /**
         * addRecordCompId为不为null，说明是从组件面板拖拽添加的组件要记录操作日志，反之其他操作导致的组件添加则不记录日志
         */
        if (addRecordCompId && addRecordCompId === layer.id) {
            historyOperator.put({
                actions: [
                    {type: OperateType.ADD, prev: addPrev, next: addNext},
                    {type: OperateType.UPDATE_LAYER, prev: updLayerPrev, next: updLayerNext}
                ]
            });
            setAddRecordCompId(null);
        }
        runInAction(() => {
            layerConfigs[layer.id + ""] = layer;
        })
        updateLayer(updLayerNext);
    }

    public doUseTemplate(
        templateInfo: { configs: Record<string, any>, layers: Record<string, ILayerItem> },
        reservedLayerIds?: Record<string, string>,
    ): UseTemplateOperateResult {
        const {configs, layers: oldLayers} = templateInfo;
        if (!configs || !oldLayers)
            return {
                appliedLayerIds: [],
                layerIdMap: {},
            };
        if (reservedLayerIds && !hasValidReservedLayerIds(oldLayers, reservedLayerIds))
            return {appliedLayerIds: [], layerIdMap: {}};
        const addPrev: IAddOperateData[] = [];
        const addNext: IAddOperateData[] = [];
        const updLayerPrev: IUpdLayerOperateData[] = [];
        const updLayerNext: IUpdLayerOperateData[] = [];

        /** 1. 获取待处理图层列表的分组映射关系 */
        const groupMapping = new Map<string, string[]>();
        const topLevelLayers: Record<string, ILayerItem> = {};
        const layerIds = Object.values(oldLayers).map(layer => layer.id!);
        layerIds.forEach(id => {
            const layer = oldLayers[id];
            if (layer.pid && layerIds.includes(layer.pid)) {
                if (!groupMapping.has(layer.pid))
                    groupMapping.set(layer.pid, [layer.id!]);
                else
                    groupMapping.get(layer.pid)!.push(layer.id!);
            } else {
                topLevelLayers[layer.id!] = layer;
                layer.pid = undefined;
            }
        })

        /** 3. 构建主干链表 */
        const {head, tail} = LayerUtil.buildADoublyLinkedListOfLayers(topLevelLayers);

        /** 4. 构建分支链表 */
        groupMapping.forEach((childIds, pid) => {
            const childLayers = childIds.reduce((acc, id) => {
                acc[id] = oldLayers[id];
                return acc;
            }, {} as Record<string, ILayerItem>);
            const {head, tail} = LayerUtil.buildADoublyLinkedListOfLayers(childLayers);
            const parentLayer = oldLayers[pid];
            parentLayer.childHeader = head!;
            parentLayer.childTail = tail!;
        })

        const newConfigs = cloneDeep(Object.values(configs));
        const newLayers = cloneDeep(Object.values(oldLayers));
        const idNewToOld: Record<string, string> = {};
        const idOldToNew: Record<string, string> = {};

        //设置新id并复位关联关系
        newLayers.forEach((layer) => {
            const oldId = layer.id;
            const newId = reservedLayerIds?.[oldId!]?.trim() || IdGenerate.generateId();
            idNewToOld[newId] = oldId!;
            idOldToNew[oldId!] = newId;
            const config = newConfigs.find(config => config.base['id'] === oldId)
            if (config && config.base)
                config && (config.base['id'] = newId);
            layer.id = newId;
            layer.pid = undefined;
            layer.childHeader = undefined;
            layer.childTail = undefined;
            layer.prev = undefined;
            layer.next = undefined;
        })
        //恢复config
        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        const {elemConfigs, layerConfigs} = layerManager!;
        const {headerId: currentHeaderId, tailId: currentTailId} = this.resolveValidRootAnchors(layerManager, layerConfigs);
        newConfigs.forEach(config => {
            elemConfigs![config.base['id']] = config;
        })

        runInAction(() => {
            //恢复layer
            newLayers.forEach(newLayer => {
                const oldLayer = oldLayers[idNewToOld[newLayer.id!]];
                if (oldLayer.pid && oldLayer.pid in oldLayers)
                    newLayer.pid = idOldToNew[oldLayer.pid];
                if (oldLayer.prev && oldLayer.prev in oldLayers)
                    newLayer.prev = idOldToNew[oldLayer.prev];
                if (oldLayer.next && oldLayer.next in oldLayers)
                    newLayer.next = idOldToNew[oldLayer.next];
                if (newLayer.type === 'group') {
                    if (oldLayer.childTail)
                        newLayer.childTail = idOldToNew[oldLayer.childTail];
                    if (oldLayer.childHeader)
                        newLayer.childHeader = idOldToNew[oldLayer.childHeader];
                }
                layerConfigs[newLayer.id!] = newLayer;
            })

            //获取新队列的头尾指针
            const queueHead = idOldToNew[head!];
            const queueTail = idOldToNew[tail!];
            const queueTailLayer = newLayers.find(layer => layer.id === queueTail)!;

            //头插法建立双向链表
            if (!currentHeaderId) {
                //首次插入
                addPrev.push({layerHeader: currentHeaderId, layerTail: currentTailId})
                layerManager!.layerHeader = queueHead!;
                layerManager!.layerTail = queueTail!;
            } else {
                //非首次插入
                addPrev.push({layerHeader: currentHeaderId, layerTail: currentTailId})
                const oldHeaderLayer = layerConfigs[currentHeaderId];
                updLayerPrev.push({id: oldHeaderLayer.id, prev: oldHeaderLayer.prev})
                oldHeaderLayer.prev = queueTail!;
                queueTailLayer.next = oldHeaderLayer.id;
                layerManager.layerHeader = queueHead!;
                layerManager.layerTail = currentTailId ?? oldHeaderLayer.id;
                updLayerNext.push({id: oldHeaderLayer.id, prev: queueTail!})
            }
        })

        newLayers.forEach(layer => {
            addNext.push({
                id: layer.id,
                layerConfig: cloneDeep(layer),
                elemConfig: elemConfigs![layer.id!],
                layerHeader: layerManager.layerHeader,
                layerTail: layerManager.layerTail
            })
        })

        historyOperator.put({
            actions: [
                {type: OperateType.ADD, prev: addPrev, next: addNext},
                {type: OperateType.UPDATE_LAYER, prev: updLayerPrev, next: updLayerNext}
            ]
        })
        return {
            appliedLayerIds: newLayers
                .map((layer) => layer.id)
                .filter((id): id is string => typeof id === "string" && id.trim().length > 0),
            layerIdMap: idOldToNew,
        };
    }

    public doDelete(targetIds: string[]): void {
        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        const {delItem, layerConfigs, controllers} = layerManager;
        const {setTargetIds} = eventOperateStore;
        if (targetIds.length === 0)
            return;

        const allHideLayer = Object.values(layerConfigs).filter(layer => layer.hide);
        const hideParentSet = new Set();
        allHideLayer.forEach(layer => {
            const parentChain = LayerUtil.findLayerGroupChain(layer.id!)
            parentChain.forEach(id => hideParentSet.add(id))
        })
        targetIds = targetIds.filter(id => !hideParentSet.has(id))

        const {setContentVisible, activeConfig, activeElem} = rightStore;
        //若被删除元素处于配置项的激活状态，则取消激活状态（关闭右侧配置项）
        if (targetIds.includes(activeElem.id!)) {
            setContentVisible(false);
            activeConfig(null, "");
        }

        const delPrev: IDelOperateData[] = [];
        const delNext: IDelOperateData[] = [];
        const updPrev: IUpdLayerOperateData[] = [];
        const updNext: IUpdLayerOperateData[] = [];

        //计算本次操作所涉及到的所有图层id
        const relatedLayerIds: Set<string> = new Set();
        targetIds.forEach((id) => {
            const layer = layerConfigs[id];
            relatedLayerIds.add(id);
            if (layer.next)
                relatedLayerIds.add(layer.next);
            if (layer.prev)
                relatedLayerIds.add(layer.prev);
            if (layer.pid) {
                const parentLayer = layerConfigs[layer.pid];
                if (parentLayer.childHeader === id)
                    relatedLayerIds.add(parentLayer.id!);
                if (parentLayer.childTail === id)
                    relatedLayerIds.add(parentLayer.id!);
            }
        });

        const toBeUpd = Array.from(relatedLayerIds).filter((id) => !targetIds.includes(id));

        //记录删除前状态
        targetIds.forEach((id) => {
            const layer = layerConfigs[id];
            const elemConfig = controllers[id] && controllers[id].getConfig();
            delPrev.push({id, layerConfig: cloneDeep(layer), elemConfig: elemConfig})
        });
        updPrev.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail})
        toBeUpd.forEach((id) => {
            const layer = layerConfigs[id];
            const {next, prev, childHeader, childTail, pid} = layer;
            updPrev.push({id, next, prev, childHeader, childTail, pid});
        })

        //修改数据
        runInAction(() => {
            targetIds.forEach((id) => {
                const layer = layerConfigs[id];
                const prevLayer = layerConfigs[layer.prev!];
                const nextLayer = layerConfigs[layer.next!];

                if (layer.id === layerManager.layerHeader)
                    layerManager.layerHeader = layer.next;
                if (layer.id === layerManager.layerTail)
                    layerManager.layerTail = layer.prev;

                prevLayer && (prevLayer.next = layer?.next);
                nextLayer && (nextLayer.prev = layer?.prev);
                if (layer.pid) {
                    const parentLayer = layerConfigs[layer.pid];
                    if (parentLayer.childHeader === id)
                        parentLayer.childHeader = layer?.next;
                    if (parentLayer.childTail === id)
                        parentLayer.childTail = layer?.prev;
                }
            });
            delItem(targetIds);
        });

        //记录删除后状态
        toBeUpd.forEach((id) => {
            const layer = layerConfigs[id];
            const {next, prev, childHeader, childTail, pid} = layer;
            updNext.push({id, next, prev, childHeader, childTail, pid});
        });
        delNext.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail})

        historyOperator.put({
            actions: [
                {type: OperateType.DEL, prev: delPrev, next: delNext},
                {type: OperateType.UPDATE_LAYER, prev: updPrev, next: updNext}
            ]
        });

        setTargetIds([]);
        //删除组件后，重新聚焦鼠标指针到容器上，避免鼠标失去焦点导致其他快捷键失效。
        eventOperateStore?.focusDesignerCanvas();
    }

    public doRepairOrphansPrepend(orphanIds: string[]): void {
        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        const {layerConfigs, updateLayer} = layerManager;
        const {setTargetIds} = eventOperateStore;
        if (!orphanIds || orphanIds.length === 0)
            return;

        const analysis = LayerRepairUtil.scanCanvasRepairAnalysis(layerManager);
        const orphanIdSet = new Set(analysis.orphanIds);
        const normalizedTargetIds = LayerRepairUtil.sortLayerIdsForRepair(
            Array.from(new Set(orphanIds.filter(id => orphanIdSet.has(id) && !!layerConfigs[id]))),
            layerConfigs
        );
        if (normalizedTargetIds.length === 0)
            return;

        const oldHeaderId = typeof layerManager.layerHeader === "string" && layerConfigs[layerManager.layerHeader]
            ? layerManager.layerHeader
            : undefined;
        const visibleTailId = analysis.renderTailId;

        const prev: IUpdLayerOperateData[] = [
            {layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail}
        ];
        const next: IUpdLayerOperateData[] = [];

        const repairLayerMap: Record<string, ILayerItem> = {};
        normalizedTargetIds.forEach((id) => {
            const layer = layerConfigs[id];
            if (!layer)
                return;
            prev.push(this.snapshotLayerForRepair(layer));
            repairLayerMap[id] = cloneDeep(layer);
        });

        const {head: orphanHeadId, tail: orphanTailId} = LayerRepairUtil.rebuildLinkedListByOrder(repairLayerMap);
        if (oldHeaderId && orphanTailId && repairLayerMap[orphanTailId]) {
            repairLayerMap[orphanTailId].next = oldHeaderId;
        }
        const repairLayers = Object.values(repairLayerMap);
        let headerLayer: ILayerItem | undefined;
        if (oldHeaderId && orphanTailId) {
            headerLayer = cloneDeep(layerConfigs[oldHeaderId]);
            headerLayer.prev = orphanTailId;
            repairLayers.push(headerLayer);
        }

        runInAction(() => {
            updateLayer(repairLayers);
            layerManager.layerHeader = orphanHeadId;
            layerManager.layerTail = visibleTailId ?? orphanTailId;
        });

        next.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail});
        repairLayers.forEach((layer) => {
            if (layer.id)
                next.push(this.snapshotLayerForRepair(layer));
        });

        historyOperator.put({
            actions: [{type: OperateType.UPDATE_LAYER, prev, next}]
        });

        setTargetIds(normalizedTargetIds);
        eventOperateStore.focusDesignerCanvas();
    }

    public doRepairTailPointer(nextTailId: string): void {
        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        if (!nextTailId || layerManager.layerTail === nextTailId)
            return;

        const prev: IUpdLayerOperateData[] = [
            {layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail}
        ];
        const next: IUpdLayerOperateData[] = [
            {layerHeader: layerManager.layerHeader, layerTail: nextTailId}
        ];

        runInAction(() => {
            layerManager.layerTail = nextTailId;
        });

        historyOperator.put({
            actions: [{type: OperateType.UPDATE_LAYER, prev, next}]
        });

        eventOperateStore.focusDesignerCanvas();
    }

    public doReplaceComponent(templateInfo: { configs: Record<string, any>, layers: Record<string, ILayerItem> }, oldId: string): string | null {
        const {configs, layers} = templateInfo ?? {};
        if (!oldId || !configs || !layers) {
            return null;
        }

        const newLayers = Object.values(layers);
        if (newLayers.length !== 1) {
            return null;
        }

        const newLayer = cloneDeep(newLayers[0]);
        if (!newLayer?.id) {
            return null;
        }

        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        const {layerConfigs, controllers, elemConfigs} = layerManager;
        const oldLayer = layerConfigs[oldId];
        if (!oldLayer || oldLayer.type === "group") {
            return null;
        }

        const newId = newLayer.id!;
        const oldController = controllers[oldId];
        const oldConfig = oldController?.getConfig?.() ?? elemConfigs?.[oldId] ?? null;
        const oldHeaderId = layerManager.layerHeader;
        const oldTailId = layerManager.layerTail;
        const oldPrevId = oldLayer.prev;
        const oldNextId = oldLayer.next;
        const oldPid = oldLayer.pid;
        const parentLayer = oldPid ? layerConfigs[oldPid] : undefined;

        const headerAfterDelete = !oldPid && oldHeaderId === oldId ? (oldNextId ?? undefined) : oldHeaderId;
        const tailAfterDelete = !oldPid && oldTailId === oldId ? (oldPrevId ?? undefined) : oldTailId;
        const finalHeaderId = !oldPid && oldHeaderId === oldId ? newId : headerAfterDelete;
        const finalTailId = !oldPid && oldTailId === oldId ? newId : tailAfterDelete;

        const delPrev: IDelOperateData[] = [
            {id: oldId, layerConfig: cloneDeep(oldLayer), elemConfig: cloneDeep(oldConfig)},
            {layerHeader: oldHeaderId, layerTail: oldTailId},
        ];
        const delNext: IDelOperateData[] = [
            {layerHeader: headerAfterDelete, layerTail: tailAfterDelete},
        ];
        const addPrev: IAddOperateData[] = [
            {layerHeader: headerAfterDelete, layerTail: tailAfterDelete},
        ];
        const addNext: IAddOperateData[] = [
            {
                id: newId,
                layerConfig: cloneDeep(newLayer),
                elemConfig: cloneDeep(configs[newId]),
                layerHeader: finalHeaderId,
                layerTail: finalTailId,
            },
        ];

        const updPrev: IUpdLayerOperateData[] = [];
        const updNext: IUpdLayerOperateData[] = [];
        const relatedLayerIds = new Set<string>();
        if (oldPrevId) {
            relatedLayerIds.add(oldPrevId);
        }
        if (oldNextId) {
            relatedLayerIds.add(oldNextId);
        }
        if (oldPid) {
            relatedLayerIds.add(oldPid);
        }
        relatedLayerIds.forEach((id) => {
            const layer = layerConfigs[id];
            if (layer) {
                updPrev.push({
                    id: layer.id!,
                    prev: layer.prev,
                    next: layer.next,
                    childHeader: layer.childHeader,
                    childTail: layer.childTail,
                    pid: layer.pid,
                });
            }
        });

        runInAction(() => {
            if (oldController) {
                oldController.destroy();
            }
            delete layerConfigs[oldId];
            delete controllers[oldId];

            layerConfigs[newId] = newLayer;
            elemConfigs![newId] = cloneDeep(configs[newId]);

            const prevLayer = oldPrevId ? layerConfigs[oldPrevId] : undefined;
            const nextLayer = oldNextId ? layerConfigs[oldNextId] : undefined;
            if (prevLayer) {
                prevLayer.next = newId;
            }
            if (nextLayer) {
                nextLayer.prev = newId;
            }

            newLayer.prev = oldPrevId;
            newLayer.next = oldNextId;
            newLayer.pid = oldPid;

            if (!oldPid) {
                if (oldHeaderId === oldId) {
                    layerManager.layerHeader = newId;
                }
                if (oldTailId === oldId) {
                    layerManager.layerTail = newId;
                }
            } else if (parentLayer) {
                if (parentLayer.childHeader === oldId) {
                    parentLayer.childHeader = newId;
                }
                if (parentLayer.childTail === oldId) {
                    parentLayer.childTail = newId;
                }
            }

        });

        layerManager.reRenderLayer();

        relatedLayerIds.forEach((id) => {
            const layer = layerConfigs[id];
            if (layer) {
                updNext.push({
                    id: layer.id!,
                    prev: layer.prev,
                    next: layer.next,
                    childHeader: layer.childHeader,
                    childTail: layer.childTail,
                    pid: layer.pid,
                });
            }
        });

        historyOperator.put({
            actions: [
                {type: OperateType.DEL, prev: delPrev, next: delNext},
                {type: OperateType.ADD, prev: addPrev, next: addNext},
                {type: OperateType.UPDATE_LAYER, prev: updPrev, next: updNext},
            ]
        });

        const {setTargetIds, focusDesignerCanvas} = eventOperateStore;
        setTargetIds([]);
        focusDesignerCanvas();

        const tempTimer = setTimeout(() => {
            componentDataGvParser.unbindFromGv([oldId]);
            componentDataGvParser.bindConfigToGvById([newId]);
            clearTimeout(tempTimer);
        }, 10);

        return newId;
    }

    /**
     * @see https://picss.sunbangyan.cn/2023/12/04/6d352450af32841a97f4784d2252eeb8.jpeg
     * 被克隆的图层可以同时包含普通图层、分组图层和分组图层的子图层。
     * 1.普通图层和没有包含分组图层的子图层，则直接克隆。
     * 2.对于分组图层和其下的子图层，克隆时需要创建克隆前后的映射关系，克隆完毕后根据映射关系恢复新图层之前的层级关系。
     * @param ids
     */
    public doClone(ids: string[]): string[] {
        if (ids.length === 0)
            return [];

        const addPrev: IAddOperateData[] = [];
        const addNext: IAddOperateData[] = [];
        const updPrev: IUpdLayerOperateData[] = [];
        const updNext: IUpdLayerOperateData[] = [];
        const layerIdOldToNew: Record<string, string> = {};
        const newLayouts: ILayerItem[] = [];
        const newIds: string[] = [];

        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        //计算复制操作所涉及的所有图层
        const relatedLayerIds: Set<string> = new Set();
        ids.forEach((id) => {
            const layer = layerManager.layerConfigs[id];
            relatedLayerIds.add(id)
            if (layer.prev)
                relatedLayerIds.add(layer.prev);
            if (layer.pid) {
                const parentLayer = layerManager.layerConfigs[layer.pid];
                if (parentLayer) {
                    relatedLayerIds.add(parentLayer.id!);
                    relatedLayerIds.add(parentLayer.childHeader!);
                }
            }
        });

        //记录操作前状态
        updPrev.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail})
        relatedLayerIds.forEach((id) => {
            const layer = layerManager.layerConfigs[id];
            const {prev, next, childHeader, childTail, pid} = layer;
            updPrev.push({id, prev, next, childHeader, childTail, pid})
        })

        //执行复制操作
        runInAction(() => {
            const {layerConfigs, controllers, elemConfigs} = layerManager;
            const oldLayerList = ids.map((id) => layerConfigs[id]);

            //复制数据
            ids.forEach((oldLayerId) => {
                const oldLayer = layerConfigs[oldLayerId];
                const newLayer = cloneDeep(oldLayer);
                newLayer.id = IdGenerate.generateId();
                newLayer.name = oldLayer.name + "-副本";
                newLayer.prev = undefined;
                newLayer.next = undefined;
                if (newLayer.type !== 'group') {
                    newLayer.x = newLayer.x! + 10;
                    newLayer.y = newLayer.y! + 10;
                }
                layerConfigs[newLayer.id] = newLayer;
                layerIdOldToNew[oldLayerId] = newLayer.id!;

                //生成新组件配置项数据
                const oldCompController = controllers[oldLayer.id!];
                if (oldCompController) {
                    const newConfig = cloneDeep(oldCompController.getConfig());
                    newConfig.base.id = newLayer.id;
                    elemConfigs![newLayer.id] = newConfig;
                }
                newIds.push(newLayer.id);
                newLayouts.push(newLayer);
            });

            //更新复制后的图层层级关系
            ids.forEach((oldLayerId) => {
                const oldLayer = layerConfigs[oldLayerId];
                const newLayer = layerConfigs[layerIdOldToNew[oldLayerId]];

                if (oldLayer.type === 'group') {
                    newLayer.childHeader = layerIdOldToNew[LayerUtil.findTopLayerInGroup(oldLayer?.id ?? '', oldLayerList)!];
                    newLayer.childTail = layerIdOldToNew[LayerUtil.findBottomLayerInGroup(oldLayer?.id ?? '', oldLayerList)!];
                }

                //不在分组内的图层
                if (!oldLayer.pid) {
                    const oldPrevLayer = layerConfigs[oldLayer.prev!];
                    newLayer.prev = oldPrevLayer?.id;
                    newLayer.next = oldLayer?.id;
                    oldLayer && (oldLayer.prev = newLayer.id);
                    oldPrevLayer && (oldPrevLayer.next = newLayer.id);
                    if (oldLayer.id === layerManager.layerHeader)
                        layerManager.layerHeader = newLayer.id;
                } else {
                    //在分组内的图层，且整个分组都需要复制
                    if (oldLayer.pid && ids.includes(oldLayer.pid)) {
                        newLayer.prev = layerIdOldToNew[oldLayer.prev!];
                        newLayer.next = layerIdOldToNew[oldLayer.next!];
                        newLayer.pid = layerIdOldToNew[oldLayer.pid!];
                    } else { //在分组内，但仅分组内的单个图层复制
                        const oldChildPrevLayer = layerConfigs[oldLayer.prev!];
                        newLayer.prev = oldChildPrevLayer?.id;
                        newLayer.next = oldLayer?.id;
                        oldLayer && (oldLayer.prev = newLayer.id);
                        oldChildPrevLayer && (oldChildPrevLayer.next = newLayer.id);

                        const parentLayer = layerConfigs[oldLayer.pid!];
                        if (parentLayer.childHeader === oldLayer.id)
                            parentLayer.childHeader = newLayer.id;
                    }
                }
            });
        });

        //记录操作后数据
        updNext.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail})
        relatedLayerIds.forEach((id) => {
            const layer = layerManager.layerConfigs[id];
            const {prev, next, childHeader, childTail, pid} = layer;
            updNext.push({id, prev, next, childHeader, childTail, pid})
        })
        newIds.forEach((id) => {
            const newLayer = layerManager.layerConfigs[id];
            const newConfig = layerManager.elemConfigs![id];
            if (newLayer.type === 'group')
                addNext.push({id: id, layerConfig: cloneDeep(newLayer)})
            else
                addNext.push({id: id, layerConfig: cloneDeep(newLayer), elemConfig: newConfig})
        })

        historyOperator.put({
            actions: [
                {type: OperateType.ADD, prev: addPrev, next: addNext},
                {type: OperateType.UPDATE_LAYER, prev: updPrev, next: updNext},
            ]
        })
        //多个组件同时复制时，需要计算多选框的新位置
        if (newLayouts.length > 1) {
            const {groupCoordinate, setGroupCoordinate} = eventOperateStore;
            setGroupCoordinate({minX: groupCoordinate.minX! + 10, minY: groupCoordinate.minY! + 10})
        }
        return newIds;
    }

}
