/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */


import {IAddOperateData, IDelOperateData, IHistoryRecord, IUpdLayerOperateData, IUpdStyleOperateData, IUpdStyleStatusOperateData, OperateType} from "./OperateType";
import {historyOperator} from "./HistoryOperator";
import eventOperateStore from "@/designer/operate-provider/EventOperateStore";
import {runInAction} from "mobx";
import rightStore from "@/designer/right/RightStore";
import {cloneDeep} from "lodash";
import {ConfigureObjectFragments} from "@datavisool/sdk/utils";
import {IdGenerate} from "@datavisool/sdk/utils";
import {Component} from "react";
import layerListStore from "@/designer/left/layer-list/LayerListStore";
import LayerUtil from "@/designer/left/layer-list/util/LayerUtil";
import {ILayerItem} from "@datavisool/sdk/types";
import CoordinateUtil from "@/util/CoordinateUtil.ts";

import {HistoryRecordOperateProxyLayer} from "./HistoryRecordOperateProxyLayer";
export type {UseTemplateOperateResult} from "./HistoryRecordOperateProxyBase";

type LayerDragPosition = 'before' | 'after';

class HistoryRecordOperateProxy extends HistoryRecordOperateProxyLayer {
    /**
     * 记录组件配置更新
     * @param newData
     * @param oldData
     */
    public doStyleUpd(
        newData: ConfigureObjectFragments,
        oldData: ConfigureObjectFragments,
        targetId?: string,
        replaceKeys?: string[],
        statusPrev?: IUpdStyleStatusOperateData,
        statusNext?: IUpdStyleStatusOperateData,
    ): void {
        const {activeElem: {id: activeId}} = rightStore;
        const id = targetId ?? activeId;
        if (!id)
            return;
        const record: IHistoryRecord = {
            type: OperateType.UPD_STYLE,
            prev: {id, data: oldData, replaceKeys, status: statusPrev} as IUpdStyleOperateData,
            next: {id, data: newData, replaceKeys, status: statusNext} as IUpdStyleOperateData
        }
        historyOperator.put({actions: [record]});
    }

    public doStyleUpdBatch(items: Array<{
        targetId: string;
        newData: ConfigureObjectFragments;
        oldData: ConfigureObjectFragments;
    }>): void {
        const actions = (Array.isArray(items) ? items : [])
            .filter((item) => item?.targetId)
            .map((item) => ({
                type: OperateType.UPD_STYLE,
                prev: {id: item.targetId, data: item.oldData} as IUpdStyleOperateData,
                next: {id: item.targetId, data: item.newData} as IUpdStyleOperateData,
            } as IHistoryRecord));
        if (actions.length === 0)
            return;
        historyOperator.put({actions});
    }

    /**
     * 图层编组
     */
    public doGrouping() {
        let {directSelectedIds: beGroupIds} = eventOperateStore;
        if (!beGroupIds || beGroupIds.length <= 1)
            return;
        //获取编组的基准图层。（以最上层的分组图层优先，最上层的非分组图层次之）
        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        const {layerConfigs, addItem} = layerManager;
        const topLayerId = beGroupIds.reduce((prev, next) => {
            return layerConfigs[prev].order! > layerConfigs[next].order! ? prev : next;
        }, beGroupIds[0]);
        const topPrevLayer = layerConfigs[layerConfigs[topLayerId].prev ?? ''];
        let topNextLayerId = layerConfigs[topLayerId].next ?? '';
        while (beGroupIds.includes(topNextLayerId)) {
            topNextLayerId = layerConfigs[topNextLayerId].next!;
        }
        const topNextLayer = layerConfigs[topNextLayerId];
        const originalPid = layerConfigs[topLayerId].pid; //原始pid，如果topLayerId是分组图层，则最终生成的分组和原始图层所在的分组相同。若没有分组，则pid为undefined


        //构建新的分组数据
        const groupId = IdGenerate.generateId();
        //计算新分组的锁定状态
        const allLock = beGroupIds.every((id) => layerConfigs[id].lock);
        //构建新的分组数据
        const groupItem: ILayerItem = {
            id: groupId,
            type: 'group',
            name: '新建分组',
            hide: false,
            lock: allLock,
        };

        //计算相关图层
        const relatedLayerIds: Set<string> = new Set();
        relatedLayerIds.add(layerManager.layerHeader!);
        beGroupIds.forEach((id) => {
            const layer = layerConfigs[id];
            relatedLayerIds.add(id);
            if (layer.next)
                relatedLayerIds.add(layer.next);
            if (layer.prev)
                relatedLayerIds.add(layer.prev);
            if (layer.pid)
                relatedLayerIds.add(layer.pid);
        });
        if (layerConfigs[topLayerId].type === 'group')
            relatedLayerIds.add(layerConfigs[topLayerId].childHeader!);

        //构建操作记录
        const updPrev: IUpdLayerOperateData[] = [];
        const updNext: IUpdLayerOperateData[] = [];
        const addNext: IAddOperateData[] = [];

        //记录操作前状态
        updPrev.push({layerHeader: layerManager.layerHeader});
        relatedLayerIds.forEach((id) => {
            const layer = layerConfigs[id];
            if (layer.type === 'group')
                updPrev.push({
                    id,
                    pid: layer.pid,
                    prev: layer.prev,
                    next: layer.next,
                    childHeader: layer.childHeader,
                    childTail: layer.childTail
                });
            else
                updPrev.push({id, pid: layer.pid, prev: layer.prev, next: layer.next});
        });

        let chileHeader = "";
        let chileTail = "";
        //修改数据
        runInAction(() => {
            //对目标图层排序
            beGroupIds = beGroupIds.sort((prev, next) => {
                const prevLayer = layerConfigs[prev];
                const nextLayer = layerConfigs[next];
                return prevLayer.order! - nextLayer.order!;
            });
            beGroupIds.forEach((id) => {
                const layer = layerConfigs[id];
                const prevLayer = layerConfigs[layer.prev!];
                const nextLayer = layerConfigs[layer.next!];
                const parentLayer = layerConfigs[layer.pid!];

                if (layerManager.layerHeader === id)
                    layerManager.layerHeader = groupItem.id;
                if (layerManager.layerTail === id) {
                    //如果当前图层加入分组前layer.prev指向为undefined，则说明所有图层都加入了分组，则最新的layerManager.layerTail指向本次分组的id
                    layerManager.layerTail = layer.prev || groupItem.id;
                }
                const topIsHeader = parentLayer?.childHeader === id;
                const topIsTail = parentLayer?.childTail === id;
                if (parentLayer?.childHeader === id)
                    parentLayer.childHeader = layer.next!;
                if (parentLayer?.childTail === id)
                    parentLayer.childTail = layer.prev!;
                if (topIsHeader && topLayerId === id)
                    parentLayer.childHeader = groupId;
                if (topIsTail && topLayerId === id)
                    parentLayer.childTail = groupId;

                layer.pid = groupId;
                //prev图层和next图层建立联系
                prevLayer && (prevLayer.next = layer.next);
                nextLayer && (nextLayer.prev = layer.prev);
                //清空原有图层的prev和next指针
                layer.prev = undefined;
                layer.next = undefined;

                //头插法建立子双向链表
                if (chileHeader) {
                    const oldChildHeader = layerConfigs[chileHeader];
                    oldChildHeader.prev = id;
                    layer.next = chileHeader;
                } else {
                    chileTail = id;
                }
                chileHeader = id;
            });
            groupItem.childHeader = chileHeader;
            groupItem.childTail = chileTail;
            groupItem.pid = originalPid;
            //将新生成的分组加入到topPreLayer之后
            groupItem.prev = topPrevLayer?.id;
            topPrevLayer && (topPrevLayer.next = groupId);
            groupItem.next = topNextLayer?.id;
            topNextLayer && (topNextLayer.prev = groupId);

            if (!groupItem.next && !groupItem.pid) {
                //如果分组加入图层队列链表后后继节点及父节点都为空，则说明分组加入图层队列链表后为最后一个图层。则将layerManager.layerTail指向此分组
                layerManager.layerTail = groupId;
            }
        });
        addItem(groupItem);
        //记录操作后状态
        updNext.push({layerHeader: layerManager.layerHeader});
        relatedLayerIds.forEach((id) => {
            const layer = layerConfigs[id];
            if (layer.type === 'group')
                updNext.push({
                    id,
                    pid: layer.pid,
                    prev: layer.prev,
                    next: layer.next,
                    childHeader: layer.childHeader,
                    childTail: layer.childTail
                });
            else
                updNext.push({id, pid: layer.pid, prev: layer.prev, next: layer.next});
        });
        addNext.push({id: groupId, layerConfig: groupItem});
        historyOperator.put({
            actions: [
                {type: OperateType.ADD, prev: null, next: addNext},
                {type: OperateType.UPDATE_LAYER, prev: updPrev, next: updNext}
            ]
        });
        eventOperateStore.setTargetIds([]);
        //特殊场景处理，如果编组时，所有的子图层都处于锁定状态，则编组后，编组图层也处于锁定状态
        if (allLock) {
            const {layerInstances} = layerListStore;
            const groupTimer = setTimeout(() => {
                (layerInstances[groupId] as Component).setState({lock: true});
                clearTimeout(groupTimer);
            }, 10);
        }
    }

    public doUnGrouping() {
        //如果蓝图中使用了分组图层节点，则需要先删除蓝图中的组件和连线，且蓝图中的删除操作目前无法回退
        const {directSelectedIds, setTargetIds} = eventOperateStore;
        if (directSelectedIds.length === 0)
            return;
        const {bluePrintManager, layerManager} = window.DATAVISOOL_ENV.currentManager;
        const {layerConfigs, delLayout} = layerManager;
        const groupIds = directSelectedIds.filter((id: string) => layerConfigs[id].type === 'group');
        if (directSelectedIds && directSelectedIds.length > 0) {
            const {delNode, currentPageManager} = bluePrintManager;
            const {bpNodeLayoutMap} = currentPageManager!;
            const preDelNodeIds: string[] = [];
            groupIds.forEach((id: string) => {
                if (bpNodeLayoutMap[id])
                    preDelNodeIds.push(id);
            });
            if (preDelNodeIds.length > 0)
                delNode(preDelNodeIds);
        }

        //对每个分组图层进行解组
        const updPrev: IUpdLayerOperateData[] = [];
        const updNext: IUpdLayerOperateData[] = [];
        const delPrev: IDelOperateData[] = [];

        const relatedLayerIds: Set<string> = new Set();
        groupIds.forEach((id) => {
            const groupLayer = layerConfigs[id];
            if (groupLayer.next)
                relatedLayerIds.add(groupLayer.next);
            if (groupLayer.prev)
                relatedLayerIds.add(groupLayer.prev);
            if (groupLayer.pid)
                relatedLayerIds.add(groupLayer.pid);
            if (groupLayer.childHeader) {
                let childLayer = layerConfigs[groupLayer.childHeader];
                if (childLayer)
                    relatedLayerIds.add(childLayer.id!)
                while (childLayer?.next) {
                    childLayer = layerConfigs[childLayer.next];
                    if (childLayer)
                        relatedLayerIds.add(childLayer.id!)
                }
            }
        });

        //记录操作前状态
        updPrev.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail});
        relatedLayerIds.forEach((id) => {
            const layer = layerConfigs[id];
            if (layer) {
                const {prev, next, pid, childHeader, childTail} = layer;
                updPrev.push({id, prev, next, pid, childHeader, childTail});
            }
        });
        groupIds.forEach((id) => {
            delPrev.push({id, layerConfig: cloneDeep(layerConfigs[id])})
        })

        //修改数据
        runInAction(() => {
            groupIds.forEach((groupId: string) => {
                const layer = layerConfigs[groupId];
                const prevLayer = layer.prev ? layerConfigs[layer.prev] : undefined;
                const nextLayer = layer.next ? layerConfigs[layer.next] : undefined;
                const parentLayer = layer.pid ? layerConfigs[layer.pid] : undefined;
                const childHeaderLayer = layer.childHeader ? layerConfigs[layer.childHeader] : undefined;
                const childTailLayer = layer.childTail ? layerConfigs[layer.childTail] : undefined;
                //分组下是否有子图层
                const hasChild = childHeaderLayer && childTailLayer;
                const newHeaderId = hasChild ? childHeaderLayer?.id : nextLayer?.id;
                const newTailId = hasChild ? childTailLayer?.id : prevLayer?.id;

                if (parentLayer?.childHeader === groupId)
                    parentLayer.childHeader = newHeaderId;
                if (parentLayer?.childTail === groupId)
                    parentLayer.childTail = newTailId;

                if (groupId === layerManager.layerHeader)
                    layerManager.layerHeader = newHeaderId;
                if (groupId === layerManager.layerTail)
                    layerManager.layerTail = newTailId;

                if (hasChild) {
                    // 将子图层重新接回父链表，避免父分组还指向已删除的 group 节点。
                    childHeaderLayer && (childHeaderLayer.prev = prevLayer?.id)
                    childTailLayer && (childTailLayer.next = nextLayer?.id)
                    prevLayer && (prevLayer.next = childHeaderLayer?.id)
                    nextLayer && (nextLayer.prev = childTailLayer?.id)

                    let childNext = layer.childHeader;
                    while (childNext) {
                        const childLayer = layerConfigs[childNext];
                        childLayer && (childLayer.pid = layer.pid)
                        childNext = childLayer?.next;
                    }
                } else {
                    prevLayer && (prevLayer.next = nextLayer?.id)
                    nextLayer && (nextLayer.prev = prevLayer?.id)
                }
            });
        });

        //记录操作后数据
        updNext.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail});
        relatedLayerIds.forEach((id) => {
            const layer = layerConfigs[id];
            const {prev, next, pid, childHeader, childTail} = layer;
            updNext.push({id, prev, next, pid, childHeader, childTail});
        });

        delLayout(groupIds);

        const actions: IHistoryRecord[] = [
            {type: OperateType.UPDATE_LAYER, prev: updPrev, next: updNext},
            {type: OperateType.DEL, prev: delPrev, next: null}
        ];
        historyOperator.put({actions});
        //清空组件选中状态
        setTargetIds([]);
        //处理右侧设置项，如果为当前选中的分组图层，则卸载该设置项（因为分组图层已经被删除）
        const {activeElem, activeConfig, setContentVisible} = rightStore;
        if (activeElem && groupIds.includes(activeElem.id!)) {
            setContentVisible(false);
            activeConfig(null, "");
        }
    }

    public doRemoveFromGroup() {
        const {targetIds} = eventOperateStore;
        if (targetIds.length === 0)
            return;
        //筛选出目标图层
        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        const {layerConfigs} = layerManager;
        const finalTargetIds = targetIds.filter(id => {
            const layer = layerConfigs[id];
            return layer.pid && !targetIds.includes(layer.pid)
        })

        const updPrev: IUpdLayerOperateData[] = [];
        const updNext: IUpdLayerOperateData[] = [];

        //计算关联图层id
        const relateIds = new Set<string>();
        relateIds.add(layerManager.layerHeader!);
        finalTargetIds.forEach(id => {
            const layer = layerConfigs[id];
            const parent = layerConfigs[layer.pid!];
            relateIds.add(layer.id!)
            if (layer.prev)
                relateIds.add(layer.prev)
            if (layer.next)
                relateIds.add(layer.next)
            if (parent.childHeader === id || parent.childTail === id)
                relateIds.add(parent.id!)
        })

        //记录数据变更前状态
        updPrev.push({layerHeader: layerManager.layerHeader!, layerTail: layerManager.layerTail!})
        relateIds.forEach((id) => {
            const layer = layerConfigs[id];
            const {prev, next, pid, childHeader, childTail} = layer;
            updPrev.push({id, prev, next, pid, childHeader, childTail});
        });

        //修改数据
        runInAction(() => {
            finalTargetIds.forEach(id => {
                const layer = layerConfigs[id];
                const prevLayer = layerConfigs[layer.prev!];
                const nextLayer = layerConfigs[layer.next!];
                const parentLayer = layerConfigs[layer.pid!];
                const oldHeaderLayer = layerConfigs[layerManager.layerHeader!];

                if (parentLayer.childHeader === id)
                    parentLayer.childHeader = nextLayer?.id;
                if (parentLayer.childTail === id)
                    parentLayer.childTail = prevLayer?.id;

                layer.pid = undefined;
                layer.prev = undefined;
                layer.next = layerManager.layerHeader;
                oldHeaderLayer.prev = layer.id;
                layerManager.layerHeader = layer.id;

                prevLayer && (prevLayer.next = nextLayer?.id)
                nextLayer && (nextLayer.prev = prevLayer?.id)
            })
        })

        //记录数据变更后状态
        updNext.push({layerHeader: layerManager.layerHeader!, layerTail: layerManager.layerTail!})
        relateIds.forEach((id) => {
            const layer = layerConfigs[id];
            const {prev, next, pid, childHeader, childTail} = layer;
            updNext.push({id, prev, next, pid, childHeader, childTail});
        });

        historyOperator.put({actions: [{type: OperateType.UPDATE_LAYER, prev: updPrev, next: updNext}]});
        layerManager.reRenderLayer();
    }

    /**
     * 将source图层移动到target图层前后
     * @param sourceId
     * @param targetId
     * @param position
     */
    public layerDragMove(sourceId: string, targetId: string, position: LayerDragPosition = 'before') {
        if (!sourceId || !targetId || sourceId === targetId)
            return;
        if (LayerUtil.isTargetChild(targetId, sourceId))
            return;
        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        const {layerConfigs} = layerManager;
        const sourceLayer = layerConfigs[sourceId];
        const targetLayer = layerConfigs[targetId];
        if (!sourceLayer || !targetLayer)
            return;
        const sourcePrevId = sourceLayer.prev;
        const sourceNextId = sourceLayer.next;
        const targetPrevId = targetLayer.prev;
        const targetNextId = targetLayer.next;
        const sourceParentId = sourceLayer.pid;
        const targetParentId = targetLayer.pid;
        const sourceParentLayer = sourceParentId ? layerConfigs[sourceParentId] : undefined;
        const targetParentLayer = targetParentId ? layerConfigs[targetParentId] : undefined;

        if (position === 'before' && sourceNextId === targetId)
            return;
        if (position === 'after' && sourcePrevId === targetId)
            return;

        //对每个分组图层进行解组
        const updPrev: IUpdLayerOperateData[] = [];
        const updNext: IUpdLayerOperateData[] = [];

        const relatedLayerIds: Set<string> = new Set([sourceId, targetId]);
        [sourceId, targetId, sourcePrevId, sourceNextId, targetPrevId, targetNextId, sourceParentId, targetParentId].forEach((id) => {
            if (!id)
                return;
            const layer = layerConfigs[id];
            if (!layer)
                return;
            layer.next && relatedLayerIds.add(layer.next);
            layer.prev && relatedLayerIds.add(layer.prev);
            layer.pid && relatedLayerIds.add(layer.pid);
        });

        //记录操作前状态
        updPrev.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail});
        relatedLayerIds.forEach((id) => {
            const layer = layerConfigs[id];
            const {prev, next, pid, childHeader, childTail} = layer;
            updPrev.push({id, prev, next, pid, childHeader, childTail});
        });

        runInAction(() => {
            // 先把 source 从原链表摘掉。
            if (sourcePrevId) {
                layerConfigs[sourcePrevId].next = sourceNextId;
            } else if (sourceParentLayer) {
                sourceParentLayer.childHeader = sourceNextId;
            } else {
                layerManager.layerHeader = sourceNextId;
            }

            if (sourceNextId) {
                layerConfigs[sourceNextId].prev = sourcePrevId;
            } else if (sourceParentLayer) {
                sourceParentLayer.childTail = sourcePrevId;
            } else {
                layerManager.layerTail = sourcePrevId;
            }

            // 再插入到 target 前后，并继承 target 当前所在的父级。
            sourceLayer.pid = targetParentId;
            if (position === 'before') {
                sourceLayer.prev = targetPrevId;
                sourceLayer.next = targetId;
                targetLayer.prev = sourceId;

                if (targetPrevId) {
                    layerConfigs[targetPrevId].next = sourceId;
                } else if (targetParentLayer) {
                    targetParentLayer.childHeader = sourceId;
                } else {
                    layerManager.layerHeader = sourceId;
                }
            } else {
                sourceLayer.prev = targetId;
                sourceLayer.next = targetNextId;
                targetLayer.next = sourceId;

                if (targetNextId) {
                    layerConfigs[targetNextId].prev = sourceId;
                } else if (targetParentLayer) {
                    targetParentLayer.childTail = sourceId;
                } else {
                    layerManager.layerTail = sourceId;
                }
            }
        })

        //记录操作后数据
        updNext.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail});
        relatedLayerIds.forEach((id) => {
            const layer = layerConfigs[id];
            const {prev, next, pid, childHeader, childTail} = layer;
            updNext.push({id, prev, next, pid, childHeader, childTail});
        });
        const actions: IHistoryRecord[] = [{type: OperateType.UPDATE_LAYER, prev: updPrev, next: updNext}];
        historyOperator.put({actions});

        layerManager.reRenderLayer();
    }

    /**
     * 组内上对其
     */
    public groupTopAligned() {
        //日志记录
        const updPrev: IUpdLayerOperateData[] = [];
        const updNext: IUpdLayerOperateData[] = [];

        const {targetIds, setTargetIds} = eventOperateStore;

        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        //记录变更前数据
        targetIds.forEach(id => {
            const layer = layerManager.layerConfigs[id];
            updPrev.push({id, y: layer.y});
        });

        //变更数据
        const coordinate = CoordinateUtil.calculateGroupRect(targetIds, layerManager.layerConfigs);
        targetIds.forEach(id => {
            const layer = layerManager.layerConfigs[id];
            layer.y = coordinate.minY;
        });

        //记录变更后的数据
        targetIds.forEach(id => {
            const layer = layerManager.layerConfigs[id];
            updNext.push({id, y: layer.y});
        });

        //写入日志
        historyOperator.put({actions: [{type: OperateType.UPDATE_LAYER, prev: updPrev, next: updNext}]});

        //重新选中最新位置的包围盒
        setTargetIds([]);
        Promise.resolve().then(() => setTargetIds(targetIds));
        layerManager.reRenderLayer();
    }

    /**
     * 组内下对齐
     */
    public groupBottomAligned() {
        //日志记录
        const updPrev: IUpdLayerOperateData[] = [];
        const updNext: IUpdLayerOperateData[] = [];

        const {targetIds, setTargetIds} = eventOperateStore;
        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        //记录变更前数据
        targetIds.forEach(id => {
            const layer = layerManager.layerConfigs[id];
            updPrev.push({id, y: layer.y});
        });

        const coordinate = CoordinateUtil.calculateGroupRectLT(targetIds, layerManager.layerConfigs);
        targetIds.forEach(id => {
            const layer = layerManager.layerConfigs[id];
            layer.y = coordinate.maxY;
        });

        //记录变更后的数据
        targetIds.forEach(id => {
            const layer = layerManager.layerConfigs[id];
            updNext.push({id, y: layer.y});
        });

        //写入日志
        historyOperator.put({actions: [{type: OperateType.UPDATE_LAYER, prev: updPrev, next: updNext}]});

        //重新选中最新位置的包围盒
        setTargetIds([]);
        Promise.resolve().then(() => setTargetIds(targetIds));
        layerManager.reRenderLayer();
    }

    /**
     * 组内左对齐
     */
    public groupLeftAligned() {
        //日志记录
        const updPrev: IUpdLayerOperateData[] = [];
        const updNext: IUpdLayerOperateData[] = [];

        const {targetIds, setTargetIds} = eventOperateStore;
        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        //记录变更前数据
        targetIds.forEach(id => {
            const layer = layerManager.layerConfigs[id];
            updPrev.push({id, x: layer.x});
        });

        const coordinate = CoordinateUtil.calculateGroupRect(targetIds, layerManager.layerConfigs);
        targetIds.forEach(id => {
            const layer = layerManager.layerConfigs[id];
            layer.x = coordinate.minX;
        });

        //记录变更后的数据
        targetIds.forEach(id => {
            const layer = layerManager.layerConfigs[id];
            updNext.push({id, x: layer.x});
        });

        //写入日志
        historyOperator.put({actions: [{type: OperateType.UPDATE_LAYER, prev: updPrev, next: updNext}]});

        //重新选中最新位置的包围盒
        setTargetIds([]);
        Promise.resolve().then(() => setTargetIds(targetIds));
        layerManager.reRenderLayer();
    }

    /**
     * 组内右对齐
     */
    public groupRightAligned() {
        //日志记录
        const updPrev: IUpdLayerOperateData[] = [];
        const updNext: IUpdLayerOperateData[] = [];

        const {targetIds, setTargetIds} = eventOperateStore;
        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        //记录变更前数据
        targetIds.forEach(id => {
            const layer = layerManager.layerConfigs[id];
            updPrev.push({id, x: layer.x});
        });

        const coordinate = CoordinateUtil.calculateGroupRectLT(targetIds, layerManager.layerConfigs);
        targetIds.forEach(id => {
            const layer = layerManager.layerConfigs[id];
            layer.x = coordinate.maxX;
        });

        //记录变更后的数据
        targetIds.forEach(id => {
            const layer = layerManager.layerConfigs[id];
            updNext.push({id, x: layer.x});
        });

        //写入日志
        historyOperator.put({actions: [{type: OperateType.UPDATE_LAYER, prev: updPrev, next: updNext}]});

        //重新选中最新位置的包围盒
        setTargetIds([]);
        Promise.resolve().then(() => setTargetIds(targetIds));
        layerManager.reRenderLayer();
    }

    /**
     * 创建循环容器
     */
    public createLoopContainer() {
        const {layerManager, canvasManager: {canvasConfig: {width = 0, height = 0}}} = window.DATAVISOOL_ENV.currentManager;
        const {layerConfigs, elemConfigs, layerHeader} = layerManager;
        const {setTargetIds} = eventOperateStore;
        const loopId = IdGenerate.generateId();
        const definition = window.DATAVISOOL_ENV.definitions!['LoopContainer'];
        if (!definition)
            return;
        const initConfig = definition.getInitConfig();
        initConfig.base.id = loopId;
        const w = 640, h = 400;
        const x = width / 2 - w / 2, y = height / 2 - h / 2;
        const loopLayer: ILayerItem = {
            id: loopId,
            type: 'LoopContainer',
            name: '循环容器',
            hide: false,
            lock: false,
            x,
            y,
            width: w,
            height: h,
            rotate: 0
        };

        runInAction(() => {
            const oldHeaderLayer = layerConfigs[layerHeader!];
            oldHeaderLayer && (oldHeaderLayer.prev = loopId);
            loopLayer.next = oldHeaderLayer?.id;
            layerManager.layerHeader = loopId;
            if (!oldHeaderLayer)
                layerManager.layerTail = loopId;
            layerConfigs[loopId] = loopLayer;
            elemConfigs![loopId] = initConfig;
        });
        Promise.resolve().then(() => setTargetIds([loopId]));
    }


}

const historyRecordOperateProxy = new HistoryRecordOperateProxy();
export default historyRecordOperateProxy;
