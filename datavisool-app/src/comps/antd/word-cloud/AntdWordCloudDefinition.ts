/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import baseWordCloudImg from "./word-cloud.webp";
import AntdWordCloudController, {AntdWordCloudProps} from "./AntdWordCloudController.ts";
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";
import {Deeplink} from "@icon-park/react";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";
import {EventInfo} from "@datavisool/sdk/types";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {lazy} from "react";

const AntdWordCloudStyle = lazy(() => import("./AntdWordCloudConfig.tsx").then((module) => ({default: module.AntdWordCloudStyle})));
const AntdWordCloudFieldMapping = lazy(() => import("./AntdWordCloudConfig.tsx").then((module) => ({default: module.AntdWordCloudFieldMapping})));

class AntdWordCloudDefinition extends AbstractDesignerDefinition<AntdWordCloudController, AntdWordCloudProps> {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd词云图",
            compKey: "AntdWordCloud",
            categorize: "chart",
            subCategorize: "wordCloud",
        };
    }

    getEventList(): EventInfo[] {
        const eventList = super.getEventList();
        eventList.push(...[
            {
                id: "dataChange",
                name: "数据变更时",
            },
            {
                id: "globalClick",
                name: "点击整个组件时",
            },
            {
                id: "elementClick",
                name: "点击图形数据元素时",
            },
        ])
        return eventList;
    }

    getController(): ClazzTemplate<AntdWordCloudController> | null {
        return AntdWordCloudController
    }

    getMenuList(): MenuInfo[] {
        const menus = super.getMenuList();
        menus.splice(3, 0, {
            icon: Deeplink,
            name: '映射',
            key: 'mapping',
        })
        return menus;
    }

    getChartImg(): string {
        return baseWordCloudImg;
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        const menuMapping = super.getMenuToConfigContentMap();
        menuMapping['style'] = AntdWordCloudStyle;
        menuMapping['mapping'] = AntdWordCloudFieldMapping;
        return menuMapping;
    }

    getInitConfig(): AntdWordCloudProps {
        const data = [
            {
                value: 9,
                name: "AntV"
            },
            {
                value: 8,
                name: "F2"
            },
            {
                value: 8,
                name: "G2"
            },
            {
                value: 8,
                name: "G6"
            },
            {
                value: 8,
                name: "DataSet"
            },
            {
                value: 8,
                name: "墨者学院"
            },
            {
                value: 6,
                name: "Analysis"
            },
            {
                value: 6,
                name: "Data Mining"
            },
            {
                value: 6,
                name: "Data Vis"
            },
            {
                value: 6,
                name: "Design"
            },
            {
                value: 6,
                name: "Grammar"
            },
            {
                value: 6,
                name: "Graphics"
            },
            {
                value: 6,
                name: "Graph"
            },
            {
                value: 6,
                name: "Hierarchy"
            },
            {
                value: 6,
                name: "Labeling"
            },
            {
                value: 6,
                name: "Layout"
            },
            {
                value: 6,
                name: "Quantitative"
            },
        ];
        return {
            base: {
                id: "",
                name: 'Antd词云图',
                type: 'AntdWordCloud',
            },
            style: {
                data,
                wordField: "name",
                weightField: "value",
                colorField: "name",
                supportCSSTransform: true,
                spiral: "rectangular",
                color: [
                    "#2294ff",
                    "#ff8934",
                    "#f370ff"
                ],
                wordStyle: {
                    fontFamily: "DingTalk JinBuTi",
                    fontWeight: 500,
                    fontSize: [
                        14,
                        37
                    ],
                    rotation: 0,
                    padding: 7
                }
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
                value: data
            },
        };
    }
}

export default AntdWordCloudDefinition;
