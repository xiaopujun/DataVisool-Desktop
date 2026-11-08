/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import React, {ForwardedRef, useEffect, useImperativeHandle, useRef, useState} from 'react';
import {CountUp} from "countup.js";
import {Odometer} from "odometer_countup";
import {ComponentBaseProps} from "@datavisool/sdk/types";

export interface DigitalFlipperComponentStyle {
    fontSize?: number;
    fontWeight?: number;
    fontFamily?: string;
    color?: string;
    type?: 'slide' | 'caper';
    alignItems?: string;
    justifyContent?: string;
    prefix?: string;
    suffix?: string;
    decimalPlaces?: number;
    separator?: string;
    duration?: number;
    backgroundImage?: string;
    backgroundClip?: string;
    WebkitBackgroundClip?: string;
    WebkitTextFillColor?: string;
}

export interface DigitalFlipperComponentProps extends ComponentBaseProps {
    style?: DigitalFlipperComponentStyle;
}

export interface DigitalFlipperComponentRef {
    updateConfig: (newConfig: DigitalFlipperComponentProps, recreate?: boolean) => void;
    changeData: (newData: number) => void;
    // eslint-disable-next-line @typescript-eslint/ban-types
    setEventHandler: (eventMap: Record<string, Function>) => void;
    changeVisible: (visible: boolean) => void;
}

const DigitalFlipperComponent = React.forwardRef((props: DigitalFlipperComponentProps,
                                                  ref: ForwardedRef<DigitalFlipperComponentRef>) => {
    const [config, setConfig] = useState<DigitalFlipperComponentProps>({...props});

    // eslint-disable-next-line @typescript-eslint/ban-types
    const eventHandlerMap = useRef<Record<string, Function>>({});
    const containerRef = useRef<HTMLDivElement | null>(null);
    const countUpAnimRef = useRef<CountUp | null>(null);
    const [count, setCount] = useState(0);
    const [visible, setVisible] = useState(true);

    useEffect(() => {
        if (!containerRef.current)
            return;
        const styleConfig = config.style ?? {};
        const {duration = 1, separator = ',', decimalPlaces = 0, prefix = '', suffix = '',} = styleConfig;
        const dataValue = Number(config.data?.value ?? 0);
        const endValue = Number.isFinite(dataValue) ? dataValue : 0;
        countUpAnimRef.current = new CountUp(containerRef.current, endValue, {
            plugin: styleConfig.type === 'slide' ? new Odometer({duration: 1, lastDigitDelay: 0}) : undefined,
            duration,
            suffix,
            prefix,
            decimalPlaces,
            separator
        });
        if (!countUpAnimRef.current.error) {
            countUpAnimRef.current.start();
        } else {
            console.error(countUpAnimRef.current.error);
        }
        return () => {
            countUpAnimRef.current = null;
        }
    }, [config.data?.value, config.style, count]);

    useImperativeHandle(ref, () => ({
        updateConfig: (newConfig, recreate) => {
            setConfig({...newConfig});
            if (recreate)
                setCount((prev) => prev + 1);
        },
        changeData: (newData) => countUpAnimRef.current?.update(newData),
        setEventHandler: (eventMap) => eventHandlerMap.current = eventMap,
        changeVisible: (visible) => setVisible(visible)
    }));

    const onClick = () => {
        if ('click' in eventHandlerMap.current) {
            eventHandlerMap.current['click']();
        }
    }

    if (!visible)
        return null;
    const style = config.style ?? {};
    return (
        <div ref={containerRef}
             style={{display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', ...style}}
             onClick={onClick}/>
    );
});

export default DigitalFlipperComponent;
