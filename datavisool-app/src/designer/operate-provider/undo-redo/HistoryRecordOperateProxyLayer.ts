/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */


import {IHideOperateData, IHistoryRecord, ILockOperateData, IUpdLayerOperateData, OperateType} from "./OperateType";
import {historyOperator} from "./HistoryOperator";
import eventOperateStore from "@/designer/operate-provider/EventOperateStore";
import {runInAction} from "mobx";
import {Component} from "react";
import layerListStore from "@/designer/left/layer-list/LayerListStore";
import LayerUtil from "@/designer/left/layer-list/util/LayerUtil";
import designerLeftStore from "@/designer/left/DesignerLeftStore";
import {ILayerItem} from "@datavisool/sdk/types";
import {BaseLayer} from "@/designer/left/layer-list/item/BaseLayer.tsx";

import {HistoryRecordOperateProxyBase} from "./HistoryRecordOperateProxyBase";

export class HistoryRecordOperateProxyLayer extends HistoryRecordOperateProxyBase {
    public doHideUpd(ids: string[], hide: boolean): void {
        //真实隐藏的数据结构列表
        const realHideList = LayerUtil.combineCalculationsActiveLayers(ids).map(id => ({id, hide}));

        const prev: IHideOperateData[] = [];
        const next: IHideOperateData[] = [];
        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        const {layerConfigs, updateLayer, controllers} = layerManager;
        realHideList.forEach((item) => {
            const {id} = item;
            const oldHideData = layerConfigs[id!];
            prev.push({id, hide: oldHideData.hide!});
            next.push({id, hide});
        })
        const data: IHistoryRecord = {type: OperateType.HIDE, prev, next}
        historyOperator.put({actions: [data]});
        //更新隐藏状态
        updateLayer(realHideList);
        //取消所有选中状态
        eventOperateStore.setTargetIds([]);

        const {layerInstances} = layerListStore;
        const {menu} = designerLeftStore;
        realHideList && realHideList.forEach((item) => {
            const controller = controllers[item.id!];
            if (controller)
                controller.setVisible(!item.hide);
            if (menu === 'layer-list')
                (layerInstances[item.id!] as BaseLayer)?.changeHide(item.hide);
        })
    }

    public doLockUpd(items: ILayerItem[]): void {
        const prev: ILockOperateData[] = [];
        const next: ILockOperateData[] = [];
        const {layerConfigs, updateLayer} = window.DATAVISOOL_ENV.currentManager.layerManager!;
        items.forEach((item) => {
            const {id, lock} = item;
            next.push({id: id!, lock: lock!});
            const oldLockData = layerConfigs[id!];
            prev.push({id: id!, lock: oldLockData.lock!});
        })
        const data: IHistoryRecord = {type: OperateType.LOCK, prev, next}
        historyOperator.put({actions: [data]});
        updateLayer(items);
        const {layerInstances} = layerListStore;
        const {menu} = designerLeftStore;
        if (menu === 'layer-list') {
            //更新图层列表
            items.forEach((item) => {
                (layerInstances[item.id!] as Component)?.setState({lock: item.lock})
            })
        }
    }

    /**
     * 判断是否可以上移图层
     * @param targetIds
     * @private
     */
    private canMoveUp(targetIds: string[]): boolean {
        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        const {layerConfigs} = layerManager;
        const layerHeaderMap: Record<string, string[]> = {};

        targetIds.forEach((id) => {
            const layer = layerConfigs[id];
            if (layer.pid) {
                const parentLayer = layerConfigs[layer.pid]
                if (parentLayer.childHeader) {
                    if (!layerHeaderMap[parentLayer.childHeader])
                        layerHeaderMap[parentLayer.childHeader] = [id];
                    else
                        layerHeaderMap[parentLayer.childHeader].push(id);
                }
            } else {
                if (!layerHeaderMap[layerManager.layerHeader!])
                    layerHeaderMap[layerManager.layerHeader!] = [id];
                else
                    layerHeaderMap[layerManager.layerHeader!].push(id);
            }
        })

        for (const [header, layerIds] of Object.entries(layerHeaderMap)) {
            if (layerIds.includes(header!)) {
                let count = 1;
                let layer = layerConfigs[header]
                while (layer.next) {
                    layer = layerConfigs[layer.next];
                    if (targetIds.includes(layer.id!))
                        count++;
                    else
                        break;
                }
                if (count >= layerIds.length)
                    targetIds = targetIds.filter(id => !layerIds.includes(id))
            }
        }

        return targetIds.length !== 0;
    }

    /**
     * 判断图层是否可以下移
     * @param targetIds
     * @private
     */
    private canMoveDown(targetIds: string[]): boolean {
        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        const {layerConfigs} = layerManager;
        const layerTailMap: Record<string, string[]> = {};

        targetIds.forEach((id) => {
            const layer = layerConfigs[id];
            if (layer.pid) {
                const parentLayer = layerConfigs[layer.pid]
                if (parentLayer.childTail) {
                    if (!layerTailMap[parentLayer.childTail])
                        layerTailMap[parentLayer.childTail] = [id];
                    else
                        layerTailMap[parentLayer.childTail].push(id);
                }
            } else {
                if (!layerTailMap[layerManager.layerTail!])
                    layerTailMap[layerManager.layerTail!] = [id];
                else
                    layerTailMap[layerManager.layerTail!].push(id);
            }
        })


        for (const [tail, layerIds] of Object.entries(layerTailMap)) {
            if (layerIds.includes(tail!)) {
                let count = 1;
                let layer = layerConfigs[tail]
                while (layer.prev) {
                    layer = layerConfigs[layer.prev];
                    if (targetIds.includes(layer.id!))
                        count++;
                    else
                        break;
                }
                if (count >= layerIds.length)
                    targetIds = targetIds.filter(id => !layerIds.includes(id))
            }
        }

        return targetIds.length !== 0;
    }

    public doLayerToTop(): void {
        const {targetIds} = eventOperateStore;
        if (targetIds.length === 0)
            return;

        if (!this.canMoveUp(targetIds))
            return;

        const {layerManager} = window.DATAVISOOL_ENV.currentManager!;
        const {layerConfigs} = layerManager!;
        const updPrev: IUpdLayerOperateData[] = [];
        const updNext: IUpdLayerOperateData[] = [];

        let finalTargetIds = targetIds.filter((id) => {
            const layer = layerConfigs[id];
            return !layer.pid || (layer.pid && !targetIds.includes(layer.pid));
        })

        //计算相关的图层id
        const relatedLayerIds: Set<string> = new Set();
        relatedLayerIds.add(layerManager?.layerHeader ?? '');
        finalTargetIds.forEach((id) => {
            const layer = layerConfigs[id];
            relatedLayerIds.add(id);
            layer.next && relatedLayerIds.add(layer.next);
            layer.prev && relatedLayerIds.add(layer.prev);
            layer.pid && relatedLayerIds.add(layer.pid);
        });

        //记录数据修改前状态
        updPrev.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail})
        relatedLayerIds.forEach((id) => {
            const {prev, next, childHeader, childTail} = layerConfigs[id];
            updPrev.push({id, prev, next, childHeader, childTail})
        });

        //将被操作图层排序
        finalTargetIds = finalTargetIds.sort((prev, next) => {
            const prevLayer = layerConfigs[prev];
            const nextLayer = layerConfigs[next];
            return prevLayer.order! - nextLayer.order!;
        });

        //修改数据
        runInAction(() => {
            for (let i = 0; i < finalTargetIds.length; i++) {
                const id = finalTargetIds[i];
                const layer = layerConfigs[id];
                const prevLayer = layerConfigs[layer.prev!];
                const nextLayer = layerConfigs[layer.next!];
                if (layer.pid) {
                    //组内元素
                    const parentLayer = layerConfigs[layer.pid!];
                    if (parentLayer.childHeader === id)
                        continue;
                    const oldHeaderChildLayer = layerConfigs[parentLayer.childHeader!];
                    layer.prev = undefined;
                    layer.next = parentLayer?.childHeader;
                    prevLayer && (prevLayer.next = nextLayer?.id);
                    nextLayer && (nextLayer.prev = prevLayer?.id);
                    oldHeaderChildLayer && (oldHeaderChildLayer.prev = id);
                    parentLayer.childHeader = id;
                    if (id === parentLayer.childTail)
                        parentLayer.childTail = prevLayer?.id;
                } else {
                    //组外元素
                    if (layerManager.layerHeader === id)
                        continue;
                    layer.prev = undefined;
                    layer.next = layerManager.layerHeader;
                    prevLayer && (prevLayer.next = nextLayer?.id);
                    nextLayer && (nextLayer.prev = prevLayer?.id);
                    const oldHeaderLayer = layerConfigs[layerManager.layerHeader!];
                    oldHeaderLayer && (oldHeaderLayer.prev = id);
                    layerManager.layerHeader = id;
                    if (id === layerManager.layerTail)
                        layerManager.layerTail = prevLayer?.id;
                }
            }
        })

        //记录数据修改后状态
        updNext.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail})
        relatedLayerIds.forEach((id) => {
            const {prev, next, childHeader, childTail} = layerConfigs[id];
            updNext.push({id, prev, next, childHeader, childTail})
        });

        historyOperator.put({actions: [{type: OperateType.UPDATE_LAYER, prev: updPrev, next: updNext}]});
        layerManager.reRenderLayer();
    }

    public doLayerToBottom(): void {
        const {targetIds} = eventOperateStore;
        if (targetIds.length === 0 || !this.canMoveDown(targetIds))
            return;

        const updPrev: IUpdLayerOperateData[] = [];
        const updNext: IUpdLayerOperateData[] = [];
        const {layerManager} = window.DATAVISOOL_ENV.currentManager!;
        const {layerConfigs} = layerManager;

        let finalTargetIds = targetIds.filter((id) => {
            const layer = layerConfigs[id];
            return !layer.pid || (layer.pid && !targetIds.includes(layer.pid));
        })

        //计算相关的图层id
        const relatedLayerIds: Set<string> = new Set();
        relatedLayerIds.add(layerManager.layerTail!)
        finalTargetIds.forEach((id) => {
            const layer = layerConfigs[id];
            relatedLayerIds.add(id);
            layer.next && relatedLayerIds.add(layer.next);
            layer.prev && relatedLayerIds.add(layer.prev);
            layer.pid && relatedLayerIds.add(layer.pid);
        });

        //记录数据修改前状态
        updPrev.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail})
        relatedLayerIds.forEach((id) => {
            const {prev, next, childHeader, childTail} = layerConfigs[id];
            updPrev.push({id, prev, next, childHeader, childTail})
        });

        //将被操作图层排序
        finalTargetIds = finalTargetIds.sort((prev, next) => {
            const prevLayer = layerConfigs[prev];
            const nextLayer = layerConfigs[next];
            return nextLayer.order! - prevLayer.order!;
        });

        //修改数据
        runInAction(() => {
            for (let i = 0; i < finalTargetIds.length; i++) {
                const id = finalTargetIds[i];
                const layer = layerConfigs[id];
                const prevLayer = layerConfigs[layer.prev!];
                const nextLayer = layerConfigs[layer.next!];
                if (layer.pid) {
                    //组内元素
                    const parentLayer = layerConfigs[layer.pid!];
                    if (id === parentLayer.childTail)
                        continue;
                    const oldTailChildLayer = layerConfigs[parentLayer.childTail!];
                    layer.prev = parentLayer.childTail;
                    layer.next = undefined;
                    prevLayer && (prevLayer.next = nextLayer?.id);
                    nextLayer && (nextLayer.prev = prevLayer?.id);
                    oldTailChildLayer && (oldTailChildLayer.next = id);
                    parentLayer.childTail = id;
                    if (id === parentLayer.childHeader)
                        parentLayer.childHeader = nextLayer?.id;
                } else {
                    //组外元素
                    if (layerManager.layerTail === id)
                        continue;
                    layer.prev = layerManager.layerTail;
                    layer.next = undefined;
                    prevLayer && (prevLayer.next = nextLayer?.id);
                    nextLayer && (nextLayer.prev = prevLayer?.id);
                    const oldTailLayer = layerConfigs[layerManager.layerTail!];
                    oldTailLayer && (oldTailLayer.next = id);
                    layerManager.layerTail = id;
                    if (id === layerManager.layerHeader)
                        layerManager.layerHeader = nextLayer?.id;
                }
            }
        })

        //记录数据修改后状态
        updNext.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail})
        relatedLayerIds.forEach((id) => {
            const {prev, next, childHeader, childTail} = layerConfigs[id];
            updNext.push({id, prev, next, childHeader, childTail})
        });

        historyOperator.put({actions: [{type: OperateType.UPDATE_LAYER, prev: updPrev, next: updNext}]});
        layerManager.reRenderLayer();
    }

    public doLayerMoveUp(): void {
        let {targetIds} = eventOperateStore;
        if (targetIds.length === 0)
            return;

        if (!this.canMoveUp(targetIds))
            return;

        const {layerManager} = window.DATAVISOOL_ENV.currentManager;
        const {layerConfigs} = layerManager;
        const prev: IUpdLayerOperateData[] = [];
        const next: IUpdLayerOperateData[] = [];

        //收集相关的图层id
        const relatedLayerIds: Set<string> = new Set();
        targetIds.forEach((id) => {
            const layer = layerConfigs[id];
            const prevLayer = layerConfigs[layer.prev!];
            relatedLayerIds.add(id);
            if (layer && layer.next)
                relatedLayerIds.add(layer.next);
            if (layer && layer.prev)
                relatedLayerIds.add(layer.prev);
            if (prevLayer && prevLayer.prev)
                relatedLayerIds.add(prevLayer.prev!);
        });

        //将被操作图层排序
        targetIds = targetIds.sort((prev, next) => {
            const prevLayer = layerConfigs[prev];
            const nextLayer = layerConfigs[next];
            return nextLayer.order! - prevLayer.order!;
        });

        //记录数据变更前状态
        prev.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail})
        relatedLayerIds.forEach((id) => {
            const layer = layerConfigs[id];
            const {childTail, childHeader} = layer;
            prev.push({id, prev: layer.prev, next: layer.next, childHeader, childTail})
        });

        //变更数据
        runInAction(() => {
            targetIds.forEach((id) => {
                const layer = layerConfigs[id];
                const prevLayer = layerConfigs[layer.prev!];
                const prevPrevLayer = layerConfigs[prevLayer?.prev ?? ''];
                const nextLayer = layerConfigs[layer.next!];
                if (layer.pid) {
                    //在分组内
                    const parent = layerConfigs[layer.pid];
                    if (id !== parent.childHeader) {
                        if (layer.prev === parent.childHeader)
                            parent.childHeader = id;
                        if (id === parent.childTail)
                            parent.childTail = layer.prev!;
                        layer.prev = prevPrevLayer?.id;
                        layer.next = prevLayer?.id;
                        if (prevLayer) {
                            prevLayer.prev = layer.id
                            prevLayer.next = nextLayer?.id
                        }
                        prevPrevLayer && (prevPrevLayer.next = layer.id)
                        nextLayer && (nextLayer.prev = prevLayer?.id)
                    }
                } else {
                    //不再分组内
                    if (id !== layerManager.layerHeader) {
                        if (layer.prev === layerManager.layerHeader)
                            layerManager.layerHeader = id;
                        if (id === layerManager.layerTail)
                            layerManager.layerTail = layer.prev!;
                        layer.prev = prevPrevLayer?.id;
                        layer.next = prevLayer?.id;
                        if (prevLayer) {
                            prevLayer.prev = layer.id
                            prevLayer.next = nextLayer?.id
                        }
                        prevPrevLayer && (prevPrevLayer.next = layer.id)
                        nextLayer && (nextLayer.prev = prevLayer?.id)
                    }
                }
            })
        })

        //记录数据变更后状态
        next.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail})
        relatedLayerIds.forEach((id) => {
            const layer = layerConfigs[id];
            const {childTail, childHeader} = layer;
            next.push({id, prev: layer.prev, next: layer.next, childHeader, childTail})
        });
        historyOperator.put({actions: [{type: OperateType.UPDATE_LAYER, prev, next}]});
        layerManager.reRenderLayer();
    }

    public doLayerMoveDown(): void {
        let {targetIds} = eventOperateStore;
        if (targetIds.length === 0 || !this.canMoveDown(targetIds))
            return;

        const {layerManager} = window.DATAVISOOL_ENV.currentManager!;
        const {layerConfigs} = layerManager;
        const prev: IUpdLayerOperateData[] = [];
        const next: IUpdLayerOperateData[] = [];

        //收集相关的图层id
        const relatedLayerIds: Set<string> = new Set();
        targetIds.forEach((id) => {
            const layer = layerConfigs[id];
            const nextLayer = layerConfigs[layer.next!];
            relatedLayerIds.add(id);
            if (layer && layer.next)
                relatedLayerIds.add(layer.next);
            if (layer && layer.prev)
                relatedLayerIds.add(layer.prev);
            if (nextLayer && nextLayer.next)
                relatedLayerIds.add(nextLayer.next!);
        });

        //将被操作图层排序
        targetIds = targetIds.sort((prev, next) => {
            const prevLayer = layerConfigs[prev];
            const nextLayer = layerConfigs[next];
            return prevLayer.order! - nextLayer.order!;
        });

        //记录数据变更前状态
        prev.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail})
        relatedLayerIds.forEach((id) => {
            const layer = layerConfigs[id];
            const {childTail, childHeader} = layer;
            prev.push({id, prev: layer.prev, next: layer.next, childHeader, childTail})
        });

        //变更数据
        runInAction(() => {
            targetIds.forEach((id) => {
                const layer = layerConfigs[id];
                const prevLayer = layerConfigs[layer.prev!];
                const nextLayer = layerConfigs[layer.next!];
                const nextNextLayer = layerConfigs[nextLayer?.next ?? ''];
                if (layer.pid) {
                    //在分组内
                    const parent = layerConfigs[layer.pid];
                    if (id !== parent.childTail) {
                        if (layer.next === parent.childTail)
                            parent.childTail = id;
                        if (id === parent.childHeader)
                            parent.childHeader = layer.next!;
                        layer.prev = nextLayer?.id;
                        layer.next = nextNextLayer?.id;
                        if (nextLayer) {
                            nextLayer.prev = prevLayer?.id
                            nextLayer.next = layer.id
                        }
                        prevLayer && (prevLayer.next = nextLayer?.id)
                        nextNextLayer && (nextNextLayer.prev = layer?.id)
                    }
                } else {
                    //不再分组内
                    if (id !== layerManager.layerTail) {
                        if (layer.next === layerManager.layerTail)
                            layerManager.layerTail = id;
                        if (id === layerManager.layerHeader)
                            layerManager.layerHeader = layer.next!;
                        layer.prev = nextLayer?.id;
                        layer.next = nextNextLayer?.id;
                        if (nextLayer) {
                            nextLayer.prev = prevLayer?.id
                            nextLayer.next = layer.id
                        }
                        prevLayer && (prevLayer.next = nextLayer?.id)
                        nextNextLayer && (nextNextLayer.prev = layer?.id)
                    }
                }
            })
        })

        //记录数据变更后状态
        next.push({layerHeader: layerManager.layerHeader, layerTail: layerManager.layerTail})
        relatedLayerIds.forEach((id) => {
            const layer = layerConfigs[id];
            const {childTail, childHeader} = layer;
            next.push({id, prev: layer.prev, next: layer.next, childHeader, childTail})
        });
        historyOperator.put({actions: [{type: OperateType.UPDATE_LAYER, prev, next}]});
        layerManager.reRenderLayer();
    }

}
