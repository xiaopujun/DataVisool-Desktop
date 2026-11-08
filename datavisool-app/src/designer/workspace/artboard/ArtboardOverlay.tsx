/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {observer} from 'mobx-react';
import {DesignerScene} from "@datavisool/sdk/types";
import managerStore from '@/designer/manager/ManagerStore.ts';
import viewportStore from '@/designer/workspace/viewport/ViewportStore.ts';
import {worldToScreen} from '@/designer/workspace/viewport/ViewportMath.ts';
import {RULER_SIZE} from '@/designer/canvas/DesignerRulerMath.ts';
import './ArtboardOverlay.less';

/** The sole project canvas remains at the workspace origin. */
const ArtboardOverlay = observer(() => {
    if (managerStore.designerScene !== DesignerScene.DEFAULT) return null;
    const config = managerStore.currentManager?.canvasManager?.canvasConfig;
    if (!config?.width || !config?.height) return null;

    const camera = viewportStore.camera;
    const topLeft = worldToScreen({x: 0, y: 0}, camera);
    const width = config.width * camera.scale;
    const height = config.height * camera.scale;

    return <div className="datavisool-app-artboard-overlay" aria-label="工作区画板">
        <div className="datavisool-app-active-artboard-frame"
             style={{left: topLeft.x, top: topLeft.y, width, height}}/>
        <div className="datavisool-app-artboard-label"
             style={{left: topLeft.x, top: topLeft.y - 27 - (viewportStore.showRulers ? RULER_SIZE : 0)}}>
            <span>画板</span>
            <small>{config.width} × {config.height}</small>
        </div>
    </div>;
});

export default ArtboardOverlay;
