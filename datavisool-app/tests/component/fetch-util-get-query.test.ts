/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import axios from 'axios';
import {beforeEach, describe, expect, it, vi} from 'vitest';
import FetchUtil from '@/util/FetchUtil.ts';

vi.mock('axios', () => ({default: vi.fn()}));

const axiosMock = vi.mocked(axios);

describe('custom GET request query parameters', () => {
    beforeEach(() => {
        axiosMock.mockReset();
        axiosMock.mockResolvedValue({
            data: {text: async () => '{"ok":true}'},
            headers: {'content-type': 'text/plain'},
        } as any);
    });

    it.each([
        [
            'https://api.example.test/echo?existing=keep',
            'https://api.example.test/echo?existing=keep&added=new+value',
        ],
        ['/api/echo?existing=keep#response', '/api/echo?existing=keep&added=new+value#response'],
        ['/api/echo?', '/api/echo?added=new+value'],
        ['/api/echo?existing=keep&', '/api/echo?existing=keep&added=new+value'],
        ['/api/echo#response', '/api/echo?added=new+value#response'],
    ])('appends parameters to %s', async (url, expectedUrl) => {
        await FetchUtil.doRequestNativeResult(
            url,
            'GET',
            {},
            {added: 'new value'},
        );

        expect(axiosMock).toHaveBeenCalledOnce();
        expect(axiosMock.mock.calls[0][0].url).toBe(expectedUrl);
    });
});
