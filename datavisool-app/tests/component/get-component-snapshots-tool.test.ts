/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, test} from 'vitest';
import {runGetComponentSnapshotsTool} from '@/frontend-effect/tools/GetComponentSnapshotsTool.ts';

describe('getComponentSnapshots tool', () => {
    afterEach(() => {
        delete (window as any).DATAVISOOL_ENV;
    });

    test('reads requested fields and prefers controller config', () => {
        (window as any).DATAVISOOL_ENV = {
            currentManager: {
                layerManager: {
                    layerConfigs: {
                        chartA: {
                            id: 'chartA',
                            type: 'AntdBaseArea',
                            name: '面积图',
                            x: 100,
                            y: 200,
                        },
                        textB: {
                            id: 'textB',
                            type: 'BaseText',
                            name: '标题',
                        },
                    },
                    elemConfigs: {
                        chartA: {
                            style: {
                                color: '#1677ff',
                            },
                        },
                        textB: {
                            base: {
                                id: 'textB',
                                type: 'BaseText',
                            },
                            style: {
                                text: 'hello',
                            },
                        },
                    },
                    controllers: {
                        chartA: {
                            getConfig: () => ({
                                base: {
                                    id: 'chartA',
                                    type: 'AntdBaseArea',
                                },
                                style: {
                                    color: '#f00',
                                },
                                data: {
                                    value: [1, 2],
                                },
                            }),
                        },
                    },
                },
            },
        };

        const result = runGetComponentSnapshotsTool({
            componentIds: ['chartA', 'textB', 'missing', 'chartA'],
            fields: ['style', 'data', 'base', 'layer'],
        });

        expect(result).toEqual({
            components: [
                {
                    id: 'chartA',
                    type: 'AntdBaseArea',
                    name: '面积图',
                    style: {
                        color: '#f00',
                    },
                    data: {
                        value: [1, 2],
                    },
                    base: {
                        id: 'chartA',
                        type: 'AntdBaseArea',
                    },
                    layer: {
                        id: 'chartA',
                        type: 'AntdBaseArea',
                        name: '面积图',
                        x: 100,
                        y: 200,
                    },
                },
                {
                    id: 'textB',
                    type: 'BaseText',
                    name: '标题',
                    style: {
                        text: 'hello',
                    },
                    data: null,
                    base: {
                        id: 'textB',
                        type: 'BaseText',
                    },
                    layer: {
                        id: 'textB',
                        type: 'BaseText',
                        name: '标题',
                    },
                },
            ],
            missingComponentIds: ['missing'],
        });
    });

    test('pages through every current-page component with stable scope metadata', () => {
        (window as any).DATAVISOOL_ENV = {
            currentManager: {
                layerManager: {
                    layerConfigs: {
                        second: {id: 'second', type: 'BaseText', order: 2},
                        first: {id: 'first', type: 'BaseText', order: 1},
                        third: {id: 'third', type: 'BaseText', order: 3},
                    },
                    elemConfigs: {},
                },
            },
        };

        const result = runGetComponentSnapshotsTool({allPage: true, offset: 1, limit: 2, fields: ['layer']});

        expect(result).toEqual({
            components: [
                {id: 'second', type: 'BaseText', layer: {id: 'second', type: 'BaseText', order: 2}},
                {id: 'third', type: 'BaseText', layer: {id: 'third', type: 'BaseText', order: 3}},
            ],
            missingComponentIds: [],
            totalComponents: 3,
            hasMore: false,
            offset: 1,
        });
    });

    test('returns the same JSON-safe layer shape that crosses Tauri IPC', () => {
        const layer = {
            id: 'textA',
            type: 'BaseText',
            x: 10,
            pid: undefined,
            childHeader: undefined,
            children: undefined,
        };
        (window as any).DATAVISOOL_ENV = {
            currentManager: {
                layerManager: {
                    layerConfigs: {textA: layer},
                    elemConfigs: {},
                },
            },
        };

        const result = runGetComponentSnapshotsTool({componentIds: ['textA'], fields: ['layer']});
        const snapshot = result.components[0].layer as Record<string, unknown>;

        expect(snapshot).toEqual({id: 'textA', type: 'BaseText', x: 10});
        expect(Object.prototype.hasOwnProperty.call(snapshot, 'pid')).toBe(false);
        expect(Object.prototype.hasOwnProperty.call(snapshot, 'children')).toBe(false);
    });
});
