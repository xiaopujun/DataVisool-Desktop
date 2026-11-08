/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import FlvPlayerImg from './flv-player.webp';
import {FlvPlayerController} from "./FlvPlayerController.ts";
import {FlvPlayerComponentProps} from "./FlvPlayerComponent.tsx";
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";
import {lazy} from "react";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {EventInfo} from "@datavisool/sdk/types";

const AnimationConfig = lazy(() => import("@/comps/common-component/animation-config/AnimationConfig.tsx"));
const FlvPlayerConfig = lazy(() => import("./FlvPlayerConfig.tsx").then((module) => ({default: module.FlvPlayerConfig})));

export default class FlvPlayerDefinition extends AbstractDesignerDefinition<FlvPlayerController, FlvPlayerComponentProps> {
    getBaseInfo(): BaseInfoType {
        return {
            compName: "FLV视频流",
            compKey: "FlvPlayer",
            categorize: "media",
        };
    }
    getChartImg(): string | null {
        return FlvPlayerImg;
    }

    getController(): ClazzTemplate<FlvPlayerController> | null {
        return FlvPlayerController;
    }

    getInitConfig(): FlvPlayerComponentProps {
        return {
            base: {
                id: "",
                name: 'FLV视频流',
                type: 'FlvPlayer',
            },
            style: {
                url: ''
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
        return super.getMenuList().filter((item: MenuInfo) => (item.key !== 'theme' && item.key !== 'data' && item.key !== 'mapping'));
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            style: FlvPlayerConfig,
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
