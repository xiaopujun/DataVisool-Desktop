/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, test, vi} from 'vitest';
import {runApplyComponentStylePatchTool} from '@/frontend-effect/tools/ApplyComponentStylePatchTool.ts';

describe('applyComponentStylePatch tool', () => {
    afterEach(() => {
        delete (window as any).DATAVISOOL_ENV;
    });

    test('applies the complete style patch without creating style.style', () => {
        const config: any = {
            base: {type: 'BaseText'},
            style: {fontSize: 16, color: '#ffffff'},
        };
        const update = vi.fn((patch: any) => {
            config.style = {...config.style, ...patch.style};
        });
        (window as any).DATAVISOOL_ENV = {
            currentManager: {
                layerManager: {
                    controllers: {
                        'text-1': {getConfig: () => config, update},
                    },
                    layerConfigs: {},
                },
            },
        };

        const result = runApplyComponentStylePatchTool({
            patches: [{
                componentId: 'text-1',
                componentType: 'BaseText',
                stylePatch: {style: {fontSize: 20}},
            }],
        });

        expect(update).toHaveBeenCalledWith({style: {fontSize: 20}}, {reRender: true});
        expect(config.style).toEqual({fontSize: 20, color: '#ffffff'});
        expect(config.style.style).toBeUndefined();
        expect(result).toEqual({
            applied: [{
                componentId: 'text-1',
                componentType: 'BaseText',
                style: {fontSize: 20, color: '#ffffff'},
            }],
            failed: [],
        });
    });

    test('reports failure when the controller did not persist the style patch', () => {
        const config = {
            base: {type: 'BaseText'},
            style: {fontSize: 16},
        };
        (window as any).DATAVISOOL_ENV = {
            currentManager: {
                layerManager: {
                    controllers: {
                        'text-1': {getConfig: () => config, update: vi.fn()},
                    },
                    layerConfigs: {},
                },
            },
        };

        const result = runApplyComponentStylePatchTool({
            patches: [{
                componentId: 'text-1',
                componentType: 'BaseText',
                stylePatch: {style: {fontSize: 20}},
            }],
        });

        expect(result.applied).toEqual([]);
        expect(result.failed).toEqual([expect.objectContaining({
            componentId: 'text-1',
            code: 'batch_apply_rolled_back',
        })]);
    });

    test('rejects style patches outside the unified style root protocol', () => {
        const run = (stylePatch: unknown) => runApplyComponentStylePatchTool({
            patches: [{componentId: 'text-1', componentType: 'BaseText', stylePatch}],
        });

        expect(() => run({fontSize: 20})).toThrow('stylePatch 顶层只能包含 style');
        expect(() => run({style: {fontSize: 20}, data: {value: 'x'}})).toThrow('stylePatch 顶层只能包含 style');
        expect(() => run({style: {}})).toThrow('stylePatch.style 必须是非空对象');
    });

    test('rejects the whole protected batch when one style baseline is stale', () => {
        const configs: Record<string, any> = {
            'text-1': {base: {type: 'BaseText'}, style: {fontSize: 16}},
            'text-2': {base: {type: 'BaseText'}, style: {fontSize: 18}},
        };
        const updates = {'text-1': vi.fn(), 'text-2': vi.fn()};
        (window as any).DATAVISOOL_ENV = {
            currentManager: {
                layerManager: {
                    controllers: {
                        'text-1': {getConfig: () => configs['text-1'], update: updates['text-1']},
                        'text-2': {getConfig: () => configs['text-2'], update: updates['text-2']},
                    },
                    layerConfigs: {},
                },
            },
        };

        const result = runApplyComponentStylePatchTool({
            patches: [
                {componentId: 'text-1', componentType: 'BaseText', stylePatch: {style: {fontSize: 20}}},
                {componentId: 'text-2', componentType: 'BaseText', stylePatch: {style: {fontSize: 22}}},
            ],
            expectedSnapshots: [
                {componentId: 'text-1', componentType: 'BaseText', style: {fontSize: 16}},
                {componentId: 'text-2', componentType: 'BaseText', style: {fontSize: 17}},
            ],
        });

        expect(updates['text-1']).not.toHaveBeenCalled();
        expect(updates['text-2']).not.toHaveBeenCalled();
        expect(result.applied).toEqual([]);
        expect(result.failed).toHaveLength(2);
        expect(result.failed.every((failure) => failure.code === 'component_revision_conflict')).toBe(true);
    });
});
