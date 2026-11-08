/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, test, vi} from 'vitest';
import {buildAiDrivenTaskSubmitPayload} from '@/designer/ai-driven/AiDrivenUtil.ts';
import {
    getAiDrivenShortcutCommands,
    resolveAiDrivenShortcutSubmission,
} from '@/designer/ai-driven/AiDrivenShortcutCommands.ts';

const storeMocks = vi.hoisted(() => ({
    managerStore: {
        currentManager: null as any,
    },
    eventOperateStore: {
        targetIds: [] as string[],
    },
}));

vi.mock('@/designer/manager/ManagerStore.ts', () => ({
    default: storeMocks.managerStore,
}));

vi.mock('@/designer/operate-provider/EventOperateStore.ts', () => ({
    default: storeMocks.eventOperateStore,
}));

describe('AI driven submit payload', () => {
    afterEach(() => {
        storeMocks.managerStore.currentManager = null;
        storeMocks.eventOperateStore.targetIds = [];
    });

    test('keeps only basic selected component fields', () => {
        storeMocks.managerStore.currentManager = {
            canvasManager: {
                canvasConfig: {
                    width: 1920,
                    height: 1080,
                },
            },
            layerManager: {
                layerConfigs: {
                    chartA: {
                        id: 'chartA',
                        type: 'AntdBaseArea',
                        name: '面积图',
                        pid: 'root',
                        x: 100,
                        y: 200,
                        width: 300,
                        height: 180,
                        order: 1,
                    },
                    groupA: {
                        id: 'groupA',
                        type: 'group',
                        name: '分组',
                    },
                },
                elemConfigs: {
                    chartA: {
                        style: {
                            color: '#1677ff',
                        },
                    },
                },
                controllers: {
                    chartA: {
                        getConfig: () => ({
                            style: {
                                color: '#f00',
                            },
                        }),
                    },
                },
            },
        } as any;
        storeMocks.eventOperateStore.targetIds = ['chartA', 'groupA', 'chartA'];

        const payload = buildAiDrivenTaskSubmitPayload({
            requestId: 'request-1',
            sessionId: 'session-1',
            projectId: 'project-1',
            threadId: 'thread-1',
            modelConfigId: 'model-config-1',
            runtimeModelId: 'runtime-model-1',
            requirement: '优化面积图',
            attachments: [],
        });

        expect(payload?.selection).toEqual({
            mode: 'single',
            targetIds: ['chartA'],
            components: [
                {
                    id: 'chartA',
                    type: 'AntdBaseArea',
                    name: '面积图',
                },
            ],
        });
        expect(payload).toMatchObject({
            schemaVersion: 'ai-driven-task/v3',
            threadId: 'thread-1',
        });
    });

    test('resolves data shortcut submission', () => {
        expect(getAiDrivenShortcutCommands('/d').map((item) => item.command)).toEqual(['data']);

        const submission = resolveAiDrivenShortcutSubmission('/data 调整面积图数据');

        expect(submission).toEqual({
            requirement: '调整面积图数据',
            shortcut: {
                command: 'data',
                rawText: '/data 调整面积图数据',
            },
        });
    });
});
