/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {describe, expect, it, vi} from 'vitest';
import {act} from '@testing-library/react';
import {execFileSync} from 'node:child_process';
import path from 'node:path';
import {exportComponentRuntime} from '@/global/ComponentRuntime';

const builder = path.resolve('tests/helpers/build-remote-fixture.mjs');

describe('remote component bundles', () => {
    it('loads and renders a component using current SDK externals', async () => {
        const code = execFileSync(process.execPath, [builder], {encoding: 'utf8'});
        expect(code).toContain('DataVisoolSDK');
        exportComponentRuntime();
        const globals = ['React', 'ReactDOM', 'DataVisoolSDK', 'DataVisoolTypes', 'DataVisoolUI'];
        // The source is the component fixture compiled above, never user-supplied code.
        const Definition = new Function(...globals, `${code}; return RemoteComponentFixture;`)(...globals.map(name => window[name]));
        const definition = new Definition();
        const Controller = definition.getController();
        const controller = new Controller();
        expect(controller).toBeInstanceOf(window.DataVisoolSDK.AbstractDesignerController);
        expect(Object.values(definition.getMenuToConfigContentMap()).every(value => typeof value === 'function')).toBe(true);
        const container = document.createElement('div');
        document.body.append(container);
        const executor = {triggerComponentEvent: vi.fn()};
        let created: Promise<void>;
        try {
            await act(async () => {
                created = controller.create(container, definition.getInitConfig(), executor);
            });
            await created!;
            expect(container.textContent).toContain('按钮');
            act(() => controller.setVisible(false));
            expect(container.style.visibility).toBe('hidden');
        } finally {
            act(() => controller.destroy());
            expect(container.childNodes.length).toBe(0);
            container.remove();
        }
    }, 20000);
});
