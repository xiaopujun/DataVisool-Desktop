/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, test, vi} from 'vitest';

const modalMocks = vi.hoisted(() => ({
    confirm: vi.fn(() => 'agent-user-input-modal'),
    destroy: vi.fn(),
}));

vi.mock('@datavisool/ui', () => ({
    globalModal: {modalApi: modalMocks},
    TextArea: () => null,
}));

import {runUserInputTool} from '@/frontend-effect/tools/UserInputTool.ts';
import aiDrivenProgressTerminalStore from '@/designer/ai-driven/AiDrivenProgressTerminalStore.ts';

describe('frontend effect cancellation', () => {
    const questions = [{
        header: '主题', id: 'theme', question: '请选择主题',
        options: [
            {label: '深色', description: '使用深色科技风'},
            {label: '浅色', description: '使用浅色简洁风'},
        ],
    }];
    afterEach(() => {
        aiDrivenProgressTerminalStore.setInlineHostAvailable(false);
        aiDrivenProgressTerminalStore.reset();
    });

    test('closes and rejects a pending user-input effect when Rust cancels it', async () => {
        const controller = new AbortController();
        const result = runUserInputTool('补充信息', {questions}, controller.signal);

        controller.abort('task_cancelled');

        await expect(result).rejects.toMatchObject({code: 'effect_cancelled'});
        expect(modalMocks.destroy).toHaveBeenCalledWith('agent-user-input-modal');
    });

    test('resolves user input inside the task timeline when available', async () => {
        aiDrivenProgressTerminalStore.startRun('request-a');
        aiDrivenProgressTerminalStore.setInlineHostAvailable(true);
        const result = runUserInputTool('补充信息', {questions});

        aiDrivenProgressTerminalStore.completeInlineInteraction({
            action: 'submit',
            data: {answers: {theme: {answers: ['深色']}}},
        });

        await expect(result).resolves.toEqual({answers: {theme: {answers: ['深色']}}});
    });
});
