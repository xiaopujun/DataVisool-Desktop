/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import percentBarImg from "./percent-bar.webp";
import {AntdBarProps} from "@/comps/antd-common/bar/AntdCommonBarController";
import AbstractBarDefinition from "@/comps/antd-common/bar/AbstractBarDefinition";
import {BaseInfoType} from "@datavisool/sdk/types";
import {ClazzTemplate} from "@datavisool/sdk/types";
import AntdPercentBarController from "./AntdPercentBarController.ts";

class AntdPercentBarDefinition extends AbstractBarDefinition {

    getController(): ClazzTemplate<AntdPercentBarController> | null {
        return AntdPercentBarController;
    }

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd百分比条形图",
            compKey: "AntdPercentBar",
            categorize: "chart",
            subCategorize: "bar",
        };
    }

    getChartImg(): string {
        return percentBarImg;
    }

    getInitConfig(): AntdBarProps {
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
                name: 'Antd百分比条形图',
                type: 'AntdPercentBar',
            },
            style: {
                data: data,
                xField: "value",
                yField: "year",
                seriesField: "country",
                isPercent: true,
                isStack: true,
                color: [
                    "#59bfff",
                    "#298aff",
                    "#004294"
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
                            stroke: "#a4a4a473",
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
                maxBarWidth: 14,
                supportCSSTransform: true,
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

export default AntdPercentBarDefinition;
