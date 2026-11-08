/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import DesignerLoader from "@/designer/loader/DesignerLoader.ts";
import {
    getAiDrivenComponentSelectionInfoRegistrySnapshot,
} from "@/designer/ai-driven/ComponentAiSelectionDocRegistry.ts";
import type {IAiDrivenComponentInfo} from "@/designer/ai-driven/ComponentAiSelectionDocRegistry.ts";
import type {BaseInfoType} from "@datavisool/sdk/types";

/** 暂不向 AI 开放、需要独立生成流程的复杂组件。 */
const AI_DRIVEN_EXCLUDED_COMPONENT_TYPES = new Set([
    "group",
    "LoopContainer",
    "G2PlotCustom",
    "ECharts",
    "AMap",
    "MapLibreGis",
    "ThreeModel",
]);

/**
 * 组件库清单项。
 */
interface ComponentCatalogItem {
    /** 组件类型，对应 compKey。 */
    componentType: string;
    /** 组件显示名。 */
    componentName: string;
    /** 主分类。 */
    categorize?: string;
    /** 子分类。 */
    subCategorize?: string;
    /** 来源。 */
    source?: string;
    /** 默认宽度。 */
    width?: number;
    /** 默认高度。 */
    height?: number;
    /** 独立 AI_SELECTION 文档提供的组件选择摘要。 */
    componentInfo?: IAiDrivenComponentInfo;
}

/**
 * 组件库清单结果。
 */
interface GetComponentCatalogResult {
    /** 清单版本。 */
    catalogVersion: string;
    /** 当前已注册组件清单。 */
    components: ComponentCatalogItem[];
}

/**
 * 执行组件库清单查询工具。
 */
export function runGetComponentCatalogTool(): GetComponentCatalogResult {
    const selectionInfoByType = new Map(getAiDrivenComponentSelectionInfoRegistrySnapshot().map((info) => [info.componentType, info]));
    const components = Object.values(DesignerLoader.definitionMap)
        .map((definition) => definition.getBaseInfo?.())
        .filter((baseInfo): baseInfo is BaseInfoType => Boolean(baseInfo?.compKey))
        .filter((baseInfo) => !AI_DRIVEN_EXCLUDED_COMPONENT_TYPES.has(baseInfo.compKey))
        .map((baseInfo) => ({
            componentType: baseInfo.compKey,
            componentName: baseInfo.compName,
            categorize: baseInfo.categorize,
            subCategorize: baseInfo.subCategorize,
            source: baseInfo.source,
            width: baseInfo.width,
            height: baseInfo.height,
            componentInfo: selectionInfoByType.get(baseInfo.compKey),
        }))
        .sort((left, right) => left.componentType.localeCompare(right.componentType));

    return {
        catalogVersion: `component-catalog/v3:${components.length}`,
        components,
    };
}
