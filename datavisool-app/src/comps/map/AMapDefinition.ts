/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import aMapImage from '@/comps/map/a-map.webp';
import {AMapController} from "@/comps/map/AMapController.ts";
import {AMapComponentProps} from "@/comps/map/AMapComponent.tsx";
import React, {lazy} from "react";
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {EventInfo} from "@datavisool/sdk/types";

const AnimationConfig = React.lazy(() => import("@/comps/common-component/animation-config/AnimationConfig.tsx"));
const AMapConfig = lazy(() => import("@/comps/map/AMapConfig.tsx").then((module) => ({default: module.AMapConfig})));

export default class AMapDefinition extends AbstractDesignerDefinition<AMapController, AMapComponentProps> {
    getBaseInfo(): BaseInfoType {
        return {
            compName: "自定义高德地图",
            compKey: "AMap",
            categorize: "map",
        };
    }
    getChartImg(): string | null {
        return aMapImage;
    }

    getController(): ClazzTemplate<AMapController> | null {
        return AMapController;
    }

    getInitConfig(): AMapComponentProps {
        return {
            base: {
                id: "",
                name: '自定义高德地图',
                type: 'AMap',
            },
            style: {
                key: "",
                securityJsCode: "",
                customCode: "function(container, AMap, bpTriggers) {\n" +
                    "    let map = new AMap.Map(container, {\n" +
                    "        zoom: 11.43,\n" +
                    "        center: [120.2446746826172, 30.199146446037616],\n" +
                    "        pitch: 55,\n" +
                    "        rotation: 20,\n" +
                    "        showLabel: true,\n" +
                    "        viewMode: '3D',\n" +
                    "        mapStyle: 'amap://styles/dark',\n" +
                    "    });\n" +
                    "    return map;\n" +
                    "}"
            },
            filter: {
                enable: false,
                blur: 0,
                brightness: 1,
                contrast: 1,
                opacity: 1,
                saturate: 1,
                hueRotate: 0
            },
        };
    }

    getMenuList(): Array<MenuInfo> {
        return super.getMenuList().filter((item: MenuInfo) => (item.key !== 'theme' && item.key !== 'data'));
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            style: AMapConfig,
            animation: AnimationConfig
        };
    }

    getEventList(): EventInfo[] {
        const events = super.getEventList();
        return events.concat([
            {
                id: "click",
                name: "点击时",
            }
        ]);
    }
}
