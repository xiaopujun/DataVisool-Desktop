/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {describe, expect, test} from 'vitest';
import {getAiDrivenComponentSelectionInfoRegistrySnapshot} from '@/designer/ai-driven/ComponentAiSelectionDocRegistry.ts';

const selectionDocs = import.meta.glob('/src/comps/**/AI_SELECTION.md', {eager: true, query: '?raw', import: 'default'});

describe('component selection info registry', () => {
    test('provides explicit capability metadata for selectable and internal-only components', () => {
        const infos = getAiDrivenComponentSelectionInfoRegistrySnapshot();
        const byType = new Map(infos.map((info) => [info.componentType, info]));

        expect(byType.get('ECharts')).toMatchObject({
            componentName: 'Echarts自定义图表',
            selectable: true,
        });
        expect(byType.get('group')).toMatchObject({
            componentName: '分组图层',
            selectable: false,
        });
        [
            'AMap',
            'DateTimePicker',
            'ECharts',
            'G2PlotCustom',
            'LoopContainer',
            'MapLibreGis',
            'ScreenReference',
            'ThreeModel',
            'group',
        ].forEach((componentType) => expect(byType.has(componentType)).toBe(true));
        expect(infos).toHaveLength(Object.keys(selectionDocs).length);
        expect(infos.every((info) => info.description.trim().length > 0)).toBe(true);
        expect(infos.every((info) => info.nativeCapabilities.trim().length > 0)).toBe(true);
        expect(infos.every((info) => info.nativeCapabilities.length <= 120)).toBe(true);
        expect(infos.every((info) => (
            Object.keys(info).sort().join(',') === 'componentName,componentType,description,nativeCapabilities,selectable'
        ))).toBe(true);
        expect(JSON.stringify(infos)).not.toContain('allowedStylePaths');
        expect(JSON.stringify(infos)).not.toContain('stylePatch');
    });
});
