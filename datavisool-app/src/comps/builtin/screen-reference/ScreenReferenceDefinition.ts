/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import baseColorBlockImg from './screen-reference.webp';
import {ScreenReferenceController} from "./ScreenReferenceController.ts";
import {ScreenReferenceComponentProps} from "./ScreenReferenceComponent.tsx";
import React, {lazy} from "react";
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {EventInfo} from "@datavisool/sdk/types";

const AnimationConfig = React.lazy(() => import("@/comps/common-component/animation-config/AnimationConfig.tsx"));
const ScreenReferenceConfig = lazy(() => import("./ScreenReferenceConfig.tsx").then((module) => ({default: module.ScreenReferenceConfig})));

export default class ScreenReferenceDefinition extends AbstractDesignerDefinition<ScreenReferenceController, ScreenReferenceComponentProps> {
    getBaseInfo(): BaseInfoType {
        return {
            compName: "大屏引用",
            compKey: "ScreenReference",
            categorize: "other",
        };
    }
    getChartImg(): string | null {
        return baseColorBlockImg;
    }

    getController(): ClazzTemplate<ScreenReferenceController> | null {
        return ScreenReferenceController;
    }

    getInitConfig(): ScreenReferenceComponentProps {
        return {
            base: {
                id: "",
                name: '大屏引用',
                type: 'ScreenReference',
            },
            style: {},
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
        return super.getMenuList().filter((item: MenuInfo) => (item.key !== 'theme' && item.key !== 'data' && item.key !== 'mapping'));
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            style: ScreenReferenceConfig,
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
