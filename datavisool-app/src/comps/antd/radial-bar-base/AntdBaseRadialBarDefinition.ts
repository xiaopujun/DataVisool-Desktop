/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import React from "react";
import {getDefaultMenuList} from "@/designer/right/util";
import baseRadialBarImg from './base-radial-bar.webp';
import AntdBaseRadialBarController, {AntdRadialBarProps} from "./AntdBaseRadialBarController.ts";
import {AntdCommonDefinition} from "@/comps/antd-common/AntdCommonDefinition";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";

const AnimationConfig = React.lazy(() => import("@/comps/common-component/animation-config/AnimationConfig"));
const AntdRadialBarConfig = React.lazy(() => import("./AntdBaseRadialBarStyleConfig.tsx"));
const AntdRadialBarFieldMapping = React.lazy(() => import("./AntdBaseRadialBarStyleConfig.tsx").then((module) => ({default: module.AntdRadialBarFieldMapping})));
const DataConfig = React.lazy(() => import("@/comps/common-component/data-config/DataConfig"));

class AntdBaseRadialBarDefinition extends AntdCommonDefinition<AntdBaseRadialBarController, AntdRadialBarProps> {

    getController(): ClazzTemplate<AntdBaseRadialBarController> | null {
        return AntdBaseRadialBarController;
    }

    getMenuList(): Array<MenuInfo> {
        return getDefaultMenuList();
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            data: DataConfig,
            style: AntdRadialBarConfig,
            animation: AnimationConfig,
            mapping: AntdRadialBarFieldMapping
        };
    }

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd基础玉珏图",
            compKey: "AntdBaseRadialBar",
            categorize: "chart",
            subCategorize: 'radial'
        };
    }
    getChartImg(): string | null {
        return baseRadialBarImg;
    }

    getInitConfig(): AntdRadialBarProps {
        const data = [
            {name: 'X6', star: 297},
            {name: 'G', star: 506},
            {name: 'AVA', star: 805},
            {name: 'G2Plot', star: 1478},
            {name: 'L7', star: 2029},
            {name: 'G6', star: 7100},
            {name: 'F2', star: 7346},
            {name: 'G2', star: 10178},
        ];
        return {
            base: {
                id: "",
                name: 'Antd基础玉珏图',
                type: 'AntdBaseRadialBar',
            },
            style: {
                data,
                xField: "name",
                yField: "star",
                maxAngle: 270,
                startAngle: -1.5707963267948966,
                endAngle: 4.71238898038469,
                radius: 1,
                innerRadius: 0.1,
                colorField: "name",
                type: "",
                supportCSSTransform: true,
                maxBarWidth: 11,
                color: [
                    "l(90) 0:#3ea2ff 1:#007bff",
                    "l(90) 0:#a362ff 1:#c375ff"
                ],
                barStyle: {
                    lineCap: "round"
                },
                xAxis: {
                    label: {
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
                }
            },
            data: {
                sourceType: 'static',
                value: data,
            },
        };
    }
}

export default AntdBaseRadialBarDefinition;
