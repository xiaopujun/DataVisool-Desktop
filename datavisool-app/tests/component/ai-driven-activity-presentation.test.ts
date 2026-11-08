/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {describe, expect, test} from "vitest";
import {getAiDrivenToolPresentation} from "@/designer/ai-driven/AiDrivenActivityPresentation.ts";

describe("AI-driven activity presentation", () => {
    test("gives component catalog reads a dedicated user-facing presentation", () => {
        const presentation = getAiDrivenToolPresentation("get_component_catalog");

        expect(presentation.title).toMatch(/目录/);
        expect(presentation.detail).toMatch(/组件/);
        expect(presentation.title).not.toContain("get_component_catalog");
    });

    test("does not retain the legacy search tool's dedicated activity label", () => {
        expect(getAiDrivenToolPresentation("search_available_components").title).not.toBe("查找可用组件");
    });
});
