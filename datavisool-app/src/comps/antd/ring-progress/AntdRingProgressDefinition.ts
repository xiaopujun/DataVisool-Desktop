/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import React from "react";
import {getDefaultMenuList} from "@/designer/right/util";
import AntdRingProgressController, {AntdRingProgressProps} from "./AntdRingProgressController";
import ringProgressImg from './ring-progress.webp';
import {AntdCommonDefinition} from "@/comps/antd-common/AntdCommonDefinition";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {EventInfo} from "@datavisool/sdk/types";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";

const AnimationConfig = React.lazy(() => import("@/comps/common-component/animation-config/AnimationConfig"));
const DataConfig = React.lazy(() => import("@/comps/common-component/data-config/DataConfig"));
const AntdRingProgressStyleConfig = React.lazy(() => import("./AntdRingProgressConfig").then((module) => ({default: module.AntdRingProgressStyleConfig})));

class AntdRingProgressDefinition extends AntdCommonDefinition<AntdRingProgressController, AntdRingProgressProps> {

    getController(): ClazzTemplate<AntdRingProgressController> | null {
        return AntdRingProgressController;
    }

    getMenuList(): Array<MenuInfo> {
        return getDefaultMenuList().filter((menuInfo) => menuInfo.key !== "mapping");
    }

    getEventList(): EventInfo[] {
        return [
            {
                id: "loaded",
                name: "组件加载完成时",
            },
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
        ];
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            data: DataConfig,
            style: AntdRingProgressStyleConfig,
            animation: AnimationConfig
        };
    }

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd迷你环图",
            compKey: "AntdRingProgress",
            categorize: "chart",
            subCategorize: "progress",
        };
    }
    getChartImg(): string | null {
        return ringProgressImg;
    }

    getInitConfig(): AntdRingProgressProps {
        return {
            base: {
                id: "",
                name: 'Antd迷你环图',
                type: 'AntdRingProgress',
            },
            style: {
                percent: 0.7,
                color: [
                    "l(140) 0:#9cd1ff 0.81:#0084ff",
                    "#a9a9a900"
                ],
                statistic: {
                    content: {
                        style: {
                            fill: "#fff",
                            fontSize: '21',
                            fontWeight: 500,
                            color: "#ffffff"
                        },
                        offsetY: 7,
                        offsetX: 0
                    },
                    title: {
                        style: {
                            fontSize: '15',
                            fontWeight: 500,
                            color: "#ffffff"
                        },
                        content: "指标1",
                        offsetY: -11
                    }
                },
                animation: {
                    appear: {
                        animation: "wave-in",
                        duration: 3000
                    }
                },
                progressStyle: {
                    lineWidth: 0,
                    stroke: "l(90) 0:#78cdff 1:#a481ff"
                }
            },
            data: {
                sourceType: 'static',
                value: 0.7,
            },
        };
    }
}

export default AntdRingProgressDefinition;
