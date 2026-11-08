/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {fireEvent, render, screen} from '@testing-library/react';
import {describe, expect, it, vi} from 'vitest';
import DataBasePanel from '@/pages/home/database/DataBasePanel.tsx';
import {getLocale, setLocale, tx} from '@/i18n/miniI18n.ts';

const initial = {
    id: '1', type: 'MySQL', name: 'sample', ip: '127.0.0.1', port: '3306',
    username: 'user', password: 'secret', databaseName: 'db', url: 'mysql://user:secret@localhost/db', des: '',
};

function setup() {
    const onSubmitted = vi.fn();
    render(<DataBasePanel visible data={initial} onClose={vi.fn()} onSubmitted={onSubmitted}/>);
    return onSubmitted;
}

describe('database connection form', () => {
    it('rejects whitespace-only names and preserves valid leading and trailing spaces', () => {
        const onSubmitted = setup();
        const nameInput = screen.getByPlaceholderText('请输入数据库显示名称');

        for (const whitespaceName of [' \t　 ', '\uFEFF', '\u0085']) {
            fireEvent.change(nameInput, {target: {value: whitespaceName}});
            fireEvent.click(screen.getByRole('button', {name: '提交'}));
            expect(onSubmitted).not.toHaveBeenCalled();
        }
        expect(screen.getByText('请输入数据库显示名称')).toBeInTheDocument();

        fireEvent.change(nameInput, {target: {value: '  saved name  '}});
        fireEvent.click(screen.getByRole('button', {name: '提交'}));
        expect(onSubmitted).toHaveBeenCalledWith(expect.objectContaining({name: '  saved name  '}));
    });

    it('shows the read-only account warning when script permission is enabled', () => {
        render(
            <DataBasePanel visible data={initial} onClose={vi.fn()} onSubmitted={vi.fn()}/>,
        );
        expect(document.querySelector('.datasource-script-risk')).toBeNull();
        fireEvent.click(screen.getByRole('switch'));
        const warning = document.querySelector('.datasource-script-risk p')?.textContent ?? '';
        expect(warning).toContain('请为此连接分配只读数据库账号');

        const originalLocale = getLocale();
        try {
            setLocale('en-US');
            expect(tx(warning)).toContain('Use a read-only database account for this connection.');
        } finally {
            setLocale(originalLocale);
        }
    });

    it('rejects an out-of-range port and oversized description on submit', () => {
        const onSubmitted = setup();
        fireEvent.change(screen.getByPlaceholderText('请输入端口'), {target: {value: '65536'}});
        fireEvent.change(screen.getByPlaceholderText('请输入描述信息...'), {target: {value: 'a'.repeat(501)}});
        fireEvent.click(screen.getByRole('button', {name: '提交'}));
        expect(onSubmitted).not.toHaveBeenCalled();
        expect(screen.getByText('端口必须在 1 至 65535 之间')).toBeInTheDocument();
        expect(screen.getByText('描述不能超过 500 个字符')).toBeInTheDocument();
        fireEvent.change(screen.getByPlaceholderText('请输入端口'), {target: {value: '65535'}});
        fireEvent.change(screen.getByPlaceholderText('请输入描述信息...'), {target: {value: 'a'.repeat(500)}});
        fireEvent.click(screen.getByRole('button', {name: '提交'}));
        expect(onSubmitted).toHaveBeenCalledWith(expect.objectContaining({port: '65535', des: 'a'.repeat(500)}));
    });

    it('masks a custom connection URL until the user explicitly reveals it', () => {
        const props = {data: initial, onClose: vi.fn(), onSubmitted: vi.fn()};
        const {rerender} = render(<DataBasePanel {...props} visible/>);
        const input = screen.getByPlaceholderText('请输入 URL（填写 IP、端口、数据库名称后可不填）');
        expect(input).toHaveAttribute('type', 'password');
        fireEvent.click(screen.getByRole('button', {name: '显示 URL'}));
        expect(input).toHaveAttribute('type', 'text');
        rerender(<DataBasePanel {...props} visible={false}/>);
        rerender(<DataBasePanel {...props} visible/>);
        expect(screen.getByPlaceholderText('请输入 URL（填写 IP、端口、数据库名称后可不填）')).toHaveAttribute('type', 'password');
    });
});
