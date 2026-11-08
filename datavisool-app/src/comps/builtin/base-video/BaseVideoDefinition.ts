/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {EventInfo} from "@datavisool/sdk/types";
import BaseVideoController, {BaseVideoComponentProps} from "./BaseVideoController.ts";
import baseVideo from './baseVideo.webp';
import {lazy} from "react";
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";

const BaseVideoStyleConfig = lazy(() => import("./BaseVideoConfig.tsx").then((module) => ({default: module.BaseVideoStyleConfig})));

export default class BaseVideoDefinition extends AbstractDesignerDefinition<BaseVideoController, BaseVideoComponentProps> {
    getBaseInfo(): BaseInfoType {
        return {
            compName: "视频",
            compKey: "BaseVideo",
            categorize: "media",
        };
    }
   getChartImg(): string | null {
        return baseVideo;
    }

    getController(): ClazzTemplate<BaseVideoController> | null {
        return BaseVideoController;
    }

    getInitConfig(): BaseVideoComponentProps {
        return {
            base: {
                id: "",
                name: '视频',
                type: 'BaseVideo',
            },
            style: {
                src: undefined,
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
            style: BaseVideoStyleConfig
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
