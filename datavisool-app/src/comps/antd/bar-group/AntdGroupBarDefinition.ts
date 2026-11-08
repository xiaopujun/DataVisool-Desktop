/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import groupBarImg from "./group-bar.webp";
import {AntdBarProps} from "@/comps/antd-common/bar/AntdCommonBarController";
import AbstractBarDefinition from "@/comps/antd-common/bar/AbstractBarDefinition";
import {BaseInfoType} from "@datavisool/sdk/types";

class AntdGroupBarDefinition extends AbstractBarDefinition {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd分组条形图",
            compKey: "AntdGroupBar",
            categorize: "chart",
            subCategorize: "bar",
        };
    }
    getChartImg(): string {
        return groupBarImg;
    }

    getInitConfig(): AntdBarProps {
        const data = [
            {
                label: "Mon.",
                type: "series1",
                value: 2120
            },
            {
                label: "Mon.",
                type: "series2",
                value: 2260
            },
            {
                label: "Tues.",
                type: "series1",
                value: 1800
            },
            {
                label: "Tues.",
                type: "series2",
                value: 1300
            },
            {
                label: "Wed.",
                type: "series1",
                value: 950
            },
            {
                label: "Wed.",
                type: "series2",
                value: 900
            }
        ];
        return {
            base: {
                id: "",
                name: "Antd分组条形图",
                type: "AntdGroupBar",
            },
            style: {
                data: data,
                isGroup: true,
                xField: "value",
                yField: "label",
                seriesField: "type",
                color: [
                    "#00a6ffff",
                    "#5dd1ffff"
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
                            stroke: "#a2a2a273",
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
                maxBarWidth: 11,
                supportCSSTransform: true,
                animation: {
                    appear: {
                        animation: "scale-in-x",
                        duration: 3000
                    }
                },
                label: {
                    position: "right",
                    style: {
                        fill: "#efefef",
                        fontSize: 10
                    }
                }
            },
            filter: {
                enable: false,
                blur: 0,
                brightness: 1,
                contrast: 1,
                opacity: 1,
                saturate: 1,
                hueRotate: 0
            },
            data: {
                sourceType: 'static',
                value: data
            },
        };
    }
}

export default AntdGroupBarDefinition;
