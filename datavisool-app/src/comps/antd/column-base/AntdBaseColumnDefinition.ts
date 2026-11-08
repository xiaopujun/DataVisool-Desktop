/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import baseColumnImg from "./base-column.webp";
import {AntdColumnProps} from "@/comps/antd-common/column/AntdCommonColumnController";
import AbstractColumnDefinition from "@/comps/antd-common/column/AbstractColumnDefinition";
import {BaseInfoType} from "@datavisool/sdk/types";

class AntdBaseColumnDefinition extends AbstractColumnDefinition {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd基础柱状图",
            compKey: "AntdBaseColumn",
            categorize: "chart",
            subCategorize: "column",
        };
    }
    getChartImg(): string {
        return baseColumnImg;
    }

    getInitConfig(): AntdColumnProps {
        const data = [
            {
                name: "1960 年",
                value: 35
            },
            {
                name: "1965 年",
                value: 42
            },
            {
                name: "1970 年",
                value: 58
            },
            {
                name: "1975 年",
                value: 50
            },
            {
                name: "1980 年",
                value: 44
            }
        ]
        return {
            base: {
                id: "",
                name: 'Antd基础柱状图',
                type: 'AntdBaseColumn',
            },
            style: {
                data: data,
                xField: "name",
                yField: "value",
                seriesField: "name",
                maxColumnWidth: 18,
                supportCSSTransform: true,
                color: [
                    "l(90) 0:#71aaff 1:#008cff"
                ],
                columnStyle: {
                    radius: 5
                },
                xAxis: {
                    grid: null,
                    label: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 12
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
                            fontSize: 12
                        }
                    },
                    line: {
                        style: {
                            stroke: "#ffffffa3",
                            lineWidth: 1
                        }
                    },
                    tickLine: null,
                    subTickLine: null,
                    position: "left",
                    title: null,
                    tickCount: 4
                },
                legend: {
                    position: "top-right",
                    layout: "horizontal",
                    itemName: {
                        style: {
                            fill: "#f9f5f5",
                            fontSize: 12
                        }
                    },
                    itemSpacing: 3,
                    marker: {
                        spacing: 7
                    }
                },
                animation: {
                    appear: {
                        animation: "scale-in-y",
                        duration: 3000
                    }
                },
                label: {
                    position: "top",
                    style: {
                        fill: "#ffffff"
                    },
                    offsetY: 9
                }
            },
            data: {
                sourceType: 'static',
                value: data
            },
        };
    }
}

export default AntdBaseColumnDefinition;
