/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import DragScaleProvider from '@/framework/drag-scale/DragScaleProvider.tsx';

type FrameMap = Map<number, FrameRequestCallback>;

let frames: FrameMap;
let nextFrameId: number;
let cancelledFrames: number[];
let providers: DragScaleProvider[];

function makeFrameQueue() {
    frames = new Map();
    nextFrameId = 1;
    cancelledFrames = [];
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
        const id = nextFrameId++;
        frames.set(id, callback);
        return id;
    });
    vi.stubGlobal('cancelAnimationFrame', (id: number) => {
        cancelledFrames.push(id);
        frames.delete(id);
    });
}

function flushFrames() {
    const pending = Array.from(frames.values());
    frames.clear();
    pending.forEach(callback => callback(16));
}

function makeProvider(options: {
    width?: number;
    height?: number;
    targetWidth?: number;
    targetHeight?: number;
    left?: number;
    top?: number;
    scale?: number;
    position?: {x: number; y: number};
    scaleCallback?: ReturnType<typeof vi.fn>;
    dragCallback?: ReturnType<typeof vi.fn>;
    dragEndCallback?: ReturnType<typeof vi.fn>;
} = {}) {
    const {
        width = 800,
        height = 600,
        targetWidth = 0,
        targetHeight = 0,
        left = 100,
        top = 50,
        scale = 1,
        position = {x: 0, y: 0},
        scaleCallback = vi.fn(),
        dragCallback = vi.fn(),
        dragEndCallback = vi.fn(),
    } = options;
    const eventContainer = document.createElement('div') as HTMLDivElement;
    const dsTarget = document.createElement('div') as HTMLDivElement;
    document.body.append(eventContainer, dsTarget);

    Object.defineProperty(eventContainer, 'getBoundingClientRect', {
        configurable: true,
        value: () => ({
            x: left, y: top, left, top,
            right: left + width, bottom: top + height,
            width, height, toJSON: () => ({}),
        } as DOMRect),
    });
    Object.defineProperty(dsTarget, 'offsetWidth', {configurable: true, value: targetWidth});
    Object.defineProperty(dsTarget, 'offsetHeight', {configurable: true, value: targetHeight});

    const capturedPointers = new Set<number>();
    const setPointerCapture = vi.fn((pointerId: number) => capturedPointers.add(pointerId));
    const releasePointerCapture = vi.fn((pointerId: number) => capturedPointers.delete(pointerId));
    Object.defineProperty(eventContainer, 'setPointerCapture', {configurable: true, value: setPointerCapture});
    Object.defineProperty(eventContainer, 'releasePointerCapture', {configurable: true, value: releasePointerCapture});
    Object.defineProperty(eventContainer, 'hasPointerCapture', {
        configurable: true,
        value: (pointerId: number) => capturedPointers.has(pointerId),
    });

    const provider = new DragScaleProvider({
        eventContainer,
        dsTarget,
        scale,
        position: {...position},
        scaleCallback,
        dragCallback,
        dragEndCallback,
    });
    providers.push(provider);
    return {provider, eventContainer, dsTarget, setPointerCapture, releasePointerCapture,
        scaleCallback, dragCallback, dragEndCallback};
}

function makePointerEvent(type: string, init: {
    button?: number;
    buttons?: number;
    pointerId?: number;
    clientX?: number;
    clientY?: number;
} = {}) {
    const event = new Event(type, {bubbles: true, cancelable: true}) as PointerEvent;
    Object.defineProperties(event, {
        button: {value: init.button ?? 2},
        buttons: {value: init.buttons ?? 2},
        pointerId: {value: init.pointerId ?? 7},
        clientX: {value: init.clientX ?? 0},
        clientY: {value: init.clientY ?? 0},
    });
    return event;
}

function worldPointAt(anchor: {x: number; y: number}, position: {x: number; y: number}, scale: number,
                      targetWidth: number, targetHeight: number) {
    return {
        x: targetWidth / 2 + (anchor.x - position.x - targetWidth / 2) / scale,
        y: targetHeight / 2 + (anchor.y - position.y - targetHeight / 2) / scale,
    };
}

function positionFromTransform(target: HTMLElement) {
    const match = target.style.transform.match(/translate3d\(([-\d.]+)px, ([-\d.]+)px, 0\)/);
    if (!match)
        throw new Error(`Unexpected transform: ${target.style.transform}`);
    return {x: Number(match[1]), y: Number(match[2])};
}

describe('DragScaleProvider viewport API', () => {
    beforeEach(() => {
        providers = [];
        makeFrameQueue();
    });

    afterEach(() => {
        providers.forEach(provider => provider.destroy());
        document.body.replaceChildren();
        vi.unstubAllGlobals();
    });

    it('synchronizes programmatic viewport state and continues wheel zoom from it', () => {
        const {provider, eventContainer, dsTarget, scaleCallback} = makeProvider({
            targetWidth: 400,
            targetHeight: 300,
            position: {x: 10, y: 20},
        });
        eventContainer.dispatchEvent(new WheelEvent('wheel', {
            bubbles: true,
            altKey: true,
            deltaY: 100,
            clientX: 300,
            clientY: 200,
        }));
        expect(frames.size).toBe(1);
        provider.setViewport(2, {x: 30, y: -40});
        expect(frames.size).toBe(0);
        flushFrames();

        expect(provider.scaleCore.scale).toBe(2);
        expect(provider.scaleCore.ratio).toBe(2);
        expect(dsTarget.style.transform).toBe('translate3d(30px, -40px, 0) scale(2)');
        expect(scaleCallback).toHaveBeenLastCalledWith(expect.objectContaining({
            scale: 2,
            ratio: 2,
            position: {x: 30, y: -40},
            dragging: false,
        }), null);

        const wheel = new WheelEvent('wheel', {
            bubbles: true,
            cancelable: true,
            altKey: true,
            deltaY: -120,
            clientX: 530,
            clientY: 320,
        });
        eventContainer.dispatchEvent(wheel);
        expect(frames.size).toBe(1);
        flushFrames();

        // Anchor=(430,270) in the viewport; target transform-origin is its center (200,150).
        expect(provider.scaleCore.scale).toBeCloseTo(2.4);
        expect(provider.scaleCore.ratio).toBeCloseTo(1.2);
        const position = positionFromTransform(dsTarget);
        expect(position.x).toBeCloseTo(-10);
        expect(position.y).toBeCloseTo(-72);
        const [scaleData, callbackEvent] = scaleCallback.mock.lastCall!;
        expect(scaleData.scale).toBeCloseTo(2.4);
        expect(scaleData.ratio).toBeCloseTo(1.2);
        expect(scaleData.position.x).toBeCloseTo(-10);
        expect(scaleData.position.y).toBeCloseTo(-72);
        expect(callbackEvent).toBe(wheel);
    });

    it('keeps the world point under a viewport-local anchor fixed for a zero-size target', () => {
        const {provider, dsTarget} = makeProvider({
            width: 900,
            height: 500,
            scale: 0.75,
            position: {x: 80, y: -30},
        });
        const anchor = {x: 330, y: 210};
        const before = worldPointAt(anchor, {x: 80, y: -30}, 0.75, 0, 0);

        provider.zoomTo(1.5, anchor);

        expect(provider.scaleCore.scale).toBe(1.5);
        const after = worldPointAt(anchor, positionFromTransform(dsTarget), provider.scaleCore.scale, 0, 0);
        expect(after.x).toBeCloseTo(before.x);
        expect(after.y).toBeCloseTo(before.y);
    });

    it('uses the nonzero target center as transform origin and defaults zoomTo anchor to viewport center', () => {
        const {provider, dsTarget} = makeProvider({
            width: 800,
            height: 600,
            targetWidth: 500,
            targetHeight: 300,
            scale: 1.25,
            position: {x: -45, y: 60},
        });
        const centerAnchor = {x: 400, y: 300};
        const before = worldPointAt(centerAnchor, {x: -45, y: 60}, 1.25, 500, 300);

        provider.zoomTo(2.5);

        const position = positionFromTransform(dsTarget);
        expect(position.x).toBeCloseTo(-240);
        expect(position.y).toBeCloseTo(-30);
        const after = worldPointAt(centerAnchor, position, provider.scaleCore.scale, 500, 300);
        expect(after.x).toBeCloseTo(before.x);
        expect(after.y).toBeCloseTo(before.y);
    });

    it('rejects invalid values and clamps valid scales to the supported range', () => {
        const {provider, dsTarget, scaleCallback} = makeProvider({position: {x: 15, y: -5}});
        const initialTransform = dsTarget.style.transform;
        const invalidCalls: Array<[number, {x: number; y: number}]> = [
            [Number.NaN, {x: 10, y: 20}],
            [Number.POSITIVE_INFINITY, {x: 10, y: 20}],
            [0, {x: 10, y: 20}],
            [-1, {x: 10, y: 20}],
            [2, {x: Number.NaN, y: 20}],
            [2, {x: 10, y: Number.NEGATIVE_INFINITY}],
        ];
        invalidCalls.forEach(([scale, position]) => provider.setViewport(scale, position));
        provider.zoomTo(Number.NaN);
        provider.zoomTo(0);

        expect(provider.scaleCore.scale).toBe(1);
        expect(provider.scaleCore.ratio).toBe(1);
        expect(dsTarget.style.transform).toBe(initialTransform);
        expect(scaleCallback).not.toHaveBeenCalled();

        provider.setViewport(0.01, {x: 4, y: 6});
        expect(provider.scaleCore.scale).toBe(0.05);
        expect(provider.scaleCore.ratio).toBe(0.05);
        provider.setViewport(8, {x: 7, y: 9});
        expect(provider.scaleCore.scale).toBe(5);
        expect(provider.scaleCore.ratio).toBe(100);
        expect(dsTarget.style.transform).toBe('translate3d(7px, 9px, 0) scale(5)');
    });

    it('cancels pending wheel and drag frames and releases pointer resources on destroy', () => {
        const {provider, eventContainer, dsTarget, setPointerCapture, releasePointerCapture,
            scaleCallback, dragCallback} = makeProvider({
            targetWidth: 300,
            targetHeight: 200,
        });
        const initialTransform = dsTarget.style.transform;
        eventContainer.dispatchEvent(new WheelEvent('wheel', {
            bubbles: true,
            cancelable: true,
            altKey: true,
            deltaY: -100,
            clientX: 350,
            clientY: 250,
        }));
        eventContainer.dispatchEvent(makePointerEvent('pointerdown', {clientX: 20, clientY: 30}));
        eventContainer.dispatchEvent(makePointerEvent('pointermove', {
            buttons: 2,
            clientX: 40,
            clientY: 50,
        }));
        expect(frames.size).toBe(2);
        expect(setPointerCapture).toHaveBeenCalledWith(7);

        provider.destroy();
        expect(frames.size).toBe(0);
        expect(cancelledFrames).toHaveLength(2);
        expect(releasePointerCapture).toHaveBeenCalledWith(7);
        expect(dsTarget.style.transform).toBe(initialTransform);

        const afterDestroyWheel = new WheelEvent('wheel', {altKey: true, deltaY: -100});
        const afterDestroyContextMenu = new MouseEvent('contextmenu', {cancelable: true});
        eventContainer.dispatchEvent(afterDestroyWheel);
        eventContainer.dispatchEvent(makePointerEvent('pointerdown'));
        eventContainer.dispatchEvent(makePointerEvent('pointermove', {buttons: 2, clientX: 80, clientY: 90}));
        eventContainer.dispatchEvent(makePointerEvent('pointerup'));
        eventContainer.dispatchEvent(afterDestroyContextMenu);
        flushFrames();

        expect(frames.size).toBe(0);
        expect(provider.scaleCore.scale).toBe(1);
        expect(dsTarget.style.transform).toBe(initialTransform);
        expect(scaleCallback).not.toHaveBeenCalled();
        expect(dragCallback).not.toHaveBeenCalled();
        expect(setPointerCapture).toHaveBeenCalledTimes(1);
        expect(afterDestroyContextMenu.defaultPrevented).toBe(false);
    });

    it('continues an active right drag from the position supplied by setViewport', () => {
        const {provider, eventContainer, dsTarget, dragCallback} = makeProvider({
            position: {x: 10, y: 20},
        });
        eventContainer.dispatchEvent(makePointerEvent('pointerdown', {
            pointerId: 8,
            clientX: 100,
            clientY: 100,
        }));
        eventContainer.dispatchEvent(makePointerEvent('pointermove', {
            pointerId: 8,
            buttons: 2,
            clientX: 120,
            clientY: 130,
        }));
        flushFrames();
        expect(positionFromTransform(dsTarget)).toEqual({x: 30, y: 50});

        provider.setViewport(2, {x: 300, y: 400});
        expect(positionFromTransform(dsTarget)).toEqual({x: 300, y: 400});
        eventContainer.dispatchEvent(makePointerEvent('pointermove', {
            pointerId: 8,
            buttons: 2,
            clientX: 126,
            clientY: 137,
        }));
        expect(frames.size).toBe(1);
        flushFrames();

        expect(provider.scaleCore.scale).toBe(2);
        expect(positionFromTransform(dsTarget)).toEqual({x: 306, y: 407});
        expect(dragCallback).toHaveBeenLastCalledWith(expect.objectContaining({
            scale: 2,
            position: {x: 306, y: 407},
            dragging: true,
        }), expect.objectContaining({pointerId: 8}));
    });

    it('cancels pending drag once on pointercancel or lost capture and ignores later moves', () => {
        for (const cancelType of ['pointercancel', 'lostpointercapture']) {
            const {eventContainer, dsTarget, dragCallback, dragEndCallback} = makeProvider();
            eventContainer.dispatchEvent(makePointerEvent('pointerdown', {
                pointerId: 9,
                clientX: 10,
                clientY: 15,
            }));
            eventContainer.dispatchEvent(makePointerEvent('pointermove', {
                pointerId: 9,
                buttons: 2,
                clientX: 30,
                clientY: 45,
            }));
            const pendingFrame = Array.from(frames.keys())[0];
            expect(frames.size).toBe(1);

            eventContainer.dispatchEvent(makePointerEvent(cancelType, {pointerId: 9}));
            eventContainer.dispatchEvent(makePointerEvent(
                cancelType === 'pointercancel' ? 'lostpointercapture' : 'pointercancel',
                {pointerId: 9},
            ));
            eventContainer.dispatchEvent(makePointerEvent('pointermove', {
                pointerId: 9,
                buttons: 2,
                clientX: 60,
                clientY: 80,
            }));
            flushFrames();

            expect(frames.size).toBe(0);
            expect(cancelledFrames).toContain(pendingFrame);
            expect(positionFromTransform(dsTarget)).toEqual({x: 0, y: 0});
            expect(dragCallback).not.toHaveBeenCalled();
            expect(dragEndCallback).toHaveBeenCalledTimes(1);
            expect(dragEndCallback.mock.lastCall?.[0].position).toEqual({x: 0, y: 0});
        }
    });
});
