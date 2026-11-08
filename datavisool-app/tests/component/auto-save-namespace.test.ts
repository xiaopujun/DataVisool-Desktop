/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, expect, test, vi} from 'vitest';

const storage = vi.hoisted(() => new Map<string, unknown>());
vi.mock('localforage', () => ({default: {
    config: vi.fn(),
    getItem: vi.fn(async (key: string) => storage.get(key) ?? null),
    setItem: vi.fn(async (key: string, value: unknown) => storage.set(key, value)),
}}));

import {autoSaveKey, restoreAutoSave, singleSave, startAutoSave, stopAutoSave} from '@/designer/footer/auto-save/AutoSaveManager.ts';

afterEach(() => {
    stopAutoSave();
    vi.useRealTimers();
    storage.clear();
});

test('automatic and emergency snapshots use the same new namespace', async () => {
    vi.useFakeTimers();
    storage.set('LCP_project-1', [{encoded: 'b2xk', meta: {}, saveTime: 0}]);
    const key = autoSaveKey('project-1');
    expect(key).toBe('DATAVISOOL_project-1');
    expect(await restoreAutoSave(key)).toEqual([]);
    await singleSave(key, () => 'emergency');
    startAutoSave(key, 1000, () => 'automatic');
    await vi.advanceTimersByTimeAsync(1000);
    expect((await restoreAutoSave(key)).map(item => item.content)).toEqual(['emergency', 'automatic']);
    expect(storage.get('LCP_project-1')).toEqual([{encoded: 'b2xk', meta: {}, saveTime: 0}]);
});
