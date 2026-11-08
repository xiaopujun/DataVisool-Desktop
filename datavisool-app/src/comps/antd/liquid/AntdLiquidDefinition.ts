/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import React from "react";
import AntdLiquidController, {AntdLiquidProps} from "./AntdLiquidController";
import liquidImg from './liquid.webp';
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";
import {EventInfo} from "@datavisool/sdk/types";

const AntdLiquidConfig = React.lazy(() => import("./AntdLiquidConfig"));
const DataConfig = React.lazy(() => import("@/comps/common-component/data-config/DataConfig"));

class AntdLiquidDefinition extends AbstractDesignerDefinition<AntdLiquidController, AntdLiquidProps> {

    getController(): ClazzTemplate<AntdLiquidController> | null {
        return AntdLiquidController;
    }

    getMenuList(): Array<MenuInfo> {
        return super.getMenuList().filter((item) => item.key !== 'mapping');
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            data: DataConfig,
            style: AntdLiquidConfig
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

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd水波图",
            compKey: "AntdLiquid",
            categorize: "chart",
            subCategorize: "progress",
        };
    }
    getChartImg(): string | null {
        return liquidImg;
    }

    getInitConfig(): AntdLiquidProps {
        return {
            base: {
                id: "",
                name: 'Antd水波图',
                type: 'AntdLiquid',
            },
            style: {
                shape: "circle",
                percent: 0.65,
                radius: 0.9,
                liquidStyle: {
                    fill: "l(90) 0:#45aeff 1:#0015ff",
                    stroke: "#1a75c8d6"
                },
                shapeStyle: {
                    fill: "#00fff716"
                },
                outline: {
                    border: 0,
                    distance: 0,
                    style: {
                        stroke: "#2B7DD35F"
                    }
                },
                wave: {
                    length: 150,
                    count: 3
                },
                statistic: {
                    content: {
                        style: {
                            fill: "#7de0ff",
                            fontSize: '20',
                            color: "#ffffff",
                            fontWeight: 200
                        }
                    },
                    title: {
                        content: "指标1",
                        style: {
                            fontSize: "22",
                            color: "#ffffffc2",
                            fontWeight: 500
                        },
                        offsetY: -10
                    }
                },
                animation: {
                    appear: {
                        animation: "wave-in",
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
                value: 0.65
            },
        };
    }
}

export default AntdLiquidDefinition;
