/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, beforeEach, describe, expect, test, vi} from 'vitest';
import {InteractionEngine} from '@datavisool/interaction-engine';

vi.mock('@/framework/core/ScaleAction.ts', () => ({default: {doScale: vi.fn()}}));
vi.mock('@/designer/event/ContextMenuStore.ts', () => ({default: {
    setPointerTarget: vi.fn(), beginRightDrag: vi.fn(), startRightDrag: vi.fn(), endRightDrag: vi.fn(),
}}));
vi.mock('@/designer/operate-provider/EventOperateStore.ts', () => ({default: {setScale: vi.fn(), setRatio: vi.fn()}}));
vi.mock('@/designer/operate-provider/hot-key/ShortcutStore.ts', () => ({default: {isActiveWithExactModifiers: () => false}}));

import ViewportController from '@/designer/workspace/viewport/ViewportController.ts';
import viewportStore from '@/designer/workspace/viewport/ViewportStore.ts';

describe('wheel input during canvas pointer gestures', () => {
    let controller: ViewportController;
    let viewport: HTMLDivElement;
    let artboard: HTMLDivElement;
    let engine: InteractionEngine | undefined;
    const camera = {translationX: 0, translationY: 0, scale: 1};

    const pointer = (target: EventTarget, type: string, init: PointerEventInit = {}) => {
        target.dispatchEvent(new PointerEvent(type, {bubbles: true, cancelable: true, pointerId: 1, ...init}));
    };
    const wheel = (target: EventTarget = artboard, init: WheelEventInit = {}) => {
        // Some browsers report buttons=0 even while a pointer is held down.
        const event = new WheelEvent('wheel', {bubbles: true, cancelable: true, deltaY: 100, buttons: 0, ...init});
        target.dispatchEvent(event);
        return event;
    };

    beforeEach(() => {
        vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
        vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
        vi.stubGlobal('cancelAnimationFrame', vi.fn());
        viewportStore.setCamera(camera);
        viewportStore.setToolMode('select');
        const ruler = document.createElement('div');
        ruler.className = 'datavisool-app-ruler-content';
        viewport = document.createElement('div');
        viewport.className = 'designer-ds-container';
        viewport.tabIndex = -1;
        viewport.setPointerCapture = vi.fn();
        viewport.releasePointerCapture = vi.fn();
        viewport.hasPointerCapture = () => false;
        artboard = document.createElement('div');
        viewport.append(artboard);
        ruler.append(viewport);
        document.body.append(ruler);
        viewport.getBoundingClientRect = () => new DOMRect(0, 0, 1280, 800);
        artboard.getBoundingClientRect = () => new DOMRect(
            viewportStore.translationX, viewportStore.translationY, 1920 * viewportStore.scale, 1080 * viewportStore.scale,
        );
        controller = new ViewportController({viewport, artboard, getArtboardBounds: () => ({x: 0, y: 0, width: 1920, height: 1080})});
    });

    afterEach(() => {
        engine?.destroy();
        engine = undefined;
        controller.destroy();
        document.body.replaceChildren();
        vi.unstubAllGlobals();
    });

    test.each([false, true])('keeps a dragged layer under the pointer despite wheel input (Alt: %s)', (altKey) => {
        const layer = {id: 'layer', x: 200, y: 250, width: 100, height: 100, rotate: 0};
        const element = document.createElement('div');
        element.id = layer.id;
        element.style.transform = 'translate(200px, 250px)';
        artboard.append(element);
        let selectedIds: string[] = [];
        const commit = vi.fn();
        engine = new InteractionEngine({
            container: artboard, eventContainer: viewport, coordinateContainer: artboard,
            getScale: () => viewportStore.scale,
            getLayers: () => [layer], getSelectedIds: () => selectedIds,
            getElementById: () => element, snapMode: 'off',
            onSelect: (ids) => { selectedIds = ids; }, onDragEnd: commit,
        });
        engine.mount();

        pointer(element, 'pointerdown', {clientX: 220, clientY: 270, buttons: 1});
        pointer(window, 'pointermove', {clientX: 280, clientY: 330, buttons: 1});
        expect(wheel(artboard, {altKey}).defaultPrevented).toBe(true);
        pointer(window, 'pointerup', {clientX: 300, clientY: 350});

        expect(viewportStore.camera).toEqual(camera);
        expect(commit).toHaveBeenCalledWith([{id: 'layer', x: 280, y: 330}]);
        wheel();
        expect(viewportStore.translationY).toBe(-100);
    });

    test.each([1, 2])('does not interrupt a canvas pan with wheel zoom (button: %s)', (button) => {
        const buttons = button === 1 ? 4 : 2;
        pointer(artboard, 'pointerdown', {button, buttons, clientX: 100, clientY: 100});
        pointer(viewport, 'pointermove', {buttons, clientX: 120, clientY: 120});
        wheel(viewport, {altKey: true});
        wheel(viewport);
        pointer(viewport, 'pointerup', {button, clientX: 140, clientY: 140});

        expect(viewportStore.camera).toEqual({...camera, translationX: 40, translationY: 40});
    });

    test.each(['pointerup', 'pointercancel', 'blur'])('resumes wheel input after %s outside the viewport', (eventType) => {
        pointer(artboard, 'pointerdown', {buttons: 1});
        wheel();
        expect(viewportStore.camera).toEqual(camera);
        if (eventType === 'blur') window.dispatchEvent(new Event('blur'));
        else pointer(document.body, eventType);

        wheel();
        expect(viewportStore.translationY).toBe(-100);
        wheel(artboard, {altKey: true});
        expect(viewportStore.scale).toBeCloseTo(1 / 1.2);
    });

    test('waits for every active pointer to finish', () => {
        pointer(artboard, 'pointerdown', {pointerId: 1, buttons: 1});
        pointer(artboard, 'pointerdown', {pointerId: 2, buttons: 1});
        pointer(window, 'pointerup', {pointerId: 1});
        wheel();
        expect(viewportStore.camera).toEqual(camera);
        pointer(window, 'pointercancel', {pointerId: 2});
        wheel();
        expect(viewportStore.translationY).toBe(-100);
    });

    test('preserves native scrolling inside a component while the pointer is held', () => {
        const scroller = document.createElement('div');
        scroller.style.overflowY = 'auto';
        Object.defineProperties(scroller, {clientHeight: {value: 100}, scrollHeight: {value: 400}, scrollTop: {value: 20}});
        artboard.append(scroller);
        pointer(scroller, 'pointerdown', {buttons: 1});

        expect(wheel(scroller).defaultPrevented).toBe(false);
        expect(viewportStore.camera).toEqual(camera);
    });
});
