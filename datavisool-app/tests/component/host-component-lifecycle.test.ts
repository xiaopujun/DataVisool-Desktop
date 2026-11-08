/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {describe, expect, it, vi} from 'vitest';
import HostController from '@/framework/core/AbstractDesignerController';
import type {ComponentBaseProps} from '@datavisool/sdk/types';

const updater = vi.hoisted(() => ({create: vi.fn(), run: vi.fn(), destroy: vi.fn()}));
vi.mock('@/framework/core/tools/ComponentDataUpdater.ts', () => ({
    default: class {
        constructor() { updater.create(); }
        run = updater.run;
        destroy = updater.destroy;
    },
}));

class Controller extends HostController<unknown, ComponentBaseProps> {
    update(config: ComponentBaseProps) { this.config = config; }
    getConfig() { return this.config; }
}

describe('host data loader adapter', () => {
    it('starts the host updater and releases it through the SDK lifecycle', async () => {
        vi.clearAllMocks();
        const controller = new Controller();
        const data = {sourceType: 'static', value: []};
        await controller.create(document.createElement('div'), {data}, {triggerComponentEvent: vi.fn()});
        controller.loadComponentData();
        await vi.waitFor(() => expect(updater.run).toHaveBeenCalledWith(data));
        controller.destroy();
        expect(updater.destroy).toHaveBeenCalledOnce();
        expect(controller.componentDataUpdater).toBeNull();
    });

    it('does not start an asynchronously imported loader after destruction', async () => {
        vi.clearAllMocks();
        const controller = new Controller();
        await controller.create(document.createElement('div'), {data: {sourceType: 'static'}}, {triggerComponentEvent: vi.fn()});
        controller.loadComponentData();
        controller.destroy();
        await new Promise(resolve => setTimeout(resolve, 20));
        expect(updater.create).not.toHaveBeenCalled();
    });
});
