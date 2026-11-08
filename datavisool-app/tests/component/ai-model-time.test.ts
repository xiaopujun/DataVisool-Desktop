/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {describe, expect, it} from 'vitest';
import {formatAiModelTime} from '@/pages/home/ai-model-management/AiModelTime.ts';

function localDisplay(iso: string): string {
    const date = new Date(iso);
    const pad = (value: number) => String(value).padStart(2, '0');
    return `${pad(date.getMonth() + 1)}/${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

describe('AI model time display', () => {
    it('treats SQLite timestamps as UTC and displays them in the local timezone', () => {
        expect(formatAiModelTime('2026-11-06 04:28:09')).toBe(localDisplay('2026-11-06T04:28:09Z'));
        expect(formatAiModelTime('2026-11-06 04:28:09.500')).toBe(localDisplay('2026-11-06T04:28:09.500Z'));
    });

    it('keeps explicit timezone offsets and preserves invalid values', () => {
        expect(formatAiModelTime('2026-11-06T04:28:09+08:00')).toBe(localDisplay('2026-11-06T04:28:09+08:00'));
        expect(formatAiModelTime('invalid time')).toBe('invalid time');
    });
});
