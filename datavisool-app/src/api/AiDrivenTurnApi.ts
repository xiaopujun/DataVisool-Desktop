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
    IAiDrivenTurn,
    IAiDrivenTurnListPayload,
    IAiDrivenTurnListResult,
    IAiDrivenTurnReadPayload,
} from "@/designer/ai-driven/thread/AiDrivenThreadTypes.ts";

export const listAiDrivenTurns = async (
    payload: IAiDrivenTurnListPayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenTurnListResult | null> => {
    return await AuthFetchUtil.post<IAiDrivenTurnListResult>("/api/aiDriven/turn/list", payload, options);
};

export const readAiDrivenTurn = async (
    payload: IAiDrivenTurnReadPayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenTurn | null> => {
    return await AuthFetchUtil.post<IAiDrivenTurn>("/api/aiDriven/turn/read", payload, options);
};
