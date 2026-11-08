/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import baseRoseImg from "./base-rose.webp";
import AbstractRoseDefinition from "@/comps/antd-common/rose/AbstractRoseDefinition";
import {AntdRoseProps} from "@/comps/antd-common/rose/AntdCommonRoseController";
import {BaseInfoType} from "@datavisool/sdk/types";

class AntdBaseRoseDefinition extends AbstractRoseDefinition {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd基础玫瑰图",
            compKey: "AntdBaseRose",
            categorize: "chart",
            subCategorize: "rose",
        };
    }
    getChartImg(): string {
        return baseRoseImg;
    }

    getInitConfig(): AntdRoseProps {
        const data = [
            {type: 'sort1', value: 27},
            {type: 'sort2', value: 25},
            {type: 'sort3', value: 18},
            {type: 'sort4', value: 15}
        ];
        return {
            base: {
                id: "",
                name: 'Antd基础玫瑰图',
                type: 'AntdBaseRose',
            },
            style: {
                data,
                xField: "type",
                yField: "value",
                seriesField: "type",
                radius: 1,
                innerRadius: 0,
                padding: [
                    50,
                    0,
                    0,
                    0
                ],
                supportCSSTransform: true,
                sectorStyle: {
                    stroke: "#fff",
                    lineWidth: 0
                },
                label: {
                    style: {
                        fill: "#ffffff",
                        fontSize: 16
                    },
                    autoRotate: true
                },
                legend: {
                    position: "left",
                    layout: "vertical",
                    itemName: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 14
                        }
                    },
                    itemSpacing: 0
                },
                animation: {
                    appear: {
                        animation: "grow-in-xy",
                        duration: 3000
                    }
                },
                color: [
                    "#0091ffff",
                    "#68beffff",
                    "#b4e0ffff",
                    "#408ec9ff"
                ]
            },
            data: {
                sourceType: 'static',
                value: data
            },
        };
    }
}

export default AntdBaseRoseDefinition;
