/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, test} from 'vitest';
import {runGetComponentDataContractTool} from '@/frontend-effect/tools/GetComponentDataContractTool.ts';

function installComponent(config: Record<string, unknown>) {
    const base = config.base as Record<string, unknown>;
    const id = String(base.id);
    (window as any).DATAVISOOL_ENV = {
        currentManager: {
            layerManager: {
                layerConfigs: {[id]: {id, type: base.type, name: base.name}},
                controllers: {[id]: {getConfig: () => config}},
            },
        },
    };
}

describe('getComponentDataContract tool', () => {
    afterEach(() => {
        delete (window as any).DATAVISOOL_ENV;
    });

    test('defaults to the static branch and returns current component data.value', () => {
        installComponent({
            base: {id: 'line-1', type: 'AntdMultiLine', name: '趋势'},
            style: {xField: 'time', yField: 'amount', seriesField: 'channel'},
            data: {
                sourceType: 'static',
                staticData: {value: [{time: 'Q1', amount: 12, channel: '线上'}]},
                value: [{time: 'Q1', amount: 12, channel: '线上'}],
            },
        });

        const result = runGetComponentDataContractTool({componentIds: ['line-1']}) as any;

        expect(result.schemaVersion).toBe('component-data-contract/v1');
        expect(result.sourceContract).toMatchObject({
            sourceType: 'static',
            branch: 'staticData',
            structure: {sourceType: 'static', staticData: {value: '由组件数据格式要求决定'}},
        });
        expect(result.sourceContract.structure).not.toHaveProperty('apiData');
        expect(result.components[0]).toMatchObject({
            componentId: 'line-1',
            componentType: 'AntdMultiLine',
            currentData: {
                sourceType: 'static',
                valueExample: [{time: 'Q1', amount: 12, channel: '线上'}],
                validation: {valid: true},
            },
            fieldMapping: {xField: 'time', yField: 'amount', seriesField: 'channel'},
        });
        expect(result.components[0]).not.toHaveProperty('supportsDataInput');
        expect(result.components[0]).not.toHaveProperty('dataFormatRequirements');
    });

    test('returns only the explicitly requested data source branch', () => {
        installComponent({
            base: {id: 'text-1', type: 'BaseText', name: '标题'},
            style: {},
            data: {sourceType: 'static', staticData: {value: '标题'}, value: '标题'},
        });

        const result = runGetComponentDataContractTool({componentIds: ['text-1'], sourceType: 'api'}) as any;

        expect(result.sourceContract).toMatchObject({sourceType: 'api', branch: 'apiData'});
        expect(Object.keys(result.sourceContract.structure)).toEqual(['sourceType', 'apiData']);
        expect(result.components[0].recommendedPatch).toEqual({
            sourceType: 'api',
            apiData: {
                url: 'https://example.com/data',
                method: 'get',
                headers: {},
                params: {},
                autoFlush: false,
            },
        });
    });

    test('diagnoses malformed current data without treating it as a valid contract', () => {
        installComponent({
            base: {id: 'text-1', type: 'BaseText'},
            style: {},
            data: [],
        });

        const result = runGetComponentDataContractTool({componentIds: ['text-1']}) as any;

        expect(result.sourceContract.structure).toHaveProperty('staticData');
        expect(result.components[0].currentData).toMatchObject({
            sourceType: null,
            valueExample: null,
            validation: {
                valid: false,
                issues: [{code: 'component.data.invalid_root_type', path: 'config.data'}],
            },
        });
    });

    test('redacts sensitive fields inside the current value sample', () => {
        installComponent({
            base: {id: 'text-1', type: 'BaseText'},
            style: {},
            data: {
                sourceType: 'static',
                staticData: {value: {label: 'safe'}},
                value: {label: 'safe', accessToken: 'secret-value'},
            },
        });

        const result = runGetComponentDataContractTool({componentIds: ['text-1']}) as any;

        expect(result.components[0].currentData.valueExample.accessToken).toEqual({redacted: true, exists: true});
        expect(result.components[0].currentData.sensitiveFields['data.value.accessToken']).toEqual({redacted: true, exists: true});
        expect(JSON.stringify(result)).not.toContain('secret-value');
    });

    test('rejects unsupported sources and more than four component ids', () => {
        expect(() => runGetComponentDataContractTool({componentIds: ['a'], sourceType: 'legacy'})).toThrow(/sourceType/);
        expect(() => runGetComponentDataContractTool({componentIds: ['a', 'b', 'c', 'd', 'e']})).toThrow(/1–4/);
    });
});
