/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {EventInfo} from "@datavisool/sdk/types";
import BaseImageController, {type BaseImageComponentProps} from "./BaseImageController";
import baseImage from './baseImage.webp';
import {lazy} from "react";
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";

const BaseImageStyleConfig = lazy(() => import("./BaseImageConfig").then((module) => ({default: module.BaseImageStyleConfig})));
const DataConfig = lazy(() => import("@/comps/common-component/data-config/DataConfig.tsx"));

export default class BaseImageDefinition extends AbstractDesignerDefinition<BaseImageController, BaseImageComponentProps> {
    getBaseInfo(): BaseInfoType {
        return {
            compName: "基础图片",
            compKey: "BaseImage",
            categorize: "media",
        };
    }
   getChartImg(): string | null {
        return baseImage;
    }

    getController(): ClazzTemplate<BaseImageController> | null {
        return BaseImageController;
    }

    getInitConfig(): BaseImageComponentProps {
        return {
            base: {
                id: "",
                name: '基础图片',
                type: 'BaseImage',
            },
            style: {
                type: 'online',
                localUrl: '',
                onLineUrl: '',
                opacity: 1,
            },
            data: {
                sourceType: 'static',
                value: ''
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
        return super.getMenuList().filter((item: MenuInfo) => item.key !== 'theme');
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            style: BaseImageStyleConfig,
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
