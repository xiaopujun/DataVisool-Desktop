/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import HlsPlayerImg from './hls-player.webp';
import {HlsPlayerController} from "./HlsPlayerController.ts";
import {HlsPlayerComponentProps} from "./HlsPlayerComponent.tsx";
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";
import {lazy} from "react";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {EventInfo} from "@datavisool/sdk/types";

const AnimationConfig = lazy(() => import("@/comps/common-component/animation-config/AnimationConfig.tsx"));
const HlsPlayerConfig = lazy(() => import("./HlsPlayerConfig.tsx").then((module) => ({default: module.HlsPlayerConfig})));

export default class HlsPlayerDefinition extends AbstractDesignerDefinition<HlsPlayerController, HlsPlayerComponentProps> {
    getBaseInfo(): BaseInfoType {
        return {
            compName: "HLS视频流",
            compKey: "HlsPlayer",
            categorize: "media",
        };
    }
    getChartImg(): string | null {
        return HlsPlayerImg;
    }

    getController(): ClazzTemplate<HlsPlayerController> | null {
        return HlsPlayerController;
    }

    getInitConfig(): HlsPlayerComponentProps {
        return {
            base: {
                id: "",
                name: 'HLS视频流',
                type: 'HlsPlayer',
            },
            style: {
                url: 'https://test-streams.mux.dev/x36xhzz/x36xhzz.m3u8'
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
            style: HlsPlayerConfig,
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
