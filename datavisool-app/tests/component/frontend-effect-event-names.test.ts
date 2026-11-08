/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {expect, test, vi} from 'vitest';

const mocks = vi.hoisted(() => ({
    listeners: new Map<string, (event: {payload: unknown}) => unknown>(),
    invoke: vi.fn(async () => undefined),
}));

vi.mock('@tauri-apps/api/event', () => ({
    listen: vi.fn(async (name: string, callback: (event: {payload: unknown}) => unknown) => {
        mocks.listeners.set(name, callback);
        return () => mocks.listeners.delete(name);
    }),
}));
vi.mock('@tauri-apps/api/core', async (original) => ({
    ...await original<typeof import('@tauri-apps/api/core')>(),
    invoke: mocks.invoke,
}));

test('uses the new event channels to dispatch and complete a frontend effect', async () => {
    vi.stubGlobal('Worker', class { postMessage() {} terminate() {} addEventListener() {} removeEventListener() {} });
    const {registerFrontendEffectListener, registerFrontendEffectTool} = await import('@/frontend-effect/FrontendEffectBridge.tsx');
    const dispose = await registerFrontendEffectListener();
    const unregister = registerFrontendEffectTool('event-name-probe', () => ({accepted: true}));
    try {
        expect([...mocks.listeners.keys()].sort()).toEqual([
            'datavisool:frontend-effect-cancel',
            'datavisool:frontend-effect-request',
        ]);
        const binding = {requestHash: 'hash', toolName: 'event-name-probe', toolCallId: 'call'};
        await mocks.listeners.get('datavisool:frontend-effect-request')?.({payload: {
            effectId: 'effect', binding, kind: 'toolCall', name: 'event-name-probe', payload: {},
        }});
        expect(mocks.invoke).toHaveBeenCalledWith('datavisool_complete_frontend_effect', {
            response: {effectId: 'effect', binding, ok: true, data: {accepted: true}},
        });
    } finally {
        unregister();
        dispose();
        vi.unstubAllGlobals();
    }
});
