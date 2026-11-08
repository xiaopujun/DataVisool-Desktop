/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import React from "react";
import AntdGaugeController, {AntdGaugeProps} from "./AntdGaugeController";
import gaugeImg from './gauge.webp';
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuToConfigMappingType} from "@datavisool/sdk/core";
import {EventInfo} from "@datavisool/sdk/types";

const AntdGaugeConfig = React.lazy(() => import("./AntdGaugeConfig"));

class AntdGaugeDefinition extends AbstractDesignerDefinition<AntdGaugeController, AntdGaugeProps> {
    getController(): ClazzTemplate<AntdGaugeController> | null {
        return AntdGaugeController;
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        const menus = super.getMenuToConfigContentMap();
        menus['style'] = AntdGaugeConfig;
        return menus;
    }

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd仪表盘",
            compKey: "AntdGauge",
            categorize: "chart",
            subCategorize: "progress",
        };
    }
    getChartImg(): string | null {
        return gaugeImg;
    }

    getInitConfig(): AntdGaugeProps {
        return {
            base: {
                id: "",
                name: 'Antd仪表盘',
                type: 'AntdGauge',
            },
            style: {
                percent: 0.75,
                radius: 0.9,
                innerRadius: 0.85,
                startAngle: -3.6651914291880923,
                endAngle: 0.5235987755982988,
                range: {
                    color: [
                        "l(128) 0:#3c90ff 1:#934bff",
                        "#5d9eff00"
                    ]
                },
                indicator: {
                    pointer: {
                        style: {
                            stroke: "#37b3ff",
                            lineWidth: 2
                        }
                    },
                    pin: {
                        style: {
                            stroke: "#37b3ff",
                            fill: "#053b5d",
                            r: 2,
                            lineWidth: 2
                        }
                    }
                },
                axis: {
                    tickLine: {
                        style: {
                            stroke: "#ffffff",
                            lineWidth: 1
                        },
                        length: -4
                    },
                    subTickLine: {
                        count: 11,
                        style: {
                            stroke: "#ffffff",
                            lineWidth: 1
                        },
                        length: -2
                    },
                    label: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 12,
                            textAlign: "center",
                            textBaseline: "bottom"
                        }
                    }
                },
                statistic: {
                    content: {
                        style: {
                            color: "#ffffff",
                            fontSize: "16"
                        }
                    }
                },
                animation: {
                    appear: {
                        animation: "grow-in-x",
                        duration: 3000
                    },
                    update: {
                        animation: "grow-in-x",
                        duration: 3000
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
                value: 0.75
            },
        };
    }

    getEventList(): EventInfo[] {
        const eventList = super.getEventList();
        eventList.push(...[
            {
                id: "dataChange",
                name: "数据变更时",
            },
            {
                id: "globalClick",
                name: "点击整个组件时",
            },
            {
                id: "elementClick",
                name: "点击图形数据元素时",
            },
        ])
        return eventList;
    }
}

export default AntdGaugeDefinition;
