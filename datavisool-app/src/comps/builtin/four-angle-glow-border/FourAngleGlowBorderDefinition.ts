/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import fourAngleGlowBorderImg from './four-angle-glow-border.webp';
import {FourAngleGlowBorderController, FourAngleGlowProps} from "./FourAngleGlowBorderController";
import {lazy} from "react";
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {EventInfo} from "@datavisool/sdk/types";

const FourAngleGlowBorderConfig = lazy(() => import("./FourAngleGlowBorderConfig").then((module) => ({default: module.FourAngleGlowBorderConfig})));

export default class FourAngleGlowBorderDefinition extends AbstractDesignerDefinition<FourAngleGlowBorderController, FourAngleGlowProps> {
    getBaseInfo(): BaseInfoType {
        return {
            compName: "四角辉光边框",
            compKey: "FourAngleGlowBorder",
            categorize: "ornament",
        };
    }
   getChartImg(): string | null {
        return fourAngleGlowBorderImg;
    }

    getController(): ClazzTemplate<FourAngleGlowBorderController> | null {
        return FourAngleGlowBorderController;
    }

    getInitConfig(): FourAngleGlowProps {
        return {
            base: {
                id: "",
                name: '基础色块',
                type: 'FourAngleGlowBorder',
            },
            style: {
                color: '#34e4ff',
                width: 2,
                radius: 4,
                length: 10,
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
        return super.getMenuList().filter((item: MenuInfo) => (item.key !== 'data'));
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            style: FourAngleGlowBorderConfig
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
