/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import * as React from 'react';
import {act, cleanup, fireEvent, render, screen} from '@testing-library/react';
import {afterEach, beforeEach, describe, expect, test, vi} from 'vitest';
import {ControlGroup, DatavisoolGUI, DataVisoolUIProvider, type Control} from '@datavisool/ui';

const localeHarness = vi.hoisted(() => ({
    current: 'zh-CN' as 'zh-CN' | 'en-US',
    listeners: new Set<(locale: string) => void>(),
    txCalls: [] as string[],
    unsubscribeCount: 0,
}));

// Keep the same translator reference when locale changes, as the app does.
vi.mock('@/i18n/miniI18n.ts', () => ({
    tx: vi.fn((text: string) => {
        localeHarness.txCalls.push(text);
        return `${localeHarness.current === 'en-US' ? 'en' : 'zh'}:${text}`;
    }),
    getLocale: () => localeHarness.current,
    subscribeLocaleChange: (listener: (locale: string) => void) => {
        localeHarness.listeners.add(listener);
        return () => {
            localeHarness.listeners.delete(listener);
            localeHarness.unsubscribeCount += 1;
        };
    },
}));

import {getLocale, tx} from '@/i18n/miniI18n.ts';
import {useLocaleVersion} from '@/i18n/useLocaleVersion.ts';

function TestAppRoot({children}: {children: React.ReactNode}) {
    useLocaleVersion();
    return <DataVisoolUIProvider locale={getLocale()} translate={tx}>{children}</DataVisoolUIProvider>;
}

const inputControl = (label: string, placeholder: string): Control => ({
    type: 'input',
    key: 'name',
    label,
    value: '',
    config: {placeholder},
});

function changeLocale(locale: 'zh-CN' | 'en-US') {
    act(() => {
        localeHarness.current = locale;
        [...localeHarness.listeners].forEach((listener) => listener(locale));
    });
}

let previousGlobalI18n: unknown;

beforeEach(() => {
    localeHarness.current = 'zh-CN';
    localeHarness.listeners.clear();
    localeHarness.txCalls.length = 0;
    localeHarness.unsubscribeCount = 0;
    vi.mocked(tx).mockClear();
    previousGlobalI18n = (globalThis as {lcI18n?: unknown}).lcI18n;
    delete (globalThis as {lcI18n?: unknown}).lcI18n;
});

afterEach(() => {
    cleanup();
    localeHarness.listeners.clear();
    if (previousGlobalI18n === undefined)
        delete (globalThis as {lcI18n?: unknown}).lcI18n;
    else
        (globalThis as {lcI18n?: unknown}).lcI18n = previousGlobalI18n;
});

describe('DatavisoolGUI translation context', () => {
    test('keeps source strings without a provider and ignores the legacy global translator', () => {
        const legacyTranslate = vi.fn((text: string) => `legacy:${text}`);
        (globalThis as {lcI18n?: unknown}).lcI18n = {tx: legacyTranslate};
        const schema = inputControl('Source label', 'Source placeholder');

        render(<DatavisoolGUI schema={schema}/>);

        const input = screen.getByPlaceholderText('Source placeholder');
        expect(screen.getByText('Source label')).toBeInTheDocument();
        fireEvent.change(input, {target: {value: 'draft'}});
        window.dispatchEvent(new Event('lc-i18n-locale-change'));

        expect(screen.getByPlaceholderText('Source placeholder')).toBe(input);
        expect(input).toHaveValue('draft');
        expect(legacyTranslate).not.toHaveBeenCalled();
        expect(tx).not.toHaveBeenCalled();
    });

    test('uses an explicit DatavisoolGUI translator for labels, tips, placeholders, buttons, and options', () => {
        const translate = (text: string) => `translated:${text}`;
        const schema: Control[] = [
            {
                type: 'input',
                key: 'name',
                label: 'Field label',
                tip: 'Field tip',
                config: {placeholder: 'Enter a name'},
            },
            {
                type: 'button',
                key: 'action',
                config: {children: 'Run action'},
            },
            {
                type: 'select',
                key: 'mode',
                label: 'Mode label',
                config: {
                    placeholder: 'Choose a mode',
                    options: [{label: 'First mode', value: 'first'}],
                },
            },
        ];

        render(<DatavisoolGUI schema={schema} translate={translate}/>);

        expect(screen.getByText('translated:Field label')).toBeInTheDocument();
        expect(screen.getByLabelText('translated:Field tip')).toBeInTheDocument();
        expect(screen.getByPlaceholderText('translated:Enter a name')).toBeInTheDocument();
        expect(screen.getByRole('button', {name: 'translated:Run action'})).toBeInTheDocument();
        fireEvent.click(screen.getByRole('combobox'));
        expect(screen.getByRole('option', {name: 'translated:First mode'})).toBeInTheDocument();
    });

    test('updates top-level and nested translations through context with stable children and translator identity', () => {
        const schema = inputControl('Name label', 'Name placeholder');
        const nestedSchema: Control = {
            type: 'control-group',
            key: 'rows',
            label: 'Rows label',
            value: [{panel: {name: 'Existing value'}}],
            config: {
                itemName: 'Entry ',
                defaultOpen: true,
                template: {
                    type: 'accordion',
                    key: 'panel',
                    label: 'Panel label',
                    config: {defaultActive: true},
                    children: [inputControl('Nested field', 'Nested placeholder')],
                },
            },
        };
        const ref = React.createRef<DatavisoolGUI>();
        const onFieldChange = vi.fn();
        // This React element is intentionally reused across the locale update.
        const stableChildren = (
            <>
                <DatavisoolGUI ref={ref} schema={schema} onFieldChange={onFieldChange}/>
                <DatavisoolGUI schema={nestedSchema}/>
            </>
        );
        const MemoFields = React.memo(() => stableChildren);
        const {unmount} = render(<TestAppRoot><MemoFields/></TestAppRoot>);
        const instance = ref.current;
        const input = screen.getByPlaceholderText('zh:Name placeholder');
        const nestedInput = screen.getByPlaceholderText('zh:Nested placeholder');

        expect(localeHarness.listeners.size).toBe(1);
        expect(tx).toHaveBeenCalledWith('Nested field');
        fireEvent.change(input, {target: {value: 'kept value'}});
        fireEvent.change(nestedInput, {target: {value: 'kept nested value'}});
        expect(onFieldChange).toHaveBeenCalledWith(expect.objectContaining({
            data: 'kept value',
            dataKeyPath: ['name'],
        }));

        changeLocale('en-US');

        expect(screen.getByText('en:Name label')).toBeInTheDocument();
        expect(screen.getByPlaceholderText('en:Name placeholder')).toBe(input);
        expect(input).toHaveValue('kept value');
        expect(screen.getByText('en:Rows label')).toBeInTheDocument();
        expect(screen.getByText('en:Entry 1')).toBeInTheDocument();
        expect(screen.getByText('en:Panel label')).toBeInTheDocument();
        expect(screen.getByText('en:Nested field')).toBeInTheDocument();
        expect(screen.getByPlaceholderText('en:Nested placeholder')).toBe(nestedInput);
        expect(nestedInput).toHaveValue('kept nested value');
        expect(ref.current).toBe(instance);
        expect(localeHarness.listeners.size).toBe(1);

        unmount();
        expect(localeHarness.listeners.size).toBe(0);
        expect(localeHarness.unsubscribeCount).toBe(1);
    });

    test('updates when the provider translator changes while locale stays the same', () => {
        const ref = React.createRef<DatavisoolGUI>();
        const child = <DatavisoolGUI ref={ref} schema={inputControl('Label', 'Placeholder')}/>;
        const {rerender} = render(
            <DataVisoolUIProvider locale="en-US" translate={(text) => `first:${text}`}>{child}</DataVisoolUIProvider>,
        );
        const instance = ref.current;
        const input = screen.getByPlaceholderText('first:Placeholder');
        fireEvent.change(input, {target: {value: 'kept value'}});

        rerender(<DataVisoolUIProvider locale="en-US" translate={(text) => `second:${text}`}>{child}</DataVisoolUIProvider>);

        expect(screen.getByText('second:Label')).toBeInTheDocument();
        expect(screen.getByPlaceholderText('second:Placeholder')).toBe(input);
        expect(input).toHaveValue('kept value');
        expect(ref.current).toBe(instance);
    });

    test('keeps explicit translators ahead of context for DatavisoolGUI and nested ControlGroup content', () => {
        const translate = (text: string) => `explicit:${text}`;
        const nestedSchema: Control = {
            type: 'control-group',
            key: 'rows',
            label: 'Rows label',
            value: [{name: 'Stored value'}],
            config: {
                itemName: 'Record ',
                defaultOpen: true,
                template: inputControl('Nested field', 'Nested placeholder'),
            },
        };

        render(
            <DataVisoolUIProvider locale="zh-CN" translate={tx}>
                <DatavisoolGUI schema={nestedSchema} translate={translate}/>
                <ControlGroup
                    label="Direct group"
                    itemName="Direct record "
                    defaultOpen
                    defaultValue={[{name: 'Direct value'}]}
                    template={inputControl('Direct nested field', 'Direct nested placeholder')}
                    translate={translate}
                />
            </DataVisoolUIProvider>,
        );

        expect(screen.getByText('explicit:Rows label')).toBeInTheDocument();
        expect(screen.getByText('explicit:Record 1')).toBeInTheDocument();
        expect(screen.getByText('explicit:Nested field')).toBeInTheDocument();
        expect(screen.getByPlaceholderText('explicit:Nested placeholder')).toHaveValue('Stored value');
        expect(screen.getByText('Direct group')).toBeInTheDocument();
        expect(screen.getByText('explicit:Direct record 1')).toBeInTheDocument();
        expect(screen.getByText('explicit:Direct nested field')).toBeInTheDocument();
        expect(screen.getByPlaceholderText('explicit:Direct nested placeholder')).toHaveValue('Direct value');
        expect(tx).not.toHaveBeenCalled();
    });

    test('isolates translations between providers', () => {
        const schema = inputControl('Shared label', 'Shared placeholder');

        render(
            <>
                <DataVisoolUIProvider locale="zh-CN" translate={(text) => `first:${text}`}>
                    <DatavisoolGUI schema={schema}/>
                </DataVisoolUIProvider>
                <DataVisoolUIProvider locale="en-US" translate={(text) => `second:${text}`}>
                    <DatavisoolGUI schema={schema}/>
                </DataVisoolUIProvider>
            </>,
        );

        expect(screen.getByText('first:Shared label')).toBeInTheDocument();
        expect(screen.getByPlaceholderText('first:Shared placeholder')).toBeInTheDocument();
        expect(screen.getByText('second:Shared label')).toBeInTheDocument();
        expect(screen.getByPlaceholderText('second:Shared placeholder')).toBeInTheDocument();
    });
});
