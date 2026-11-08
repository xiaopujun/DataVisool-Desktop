/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, it, vi} from 'vitest';

const {useTemplate} = vi.hoisted(() => {
    vi.stubGlobal('Worker', class { postMessage() {} terminate() {} });
    return {useTemplate: vi.fn()};
});
vi.mock('@/designer/operate-provider/undo-redo/HistoryRecordOperateProxy', () => ({
    default: {doUseTemplate: useTemplate},
}));

import {doPaste} from '@/designer/operate-provider/hot-key/HotKeyImpl';

afterEach(() => {
    useTemplate.mockClear();
    delete (window as Partial<Window>).DATAVISOOL_ENV;
});

describe('clipboard format', () => {
    it('pastes current content and ignores old or invalid content', () => {
        window.DATAVISOOL_ENV = {currentManager: {canvasManager: {canvasConfig: {width: 200, height: 100}}}} as typeof window.DATAVISOOL_ENV;
        const layers = {item: {type: 'text', x: 10, y: 5}};
        doPaste(JSON.stringify({flag: 'LCP_COPY', layers}));
        doPaste('invalid json');
        expect(useTemplate).not.toHaveBeenCalled();

        doPaste(JSON.stringify({flag: 'DATAVISOOL_COPY', layers}));
        expect(useTemplate).toHaveBeenCalledOnce();
        expect(useTemplate).toHaveBeenCalledWith(expect.objectContaining({
            layers: {item: {type: 'text', x: 110, y: 55}},
        }));
    });
});
