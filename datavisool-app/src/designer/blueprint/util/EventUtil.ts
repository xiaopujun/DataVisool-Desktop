/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import DesignerManager from "@/designer/manager/DesignerManager.ts";
import {isBlueprintNodeEnabled} from "@/designer/blueprint/util/BPNodeEnableUtil.ts";

/**
 * 事件工具类
 */
export default class EventUtil {

    /**
     * 触发全局事件
     * @param manager 设计器管理器
     */
    public static triggerGlobalEvent = (manager: DesignerManager) => {
        let count = 0;
        const total = Object.keys(manager.layerManager.layerConfigs).length;
        const disabledNodeIds = new Set<string>();
        manager.bluePrintManager?.bpPageMangers.forEach((pageManager) => {
            Object.entries(pageManager.bpNodeConfigMap ?? {}).forEach(([nodeId, config]) => {
                if (!isBlueprintNodeEnabled(config))
                    disabledNodeIds.add(nodeId);
            });
        });
        const triggerTimer = setInterval(() => {
            const rendered = Object.keys(manager.layerManager.controllers).length;
            if (rendered === total || count > 100) {
                //触发组件加载完成事件
                manager.layerManager.controllers && Object.values(manager.layerManager.controllers).forEach((controller) => {
                    const componentId = controller.config.base.id;
                    manager.bpExecutor?.triggerComponentEvent(componentId, 'loaded', controller.config, controller.index);
                });
                //触发蓝图事件
                manager.bluePrintManager?.bpPageMangers.forEach((pageManager) => {
                    Object.values(pageManager?.bpNodeLayoutMap ?? {}).forEach((nodeLayout) => {
                        const {type, id = ''} = nodeLayout;
                        if (disabledNodeIds.has(id))
                            return;
                        if (type === 'BPGlobalNode') //全局节点事件
                            manager.bpExecutor?.triggerComponentEvent(id, 'allLoaded', null);
                        if (type === 'BPTimerNode') { //定时器节点事件
                            const {interval = 5, loop = true} = pageManager.bpNodeConfigMap[id] ?? {};
                            manager.bpExecutor?.triggerComponentEvent(id, 'scheduledExecute', null);
                            const timer = setInterval(() => {
                                manager.bpExecutor?.triggerComponentEvent(id, 'scheduledExecute', null);
                                if (!loop)
                                    clearInterval(timer);
                            }, interval * 1000);
                        }
                    })
                })
                //启动全局变量设定了自动更新的任务
                Object.values(manager.globalVariableManager.globalVariableInstance).forEach((gv) => {
                    if (disabledNodeIds.has(gv.id))
                        return;
                    gv.start(
                        {controllers: manager.layerManager.controllers, manager},
                        manager.bpExecutor
                    )
                })
                clearInterval(triggerTimer);
            }
            count++;
        }, 50);
    }
}
