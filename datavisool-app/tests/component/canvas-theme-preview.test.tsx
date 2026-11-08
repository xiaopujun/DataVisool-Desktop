/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {act, cleanup, fireEvent, render, screen, within} from '@testing-library/react';
import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import ThemeHdItemImpl from '@/designer/header/items/theme/ThemeHdItemImpl.tsx';
import ThemePreviewCharts from '@/designer/header/items/theme/preview/ThemePreviewCharts.tsx';
import ThemeManager from '@/designer/header/items/theme/ThemeManager.ts';
import designerHeaderStore from '@/designer/header/DesignerHeaderStore.ts';
import type {ThemeColors} from "@datavisool/sdk/types";

const echarts = vi.hoisted(() => ({instances: [] as Array<{
    setOption: ReturnType<typeof vi.fn>;
    resize: ReturnType<typeof vi.fn>;
    dispose: ReturnType<typeof vi.fn>;
}>}));

vi.mock('echarts/core', () => ({
    use: vi.fn(),
    init: vi.fn(() => {
        const chart = {setOption: vi.fn(), resize: vi.fn(), dispose: vi.fn()};
        echarts.instances.push(chart);
        return chart;
    }),
}));

describe('canvas theme preview isolation', () => {
    const previousEnvironment = Object.getOwnPropertyDescriptor(window, 'DATAVISOOL_ENV');
    let themeManager: ThemeManager;
    let flashGlobalTheme: ReturnType<typeof vi.fn>;
    let observers: Array<{callback: ResizeObserverCallback; disconnect: ReturnType<typeof vi.fn>}>;
    let frames: Map<number, FrameRequestCallback>;
    let nextFrame: number;

    beforeEach(() => {
        themeManager = new ThemeManager();
        flashGlobalTheme = vi.fn();
        window.DATAVISOOL_ENV = {currentManager: {themeManager, layerManager: {flashGlobalTheme}}} as unknown as Window['DATAVISOOL_ENV'];
        designerHeaderStore.setCurrentKey('theme');
        echarts.instances.length = 0;
        observers = [];
        frames = new Map();
        nextFrame = 0;
        vi.stubGlobal('ResizeObserver', class {
            disconnect = vi.fn();
            observe = vi.fn();
            constructor(callback: ResizeObserverCallback) {
                observers.push({callback, disconnect: this.disconnect});
            }
        });
        vi.stubGlobal('requestAnimationFrame', vi.fn((callback: FrameRequestCallback) => {
            frames.set(++nextFrame, callback);
            return nextFrame;
        }));
        vi.stubGlobal('cancelAnimationFrame', vi.fn((id: number) => frames.delete(id)));
    });

    afterEach(() => {
        cleanup();
        designerHeaderStore.setCurrentKey('');
        if (previousEnvironment)
            Object.defineProperty(window, 'DATAVISOOL_ENV', previousEnvironment);
        else
            delete (window as Partial<Window>).DATAVISOOL_ENV;
        vi.unstubAllGlobals();
        vi.restoreAllMocks();
    });

    const selectTheme = (name: string) => {
        const list = document.querySelector('.datavisool-app-theme-list-container') as HTMLElement;
        fireEvent.click(within(list).getByText(name, {exact: true}));
    };

    it('shows an initial example without selecting or applying a theme', () => {
        render(<ThemeHdItemImpl/>);
        expect(screen.getByLabelText('主题效果预览')).toHaveTextContent('科技风格(默认主题)');
        expect(document.querySelector('.datavisool-app-theme-item-active')).toBeNull();

        fireEvent.click(screen.getByRole('button', {name: '更新全局主题'}));
        expect(flashGlobalTheme).not.toHaveBeenCalled();
        expect(designerHeaderStore.currentKey).toBe('theme');
    });

    it('updates only the example until the latest exact theme payload is applied', () => {
        const initialThemes = JSON.stringify(themeManager.getData());
        render(<ThemeHdItemImpl/>);
        selectTheme('烈焰战区');
        const preview = screen.getByLabelText('主题效果预览');
        expect(preview).toHaveTextContent('烈焰战区');
        expect(preview).toHaveTextContent('#FF5500');
        expect(echarts.instances[0].setOption.mock.lastCall?.[0].series[0].lineStyle.color).toBe('#FF5500');
        selectTheme('数字森林');
        expect(echarts.instances[0].setOption.mock.lastCall?.[0].series[0].lineStyle.color).toBe('#00FF41');
        expect(flashGlobalTheme).not.toHaveBeenCalled();
        expect(JSON.stringify(themeManager.getData())).toBe(initialThemes);

        fireEvent.click(screen.getByRole('button', {name: '更新全局主题'}));
        expect(flashGlobalTheme).toHaveBeenCalledTimes(1);
        expect(flashGlobalTheme).toHaveBeenCalledWith(themeManager.getData().find(theme => theme.id === '4'));
        expect(designerHeaderStore.currentKey).toBe('');
    });

    it('preserves the existing repeated selection behavior and selected payload', () => {
        render(<ThemeHdItemImpl/>);
        selectTheme('烈焰战区');
        selectTheme('烈焰战区');
        expect(document.querySelector('.datavisool-app-theme-item-active')).toBeNull();
        expect(flashGlobalTheme).not.toHaveBeenCalled();

        fireEvent.click(screen.getByRole('button', {name: '更新全局主题'}));
        expect(flashGlobalTheme).toHaveBeenCalledWith(themeManager.getData()[1]);
    });

    it.each(['cancel', 'close', 'mask'])('does not apply the preview on %s', (action) => {
        render(<ThemeHdItemImpl/>);
        selectTheme('复古工业');
        if (action === 'mask')
            fireEvent.click(document.querySelector('.datavisool-dialog-mask')!);
        else
            fireEvent.click(screen.getByRole('button', {name: action === 'cancel' ? '取消' : '关闭'}));

        expect(flashGlobalTheme).not.toHaveBeenCalled();
        expect(designerHeaderStore.currentKey).toBe('');
    });

    it('keeps custom theme saving separate from applying the selected global theme', () => {
        render(<ThemeHdItemImpl/>);
        selectTheme('烈焰战区');
        fireEvent.click(screen.getByRole('button', {name: '自定义主题'}));
        const editor = screen.getByText('编辑主题').closest('[role="dialog"]') as HTMLElement;
        fireEvent.change(within(editor).getByPlaceholderText('请输入主题名称'), {target: {value: '预览隔离测试主题'}});
        fireEvent.click(within(editor).getByRole('button', {name: '添加 / 更新'}));
        expect(themeManager.getData()).toHaveLength(8);
        expect(themeManager.getData()[7].name).toBe('预览隔离测试主题');
        expect(flashGlobalTheme).not.toHaveBeenCalled();
        fireEvent.click(within(editor).getByRole('button', {name: '关闭'}));
        expect(designerHeaderStore.currentKey).toBe('theme');

        fireEvent.click(screen.getByRole('button', {name: '更新全局主题'}));
        expect(flashGlobalTheme).toHaveBeenCalledWith(themeManager.getData()[1]);
    });

    it('uses fallback colors for an incomplete custom theme without changing its saved payload', () => {
        themeManager.updateThemeConfig([{id: 'partial', name: '部分配色', colors: {main: '#123456'}}]);
        render(<ThemeHdItemImpl/>);
        selectTheme('部分配色');
        expect(screen.getByLabelText('主题效果预览')).toHaveTextContent('#62EDFF');
        fireEvent.click(screen.getByRole('button', {name: '更新全局主题'}));
        expect(flashGlobalTheme).toHaveBeenCalledWith({id: 'partial', name: '部分配色', colors: {main: '#123456'}});
    });

    it('reuses chart instances on color changes and cancels resizing when unmounted', () => {
        const colors = themeManager.getData()[0].colors as Required<ThemeColors>;
        const view = render(<ThemePreviewCharts colors={colors}/>);
        expect(echarts.instances).toHaveLength(2);
        view.rerender(<ThemePreviewCharts colors={{...colors, main: '#FF5500'}}/>);
        expect(echarts.instances).toHaveLength(2);
        expect(echarts.instances[0].setOption.mock.lastCall?.[0].series[0].lineStyle.color).toBe('#FF5500');

        const observer = observers[0];
        act(() => observer.callback([], {} as ResizeObserver));
        expect(frames.size).toBe(1);
        act(() => {
            for (const [id, callback] of frames) {
                frames.delete(id);
                callback(0);
            }
        });
        expect(echarts.instances[0].resize).toHaveBeenCalledTimes(1);
        act(() => observer.callback([], {} as ResizeObserver));
        view.unmount();
        expect(frames.size).toBe(0);
        expect(observer.disconnect).toHaveBeenCalledTimes(1);
        echarts.instances.forEach(chart => expect(chart.dispose).toHaveBeenCalledTimes(1));
        act(() => observer.callback([], {} as ResizeObserver));
        expect(frames.size).toBe(0);
    });
});
