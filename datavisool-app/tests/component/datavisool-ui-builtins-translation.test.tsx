/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import * as React from 'react';
import {cleanup, render, screen} from '@testing-library/react';
import {afterEach, describe, expect, test, vi} from 'vitest';
import {ColorMode, ControlGroup, Empty, DatavisoolGUI, DataVisoolUIProvider, Pagination, Select, type Control} from '@datavisool/ui';

afterEach(cleanup);

describe('DataVisool UI built-in translations', () => {
    test('keeps the original defaults without a provider', () => {
        render(<><ColorMode/><Empty/><Pagination total={12} showTotal/></>);

        expect(screen.getAllByText('单色')).not.toHaveLength(0);
        expect(screen.getByText('暂无数据')).toBeInTheDocument();
        expect(screen.getByText('共 12 条')).toBeInTheDocument();
    });

    test('updates built-in text and templates when locale changes with stable children and translator', () => {
        const translations: Record<string, string> = {
            '单色': 'Solid',
            '当前使用单色配置': 'Using a solid color',
            '暂无数据': 'No data',
            '请选择': 'Choose',
            '共 {total} 条': '{total} records',
            '第 {page} 页': 'Page {page}',
        };
        let locale = 'zh-CN';
        const translate = (text: string) => locale === 'en-US' ? translations[text] ?? text : text;
        const stableChildren = <><ColorMode/><Empty/><Select options={[]}/><Pagination total={12} showTotal/></>;
        const MemoChildren = React.memo(() => stableChildren);
        const {container, rerender} = render(
            <DataVisoolUIProvider locale="zh-CN" translate={translate}><MemoChildren/></DataVisoolUIProvider>,
        );

        expect(screen.getAllByText('单色')).not.toHaveLength(0);
        expect(screen.getByRole('combobox')).toHaveTextContent('请选择');

        locale = 'en-US';
        rerender(<DataVisoolUIProvider locale="en-US" translate={translate}><MemoChildren/></DataVisoolUIProvider>);

        expect(screen.getAllByText('Solid')).not.toHaveLength(0);
        expect(container.querySelector('.datavisool-color-picker-mode-badge')).toHaveTextContent('Solid');
        expect(screen.getByText('Using a solid color')).toBeInTheDocument();
        expect(screen.getByText('No data')).toBeInTheDocument();
        expect(screen.getByRole('combobox')).toHaveTextContent('Choose');
        expect(screen.getByText('12 records')).toBeInTheDocument();
        expect(screen.getByRole('button', {name: 'Page 2'})).toBeInTheDocument();
    });

    test('keeps explicit text props and passes an explicit DatavisoolGUI translator to built-in defaults', () => {
        const providerTranslate = vi.fn((text: string) => `provider:${text}`);
        const explicitTranslate = (text: string) => `explicit:${text}`;
        const schema: Control = {type: 'select', key: 'mode', config: {options: []}};

        render(
            <DataVisoolUIProvider locale="en-US" translate={providerTranslate}>
                <Empty title="Custom empty text"/>
                <DatavisoolGUI schema={schema} translate={explicitTranslate}/>
            </DataVisoolUIProvider>,
        );

        expect(screen.getByText('Custom empty text')).toBeInTheDocument();
        expect(screen.getByRole('combobox')).toHaveTextContent('explicit:请选择');
        expect(providerTranslate).not.toHaveBeenCalledWith('请选择');
    });

    test('translates ControlGroup default item names with caller-provided entries', () => {
        const translations: Record<string, string> = {
            '系列': 'Series ',
            '新增{itemName}': 'Add {itemName}',
            '删除{itemName}': 'Remove {itemName}',
        };

        render(
            <DataVisoolUIProvider translate={(text) => translations[text] ?? text}>
                <ControlGroup
                    defaultOpen
                    defaultValue={[{}]}
                    template={{type: 'input', key: 'name'}}
                />
            </DataVisoolUIProvider>,
        );

        expect(screen.getByText('Series 1')).toBeInTheDocument();
        expect(screen.getByRole('button', {name: 'Add Series'})).toBeInTheDocument();
        expect(screen.getByRole('button', {name: 'Remove Series'})).toBeInTheDocument();
    });
});
