/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import rangeColumnImg from "./range-column.webp";
import {AntdColumnProps} from "@/comps/antd-common/column/AntdCommonColumnController";
import AbstractColumnDefinition from "@/comps/antd-common/column/AbstractColumnDefinition";
import {BaseInfoType} from "@datavisool/sdk/types";

class AntdRangeColumnDefinition extends AbstractColumnDefinition {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd区间柱状图",
            compKey: "AntdRangeColumn",
            categorize: "chart",
            subCategorize: "column",
        };
    }
    getChartImg(): string {
        return rangeColumnImg;
    }

    getInitConfig(): AntdColumnProps {
        const data = [
            {
                type: "sort1",
                values: [
                    30,
                    150
                ]
            },
            {
                type: "sort2",
                values: [
                    40,
                    160
                ]
            },
            {
                type: "sort3",
                values: [
                    50,
                    165
                ]
            },
            {
                type: "sort4",
                values: [
                    60,
                    120
                ]
            }
        ];
        return {
            base: {
                id: "",
                name: 'Antd区间柱状图',
                type: 'AntdRangeColumn',
            },
            style: {
                data: [
                    {
                        type: "sort1",
                        values: [
                            30,
                            150
                        ]
                    },
                    {
                        type: "sort2",
                        values: [
                            40,
                            160
                        ]
                    },
                    {
                        type: "sort3",
                        values: [
                            50,
                            165
                        ]
                    },
                    {
                        type: "sort4",
                        values: [
                            60,
                            120
                        ]
                    }
                ],
                xField: "type",
                yField: "values",
                seriesField: "type",
                isRange: true,
                maxColumnWidth: 16,
                supportCSSTransform: true,
                color: [
                    "l(90) 0:#6e86ff 1:#007ccf"
                ],
                columnStyle: {
                    radius: 10
                },
                xAxis: {
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
                yAxis: {
                    grid: null,
                    label: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 14
                        }
                    },
                    line: {
                        style: {
                            stroke: "#9e9e9e91",
                            lineWidth: 1
                        }
                    },
                    tickLine: null,
                    subTickLine: null,
                    position: "left",
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
                    itemSpacing: 7
                },
                animation: {
                    appear: {
                        animation: "scale-in-y",
                        duration: 3000
                    }
                },
                label: {
                    position: "middle",
                    style: {
                        fill: "#ffffff",
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

export default AntdRangeColumnDefinition;
