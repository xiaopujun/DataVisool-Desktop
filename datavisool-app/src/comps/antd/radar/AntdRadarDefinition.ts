/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import React from "react";
import {getDefaultMenuList} from "@/designer/right/util";
import radarImg from './radar.webp';
import AntdRadarController, {AntdRadarProps} from "./AntdRadarController.ts";
import {AntdCommonDefinition} from "@/comps/antd-common/AntdCommonDefinition";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";

const AnimationConfig = React.lazy(() => import("@/comps/common-component/animation-config/AnimationConfig"));
const AntdRadarStyleConfig = React.lazy(() => import("./AntdRadarStyleConfig.tsx"));
const DataConfig = React.lazy(() => import("@/comps/common-component/data-config/DataConfig"));
const AntdRadarFieldMapping = React.lazy(() => import("./AntdRadarStyleConfig.tsx").then((module) => ({default: module.AntdRadarFieldMapping})));

class AntdRadarDefinition extends AntdCommonDefinition<AntdRadarController, AntdRadarProps> {

    getController(): ClazzTemplate<AntdRadarController> | null {
        return AntdRadarController;
    }

    getMenuList(): Array<MenuInfo> {
        return getDefaultMenuList();
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            data: DataConfig,
            style: AntdRadarStyleConfig,
            animation: AnimationConfig,
            mapping: AntdRadarFieldMapping
        };
    }

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd雷达图",
            compKey: "AntdRadar",
            categorize: "chart",
            subCategorize: "radar",
        };
    }
    getChartImg(): string | null {
        return radarImg;
    }

    getInitConfig(): AntdRadarProps {
        const data = [
            {
                name: "G2",
                star: 10371
            },
            {
                name: "G6",
                star: 7380
            },
            {
                name: "F2",
                star: 7414
            },
            {
                name: "L7",
                star: 2140
            },
            {
                name: "X6",
                star: 6601
            },
            {
                name: "AVA",
                star: 8851
            },
            {
                name: "G2Plot",
                star: 1626
            }
        ];
        return {
            base: {
                id: "",
                name: 'Antd雷达图',
                type: 'AntdRadar',
            },
            style: {
                data: data,
                xField: "name",
                yField: "star",
                radius: 1,
                smooth: true,
                lineStyle: {
                    stroke: "l(90) 0:#49a4ff 1:#ff70ff",
                    lineWidth: 2,
                    opacity: 0.5
                },
                point: {
                    size: 3,
                    shape: "circle",
                    style: {
                        fill: "l(90) 0:#248eff 1:#f36fff",
                        stroke: "#64d6ff",
                        lineWidth: 0
                    }
                },
                area: {
                    style: {
                        fill: "l(90) 0:#57a3ff 1:#c041ff",
                        fillOpacity: 0.6
                    }
                },
                xAxis: {
                    line: null,
                    tickLine: null,
                    grid: {
                        alignTick: true,
                        line: {
                            style: {
                                stroke: "#a2a2a2bb",
                                lineWidth: 1
                            },
                            type: "line"
                        }
                    },
                    label: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 13
                        }
                    }
                },
                yAxis: {
                    line: {
                        style: {
                            stroke: "#595959",
                            lineWidth: 1
                        }
                    },
                    tickLine: null,
                    grid: {
                        alignTick: true,
                        line: {
                            style: {
                                stroke: "l(90) 0:#f9a5ff 1:#89c6ff",
                                lineWidth: 1
                            },
                            type: "line"
                        }
                    },
                    label: {
                        style: {
                            fill: "#ffffff"
                        }
                    },
                    tickCount: 3
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

export default AntdRadarDefinition;
