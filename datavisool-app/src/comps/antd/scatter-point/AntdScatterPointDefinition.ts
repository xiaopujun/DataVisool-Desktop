/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import scatterPoint from "./scatter-point.webp";
import AbstractScatterDefinition from "@/comps/antd-common/scatter/AbstractScatterDefinition";
import {AntdScatterProps} from "@/comps/antd-common/scatter/AntdCommonScatterController";
import {BaseInfoType} from "@datavisool/sdk/types";

class AntdScatterPointDefinition extends AbstractScatterDefinition {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd散点图",
            compKey: "AntdScatterPoint",
            categorize: "chart",
            subCategorize: "scatter",
        };
    }
    getChartImg(): string {
        return scatterPoint;
    }

    getInitConfig(): AntdScatterProps {
        const data = [
            {
                x: 1,
                y: 23,
                type: "one",
            },
            {
                x: 2,
                y: 15,
                type: "two",
            },
            {
                x: 3,
                y: 17,
                type: "three",
            },
            {
                x: 4,
                y: 48,
                type: "four",
            },
            {
                x: 5,
                y: 8,
                type: "five",
            },
        ];
        return {
            base: {
                id: "",
                name: 'Antd散点图',
                type: 'AntdScatterPoint',
            },
            style: {
                data,
                xField: "x",
                yField: "y",
                colorField: "type",
                sizeField: "y",
                size: 17,
                shape: "circle",
                pointStyle: {
                    fillOpacity: 1,
                    lineWidth: 0
                },
                xAxis: {
                    grid: null,
                    label: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 12
                        }
                    },
                    line: {
                        style: {
                            stroke: "#a4a4a470",
                            lineWidth: 1
                        }
                    },
                    tickLine: null,
                    subTickLine: null,
                    position: "bottom",
                    title: null,
                    tickCount: 3
                },
                yAxis: {
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
                    position: "left",
                    title: null,
                    tickCount: 4
                },
                legend: {
                    position: "top",
                    layout: "horizontal",
                    itemName: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 14
                        }
                    }
                },
                supportCSSTransform: true,
                animation: {
                    appear: {
                        animation: "fade-in",
                        duration: 3000
                    }
                },
                color: [
                    "#00b7ffff",
                    "#006d98ff"
                ]
            },
            data: {
                sourceType: 'static',
                value: data
            },
        };
    }
}

export default AntdScatterPointDefinition;
