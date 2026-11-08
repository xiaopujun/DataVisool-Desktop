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
    IAiDrivenItem,
    IAiDrivenItemListPayload,
    IAiDrivenItemListResult,
    IAiDrivenItemReadPayload,
} from "@/designer/ai-driven/thread/AiDrivenThreadTypes.ts";

export const listAiDrivenItems = async (
    payload: IAiDrivenItemListPayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenItemListResult | null> => {
    return await AuthFetchUtil.post<IAiDrivenItemListResult>("/api/aiDriven/item/list", payload, options);
};

export const readAiDrivenItem = async (
    payload: IAiDrivenItemReadPayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenItem | null> => {
    return await AuthFetchUtil.post<IAiDrivenItem>("/api/aiDriven/item/read", payload, options);
};
