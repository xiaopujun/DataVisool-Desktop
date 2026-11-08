/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {BaseResourceStore, IBaseResource} from "@/pages/home/common/BaseResourceStore.ts";

export interface IThreeDModelResource extends IBaseResource {
    name?: string;
    url?: string;
    size?: number;
    type?: string;
    userId?: string;
    isCompressed?: number; // 0: 否, 1: 是
    createTime?: string;
}

export class ThreeDModelResourceStore extends BaseResourceStore<IThreeDModelResource> {
    getListApiUrl = '/api/model/page';
    deleteApiUrl = '/api/model/batchDelete';
    updateApiUrl = '/api/model/update';
}

const threeDModelResourceStore = new ThreeDModelResourceStore();
export default threeDModelResourceStore;
