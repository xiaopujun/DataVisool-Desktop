/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
/*
 * Copyright © 2023-2025 puyinzhen
 * All rights reserved.
 */

import AuthFetchUtil, {type AuthRequestInit} from "@/util/AuthFetchUtil.ts";
import type {
    IAiDrivenThread,
    IAiDrivenThreadArchivePayload,
    IAiDrivenThreadCreatePayload,
    IAiDrivenThreadEventsPayload,
    IAiDrivenThreadEventsResult,
    IAiDrivenThreadListPayload,
    IAiDrivenThreadListResult,
    IAiDrivenThreadReadPayload,
    IAiDrivenThreadSummary,
    IAiDrivenThreadUpdatePayload,
} from "@/designer/ai-driven/thread/AiDrivenThreadTypes.ts";

export const createAiDrivenThread = async (
    payload: IAiDrivenThreadCreatePayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenThread | null> => {
    return await AuthFetchUtil.post<IAiDrivenThread>("/api/aiDriven/thread/create", payload, options);
};

export const listAiDrivenThreads = async (
    payload: IAiDrivenThreadListPayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenThreadListResult | null> => {
    return await AuthFetchUtil.post<IAiDrivenThreadListResult>("/api/aiDriven/thread/list", payload, options);
};

export const readAiDrivenThread = async (
    payload: IAiDrivenThreadReadPayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenThreadSummary | null> => {
    return await AuthFetchUtil.post<IAiDrivenThreadSummary>("/api/aiDriven/thread/read", payload, options);
};

export const updateAiDrivenThread = async (
    payload: IAiDrivenThreadUpdatePayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenThread | null> => {
    return await AuthFetchUtil.post<IAiDrivenThread>("/api/aiDriven/thread/update", payload, options);
};

export const archiveAiDrivenThread = async (
    payload: IAiDrivenThreadArchivePayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenThread | null> => {
    return await AuthFetchUtil.post<IAiDrivenThread>("/api/aiDriven/thread/archive", payload, options);
};

export const getAiDrivenThreadEvents = async (
    payload: IAiDrivenThreadEventsPayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenThreadEventsResult | null> => {
    return await AuthFetchUtil.post<IAiDrivenThreadEventsResult>("/api/aiDriven/thread/events", payload, options);
};
