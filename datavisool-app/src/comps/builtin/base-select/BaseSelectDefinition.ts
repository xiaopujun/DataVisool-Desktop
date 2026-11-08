/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuToConfigMappingType} from "@datavisool/sdk/core";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {EventInfo} from "@datavisool/sdk/types";
import BaseSelectImg from './base-select.webp';
import {BaseSelectController} from "./BaseSelectController.ts";
import {BaseSelectComponentProps} from "./BaseSelectComponent.tsx";
import React, {lazy} from "react";
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";

const AnimationConfig = React.lazy(() => import("@/comps/common-component/animation-config/AnimationConfig.tsx"));
const DataConfig = React.lazy(() => import("@/comps/common-component/data-config/DataConfig.tsx"));
const BaseSelectStyleConfig = lazy(() => import("./BaseSelectConfig.tsx").then((module) => ({default: module.BaseSelectStyleConfig})));

export default class BaseSelectDefinition extends AbstractDesignerDefinition<BaseSelectController, BaseSelectComponentProps> {
    getBaseInfo(): BaseInfoType {
        return {
            compName: "下拉框",
            compKey: "BaseSelect",
            categorize: "web",
            width: 320,
            height: 26,
        };
    }
   getChartImg(): string | null {
        return BaseSelectImg;
    }

    getController(): ClazzTemplate<BaseSelectController> | null {
        return BaseSelectController;
    }

    getInitConfig(): BaseSelectComponentProps {
        return {
            base: {
                id: "",
                name: '下拉框',
                type: 'BaseSelect',
            },
            style: {
                fontSize: 16, // 改为规范的小标题尺寸（16-18px），原18px略大
                fontFamily: '"Poppins", "Roboto", "Helvetica Neue", Arial, sans-serif', // 使用规范字体家族
                selectorBg: '#27293D', // 改为卡片背景色
                colorText: '#FFFFFF', // 主要文本使用纯白
                lineWidth: 1,
                borderRadius: 4, // 标准圆角（从3改为4）
                colorBorder: 'rgba(255, 255, 255, 0.2)', // 使用浅色半透明边框
                colorPrimaryHover: '#FD5D93', // 使用主色调作为悬停色
                controlOutline: 'rgba(253, 93, 147, 0.3)', // 使用主色调半透明作为焦点轮廓
                borderRadiusLG: 4, // 保持一致的圆角
                optionFontSize: 14, // 选项字体使用正文尺寸
                optionHeight: 32, // 增加选项高度（从20改为32）提高可操作性
                optionSelectedBg: 'rgba(253, 93, 147, 0.15)', // 选中背景使用主色调透明版
                optionSelectedColor: '#FFFFFF', // 选中文字使用纯白
                optionActiveBg: 'rgba(29, 140, 248, 0.2)', // 活动状态使用信息色透明版
                colorBgElevated: '#1E1E2F', // 弹出层使用应用背景色
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
                value: [
                    {value: '0', label: 'Option0'},
                    {value: '1', label: 'Option1'},
                ]
            }
        };
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        return {
            style: BaseSelectStyleConfig,
            animation: AnimationConfig,
            data: DataConfig
        };
    }

    getEventList(): EventInfo[] {
        const events = super.getEventList();
        return events.concat([
            {
                id: "click",
                name: "点击时",
            },
            {
                id: 'selectedChange',
                name: '选项改变时',
            }
        ]);
    }
}
