/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import stackAreaImg from "./stack-area.webp";
import AbstractAreaDefinition from "@/comps/antd-common/area/AbstractAreaDefinition";
import {AntdAreaProps} from "@/comps/antd-common/area/AntdCommonAreaController";
import {BaseInfoType} from "@datavisool/sdk/types";

class AntdStackAreaDefinition extends AbstractAreaDefinition {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd堆叠面积图",
            compKey: "AntdStackArea",
            categorize: "chart",
            subCategorize: "area",
        };
    }
    getChartImg(): string {
        return stackAreaImg;
    }

    getInitConfig(): AntdAreaProps {
        const data = [
            {
                country: "北美",
                year: 1965,
                value: 1390.5
            },
            {
                country: "北美",
                year: 1966,
                value: 1469.5
            },
            {
                country: "北美",
                year: 1967,
                value: 1521.7
            },
            {
                country: "北美",
                year: 1968,
                value: 1615.9
            },
            {
                country: "北美",
                year: 1969,
                value: 1703.7
            },
            {
                country: "中南美",
                year: 1965,
                value: 1109.2
            },
            {
                country: "中南美",
                year: 1966,
                value: 615.7
            },
            {
                country: "中南美",
                year: 1967,
                value: 720.5
            },
            {
                country: "中南美",
                year: 1968,
                value: 1128
            },
            {
                country: "中南美",
                year: 1969,
                value: 434.4
            },
            {
                country: "欧洲",
                year: 1965,
                value: 1058.1
            },
            {
                country: "欧洲",
                year: 1966,
                value: 1089.7
            },
            {
                country: "欧洲",
                year: 1967,
                value: 1121.7
            },
            {
                country: "欧洲",
                year: 1968,
                value: 1196.6
            },
            {
                country: "欧洲",
                year: 1969,
                value: 1285.5
            }
        ];
        return {
            base: {
                id: "",
                name: '堆叠面积图',
                type: 'AntdStackArea',
            },
            style: {
                data: data,
                xField: "year",
                yField: "value",
                seriesField: "country",
                smooth: true,
                supportCSSTransform: true,
                color: [
                    "l(90) 0:#016eb7 1:#016eb700",
                    "l(90) 0:#0489e2 1:#016eb700",
                    "l(90) 0:#0099ff 1:#016eb700"
                ],
                point: {
                    size: 0,
                    color: [
                        "#4fa3ff67",
                        "#009aff67",
                        "#006d7f67"
                    ],
                    style: {
                        stroke: "#ffffff",
                        lineWidth: 0
                    },
                    shape: "circle"
                },
                line: {
                    style: {
                        lineWidth: 2
                    },
                    color: [
                        "#0099ff"
                    ]
                },
                areaStyle: {
                    fillOpacity: 1
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
                    title: null,
                    position: "bottom",
                    tickCount: 3
                },
                yAxis: {
                    grid: null,
                    label: {
                        style: {
                            fill: "#f9f9f9",
                            fontSize: 12
                        }
                    },
                    line: {
                        style: {
                            stroke: "#9e9e9e6e",
                            lineWidth: 1
                        }
                    },
                    tickLine: null,
                    subTickLine: null,
                    title: null,
                    position: "left",
                    tickCount: 3
                },
                legend: {
                    position: "top",
                    layout: "horizontal",
                    itemName: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 10
                        }
                    }
                },
                animation: {
                    appear: {
                        animation: "wave-in",
                        duration: 3000
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

export default AntdStackAreaDefinition;
