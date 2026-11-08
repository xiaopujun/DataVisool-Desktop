/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {describe, expect, it} from 'vitest';
import ProjectDataCodecUtil from '@/util/ProjectDataCodecUtil';

describe('project data compression', () => {
    it('writes and reads the current compression marker', () => {
        const value = JSON.stringify({text: 'data'.repeat(500)});
        const current = ProjectDataCodecUtil.compress(value);
        const legacy = current.replace(/^DVZ1:/, 'LCPZ1:');
        expect(current.startsWith('DVZ1:gzip:')).toBe(true);
        expect(ProjectDataCodecUtil.decompress(current)).toBe(value);
        expect(ProjectDataCodecUtil.decompress(legacy)).toBe(legacy);
        expect(ProjectDataCodecUtil.decompressIfNeeded(legacy)).toBe(legacy);
    });

});
