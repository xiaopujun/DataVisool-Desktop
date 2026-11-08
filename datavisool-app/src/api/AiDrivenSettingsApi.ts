/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
/*
 * Copyright © 2023-2025 puyinzhen
 * All rights reserved.
 */

import AuthFetchUtil, {type AuthRequestInit} from "@/util/AuthFetchUtil.ts";

export interface IAiDrivenRuntimeSettings {
    schemaVersion: number;
    modelRequest: {
        timeoutSeconds: number;
    };
    context: {
        maxActiveContextTokens: number;
        compactAtPercent: number;
        maxToolResultTokens: number;
        compactSummaryTokens: number;
        maxToolHistoryTokens: number;
        maxStateDigestTokens: number;
    };
    execution: {
        tokenBudgetEnabled: boolean;
        callBudgetEnabled: boolean;
        durationBudgetEnabled: boolean;
        maxModelCalls: number;
        maxToolCalls: number;
        maxWriteCalls: number;
        maxTotalTokens: number;
        maxDurationSeconds: number;
        maxToolDurationSeconds: number;
        maxConsecutiveFailures: number;
        maxFrontendInteractions: number;
        maxSameReadRepeats: number;
        maxCompletionContinuations: number;
    };
}

type AiDrivenRuntimeSettingsInput = {
    schemaVersion?: number;
    modelRequest?: Partial<IAiDrivenRuntimeSettings["modelRequest"]>;
    context?: Partial<IAiDrivenRuntimeSettings["context"]>;
    execution?: Partial<IAiDrivenRuntimeSettings["execution"]>;
};

const DEFAULT_AI_DRIVEN_RUNTIME_SETTINGS: IAiDrivenRuntimeSettings = {
    schemaVersion: 2,
    modelRequest: {timeoutSeconds: 120},
    context: {
        maxActiveContextTokens: 128_000,
        compactAtPercent: 90,
        maxToolResultTokens: 10_000,
        compactSummaryTokens: 4_096,
        maxToolHistoryTokens: 16_000,
        maxStateDigestTokens: 4_096,
    },
    execution: {
        tokenBudgetEnabled: false,
        callBudgetEnabled: false,
        durationBudgetEnabled: false,
        maxModelCalls: 120,
        maxToolCalls: 320,
        maxWriteCalls: 120,
        maxTotalTokens: 1_000_000,
        maxDurationSeconds: 4_500,
        maxToolDurationSeconds: 600,
        maxConsecutiveFailures: 25,
        maxFrontendInteractions: 40,
        maxSameReadRepeats: 3,
        maxCompletionContinuations: 12,
    },
};

/** 使用当前设置的默认值补齐缺失字段。 */
export const normalizeAiDrivenRuntimeSettings = (
    value: AiDrivenRuntimeSettingsInput,
): IAiDrivenRuntimeSettings => {
    if (value.schemaVersion !== undefined && value.schemaVersion !== 2)
        throw new Error("不支持的运行设置版本");
    const execution = value.execution ?? {};
    return {
        schemaVersion: 2,
        modelRequest: {...DEFAULT_AI_DRIVEN_RUNTIME_SETTINGS.modelRequest, ...value.modelRequest},
        context: {...DEFAULT_AI_DRIVEN_RUNTIME_SETTINGS.context, ...value.context},
        execution: {
            tokenBudgetEnabled: execution.tokenBudgetEnabled ?? false,
            callBudgetEnabled: execution.callBudgetEnabled ?? false,
            durationBudgetEnabled: execution.durationBudgetEnabled ?? false,
            maxModelCalls: execution.maxModelCalls ?? DEFAULT_AI_DRIVEN_RUNTIME_SETTINGS.execution.maxModelCalls,
            maxToolCalls: execution.maxToolCalls ?? DEFAULT_AI_DRIVEN_RUNTIME_SETTINGS.execution.maxToolCalls,
            maxWriteCalls: execution.maxWriteCalls ?? DEFAULT_AI_DRIVEN_RUNTIME_SETTINGS.execution.maxWriteCalls,
            maxTotalTokens: execution.maxTotalTokens ?? DEFAULT_AI_DRIVEN_RUNTIME_SETTINGS.execution.maxTotalTokens,
            maxDurationSeconds: execution.maxDurationSeconds ?? DEFAULT_AI_DRIVEN_RUNTIME_SETTINGS.execution.maxDurationSeconds,
            maxToolDurationSeconds: execution.maxToolDurationSeconds ?? DEFAULT_AI_DRIVEN_RUNTIME_SETTINGS.execution.maxToolDurationSeconds,
            maxConsecutiveFailures: execution.maxConsecutiveFailures ?? DEFAULT_AI_DRIVEN_RUNTIME_SETTINGS.execution.maxConsecutiveFailures,
            maxFrontendInteractions: execution.maxFrontendInteractions ?? DEFAULT_AI_DRIVEN_RUNTIME_SETTINGS.execution.maxFrontendInteractions,
            maxSameReadRepeats: execution.maxSameReadRepeats ?? DEFAULT_AI_DRIVEN_RUNTIME_SETTINGS.execution.maxSameReadRepeats,
            maxCompletionContinuations: execution.maxCompletionContinuations ?? DEFAULT_AI_DRIVEN_RUNTIME_SETTINGS.execution.maxCompletionContinuations,
        },
    };
};

export interface IAiDrivenRuntimeSettingsResponse {
    settings: IAiDrivenRuntimeSettings;
    defaults: IAiDrivenRuntimeSettings;
}

export const getAiDrivenRuntimeSettings = async (
    options?: AuthRequestInit,
): Promise<IAiDrivenRuntimeSettingsResponse> => {
    const response = await AuthFetchUtil.get<IAiDrivenRuntimeSettingsResponse>(
        "/api/aiDriven/settings/get",
        options,
    );
    return {
        settings: normalizeAiDrivenRuntimeSettings(response.settings),
        defaults: normalizeAiDrivenRuntimeSettings(response.defaults),
    };
};

export const saveAiDrivenRuntimeSettings = async (
    settings: IAiDrivenRuntimeSettings,
    options?: AuthRequestInit,
): Promise<IAiDrivenRuntimeSettings> => {
    const saved = await AuthFetchUtil.post<IAiDrivenRuntimeSettings>(
        "/api/aiDriven/settings/save",
        settings,
        options,
    );
    return normalizeAiDrivenRuntimeSettings(saved);
};
