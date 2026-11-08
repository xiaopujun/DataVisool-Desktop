/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import FetchUtil, {type FetchRequestInit} from "@/util/FetchUtil.ts";
import {globalMessage} from "@datavisool/ui";

export interface AuthRequestInit extends FetchRequestInit {
    loadingText?: string;
}

/**
 * 提供请求加载提示的工具类
 */
export default class AuthFetchUtil {
    public static async post<T>(url: string, data: any, options?: AuthRequestInit): Promise<T> {
        const loadingKey = options?.loadingText ? `${url}-${Date.now()}` : '';
        if (options?.loadingText) {
            globalMessage.openLoading({key: loadingKey, content: options.loadingText});
        }
        try {
            return await FetchUtil.post<T>(url, data, options);
        } finally {
            if (options?.loadingText) {
                globalMessage.destroy(loadingKey);
            }
        }
    }

    public static async get<T>(url: string, options?: AuthRequestInit): Promise<T> {
        const loadingKey = options?.loadingText ? `${url}-${Date.now()}` : '';
        if (options?.loadingText) {
            globalMessage.openLoading({key: loadingKey, content: options.loadingText});
        }
        try {
            return await FetchUtil.get<T>(url, options);
        } finally {
            if (options?.loadingText) {
                globalMessage.destroy(loadingKey);
            }
        }
    }

}
