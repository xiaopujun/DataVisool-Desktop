/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {beforeEach, describe, expect, test, vi} from "vitest";

const mocks = vi.hoisted(() => ({
    get: vi.fn(),
    post: vi.fn(),
}));

vi.mock("@/util/AuthFetchUtil.ts", () => ({
    default: {
        get: mocks.get,
        post: mocks.post,
    },
}));

import {
    getAiDrivenRuntimeSettings,
    normalizeAiDrivenRuntimeSettings,
} from "@/api/AiDrivenSettingsApi.ts";

describe("AI driven runtime settings", () => {
    beforeEach(() => vi.clearAllMocks());

    test("normalizes settings and defaults returned by the API", async () => {
        mocks.get.mockResolvedValue({
            settings: {execution: {maxModelCalls: 8}},
            defaults: {schemaVersion: 2},
        });

        const response = await getAiDrivenRuntimeSettings();

        expect(response.settings.schemaVersion).toBe(2);
        expect(response.settings.execution.maxModelCalls).toBe(8);
        expect(response.defaults.schemaVersion).toBe(2);
    });

    test("keeps all budgets disabled for a v2 partial settings object without execution", () => {
        const settings = normalizeAiDrivenRuntimeSettings({
            schemaVersion: 2,
            modelRequest: {timeoutSeconds: 45},
        });

        expect(settings.modelRequest.timeoutSeconds).toBe(45);
        expect(settings.execution.tokenBudgetEnabled).toBe(false);
        expect(settings.execution.callBudgetEnabled).toBe(false);
        expect(settings.execution.durationBudgetEnabled).toBe(false);
    });
});
