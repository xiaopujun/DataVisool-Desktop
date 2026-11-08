/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import React from 'react';
import {act, fireEvent, render, screen} from '@testing-library/react';
import {afterEach, describe, expect, it, vi} from 'vitest';
import {Dialog} from '@datavisool/ui';

afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
});

describe('弹窗显示与定位', () => {
    it('动画帧不执行时仍可显示并居中，窗口缩放后重新居中', () => {
        const frames = vi.spyOn(window, 'requestAnimationFrame').mockImplementation(() => 1);
        vi.spyOn(HTMLElement.prototype, 'offsetWidth', 'get').mockReturnValue(400);
        vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockReturnValue(200);
        const view = render(<Dialog open={false} mask centered>弹窗内容</Dialog>);
        view.rerender(<Dialog open mask centered>弹窗内容</Dialog>);
        const dialog = screen.getByRole('dialog');
        const wrapper = dialog.parentElement!;
        expect(dialog).toHaveClass('visible');
        expect(document.querySelector('.datavisool-dialog-mask')).toHaveClass('visible');
        expect(wrapper.style.transform).toBe(`translate(${(window.innerWidth - 400) / 2}px, ${(window.innerHeight - 200) / 2}px)`);
        expect(frames).not.toHaveBeenCalled();
        const oldWidth = window.innerWidth;
        Object.defineProperty(window, 'innerWidth', {configurable: true, writable: true, value: 800});
        fireEvent(window, new Event('resize'));
        expect(wrapper.style.transform).toBe(`translate(200px, ${(window.innerHeight - 200) / 2}px)`);
        Object.defineProperty(window, 'innerWidth', {configurable: true, writable: true, value: oldWidth});
    });

    it('关闭后卸载，关闭动画期间重新打开不会被旧定时器移除', () => {
        vi.useFakeTimers();
        const onClose = vi.fn();
        const view = render(<Dialog open mask onClose={onClose}>弹窗内容</Dialog>);
        fireEvent.click(document.querySelector('.datavisool-dialog-mask')!);
        expect(onClose).toHaveBeenCalledOnce();
        view.rerender(<Dialog open={false} mask onClose={onClose}>弹窗内容</Dialog>);
        expect(screen.getByRole('dialog')).not.toHaveClass('visible');
        act(() => vi.advanceTimersByTime(100));
        view.rerender(<Dialog open mask onClose={onClose}>弹窗内容</Dialog>);
        act(() => vi.advanceTimersByTime(150));
        expect(screen.getByRole('dialog')).toHaveClass('visible');
        view.rerender(<Dialog open={false} mask onClose={onClose}>弹窗内容</Dialog>);
        act(() => vi.advanceTimersByTime(200));
        expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });
});
