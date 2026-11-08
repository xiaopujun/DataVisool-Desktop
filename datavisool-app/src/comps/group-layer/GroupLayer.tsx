/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import React, { useEffect, useRef, useState } from "react";
import GroupLayerController from "@/comps/group-layer/GroupLayerController";
import { AbstractDefinition } from "@/framework/core/AbstractDefinition";
import {ILayerItem} from "@datavisool/sdk/types";
import DesignerManager from "@/designer/manager/DesignerManager.ts";
import {DesignerMode} from "@datavisool/sdk/types";
import DesignerLoader from "@/designer/loader/DesignerLoader.ts";
import {RenderErrorBoundary} from "@datavisool/ui/feedback";

export interface GroupLayerStyleProps {
    index?: number;
    children?: React.ReactNode;
    layer: ILayerItem;
    designerManager: DesignerManager;
}

export interface GroupLayerInstance {
    updateVisible: (visible: boolean) => void;
    // eslint-disable-next-line @typescript-eslint/ban-types
    setEventHandler: (eventMap: Record<string, Function>) => void;
}

const GroupLayer = (props: GroupLayerStyleProps) => {
    const { layer, designerManager, children, index } = props;
    const groupLayerRef = useRef<HTMLDivElement | null>(null);
    const [visible, setVisible] = useState(true);
    const initializedRef = useRef(false);
    const instanceRef = useRef({
        updateVisible: (visible: boolean) => setVisible(visible),
        // eslint-disable-next-line @typescript-eslint/ban-types
        setEventHandler: (eventMap: Record<string, Function>) => eventHandlerMap.current = eventMap,
    });
    // eslint-disable-next-line @typescript-eslint/ban-types
    const eventHandlerMap = useRef<Record<string, Function>>({});

    const onClick = () => {
        if ('click' in eventHandlerMap.current)
            eventHandlerMap.current['click']();
    }

    useEffect(() => {
        if (initializedRef.current) return;
        initializedRef.current = true;
        const { elemConfigs, controllers } = designerManager.layerManager!;
        const groupDefinition: AbstractDefinition = DesignerLoader.definitionMap['group'];
        let config;
        if (layer.id! in controllers!) {
            //重新编组后，被编组组件会重新渲染，需从之前的实例中获取原有数据
            config = controllers![layer.id!].getConfig();
        } else if (layer.id! in elemConfigs!) {
            config = elemConfigs![layer.id!];
        } else {
            config = groupDefinition.getInitConfig();
            config.base.id = layer.id!;
        }

        const controller = new GroupLayerController();
        controllers[layer.id + ''] = controller;
        if (index != null)
            controllers[`${layer.id}_${index}`] = controller;
        controller.createGroupLayerInstance(groupLayerRef.current!, config, instanceRef.current, designerManager.bpExecutor, index).then(() => {
            controller.setVisible(!layer.hide);
            //在组件完全渲染完毕后进行数据的加载和事件的注册
            if (window.DATAVISOOL_ENV.mode !== DesignerMode.EDIT)
                controller.registerEvent();
            controller.updateFilter(controller.getConfig()?.filter ?? {});
            //渲染后删除elemConfigs中的映射关系（需要观察是否会造成其他问题）
            delete elemConfigs![layer.id!];
        });

    }, [designerManager, index, layer.hide, layer.id]);

    return (
        <RenderErrorBoundary>
            {
                visible && <div className={'component-group'} ref={groupLayerRef} onClick={onClick}
                    style={{ position: 'absolute' }}>{children}</div>
            }
        </RenderErrorBoundary>
    )
}

export default GroupLayer;
