/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import groupRoseImg from "./group-rose.webp";
import AbstractRoseDefinition from "@/comps/antd-common/rose/AbstractRoseDefinition";
import {AntdRoseProps} from "@/comps/antd-common/rose/AntdCommonRoseController";
import {BaseInfoType} from "@datavisool/sdk/types";

class AntdGroupRoseDefinition extends AbstractRoseDefinition {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "Antd分组玫瑰图",
            compKey: "AntdGroupRose",
            categorize: "chart",
            subCategorize: "rose",
        };
    }
    getChartImg(): string {
        return groupRoseImg;
    }

    getInitConfig(): AntdRoseProps {
        const data = [
            {
                type: "sort1",
                value: 27,
                user: "user1"
            },
            {
                type: "sort2",
                value: 25,
                user: "user2"
            },
            {
                type: "sort3",
                value: 18,
                user: "user1"
            },
            {
                type: "sort4",
                value: 15,
                user: "user2"
            },
            {
                type: "sort5",
                value: 18,
                user: "user1"
            },
            {
                type: "sort6",
                value: 20,
                user: "user2"
            }
        ];
        return {
            base: {
                id: "",
                name: 'Antd分组玫瑰图',
                type: 'AntdGroupRose',
            },
            style: {
                data,
                xField: "type",
                yField: "value",
                isGroup: true,
                seriesField: "user",
                padding: [
                    60,
                    0,
                    0,
                    0
                ],
                radius: 1,
                innerRadius: 0.13,
                startAngle: 0,
                endAngle: 6.283185307179586,
                label: {
                    offset: 16,
                    style: {
                        fill: "#ffffff"
                    },
                    autoRotate: false
                },
                legend: {
                    position: "top-left",
                    layout: "horizontal",
                    itemName: {
                        style: {
                            fill: "#ffffff",
                            fontSize: 14
                        }
                    }
                },
                supportCSSTransform: true,
                animation: {
                    appear: {
                        animation: "grow-in-xy",
                        duration: 3000
                    }
                },
                color: [
                    "#00b7ffff",
                    "#81dbffff"
                ],
                sectorStyle: {
                    lineWidth: 0
                }
            },
            data: {
                sourceType: 'static',
                value: data
            },
        };
    }
}

export default AntdGroupRoseDefinition;
