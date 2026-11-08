/* Licensed under the DataVisool License. See the LICENSE file distributed with this software. */
import {afterEach, expect, it, vi} from 'vitest';
import EventUtil from '@/designer/blueprint/util/EventUtil';

afterEach(() => vi.useRealTimers());

it.each([0, 2, undefined])('passes the component index %s to the global loaded event', index => {
    vi.useFakeTimers();
    const triggerComponentEvent = vi.fn();
    const config = {base: {id: 'component'}};
    const manager = {
        layerManager: {layerConfigs: {component: config}, controllers: {component: {config, index}}},
        bluePrintManager: {bpPageMangers: []},
        bpExecutor: {triggerComponentEvent},
        globalVariableManager: {globalVariableInstance: {}},
    };
    EventUtil.triggerGlobalEvent(manager as any);
    vi.advanceTimersByTime(50);
    expect(triggerComponentEvent).toHaveBeenCalledWith('component', 'loaded', config, index);
});
