/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import baseBarImg from "./base-bar.webp";
import {AntdBarProps} from "@/comps/antd-common/bar/AntdCommonBarController";
import AbstractBarDefinition from "@/comps/antd-common/bar/AbstractBarDefinition";
import {BaseInfoType} from "@datavisool/sdk/types";

class AntdBaseBarDefinition extends AbstractBarDefinition {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd基础条形图",
            compKey: "AntdBaseBar",
            categorize: "chart",
            subCategorize: "bar",
        };
    }
    getChartImg(): string {
        return baseBarImg;
    }

    getInitConfig(): AntdBarProps {
        const data = [
            {
                name: "1951 年",
                value: 48
            },
            {
                name: "1952 年",
                value: 52
            },
            {
                name: "1956 年",
                value: 22
            }
        ]
        return {
            base: {
                id: "",
                name: '基础条形图',
                type: 'AntdBaseBar',
            },
            style: {
                data: data,
                label: {
                    style: {
                        fill: "#ffffff",
                        fontWeight: 500
                    },
                    position: "right"
                },
                xField: "value",
                yField: "name",
                seriesField: "name",
                maxBarWidth: 12,
                color: [
                    "l(135) 0:#43cbff 1:#0090d9",
                    "l(135) 0:#3e95ff 1:#256eff",
                    "l(135) 0:#428bff 1:#7e64ff"
                ],
                barStyle: {
                    radius: 6
                },
                supportCSSTransform: true,
                xAxis: {
                    grid: null,
                    label: {
                        style: {
                            fill: "#d9d9d9",
                            fontSize: 12
                        }
                    },
                    line: {
                        style: {
                            stroke: "#bcbcbc91",
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
                            fill: "#d6d6d6",
                            fontSize: 12
                        },
                        offset: 9
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
                            fill: "#e4e4e4",
                            fontSize: 12
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

export default AntdBaseBarDefinition;
