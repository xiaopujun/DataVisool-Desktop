/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';

beforeEach(() => {
    vi.resetModules();
    vi.useFakeTimers();
    vi.stubGlobal('navigator', {languages: ['zh-CN'], language: 'zh-CN'});
    localStorage.clear();
});

afterEach(() => {
    document.body.replaceChildren();
    localStorage.clear();
    delete window.datavisoolI18n;
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
});

describe('DataVisool language settings', () => {
    it('ignores the old setting and falls back to the system language', async () => {
        localStorage.setItem('lc-i18n-locale', 'en-US');
        const i18n = await import('@/i18n/miniI18n.ts');
        expect(i18n.getLocale()).toBe('zh-CN');
        expect(localStorage.getItem('lc-i18n-locale')).toBe('en-US');
        expect(localStorage.getItem('datavisool-i18n-locale')).toBeNull();
        expect(window.datavisoolI18n?.getLocale()).toBe('zh-CN');
    });

    it('reads and updates the new setting and emits the new event', async () => {
        localStorage.setItem('datavisool-i18n-locale', 'en-US');
        const i18n = await import('@/i18n/miniI18n.ts');
        expect(i18n.getLocale()).toBe('en-US');
        const currentEvent = vi.fn();
        const oldEvent = vi.fn();
        window.addEventListener('datavisool-i18n-locale-change', currentEvent);
        window.addEventListener('lc-i18n-locale-change', oldEvent);
        i18n.setLocale('zh-CN');
        expect(localStorage.getItem('datavisool-i18n-locale')).toBe('zh-CN');
        expect(currentEvent).toHaveBeenCalledOnce();
        expect((currentEvent.mock.calls[0][0] as CustomEvent).detail).toEqual({locale: 'zh-CN'});
        expect(oldEvent).not.toHaveBeenCalled();
        window.removeEventListener('datavisool-i18n-locale-change', currentEvent);
        window.removeEventListener('lc-i18n-locale-change', oldEvent);
    });

    it('uses the new DOM markers for skipped content and original attributes', async () => {
        const i18n = await import('@/i18n/miniI18n.ts');
        const container = document.createElement('div');
        container.innerHTML = '<div data-datavisool-i18n-skip="true">保存</div><span>保存</span><input placeholder="保存">';
        document.body.append(container);
        i18n.setLocale('en-US');
        i18n.translateDom(container);
        expect(container.querySelector('div')?.textContent).toBe('保存');
        expect(container.querySelector('span')?.textContent).toBe('Save');
        expect(container.querySelector('input')?.getAttribute('placeholder')).toBe('Save');
        expect(container.querySelector('input')?.getAttribute('data-datavisool-i18n-source-placeholder')).toBe('保存');
        i18n.setLocale('zh-CN');
        i18n.translateDom(container);
        expect(container.querySelector('span')?.textContent).toBe('保存');
        expect(container.querySelector('input')?.getAttribute('placeholder')).toBe('保存');
    });
});
