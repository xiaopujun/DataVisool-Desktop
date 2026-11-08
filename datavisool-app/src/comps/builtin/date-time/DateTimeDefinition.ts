/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import DateTimeImg from './date-time.webp';
import {DateTimeController} from "./DateTimeController.ts";
import {DateTimeComponentProps} from "./DateTimeComponent.tsx";
import {FormatType} from "./DateTimeTypes.ts";
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {EventInfo} from "@datavisool/sdk/types";
import {lazy} from "react";

const DateTimeConfig = lazy(() => import("./DateTimeConfig.tsx").then((module) => ({default: module.DateTimeConfig})));

export default class DateTimeDefinition extends AbstractDesignerDefinition<DateTimeController, DateTimeComponentProps> {
    getBaseInfo(): BaseInfoType {
        return {
            compName: "当前时间",
            compKey: "DateTime",
            categorize: "info",
            subCategorize: "text",
            width: 320,
            height: 26,
        };
    }
    getChartImg(): string | null {
        return DateTimeImg;
    }

    getController(): ClazzTemplate<DateTimeController> | null {
        return DateTimeController;
    }

    getInitConfig(): DateTimeComponentProps {
        return {
            base: {
                id: "",
                name: '当前时间',
                type: 'DateTime',
            },
            style: {
                fontSize: 16,
                fontWeight: 500,
                color: '#fff',
                fontFamily: 'Arial',
                alignItems: 'center',
                justifyContent: 'center',
                letterSpacing: '1',
                formatType: FormatType.CN
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
        const menuMapping = super.getMenuToConfigContentMap();
        menuMapping['style'] = DateTimeConfig;
        return menuMapping;
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
