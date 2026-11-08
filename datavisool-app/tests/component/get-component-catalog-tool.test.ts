/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, beforeEach, describe, expect, test, vi} from "vitest";

vi.mock("@/designer/loader/DesignerLoader.ts", () => ({
    default: {definitionMap: {}},
}));

import DesignerLoader from "@/designer/loader/DesignerLoader.ts";
import {runGetComponentCatalogTool} from "@/frontend-effect/tools/GetComponentCatalogTool.ts";

type CatalogDefinition = {
    getBaseInfo: () => Record<string, unknown>;
};

const definitionMap = DesignerLoader as unknown as {
    definitionMap: Record<string, CatalogDefinition>;
};

const definition = (baseInfo: Record<string, unknown>): CatalogDefinition => ({
    getBaseInfo: () => baseInfo,
});

describe("getComponentCatalog tool", () => {
    let originalDefinitionMap: Record<string, CatalogDefinition>;

    beforeEach(() => {
        originalDefinitionMap = definitionMap.definitionMap;
    });

    afterEach(() => {
        definitionMap.definitionMap = originalDefinitionMap;
    });

    test("returns registered components in stable type order with the catalog version", () => {
        definitionMap.definitionMap = {
            zeta: definition({
                compKey: "ZetaWidget",
                compName: "Zeta 组件",
                categorize: "业务组件",
                subCategorize: "指标",
                source: "local",
                width: 320,
                height: 180,
            }),
            text: definition({
                compKey: "BaseText",
                compName: "基础文本",
            }),
            chart: definition({
                compKey: "AntdBaseArea",
                compName: "面积图",
            }),
        };

        const result = runGetComponentCatalogTool();

        expect(result.catalogVersion).toBe("component-catalog/v3:3");
        expect(result.components.map(({componentType, componentName}) => ({componentType, componentName}))).toEqual([
            {componentType: "AntdBaseArea", componentName: "面积图"},
            {componentType: "BaseText", componentName: "基础文本"},
            {componentType: "ZetaWidget", componentName: "Zeta 组件"},
        ]);
        expect(result.components[2]).toMatchObject({
            categorize: "业务组件",
            subCategorize: "指标",
            source: "local",
            width: 320,
            height: 180,
        });
    });

    test("excludes internal-only component types and definitions without a component key", () => {
        definitionMap.definitionMap = {
            supported: definition({compKey: "BaseText", compName: "基础文本"}),
            group: definition({compKey: "group", compName: "分组图层"}),
            loopContainer: definition({compKey: "LoopContainer", compName: "循环容器"}),
            echarts: definition({compKey: "ECharts", compName: "自定义 ECharts"}),
            g2plot: definition({compKey: "G2PlotCustom", compName: "自定义 G2Plot"}),
            threeModel: definition({compKey: "ThreeModel", compName: "三维模型"}),
            missingKey: definition({compName: "无类型组件"}),
        };

        const result = runGetComponentCatalogTool();

        expect(result).toEqual({
            catalogVersion: "component-catalog/v3:1",
            components: [{
                componentType: "BaseText",
                componentName: "基础文本",
                categorize: undefined,
                subCategorize: undefined,
                source: undefined,
                width: undefined,
                height: undefined,
                componentInfo: undefined,
            }],
        });
    });
});
