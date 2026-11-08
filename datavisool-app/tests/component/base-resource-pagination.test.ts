/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {beforeEach, describe, expect, it, vi} from 'vitest';
import AuthFetchUtil from '@/util/AuthFetchUtil.ts';
import {BaseResourceStore} from '@/pages/home/common/BaseResourceStore.ts';

vi.mock('@/util/AuthFetchUtil.ts', () => ({default: {post: vi.fn()}}));

class TestStore extends BaseResourceStore<{id?: string}> {
    getListApiUrl = '/list';
    deleteApiUrl = '/delete';
}

const post = vi.mocked(AuthFetchUtil.post);
const page = (current: number, total: number, ids: string[]) => ({
    current, size: 10, total, records: ids.map(id => ({id})),
});

describe('resource list pagination', () => {
    beforeEach(() => post.mockReset());

    it('keeps the active filter when changing pages', async () => {
        post.mockResolvedValueOnce(page(1, 12, ['first'])).mockResolvedValueOnce(page(2, 12, ['second']));
        const store = new TestStore();
        store.searchValue = 'matched';
        store.getList();
        await vi.waitFor(() => expect(store.pageData.records[0]?.id).toBe('first'));
        store.changeCurrentPage(2);
        await vi.waitFor(() => expect(store.pageData.records[0]?.id).toBe('second'));
        expect(post).toHaveBeenLastCalledWith('/list', {current: 2, size: 10, searchValue: 'matched'});
    });

    it('reloads the last valid page with the same filter and extra parameters', async () => {
        post.mockResolvedValueOnce(page(2, 10, [])).mockResolvedValueOnce(page(1, 10, ['remaining']));
        const store = new TestStore();
        store.pageData.current = 2;
        store.searchValue = 'matched';
        store.getList({projectId: 'project-1'});
        await vi.waitFor(() => expect(store.pageData.records[0]?.id).toBe('remaining'));
        expect(store.pageData.current).toBe(1);
        expect(post).toHaveBeenLastCalledWith('/list', {current: 1, size: 10, searchValue: 'matched', projectId: 'project-1'});
    });

    it('ignores an older response after a newer search', async () => {
        let resolveFirst!: (value: ReturnType<typeof page>) => void;
        post.mockImplementationOnce(() => new Promise(resolve => { resolveFirst = resolve; }))
            .mockResolvedValueOnce(page(1, 1, ['new']));
        const store = new TestStore();
        store.searchValue = 'old';
        store.getList();
        store.searchValue = 'new';
        store.getList();
        await vi.waitFor(() => expect(store.pageData.records[0]?.id).toBe('new'));
        resolveFirst(page(1, 1, ['old']));
        await Promise.resolve();
        expect(store.pageData.records[0]?.id).toBe('new');
        expect(store.searchValue).toBe('new');
    });

    it('ignores a response after the store is destroyed', async () => {
        let resolveRequest!: (value: ReturnType<typeof page>) => void;
        post.mockImplementationOnce(() => new Promise(resolve => { resolveRequest = resolve; }));
        const store = new TestStore();
        store.getList();
        store.destroy();
        resolveRequest(page(1, 1, ['stale']));
        await Promise.resolve();
        expect(store.pageData.records).toEqual([]);
    });
});
