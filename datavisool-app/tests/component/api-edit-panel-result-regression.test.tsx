/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {act, fireEvent, render, screen, waitFor} from '@testing-library/react';
import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import apiManagementStore from '@/pages/home/api-management/ApiManagementStore.ts';
import ApiEditPanel from '@/pages/home/api-management/ApiEditPanel.tsx';
import FetchUtil from '@/util/FetchUtil.ts';

vi.mock('@datavisool/ui', () => ({
    Button: ({children, onClick}: any) => <button onClick={onClick}>{children}</button>,
    CodeEditor: ({value, readonly, onChange}: any) => (
        <textarea
            aria-label={readonly ? '测试结果' : '代码编辑器'}
            value={value}
            readOnly={readonly}
            onChange={onChange ? (event) => onChange(event.target.value) : undefined}
        />
    ),
    Dialog: ({open, onClose, children}: any) => open
        ? <div role="dialog"><button aria-label="关闭对话框" onClick={onClose}>关闭</button>{children}</div>
        : null,
    Input: ({value, onChange, placeholder}: any) => (
        <input value={value} placeholder={placeholder} onChange={(event) => onChange(event.target.value)} />
    ),
    Select: () => <div />,
    globalMessage: {messageApi: {success: vi.fn(), warning: vi.fn(), error: vi.fn()}},
}));

vi.mock('@/util/FetchUtil.ts', () => ({
    default: {doRequestNativeResultForTest: vi.fn()},
}));

vi.mock('@/util/AuthFetchUtil.ts', () => ({
    default: {
        post: vi.fn(async (url: string) => url.endsWith('/pageList') ? null : {}),
    },
}));

const requestMock = vi.mocked(FetchUtil.doRequestNativeResultForTest);

function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>((finish) => {
        resolve = finish;
    });
    return {promise, resolve};
}

function openForm() {
    act(() => {
        apiManagementStore.setCurrentItem({
            name: '测试 API',
            url: 'https://example.test/api',
            method: 'GET',
            headers: '{}',
            params: '{}',
        });
        apiManagementStore.setModalVisible(true);
    });
}

async function waitForRequests(count: number) {
    await waitFor(() => expect(requestMock).toHaveBeenCalledTimes(count));
}

describe('API editor request result lifecycle', () => {
    beforeEach(() => {
        requestMock.mockReset();
        act(() => apiManagementStore.setModalVisible(false));
        openForm();
    });

    afterEach(() => {
        act(() => apiManagementStore.setModalVisible(false));
        vi.restoreAllMocks();
    });

    it('shows only the newest response when requests finish out of order', async () => {
        const first = deferred<any>();
        const second = deferred<any>();
        requestMock.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
        render(<ApiEditPanel />);

        fireEvent.click(screen.getByRole('button', {name: '测试'}));
        fireEvent.click(screen.getByRole('button', {name: '测试'}));
        await waitForRequests(2);

        await act(async () => {
            second.resolve({ok: true, data: {request: 'new'}});
            await second.promise;
        });
        expect(screen.getByLabelText('测试结果')).toHaveValue(JSON.stringify({request: 'new'}, null, 2));

        await act(async () => {
            first.resolve({ok: true, data: {request: 'old'}});
            await first.promise;
        });
        expect(screen.getByLabelText('测试结果')).toHaveValue(JSON.stringify({request: 'new'}, null, 2));
    });

    it.each(['关闭', '保存'])('discards a late response after %s and reopening', async (action) => {
        const pending = deferred<any>();
        requestMock.mockReturnValueOnce(pending.promise);
        render(<ApiEditPanel />);

        fireEvent.click(screen.getByRole('button', {name: '测试'}));
        await waitForRequests(1);

        if (action === '关闭') {
            fireEvent.click(screen.getByRole('button', {name: '取消'}));
        } else {
            fireEvent.click(screen.getByRole('button', {name: '创建'}));
            await waitFor(() => expect(apiManagementStore.modalVisible).toBe(false));
            await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
        }

        openForm();
        await waitFor(() => expect(screen.getByRole('dialog')).toBeInTheDocument());
        await act(async () => {
            pending.resolve({ok: true, data: {request: 'late'}});
            await pending.promise;
        });

        expect(screen.getByLabelText('测试结果')).toHaveValue('');
    });
});
