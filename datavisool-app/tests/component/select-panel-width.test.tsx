/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {fireEvent, render, screen, waitFor} from '@testing-library/react';
import {afterEach, beforeEach, describe, expect, test, vi} from 'vitest';
import {Select} from '@datavisool/ui';

const buildRect = (rect: Partial<DOMRect> = {}): DOMRect => ({
    x: rect.x ?? rect.left ?? 0,
    y: rect.y ?? rect.top ?? 0,
    width: rect.width ?? 0,
    height: rect.height ?? 0,
    top: rect.top ?? 0,
    right: rect.right ?? ((rect.left ?? 0) + (rect.width ?? 0)),
    bottom: rect.bottom ?? ((rect.top ?? 0) + (rect.height ?? 0)),
    left: rect.left ?? 0,
    toJSON: () => undefined,
} as DOMRect);

describe('Select panel width', () => {
    beforeEach(() => {
        Object.defineProperty(window, 'innerWidth', {
            configurable: true,
            writable: true,
            value: 1280,
        });
        Object.defineProperty(window, 'innerHeight', {
            configurable: true,
            writable: true,
            value: 900,
        });

        vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function() {
            const element = this as HTMLElement;
            if (element.classList.contains('datavisool-select-shell')) {
                return buildRect({
                    x: 80,
                    y: 100,
                    top: 100,
                    left: 80,
                    width: 260,
                    height: 42,
                });
            }
            if (element.classList.contains('datavisool-select-panel')) {
                return buildRect({
                    x: 80,
                    y: 150,
                    top: 150,
                    left: 80,
                    width: 420,
                    height: 140,
                });
            }
            return buildRect();
        });
    });

    afterEach(() => {
        vi.restoreAllMocks();
    });

    test('lets the panel grow with content and supports custom max width', async () => {
        render(
            <Select
                placeholder="请选择模型"
                panelMaxWidth={720}
                options={[
                    {label: '这是一个很长的模型名称，用来验证面板宽度不会过早被压缩成省略号展示', value: 'long-model'},
                    {label: '短名称', value: 'short-model'},
                ]}
            />,
        );

        fireEvent.click(screen.getByRole('combobox'));
        const panel = await waitFor(() => {
            const currentPanel = document.querySelector('.datavisool-select-panel') as HTMLDivElement | null;
            expect(currentPanel).toBeTruthy();
            return currentPanel!;
        });

        await waitFor(() => {
            expect(panel.style.position).toBe('fixed');
            expect(panel.style.minWidth).toBe('260px');
        });
        expect(panel.style.width).toBe('max-content');
        expect(panel.style.maxWidth).toBe('min(720px, calc(100vw - 32px))');
    });

    test('uses the default max width when the consumer does not override it', async () => {
        render(
            <Select
                placeholder="请选择模型"
                options={[
                    {label: '这是一个很长的模型名称，用来验证默认最大宽度策略', value: 'long-model'},
                    {label: '短名称', value: 'short-model'},
                ]}
            />,
        );

        fireEvent.click(screen.getByRole('combobox'));
        const panel = await waitFor(() => {
            const currentPanel = document.querySelector('.datavisool-select-panel') as HTMLDivElement | null;
            expect(currentPanel).toBeTruthy();
            return currentPanel!;
        });

        await waitFor(() => {
            expect(panel.style.position).toBe('fixed');
        });
        expect(panel.style.maxWidth).toBe('min(560px, calc(100vw - 32px))');
    });
});
