/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuToConfigMappingType} from "@datavisool/sdk/core";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {EventInfo} from "@datavisool/sdk/types";
import baseTextImg from './base-text.webp';
import {BaseTextController} from "./BaseTextController";
import {BaseTextComponentProps} from "./BaseTextComponent";
import React, {lazy} from "react";
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";

const AnimationConfig = React.lazy(() => import("@/comps/common-component/animation-config/AnimationConfig.tsx"));
const DataConfig = React.lazy(() => import("@/comps/common-component/data-config/DataConfig.tsx"));
const BaseTextStyleConfig = lazy(() => import("./BaseTextConfig").then((module) => ({default: module.BaseTextStyleConfig})));

export default class BaseTextDefinition extends AbstractDesignerDefinition<BaseTextController, BaseTextComponentProps> {
    getBaseInfo(): BaseInfoType {
        return {
            compName: "基础文本",
            compKey: "BaseText",
            categorize: "info",
            width: 64,
            height: 26,
        };
    }
    getChartImg(): string | null {
        return baseTextImg;
    }

    getController(): ClazzTemplate<BaseTextController> | null {
        return BaseTextController;
    }

    getInitConfig(): BaseTextComponentProps {
        return {
            base: {
                id: "",
                name: '基础文本',
                type: 'BaseText',
            },
            style: {
                color: '#a7a7a7',
                fontSize: 16,
                alignItems: 'center',
                justifyContent: 'center',
                strokeColor: '#ffffff',
                strokeWidth: 0,
                lineHeight: 1,
                letterSpacing: 0,
                textAlign: 'left'
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
            data: {
                sourceType: 'static',
                value: "基础文本"
            },
        };
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            style: BaseTextStyleConfig,
            animation: AnimationConfig,
            data: DataConfig
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
