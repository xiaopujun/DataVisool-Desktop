/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import React from "react";
import {getDefaultMenuList} from "@/designer/right/util";
import pieImg from './pie.webp';
import AntdPieController, {AntdPieProps} from "./AntdPieController";
import {AntdCommonDefinition} from "@/comps/antd-common/AntdCommonDefinition";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";

const AnimationConfig = React.lazy(() => import("@/comps/common-component/animation-config/AnimationConfig"));
const AntdPieConfig = React.lazy(() => import("./AntdPieStyleConfig"));
const AntdPieFieldMapping = React.lazy(() => import("./AntdPieStyleConfig").then((module) => ({default: module.AntdPieFieldMapping})));
const DataConfig = React.lazy(() => import("@/comps/common-component/data-config/DataConfig"));

class AntdPieDefinition extends AntdCommonDefinition<AntdPieController, AntdPieProps> {

    getController(): ClazzTemplate<AntdPieController> | null {
        return AntdPieController;
    }

    getMenuList(): Array<MenuInfo> {
        return getDefaultMenuList();
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            data: DataConfig,
            style: AntdPieConfig,
            animation: AnimationConfig,
            mapping: AntdPieFieldMapping
        };
    }

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd饼图",
            compKey: "AntdPie",
            categorize: "chart",
            subCategorize: "pie",
        };
    }
    getChartImg(): string | null {
        return pieImg;
    }

    getInitConfig(): AntdPieProps {
        const data = [
            {type: 'sort1', value: 30},
            {type: 'sort2', value: 25},
            {type: 'sort3', value: 20},
            {type: 'sort4', value: 25}
        ];
        return {
            base: {
                id: "",
                name: 'Antd饼图',
                type: 'AntdPie',
            },
            style: {
                data: data,
                supportCSSTransform: true,
                angleField: "value",
                colorField: "type",
                radius: 0.7,
                innerRadius: 0.6,
                startAngle: 0,
                endAngle: 6.283185307179586,
                pieStyle: {
                    stroke: "#fff",
                    lineWidth: 0,
                },
                label: {
                    type: "outer",
                    offset: 27,
                    content: "{name} {percentage}",
                    autoRotate: true,
                    rotate: 0,
                    style: {
                        textAlign: "center",
                        fontSize: 12,
                        fontWeight: 400,
                        fill: "#ffffff",
                        stroke: "#f10d0d"
                    },
                    labelLine: false
                },
                legend: {
                    position: "right",
                    layout: "vertical",
                    itemName: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 14
                        }
                    }
                },
                interactions: [
                    {
                        type: "element-selected"
                    },
                    {
                        type: "element-active"
                    }
                ],
                statistic: {
                    title: {
                        style: {
                            fontSize: '12',
                            color: "#ffffff",
                            fontWeight: 300
                        },
                        content: "总计",
                        offsetY: -6
                    },
                    content: {
                        style: {
                            fontSize: "16px",
                            color: "#ffffff",
                            fontWeight: 400
                        },
                        content: "100",
                        offsetY: 2,
                        offsetX: -2
                    }
                },
                animation: {
                    appear: {
                        animation: "wave-in",
                        duration: 3000
                    }
                },
                color: [
                    "l(90) 0:#4ca9ff 1:#0082d9",
                    "l(90) 0:#8f4dff 1:#f0b3fc",
                    "l(90) 0:#ff5e62 1:#ff9966",
                    "l(90) 0:#28f7ac 1:#348ac7"
                ]
            },
            data: {
                sourceType: 'static',
                value: data
            },
        };
    }
}

export default AntdPieDefinition;
