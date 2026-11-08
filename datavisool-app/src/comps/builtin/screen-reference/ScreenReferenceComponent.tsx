/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import React, {ForwardedRef, useCallback, useEffect, useImperativeHandle, useRef, useState} from 'react';
import DesignerManager from "@/designer/manager/DesignerManager.ts";
import canvasRender from "@/designer/left/layer-list/CanvasRender.ts";
import {ComponentInfoType} from "@datavisool/sdk/types";
import {IFilterConfigType} from "@datavisool/sdk/types";
import {DesignerMode} from "@datavisool/sdk/types";
import {globalMessage, Loading} from "@datavisool/ui";
import DesignerLoader from "@/designer/loader/DesignerLoader.ts";
import EnvUtil from "@/util/EnvUtil.ts";

export interface ScreenReferenceComponentStyle {
    screenId?: string;
}

export interface ScreenReferenceComponentProps {
    base?: ComponentInfoType;
    style?: ScreenReferenceComponentStyle;
    filter?: IFilterConfigType;
}

export interface ScreenReferenceComponentRef {
    updateConfig: (newConfig: ScreenReferenceComponentProps) => void;
    // eslint-disable-next-line @typescript-eslint/ban-types
    setEventHandler: (eventMap: Record<string, Function>) => void;
    changeVisible: (visible: boolean) => void;
}

const ScreenReferenceComponent = React.forwardRef((props: ScreenReferenceComponentProps,
                                                   ref: ForwardedRef<ScreenReferenceComponentRef>) => {
    const [config, setConfig] = useState<ScreenReferenceComponentProps>({...props});
    const [visible, setVisible] = useState(true);
    const prevScreenId = useRef<string>(config?.style?.screenId || '');
    // eslint-disable-next-line @typescript-eslint/ban-types
    const eventHandlerMap = useRef<Record<string, Function>>({});
    const [loaded, setLoaded] = useState<boolean>(false);
    const designerLoaderRef = useRef<DesignerLoader | null>(null);

    const exportVarToWindow = useCallback((designerManager: DesignerManager) => {
        window.DATAVISOOL_ENV = {...window.DATAVISOOL_ENV, ...{managers: {[config.base?.id ?? '']: designerManager}}}
    }, [config.base?.id])

    const getLoaderClass = useCallback(async () => {
        let currentMode = EnvUtil.getModeByUrl();
        if (currentMode == null) {
            console.error('无法识别当前环境')
            return null;
        }
        if (currentMode === DesignerMode.EDIT)
            currentMode = DesignerMode.PREVIEW
        if (currentMode === DesignerMode.PREVIEW)
            return import('@/designer/loader/ViewDesignerLoader.ts');
        if (currentMode === DesignerMode.RELEASE)
            return import('@/designer/loader/ReleaseDesignerLoader.ts');
        return null;
    }, [])

    const loadScreen = useCallback(() => {
        if (!config.style?.screenId)
            return;
        designerLoaderRef.current?.designerManager?.destroy();
        getLoaderClass().then((module: any) => {
            if (!module)
                return;
            const LoaderClass = module.default;
            const designerLoader = new LoaderClass() as DesignerLoader;
            designerLoader?.simpleLoad(config.style?.screenId ?? '').then(() => {
                exportVarToWindow(designerLoader.designerManager);
                designerLoaderRef.current = designerLoader;
                setLoaded(true);
            }).catch((e) => {
                console.error(e);
                globalMessage?.messageApi?.error("大屏引用失败")
            });
        })
    }, [config.style?.screenId, exportVarToWindow, getLoaderClass])

    useImperativeHandle(ref, () => ({
        updateConfig: (newConfig) => setConfig({...newConfig}),
        setEventHandler: (eventMap) => eventHandlerMap.current = eventMap,
        changeVisible: (visible) => setVisible(visible)
    }));

    /**
     * 动态切换大屏时调用
     */
    useEffect(() => {
        if (prevScreenId.current !== config.style?.screenId) {
            setLoaded(false);
            prevScreenId.current = config.style?.screenId || '';
            loadScreen();
        }
    }, [config.style?.screenId, loadScreen]);

    /**
     * 首次加载，只执行一次
     */
    const initializedRef = useRef(false);

    useEffect(() => {
        if (initializedRef.current) return;
        initializedRef.current = true;
        if (config.style?.screenId)
            loadScreen();
        return () => {
            designerLoaderRef.current?.designerManager?.destroy();
        }
    }, [config.style?.screenId, loadScreen]);

    if (!visible)
        return null;
    if (!loaded)
        return <Loading/>;

    const {width, height} = designerLoaderRef.current?.designerManager.canvasManager.canvasConfig ?? {};
    if (!config.style?.screenId)
        return <div style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            height: '100%',
            color: '#b0b0b0',
        }}>请引用大屏</div>;

    return (
        <>
            {visible && <div style={{...{height: '100%', overflow: 'hidden'}}}>
                <div style={{width, height, background: 'rgba(28,28,28,0)', overflow: 'hidden', position: "relative"}}>
                    {canvasRender.buildCanvasComponents(designerLoaderRef.current!.designerManager, "screen-reference")}
                </div>
            </div>}
        </>
    );
});

export default ScreenReferenceComponent;
