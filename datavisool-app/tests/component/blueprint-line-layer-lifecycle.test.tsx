/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, test, vi} from 'vitest';
import {act, cleanup, render} from '@testing-library/react';

const pixiHarness = vi.hoisted(() => ({
    resolveInit: null as null | (() => void),
    destroy: vi.fn(),
}));

vi.mock('pixi.js', () => ({
    Application: class {
        canvas = document.createElement('canvas');
        stage = {interactive: false, addChild: vi.fn()};
        renderer: {resize: ReturnType<typeof vi.fn>} | undefined;
        init = vi.fn(() => new Promise<void>((resolve) => {
            pixiHarness.resolveInit = () => {
                this.renderer = {resize: vi.fn()};
                resolve();
            };
        }));
        destroy = vi.fn(() => { pixiHarness.destroy(); });
    },
    Graphics: class {
        clear = vi.fn();
        destroy = vi.fn();
    },
}));

vi.mock('@/designer/blueprint/util/BpCanvasUtil.ts', () => ({
    default: {
        calculateControlPoint: vi.fn(),
        reDrawLine: vi.fn(),
        reRenderAllLine: vi.fn(),
    },
    bpLineConfig: {
        connectingColor: '#8c55ff',
        connectingWidth: 2,
        lineColor: '#7654ff',
        lineWidth: 1.6,
        glowWidth: 5,
        hoverColor: '#9d7dff',
        hoverWidth: 2.4,
        circleRadius: 3,
        selectedColor: '#4aa8ff',
        selectedWidth: 2.6,
    },
}));

vi.mock('@/designer/operate-provider/undo-redo/BPHistoryRecordOperateProxy', () => ({
    default: {doAddLine: vi.fn()},
    syncBpAnchorColorsFromLines: vi.fn(),
}));

import LineLayer from '@/designer/blueprint/line/LineLayer.tsx';

let restoreEnv: PropertyDescriptor | undefined;

describe('blueprint line layer lifecycle', () => {
    afterEach(() => {
        cleanup();
        vi.restoreAllMocks();
        pixiHarness.resolveInit = null;
        pixiHarness.destroy.mockClear();
        if (restoreEnv)
            Object.defineProperty(window, 'DATAVISOOL_ENV', restoreEnv);
        else
            delete (window as Window & {DATAVISOOL_ENV?: unknown}).DATAVISOOL_ENV;
        restoreEnv = undefined;
    });

    test('does not attach a late Pixi canvas or document listeners after closing during init', async () => {
        restoreEnv = Object.getOwnPropertyDescriptor(window, 'DATAVISOOL_ENV');
        const bluePrintManager = {
            canvasOffset: {x: 0, y: 0},
            currentPageManager: {pageId: 'page-1', bpLines: {}, bpNodeLayoutMap: {}},
            currPageNodeControllers: {},
            lineInstances: {},
            pixiApp: null as unknown,
        };
        Object.defineProperty(window, 'DATAVISOOL_ENV', {
            configurable: true,
            value: {currentManager: {bluePrintManager}},
        });

        const addEventListener = vi.spyOn(document, 'addEventListener');
        const removeEventListener = vi.spyOn(document, 'removeEventListener');
        const {container, unmount} = render(
            <div className="blue-print-canvas">
                <LineLayer metrics={{left: 0, top: 0, width: 800, height: 600, resolution: 1}}/>
            </div>
        );
        expect(pixiHarness.resolveInit).toBeTypeOf('function');

        unmount();
        const finishInitialization = pixiHarness.resolveInit;
        await act(async () => {
            finishInitialization?.();
            await Promise.resolve();
        });

        const mouseListenerAdds = addEventListener.mock.calls.filter(([type]) => ['mousedown', 'mousemove', 'mouseup'].includes(String(type)));
        const mouseListenerRemoves = removeEventListener.mock.calls.filter(([type]) => ['mousedown', 'mousemove', 'mouseup'].includes(String(type)));
        expect(mouseListenerAdds).toHaveLength(0);
        expect(mouseListenerRemoves).toHaveLength(3);
        expect(container.querySelector('canvas')).toBeNull();
        expect(bluePrintManager.pixiApp).toBeNull();
        expect(pixiHarness.destroy).toHaveBeenCalledOnce();
    });
});
