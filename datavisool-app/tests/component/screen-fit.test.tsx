/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {act, render} from '@testing-library/react';
import {afterEach, beforeEach, describe, expect, test, vi} from 'vitest';
import ScreenFit from '@/framework/screen-fit/ScreenFit.tsx';

describe('ScreenFit', () => {
    let availableWidth: number;
    let availableHeight: number;
    let resizeCallback: ResizeObserverCallback;

    beforeEach(() => {
        vi.useFakeTimers();
        availableWidth = 1200;
        availableHeight = 755;

        vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(function () {
            if (this.classList.contains('react-screen-box'))
                return availableWidth;
            return Number.parseFloat(this.style.width) || 0;
        });
        vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(function () {
            if (this.classList.contains('react-screen-box'))
                return availableHeight;
            return Number.parseFloat(this.style.height) || 0;
        });

        class ResizeObserverMock {
            constructor(callback: ResizeObserverCallback) {
                resizeCallback = callback;
            }

            observe() {}

            unobserve() {}

            disconnect() {}
        }

        vi.stubGlobal('ResizeObserver', ResizeObserverMock);
    });

    afterEach(() => {
        vi.useRealTimers();
        vi.restoreAllMocks();
        vi.unstubAllGlobals();
    });

    test('uses the available container height instead of the full body height', () => {
        const {container} = render(
            <ScreenFit width={1080} height={1920} bodyOverflowHidden={false}>
                <div/>
            </ScreenFit>,
        );

        const wrapper = container.querySelector('.screen-wrapper') as HTMLDivElement;
        const expectedScale = availableHeight / 1920;

        expect(wrapper.style.transform).toBe(`scale(${expectedScale},${expectedScale})`);
        expect(Number.parseFloat(wrapper.style.marginTop)).toBeCloseTo(0, 10);
        expect(1920 * expectedScale).toBeCloseTo(availableHeight, 10);
    });

    test('recalculates when the available container size changes', () => {
        const {container} = render(
            <ScreenFit width={1080} height={1920} bodyOverflowHidden={false} delay={0}>
                <div/>
            </ScreenFit>,
        );
        const wrapper = container.querySelector('.screen-wrapper') as HTMLDivElement;

        availableHeight = 700;
        act(() => {
            resizeCallback([], {} as ResizeObserver);
            vi.runAllTimers();
        });

        const expectedScale = availableHeight / 1920;
        expect(wrapper.style.transform).toBe(`scale(${expectedScale},${expectedScale})`);
        expect(1920 * expectedScale).toBeCloseTo(availableHeight, 10);
    });
});
