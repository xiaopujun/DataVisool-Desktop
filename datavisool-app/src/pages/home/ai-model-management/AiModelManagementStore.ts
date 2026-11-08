/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import { BaseResourceStore, IBaseResource } from "@/pages/home/common/BaseResourceStore.ts";

export interface IAiModel extends IBaseResource {
    key?: string;
    name?: string;
    provider?: string;
    providerLabel?: string;
    providerFamily?: string;
    baseUrl?: string;
    apiKey?: string;
    hasApiKey?: boolean;
    clearApiKey?: number;
    description?: string;
    temperature?: number;
    maxTokens?: number;
    enabled?: number;
    isDefault?: number;
    createTime?: string;
    updateTime?: string;
}

class AiModelManagementStore extends BaseResourceStore<IAiModel> {
    getListApiUrl = "/api/aiModel/pageList";
    deleteApiUrl = "/api/aiModel/batchDel";
    createApiUrl = "/api/aiModel/create";
    updateApiUrl = "/api/aiModel/update";
}

const aiModelManagementStore = new AiModelManagementStore();

export default aiModelManagementStore;
