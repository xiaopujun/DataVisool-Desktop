/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, it, vi} from 'vitest';

const messageMock = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/core', () => ({isTauri: () => true}));
vi.mock('@tauri-apps/plugin-dialog', () => ({message: messageMock}));

describe('installNativeScriptAlertBridge', () => {
    const originalAlert = window.alert;

    afterEach(() => {
        window.alert = originalAlert;
        vi.clearAllMocks();
    });

    it('等待前一个原生提示关闭后再显示下一个', async () => {
        let closeFirst: (() => void) | undefined;
        messageMock.mockImplementationOnce(() => new Promise<void>((resolve) => {
            closeFirst = resolve;
        }));
        messageMock.mockResolvedValueOnce('Ok');

        const {installNativeScriptAlertBridge} = await import('@/designer/loader/NativeScriptAlertBridge.ts');
        installNativeScriptAlertBridge();
        window.alert('第一个');
        window.alert('第二个');

        await vi.waitFor(() => expect(messageMock).toHaveBeenCalledTimes(1));
        expect(messageMock).toHaveBeenLastCalledWith('第一个', {title: 'DataVisool', kind: 'info'});

        closeFirst?.();
        await vi.waitFor(() => expect(messageMock).toHaveBeenCalledTimes(2));
        expect(messageMock).toHaveBeenLastCalledWith('第二个', {title: 'DataVisool', kind: 'info'});
    });
});
