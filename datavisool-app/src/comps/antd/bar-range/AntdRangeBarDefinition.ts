/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import rangeBarImg from "./range-bar.webp";
import {AntdBarProps} from "@/comps/antd-common/bar/AntdCommonBarController";
import AbstractBarDefinition from "@/comps/antd-common/bar/AbstractBarDefinition";
import {BaseInfoType} from "@datavisool/sdk/types";

class AntdRangeBarDefinition extends AbstractBarDefinition {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd区间条形图",
            compKey: "AntdRangeBar",
            categorize: "chart",
            subCategorize: "bar",
        };
    }
    getChartImg(): string {
        return rangeBarImg;
    }

    getInitConfig(): AntdBarProps {
        const data =  [
            {
                type: "sort1",
                values: [
                    16,
                    130
                ]
            },
            {
                type: "sort2",
                values: [
                    26,
                    135
                ]
            },
            {
                type: "sort3",
                values: [
                    36,
                    110
                ]
            },
            {
                type: "sort4",
                values: [
                    20,
                    110
                ]
            }
        ];
        return {
            base: {
                id: "",
                name: 'Antd区间条形图',
                type: 'AntdRangeBar',
            },
            style: {
                data: [
                    {
                        type: "sort1",
                        values: [
                            16,
                            130
                        ]
                    },
                    {
                        type: "sort2",
                        values: [
                            26,
                            135
                        ]
                    },
                    {
                        type: "sort3",
                        values: [
                            36,
                            110
                        ]
                    },
                    {
                        type: "sort4",
                        values: [
                            20,
                            110
                        ]
                    }
                ],
                xField: "values",
                yField: "type",
                seriesField: "type",
                isRange: true,
                color: [
                    "#2ea4ffff",
                    "#7cdaffff",
                    "#6cc2ffff"
                ],
                barStyle: {
                    radius: 5
                },
                xAxis: {
                    grid: null,
                    label: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 14
                        }
                    },
                    line: {
                        style: {
                            stroke: "#b9b9b975",
                            lineWidth: 1
                        }
                    },
                    tickLine: null,
                    subTickLine: null,
                    position: "right",
                    title: null
                },
                yAxis: {
                    grid: null,
                    label: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 14
                        }
                    },
                    line: null,
                    tickLine: null,
                    subTickLine: null,
                    position: "bottom",
                    title: null
                },
                legend: {
                    position: "top",
                    layout: "horizontal",
                    itemName: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 14
                        }
                    },
                    itemSpacing: 2
                },
                maxBarWidth: 12,
                supportCSSTransform: true,
                animation: {
                    appear: {
                        animation: "scale-in-x",
                        duration: 3000
                    }
                },
                label: {
                    position: "top",
                    style: {
                        fill: "#cccccc",
                        fontSize: 10
                    }
                }
            },
            data: {
                sourceType: 'static',
                value: data
            },
        };
    }
}

export default AntdRangeBarDefinition;
