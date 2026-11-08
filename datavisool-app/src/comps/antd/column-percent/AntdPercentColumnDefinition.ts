/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import percentColumnImg from "./percent-column.webp";
import AbstractColumnDefinition from "@/comps/antd-common/column/AbstractColumnDefinition";
import {AntdColumnProps} from "@/comps/antd-common/column/AntdCommonColumnController";
import {BaseInfoType} from "@datavisool/sdk/types";

class AntdPercentColumnDefinition extends AbstractColumnDefinition {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd百分比柱状图",
            compKey: "AntdPercentColumn",
            categorize: "chart",
            subCategorize: "column",
        };
    }
    getChartImg(): string {
        return percentColumnImg;
    }

    getInitConfig(): AntdColumnProps {
        const data = [
            {
                country: "Asia",
                year: "1750",
                value: 502
            },
            {
                country: "Asia",
                year: "1800",
                value: 635
            },
            {
                country: "Asia",
                year: "1850",
                value: 809
            },
            {
                country: "Africa",
                year: "1750",
                value: 106
            },
            {
                country: "Africa",
                year: "1800",
                value: 107
            },
            {
                country: "Africa",
                year: "1850",
                value: 111
            },
            {
                country: "Europe",
                year: "1750",
                value: 163
            },
            {
                country: "Europe",
                year: "1800",
                value: 203
            },
            {
                country: "Europe",
                year: "1850",
                value: 276
            }
        ];
        return {
            base: {
                id: "",
                name: 'Antd百分比柱状图',
                type: 'AntdPercentColumn',
            },
            style: {
                data: data,
                xField: "year",
                yField: "value",
                seriesField: "country",
                isPercent: true,
                isStack: true,
                maxColumnWidth: 18,
                supportCSSTransform: true,
                color: [
                    "#59bfff",
                    "#298aff",
                    "#004294"
                ],
                columnStyle: {
                    radius: 6
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
                            stroke: "#7c7c7c",
                            lineWidth: 1
                        }
                    },
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
                    line: null,
                    tickLine: null,
                    subTickLine: null,
                    position: "left",
                    title: null,
                    tickCount: 3
                },
                legend: {
                    position: "top",
                    layout: "horizontal",
                    itemName: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 13
                        }
                    }
                },
                animation: {
                    appear: {
                        animation: "scale-in-y",
                        duration: 3000
                    }
                },
                label: {
                    position: "left",
                    style: {
                        fontSize: 0
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

export default AntdPercentColumnDefinition;
