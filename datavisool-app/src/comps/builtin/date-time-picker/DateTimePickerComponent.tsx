/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {ForwardedRef, forwardRef, useImperativeHandle, useRef, useState} from 'react';
import {ComponentBaseProps} from "@datavisool/sdk/types";
import './DateTimePickerComponent.less';
import {Input} from '@datavisool/ui';
import dayjs from 'dayjs';
import 'dayjs/locale/zh-cn';
import {getLocale} from "@/i18n/miniI18n.ts";
import {useLocaleVersion} from "@/i18n/useLocaleVersion.ts";

// 组件样式接口定义
export interface DateTimePickerComponentStyle {
    // 日期时间选择器特有样式
    pickerType?: 'date' | 'week' | 'month' | 'quarter' | 'year' | 'datetime' | 'range';
    placeholder?: string;
    showTime?: boolean;
    format?: string;
    allowClear?: boolean;
    disabled?: boolean;
    showNow?: boolean;

    token?: {
        lineWidth: number;
        fontSize: number;
        colorTextHeading: string;
        colorPrimary: string;
        colorSplit: string;
        colorText: string;
        colorBorder: string;
        colorBgContainerDisabled: string;          // 禁用状态的容器背景
        colorTextPlaceholder: string;              // 占位符文字色
        colorIcon: string;                         // 图标颜色
        colorIconHover: string;                    // 图标悬停颜色
        borderRadius: number;                              // 卡片圆角
        lineType: string;
        fontFamily: string;                         // 无衬线字体
        fontWeightStrong: number;
        padding: number;                                  // 次轴间距
        colorBgElevated: string;
    },
    components?: {
        colorBgContainer: string;                     // container 背景
        colorBorder: string;
        colorText: string;
        colorPrimary: string;                         // 主色
        colorTextHeading: string;                     // 和 colorText 一致
        colorSplit: string;
        hoverBorderColor: string;                     // 输入 hover
        activeBorderColor: string;                    // 输入 active
        hoverBg: string;                              // 输入 hover 背景（亮度提升）
        activeBg: string;                             // 输入 active 背景
        activeShadow: string;  // active 阴影
        cellActiveWithRangeBg: string;
        cellHoverBg: string;
        cellHoverWithRangeBg: string;
        cellHeight: number;
        cellWidth: number;
        withoutTimeCellHeight: number;
        textHeight: number;
        timeCellHeight: number;
        timeColumnHeight: number;
        timeColumnWidth: number;
        inputFontSize: number;
        paddingBlock: number;
        paddingInline: number;
    },
}

// 组件属性接口定义
export interface DateTimePickerComponentProps extends ComponentBaseProps {
    style?: DateTimePickerComponentStyle;
}

// 组件引用接口定义
export interface DateTimePickerComponentRef {
    updateConfig: (newConfig: DateTimePickerComponentProps) => void;
    // eslint-disable-next-line @typescript-eslint/ban-types
    setEventHandler: (eventMap: Record<string, Function>) => void;
    getValue: () => string | [string, string] | undefined;
    setValue: (value: string | [string, string]) => void;
}

// 日期时间选择器组件实现
const DateTimePickerComponent = forwardRef((
            props: DateTimePickerComponentProps,
            ref: ForwardedRef<DateTimePickerComponentRef>
        ) => {
            useLocaleVersion();
            const [config, setConfig] = useState<DateTimePickerComponentProps>({...props});
            const {style, data} = config;
            const locale = getLocale();
            dayjs.locale(locale === 'zh-CN' ? 'zh-cn' : 'en');
            // eslint-disable-next-line @typescript-eslint/ban-types
            const eventHandlerMap = useRef<Record<string, Function>>({});
            const containerRef = useRef<HTMLDivElement>(null);

            // 暴露组件方法给外部调用
            useImperativeHandle(ref, () => ({
                updateConfig: (newConfig) => setConfig({...newConfig}),
                setEventHandler: (eventMap) => eventHandlerMap.current = eventMap,
                getValue: () => data?.value,
                setValue: (newValue) => {
                    setConfig(prev => ({
                        ...prev,
                        data: {value: newValue}
                    }));
                    if ('valueChange' in eventHandlerMap.current) {
                        eventHandlerMap.current['valueChange'](newValue);
                    }
                },
            }));

            const getNativeInputType = () => {
                if (style?.pickerType === 'datetime' || style?.showTime)
                    return 'datetime-local';
                if (style?.pickerType === 'month')
                    return 'month';
                if (style?.pickerType === 'week')
                    return 'week';
                return 'date';
            };

            const normalizeInputValue = (value?: string) => {
                if (!value)
                    return '';
                const inputType = getNativeInputType();
                if (inputType === 'datetime-local')
                    return dayjs(value).isValid() ? dayjs(value).format('YYYY-MM-DDTHH:mm') : value.replace(' ', 'T').slice(0, 16);
                if (inputType === 'month')
                    return dayjs(value).isValid() ? dayjs(value).format('YYYY-MM') : value.slice(0, 7);
                if (inputType === 'week')
                    return value;
                return dayjs(value).isValid() ? dayjs(value).format('YYYY-MM-DD') : value.slice(0, 10);
            };

            const formatInputValue = (value: string) => {
                if (!value)
                    return '';
                const inputType = getNativeInputType();
                if (inputType === 'week')
                    return value;
                const parsed = dayjs(value);
                if (!parsed.isValid())
                    return value;
                return parsed.format(style?.format || (inputType === 'datetime-local' ? 'YYYY-MM-DD HH:mm:ss' : 'YYYY-MM-DD'));
            };

            const emitValue = (formattedValue: string | [string, string]) => {
                setConfig(prev => ({
                    ...prev,
                    data: {value: formattedValue}
                }));

                if ('valueChange' in eventHandlerMap.current) {
                    eventHandlerMap.current['valueChange'](formattedValue);
                }
                if ('confirm' in eventHandlerMap.current) {
                    eventHandlerMap.current['confirm'](formattedValue);
                }
            };

            // 处理打开事件
            const handleOpenChange = (open: boolean) => {
                if (open && 'open' in eventHandlerMap.current) {
                    eventHandlerMap.current['open']();
                }
                if (!open && 'close' in eventHandlerMap.current) {
                    eventHandlerMap.current['close']();
                }
            };

            // 根据 pickerType 渲染不同的选择器
            const renderPicker = () => {
                const {pickerType = 'date'} = style || {};
                if (pickerType === 'range') {
                    const rangeValue = Array.isArray(data?.value) ? data.value : ['', ''];
                    const updateRangeValue = (index: 0 | 1, value: string) => {
                        const next: [string, string] = [
                            rangeValue[0] || '',
                            rangeValue[1] || '',
                        ];
                        next[index] = formatInputValue(value);
                        emitValue(next);
                    };

                    return (
                        <div className="date-time-picker-range">
                            <Input
                                type={getNativeInputType()}
                                value={normalizeInputValue(rangeValue[0])}
                                placeholder={style?.placeholder || '开始时间'}
                                disabled={style?.disabled}
                                allowClear={style?.allowClear}
                                onFocus={() => handleOpenChange(true)}
                                onBlur={() => handleOpenChange(false)}
                                onChange={(value) => updateRangeValue(0, value)}
                            />
                            <span className="date-time-picker-range-separator">-</span>
                            <Input
                                type={getNativeInputType()}
                                value={normalizeInputValue(rangeValue[1])}
                                placeholder={style?.placeholder || '结束时间'}
                                disabled={style?.disabled}
                                allowClear={style?.allowClear}
                                onFocus={() => handleOpenChange(true)}
                                onBlur={() => handleOpenChange(false)}
                                onChange={(value) => updateRangeValue(1, value)}
                            />
                        </div>
                    );
                }

                return (
                    <Input
                        type={getNativeInputType()}
                        value={normalizeInputValue(data?.value as string)}
                        placeholder={style?.placeholder}
                        disabled={style?.disabled}
                        allowClear={style?.allowClear}
                        onFocus={() => handleOpenChange(true)}
                        onBlur={() => handleOpenChange(false)}
                        onChange={(value) => emitValue(formatInputValue(value))}
                    />
                );
            };

            return (
                <div className="date-time-picker-component" ref={containerRef}>
                    {renderPicker()}
                </div>
            );
        }
    )
;

export default DateTimePickerComponent;
