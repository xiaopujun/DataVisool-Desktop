/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {ForwardedRef, forwardRef, useImperativeHandle, useRef, useState} from 'react';
import {ComponentBaseProps} from "@datavisool/sdk/types";
import './BaseSelectComponent.less';
import {Select} from '@datavisool/ui';

interface BaseSelectComponentStyle {
    fontFamily: string;
    fontSize: number
    selectorBg: string,
    colorText: string,
    borderRadius: number,
    lineWidth: number;
    colorBorder: string,
    colorPrimaryHover: string,
    controlOutline: string,
    borderRadiusLG: number,
    optionFontSize: number,
    optionHeight: number,
    optionSelectedBg: string,
    optionSelectedColor: string,
    optionActiveBg: string,
    colorBgElevated: string,
}

export interface BaseSelectComponentProps extends ComponentBaseProps {
    style?: BaseSelectComponentStyle;
}

export interface BaseSelectComponentRef {
    updateConfig: (newConfig: BaseSelectComponentProps) => void;
    // eslint-disable-next-line @typescript-eslint/ban-types
    setEventHandler: (eventMap: Record<string, Function>) => void;
    changeVisible: (visible: boolean) => void;
}

export const BaseSelectComponent = forwardRef((props: BaseSelectComponentProps, ref: ForwardedRef<BaseSelectComponentRef>) => {
    const [config, setConfig] = useState<BaseSelectComponentProps>({...props});
    const [visible, setVisible] = useState(true);
    const {style, data} = config;
    // eslint-disable-next-line @typescript-eslint/ban-types
    const eventHandlerMap = useRef<Record<string, Function>>({});
    const inputRef = useRef<HTMLDivElement>(null);

    useImperativeHandle(ref, () => ({
        updateConfig: (newConfig) => {
            setConfig({...newConfig});
        },
        changeVisible: (visible) => setVisible(visible),
        setEventHandler: (eventMap) => eventHandlerMap.current = eventMap,
    }));


    const onClick = () => {
        if ('click' in eventHandlerMap.current)
            eventHandlerMap.current['click']();
    }

    const onChange = (value: string) => {
        if ('selectedChange' in eventHandlerMap.current)
            eventHandlerMap.current['selectedChange'](value);

    }

    if (!visible)
        return null;

    return (
        <div className="base-select-component" onClick={onClick} ref={inputRef}>
            <Select
                width="100%"
                height="100%"
                controlStyle={{
                    fontFamily: style?.fontFamily,
                    fontSize: style?.fontSize,
                    background: style?.selectorBg,
                    color: style?.colorText,
                    borderRadius: style?.borderRadius,
                    borderWidth: style?.lineWidth,
                    borderColor: style?.colorBorder,
                }}
                onChange={(value) => onChange(Array.isArray(value) ? String(value[0] ?? '') : String(value))}
                options={data?.value ?? []}
            />
        </div>
    );
});

export default BaseSelectComponent;
