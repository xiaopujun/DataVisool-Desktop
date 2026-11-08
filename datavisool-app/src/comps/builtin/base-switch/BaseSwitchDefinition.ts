/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import React from 'react';
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuToConfigMappingType} from "@datavisool/sdk/core";
import {SwitchController} from "./BaseSwitchController.ts";
import {SwitchComponentProps} from "./BaseSwitchComponent.tsx";
import SwitchImg from './base-switch.webp';
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {EventInfo} from "@datavisool/sdk/types";

// 懒加载配置组件
const AnimationConfig = React.lazy(() => import("@/comps/common-component/animation-config/AnimationConfig.tsx"));
const DataConfig = React.lazy(() => import("@/comps/common-component/data-config/DataConfig.tsx"));
const SwitchStyleConfig = React.lazy(() => import("./BaseSwitchStyleConfig.tsx").then((module) => ({default: module.SwitchStyleConfig})));

export default class SwitchDefinition extends AbstractDesignerDefinition<SwitchController, SwitchComponentProps> {

    getBaseInfo(): BaseInfoType {
        return {
            compName: "开关组件",
            compKey: "Switch",
            categorize: "web",
            width: 120,
            height: 32,
        };
    }
    getChartImg(): string | null {
        return SwitchImg;
    }

    getController(): ClazzTemplate<SwitchController> | null {
        return SwitchController;
    }

    getInitConfig(): SwitchComponentProps {
        return {
            base: {
                id: "",
                name: '开关组件',
                type: 'Switch',
            },
            style: {
                token: {
                    colorPrimary: "#4FB8FF",                  // 主高亮色
                    colorPrimaryHover: "#7ACDFF",             // 悬停亮度提升10%（#4FB8FF亮度提升）
                    colorPrimaryBorder: "#4FB8FF",            // 主色边框
                    colorText: "#FFFFFF",                     // 主要文字
                    colorTextLightSolid: "#FFFFFF",           // 主按钮文字
                    fontSize: 14,                             // 字体大小 14px
                },
                components: {
                    handleSize: 20,                           // 开关把手大小
                    trackHeight: 24,                          // 开关轨道高度
                    trackMinWidth: 44,                        // 轨道最小宽度
                    trackPadding: 2,                          // 轨道内边距
                    handleBg: "#FFFFFF",                      // 把手背景色
                    colorBgContainer: "#2C2C3E",              // 容器背景色
                },
                props: {
                    checkedChildren: "开",
                    unCheckedChildren: "关",
                    loading: false,                               // 非加载状态
                    disabled: false,                              // 非禁用状态
                    size: 'default',                        // 默认尺寸
                },
                label: {
                    content: "",
                    position: 'left',
                    color: '#FFFFFF',
                    fontSize: 14,
                    fontWeight: 400,
                    fontFamily: "",
                },
            },
            data: {
                sourceType: 'static',
                value: false
            },
            filter: {
                enable: false,
                blur: 0,
                brightness: 1,
                contrast: 1,
                opacity: 1,
                saturate: 1,
                hueRotate: 0
            }
        };
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            style: SwitchStyleConfig,
            animation: AnimationConfig,
            data: DataConfig
        };
    }

    getEventList(): EventInfo[] {
        const events = super.getEventList();
        return events.concat([
            {
                id: "change",
                name: "状态改变时",
            },
            {
                id: "click",
                name: "点击时",
            },
            {
                id: "on",
                name: "开启时",
            },
            {
                id: "off",
                name: "关闭时",
            },
            {
                id: "toggle",
                name: "切换时",
            },
            {
                id: "valueChange",
                name: "值变化时",
            }
        ]);
    }
}
