/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import multiLineImg from "./multi-line.webp";
import AbstractLineDefinition from "@/comps/antd-common/line/AbstractLineDefinition";
import {AntdLineProps} from "@/comps/antd-common/line/AntdCommonLineController";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuToConfigMappingType} from "@datavisool/sdk/core";
import {lazy} from "react";

const AntdMultiLineStyleConfig = lazy(() => import("./AntdMultiLineConfig.tsx"));
const AntdMultiLineFieldMapping = lazy(() => import("./AntdMultiLineConfig.tsx").then((module) => ({default: module.AntdMultiLineFieldMapping})));

class AntdMultiLineDefinition extends AbstractLineDefinition {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd多折线图",
            compKey: "AntdMultiLine",
            categorize: "chart",
            subCategorize: "line",
        };
    }
    getChartImg(): string {
        return multiLineImg;
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        const menuToConfigContentMap = super.getMenuToConfigContentMap();
        menuToConfigContentMap!['style'] = AntdMultiLineStyleConfig;
        menuToConfigContentMap!['mapping'] = AntdMultiLineFieldMapping;
        return menuToConfigContentMap;
    }

    getInitConfig(): AntdLineProps {
        const data = [
            {
                year: "2011",
                value: 1523,
                category: "sort1"
            },
            {
                year: "2011",
                value: 634,
                category: "sort2"
            },
            {
                year: "2012",
                value: 1788,
                category: "sort1"
            },
            {
                year: "2012",
                value: 502,
                category: "sort2"
            },
            {
                year: "2013",
                value: 1620,
                category: "sort1"
            },
            {
                year: "2013",
                value: 715,
                category: "sort2"
            },
            {
                year: "2014",
                value: 1487,
                category: "sort1"
            },
            {
                year: "2014",
                value: 482,
                category: "sort2"
            },
            {
                year: "2015",
                value: 1690,
                category: "sort1"
            },
            {
                year: "2015",
                value: 659,
                category: "sort2"
            },
            {
                year: "2016",
                value: 1540,
                category: "sort1"
            },
            {
                year: "2016",
                value: 530,
                category: "sort2"
            },
            {
                year: "2017",
                value: 1725,
                category: "sort1"
            },
            {
                year: "2017",
                value: 685,
                category: "sort2"
            },
            {
                year: "2018",
                value: 1582,
                category: "sort1"
            },
            {
                year: "2018",
                value: 470,
                category: "sort2"
            },
            {
                year: "2019",
                value: 1667,
                category: "sort1"
            },
            {
                year: "2019",
                value: 540,
                category: "sort2"
            },
            {
                year: "2020",
                value: 1604,
                category: "sort1"
            },
            {
                year: "2020",
                value: 612,
                category: "sort2"
            }
        ]

        return {
            base: {
                id: "",
                name: '多折线图',
                type: 'AntdMultiLine',
            },
            style: {
                data: data,
                xField: "year",
                yField: "value",
                seriesField: "category",
                supportCSSTransform: true,
                color: [
                    "l(0) 0:#266bff 1:#af47ff",
                    "l(0) 0:#ff9a6e 1:#ff5e00"
                ],
                point: {
                    color: [
                        "#00a8ff",
                        "#00ffc9"
                    ],
                    size: 0,
                    shape: "circle",
                    style: {
                        stroke: "#00a8ff",
                        lineWidth: 0
                    }
                },
                lineStyle: {
                    lineWidth: 4
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
                            stroke: "#adadad75",
                            lineWidth: 1
                        }
                    },
                    tickLine: null,
                    subTickLine: null,
                    position: "left",
                    title: null
                },
                smooth: true,
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
                        animation: "wave-in",
                        duration: 3000
                    }
                },
                label: {
                    position: "middle",
                    style: {
                        fontSize: 8,
                        fill: "#ffffff"
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

export default AntdMultiLineDefinition;
