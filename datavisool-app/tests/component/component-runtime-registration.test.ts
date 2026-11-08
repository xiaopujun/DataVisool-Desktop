/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, it, vi} from 'vitest';
import {DesignerMode} from '@datavisool/sdk/types';
import {waitFor} from '@testing-library/react';
import DesignerLoader from '@/designer/loader/DesignerLoader';
import {exportComponentRuntime} from '@/global/ComponentRuntime';

vi.hoisted(() => {
    vi.stubGlobal('Worker', class { postMessage() {} terminate() {} });
});
vi.mock('@/designer/manager/DesignerManager.ts', () => ({default: class {}}));
vi.mock('@/designer/left/compoent-lib/ComponentCategorize.ts', () => ({componentCategorize: []}));
vi.mock('@/designer/blueprint/node/core/AbstractBPNodeController.ts', () => ({AbstractBPNodeController: class {}}));
vi.mock('@/designer/blueprint/util/EventUtil.ts', () => ({default: {}}));
vi.mock('@/api/engineering/BaseApi.ts', () => ({default: {}}));

afterEach(() => {
    document.querySelectorAll('script[data-id^="compat-test-"]').forEach(script => script.remove());
    Reflect.deleteProperty(window, '__LIGHT_CHASER_REMOTE_COMPONENT_TEMP__');
    vi.restoreAllMocks();
});

describe('component runtime registration', () => {
    it('loads a component registered with the current marker', async () => {
        exportComponentRuntime();
        const loader = new DesignerLoader(DesignerMode.EDIT);
        const definition = {getBaseInfo: () => ({compKey: 'compat-test-component'})};
        loader.initDefinition = vi.fn((_constructor, callback) => callback?.(definition as never));
        const result = loader.loadScript({id: 'compat-test-current', url: 'https://example.test/components/current.js'});
        await waitFor(() => expect(document.querySelector('script[data-id="compat-test-current"]')).not.toBeNull());
        const script = document.querySelector('script[data-id="compat-test-current"]') as HTMLScriptElement;
        expect(window.__DATAVISOOL_REMOTE_COMPONENT_BASE_URL_MAP__['current.js']).toBe('https://example.test/components/');
        const constructor = function Component() {};
        window.__DATAVISOOL_REMOTE_COMPONENT_TEMP__ = constructor;
        script.dispatchEvent(new Event('load'));
        await expect(result).resolves.toBeNull();
        expect(loader.initDefinition).toHaveBeenCalledWith(constructor, expect.any(Function));
        expect(window.__DATAVISOOL_REMOTE_COMPONENT_TEMP__).toBeUndefined();
    });

    it('does not accept the legacy registration marker', async () => {
        exportComponentRuntime();
        const loader = new DesignerLoader(DesignerMode.PREVIEW);
        const result = loader.loadScript({id: 'compat-test-legacy', url: 'https://example.test/components/legacy.js'});
        await waitFor(() => expect(document.querySelector('script[data-id="compat-test-legacy"]')).not.toBeNull());
        const script = document.querySelector('script[data-id="compat-test-legacy"]') as HTMLScriptElement;
        Reflect.set(window, '__LIGHT_CHASER_REMOTE_COMPONENT_TEMP__', function LegacyComponent() {});
        script.dispatchEvent(new Event('load'));
        await expect(result).rejects.toThrow('Remote component not found on window.');
        expect(document.querySelector('script[data-id="compat-test-legacy"]')).toBeNull();
    });
});
