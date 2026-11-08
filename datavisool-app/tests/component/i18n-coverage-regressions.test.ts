/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, it} from 'vitest';
import {createElement} from 'react';
import {cleanup, render, screen} from '@testing-library/react';
import {DataVisoolUIProvider, Pagination} from '@datavisool/ui';
import {getLocale, setLocale, translateDom, tx} from '@/i18n/miniI18n.ts';

const previousLocale = getLocale();

afterEach(() => {
    setLocale(previousLocale);
    cleanup();
    document.body.replaceChildren();
});

describe('English application translations', () => {
    it('translates representative AI, component, and resource management text', () => {
        setLocale('en-US');
        expect(tx('请输入需求')).toBe('Enter your request');
        expect(tx('AI 运行设置')).toBe('AI run settings');
        expect(tx('调整当前组件的模糊、明暗和色彩。')).toBe('Adjust the blur, brightness, and colors of the current component.');
        expect(tx('主题预设列表')).toBe('Theme presets');
        expect(tx('确定删除这个脚本吗？此操作无法撤销。')).toBe('Delete this script? This cannot be undone.');
        expect(tx('第 {count} 轮').replace('{count}', '3')).toBe('Turn 3');
        expect(tx('共 {total} 条').replace('{total}', '7')).toBe('7 items');
    });

    it('translates dynamic messages without changing filenames or server details', () => {
        setLocale('en-US');
        expect(tx('文件过大：示例视频.mp4，请控制在 200MB 以内'))
            .toBe('File too large: 示例视频.mp4. The limit is 200 MB.');
        expect(tx('上传失败: network error')).toBe('Upload failed: network error');
        expect(tx('最多上传 8 个字体文件')).toBe('Upload up to 8 font files');
        expect(tx('12 个组件')).toBe('12 components');
        expect(tx('4 个图层')).toBe('4 layers');
        expect(tx('将删除 2 个组件：标题、图表')).toBe('Delete 2 components: 标题、图表');
        expect(tx('临时草稿包含 4 个图层')).toBe('The temporary draft contains 4 layers');
        expect(tx('请选择 1–3 个组件（已选 2 个）')).toBe('Select 1–3 components (2 selected)');
    });

    it('updates interface text and attributes while preserving skipped project content', () => {
        document.body.innerHTML = '<p>调整当前组件的模糊、明暗和色彩。</p><input placeholder="请输入需求"><div data-datavisool-i18n-skip="true">主题预设列表</div>';
        setLocale('en-US');
        translateDom(document.body);
        expect(document.querySelector('p')?.textContent).toBe('Adjust the blur, brightness, and colors of the current component.');
        expect(document.querySelector('input')?.getAttribute('placeholder')).toBe('Enter your request');
        expect(document.querySelector('div')?.textContent).toBe('主题预设列表');
        setLocale('zh-CN');
        translateDom(document.body);
        expect(document.querySelector('p')?.textContent).toBe('调整当前组件的模糊、明暗和色彩。');
        expect(document.querySelector('input')?.getAttribute('placeholder')).toBe('请输入需求');
    });

    it('supplies translated templates to shared UI components', () => {
        setLocale('en-US');
        render(createElement(DataVisoolUIProvider, {locale: 'en-US', translate: tx},
            createElement(Pagination, {total: 12, showTotal: true})));
        expect(screen.getByText('12 items')).toBeInTheDocument();
        expect(screen.getByRole('button', {name: 'Page 2'})).toBeInTheDocument();
    });
});
