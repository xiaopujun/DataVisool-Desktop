/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
/*
 * Copyright © 2023-2025 puyinzhen
 * All rights reserved.
 */

/** AI 组件选择阶段使用的组件信息摘要。 */
export interface IAiDrivenComponentInfo {
    componentType: string;
    componentName: string;
    description: string;
    nativeCapabilities: string;
    selectable: boolean;
}

const AI_SELECTION_DOC_MODULES = import.meta.glob('/src/comps/**/AI_SELECTION.md', {
    eager: true,
    query: '?raw',
    import: 'default',
}) as Record<string, string>;

let selectionInfoSnapshot: IAiDrivenComponentInfo[] | undefined;

/** 读取独立 AI_SELECTION 文档中的组件选择摘要。 */
export const getAiDrivenComponentSelectionInfoRegistrySnapshot = (): IAiDrivenComponentInfo[] => {
    if (!selectionInfoSnapshot) {
        selectionInfoSnapshot = Object.entries(AI_SELECTION_DOC_MODULES)
            .map(([sourcePath, content]) => extractComponentInfo(content, sourcePath))
            .filter((info): info is IAiDrivenComponentInfo => Boolean(info));
    }
    return selectionInfoSnapshot;
};

function extractComponentInfo(content: string, sourcePath: string): IAiDrivenComponentInfo | undefined {
    const section = extractMarkdownSection(content, "组件信息");
    if (!section)
        return undefined;
    const jsonBlock = /```json\s*([\s\S]*?)```/.exec(section)?.[1];
    if (!jsonBlock)
        return undefined;
    try {
        const raw = JSON.parse(jsonBlock) as Partial<IAiDrivenComponentInfo>;
        const componentType = text(raw.componentType) || sourcePath.split('/').filter(Boolean).at(-2)?.trim() || "";
        const componentName = text(raw.componentName);
        const description = text(raw.description);
        const nativeCapabilities = text(raw.nativeCapabilities);
        if (!componentType || !componentName || !description || !nativeCapabilities || nativeCapabilities.length > 120)
            return undefined;
        return {
            componentType,
            componentName,
            description,
            nativeCapabilities,
            selectable: typeof raw.selectable === "boolean" ? raw.selectable : true,
        };
    } catch {
        return undefined;
    }
}

function extractMarkdownSection(content: string, title: string): string {
    const heading = new RegExp(`^##\\s+${title}\\s*$`, "m").exec(content);
    if (!heading)
        return "";
    const rest = content.slice(heading.index + heading[0].length);
    const nextHeading = /^##\s+/m.exec(rest);
    return (nextHeading ? rest.slice(0, nextHeading.index) : rest).trim();
}

function text(value: unknown): string {
    return typeof value === "string" ? value.trim() : "";
}
