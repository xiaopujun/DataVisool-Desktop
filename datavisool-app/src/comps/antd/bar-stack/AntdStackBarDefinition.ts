/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import stackBarImg from "./stack-bar.webp";
import {AntdBarProps} from "@/comps/antd-common/bar/AntdCommonBarController";
import AbstractBarDefinition from "@/comps/antd-common/bar/AbstractBarDefinition";
import {BaseInfoType} from "@datavisool/sdk/types";

class AntdStackBarDefinition extends AbstractBarDefinition {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd堆叠条形图",
            compKey: "AntdStackBar",
            categorize: "chart",
            subCategorize: "bar",
        };
    }
    getChartImg(): string {
        return stackBarImg;
    }

    getInitConfig(): AntdBarProps {
        const data = [
            {
                year: "1991",
                value: 300,
                type: "Lon"
            },
            {
                year: "1992",
                value: 400,
                type: "Lon"
            },
            {
                year: "1993",
                value: 350,
                type: "Lon"
            },
            {
                year: "1991",
                value: 300,
                type: "Bor"
            },
            {
                year: "1992",
                value: 400,
                type: "Bor"
            },
            {
                year: "1993",
                value: 350,
                type: "Bor"
            }
        ];
        return {
            base: {
                id: "",
                name: 'Antd堆叠条形图',
                type: 'AntdStackBar',
            },
            style: {
                data: data,
                xField: "value",
                yField: "year",
                seriesField: "type",
                isStack: true,
                maxBarWidth: 15,
                supportCSSTransform: true,
                color: [
                    "l(180) 0:#398fff 1:#62bbff",
                    "l(180) 0:#e62dff 1:#ac40ff"
                ],
                barStyle: {
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
                    line: {
                        style: {
                            stroke: "#b9b9b980",
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
                            fontSize: 12
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
                    }
                },
                animation: {
                    appear: {
                        animation: "scale-in-x",
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

export default AntdStackBarDefinition;
