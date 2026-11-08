/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import LoopContainerController, {LoopContainerProps} from "./LoopContainerController.ts";
import AbstractController from "@/framework/core/AbstractController.ts";
import {Data, Platte, SettingOne} from "@icon-park/react";
import React from "react";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";
import {ClazzTemplate} from "@datavisool/sdk/types";
import AbstractDesignerDefinition, {ActionInfo} from "@/framework/core/AbstractDesignerDefinition.ts";
import {EventInfo} from "@datavisool/sdk/types";
import type {AbstractDesignerController} from "@datavisool/sdk/core";

const DataConfig = React.lazy(() => import("@/comps/common-component/data-config/DataConfig.tsx"));
const LoopContainerConfig = React.lazy(() => import("./LoopContainerConfig.tsx"));

export default class LoopContainerDefinition extends AbstractDesignerDefinition<LoopContainerController, LoopContainerProps> {
    getBaseInfo(): BaseInfoType {
        return {
            compName: "循环容器",
            compKey: "LoopContainer",
        };
    }
   getChartImg(): string | null {
        return null;
    }

    getController(): ClazzTemplate<LoopContainerController> | null {
        return LoopContainerController;
    }

    getInitConfig(): LoopContainerProps {
        return {
            base: {
                id: "",
                name: '循环容器',
                type: 'LoopContainer',
            },
            style: {
                overflow: 'auto',
                flexDirection: 'column',
                flexWrap: 'nowrap',
                horizontalGap: 10,
                verticalGap: 10,
                enableCarousel: false,
                carouselDirection: 'column',
                carouselSpeed: 3,
                carouselPauseSeconds: 0.08,
            },
            data: {
                sourceType: 'static',
                value: [],
            },
            managerData: {}
        };
    }

    getMenuList(): Array<MenuInfo> {
        return [
            {
                icon: SettingOne,
                key: 'base',
                name: '基础',
            },
            {
                icon: Platte,
                key: 'style',
                name: '样式',
            },
            {
                icon: Data,
                name: '数据',
                key: 'data',
            },
        ];
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            style: LoopContainerConfig,
            data: DataConfig
        };
    }

    getEventList(): EventInfo[] {
        return [
            ...super.getEventList(),
            {
                id: "click",
                name: "点击时",
            }
        ];
    }

    getActionList(): ActionInfo[] {
        return [
            {
                name: "显示",
                id: "show",
                handler: (controller: AbstractController) => {
                    (controller as LoopContainerController).setVisible(true);
                }
            },
            {
                name: "隐藏",
                id: "hide",
                handler: (controller: AbstractController) => {
                    (controller as LoopContainerController).setVisible(false);
                }
            },
            {
                name: "更新组件数据",
                id: "updateData",
                handler: (controller: AbstractDesignerController, params?: object) => {
                    controller.changeData(params);
                }
            },
        ];
    }

}
