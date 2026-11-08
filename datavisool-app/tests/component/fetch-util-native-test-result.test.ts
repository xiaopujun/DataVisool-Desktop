/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import axios from 'axios';
import {invoke} from '@tauri-apps/api/core';
import {beforeEach, describe, expect, it, vi} from 'vitest';
import FetchUtil from '@/util/FetchUtil.ts';

vi.mock('axios', () => ({default: vi.fn()}));
vi.mock('@tauri-apps/api/core', () => ({invoke: vi.fn()}));

const axiosMock = vi.mocked(axios);
const invokeMock = vi.mocked(invoke);

describe('native API test request results', () => {
    beforeEach(() => {
        axiosMock.mockReset();
        invokeMock.mockReset();
        vi.spyOn(console, 'error').mockImplementation(() => undefined);
        delete (window as any).__TAURI_INTERNALS__;
    });

    it('returns parsed JSON from a successful response', async () => {
        axiosMock.mockResolvedValue({
            data: new Blob(['{"ready":true}']),
            headers: {'content-type': 'application/json'},
        } as any);

        await expect(FetchUtil.doRequestNativeResultForTest(
            'https://api.example.test/status', 'GET', {}, {},
        )).resolves.toEqual({ok: true, data: {ready: true}});
    });

    it('preserves the complete successful response from the desktop API bridge', async () => {
        Object.defineProperty(window, '__TAURI_INTERNALS__', {value: {}, configurable: true});
        const response = {code: 200, msg: '操作成功', data: {id: 42}};
        invokeMock.mockResolvedValue(response as any);

        await expect(FetchUtil.doRequestNativeResultForTest(
            '/api/commonApi/getCommonApi/42', 'GET', {}, {},
        )).resolves.toEqual({ok: true, data: response});
        await expect(FetchUtil.doRequestNativeResult(
            '/api/commonApi/getCommonApi/42', 'GET', {}, {},
        )).resolves.toEqual(response);
    });

    it('reports a failed application response from the desktop API bridge', async () => {
        Object.defineProperty(window, '__TAURI_INTERNALS__', {value: {}, configurable: true});
        invokeMock.mockResolvedValue({code: 404, msg: '记录不存在', data: null} as any);

        await expect(FetchUtil.doRequestNativeResultForTest(
            '/api/commonApi/getCommonApi/42', 'GET', {}, {},
        )).resolves.toEqual({
            ok: false,
            error: '记录不存在',
            status: 404,
            response: null,
        });
    });

    it('returns HTTP status and response body when the server rejects the request', async () => {
        axiosMock.mockRejectedValue(Object.assign(new Error('Request failed with status code 403'), {
            response: {
                status: 403,
                statusText: 'Forbidden',
                headers: {'content-type': 'text/plain'},
                data: new Blob(['denied']),
            },
        }));

        await expect(FetchUtil.doRequestNativeResultForTest(
            'https://api.example.test/status', 'GET', {}, {},
        )).resolves.toEqual({
            ok: false,
            error: 'HTTP 403 Forbidden',
            status: 403,
            response: 'denied',
        });
    });

    it('returns a visible error for an invalid JSON response', async () => {
        axiosMock.mockResolvedValue({
            data: new Blob(['{broken']),
            headers: {'content-type': 'application/json'},
        } as any);

        const result = await FetchUtil.doRequestNativeResultForTest(
            'https://api.example.test/status', 'GET', {}, {},
        );

        expect(result.ok).toBe(false);
        if (!result.ok) {
            expect(result.error).toContain('响应 JSON 无法解析');
        }
    });

    it('returns network errors while keeping the existing API result helper behavior', async () => {
        axiosMock.mockRejectedValueOnce(new Error('Network Error'));

        await expect(FetchUtil.doRequestNativeResultForTest(
            'https://api.example.test/status', 'GET', {}, {},
        )).resolves.toEqual({ok: false, error: 'Network Error'});

        axiosMock.mockRejectedValueOnce(new Error('Network Error'));
        await expect(FetchUtil.doRequestNativeResult(
            'https://api.example.test/status', 'GET', {}, {},
        )).resolves.toBeNull();
    });
});
