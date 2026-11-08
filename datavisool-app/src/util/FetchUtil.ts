/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import responseHandler from "@/api/ReponseHandler.ts";
import type {ResponseHandlerOptions} from "@/api/ReponseHandler.ts";
import axios, {AxiosRequestConfig} from 'axios';
import {invoke} from "@tauri-apps/api/core";

export interface HttpResponse {
    code: number;
    msg: string;
    data: any;
}

export type NativeRequestTestResult =
    | {ok: true; data: unknown}
    | {ok: false; error: string; status?: number; response?: unknown};

export interface FetchRequestInit extends RequestInit {
    suppressErrorMessage?: ResponseHandlerOptions["suppressErrorMessage"];
}

export default class FetchUtil {
    private static nextDesktopRequestSeed = 0;

    private static recordPerfCounter(kind: "api" | "ipc", key: string): void {
        const perf = (window as any).__DATAVISOOL_PERF_BASELINE__;
        if (!perf)
            return;
        perf.counters = perf.counters || {};
        perf.counters[kind] = perf.counters[kind] || {total: 0, byKey: {}};
        perf.counters[kind].total += 1;
        perf.counters[kind].byKey[key] = (perf.counters[kind].byKey[key] || 0) + 1;
    }

    private static createDesktopRequestId(): number {
        const seed = FetchUtil.nextDesktopRequestSeed = (FetchUtil.nextDesktopRequestSeed + 1) % 1000;
        return Date.now() * 1000 + seed;
    }

    private static async registerDesktopRequest(requestId: number): Promise<void> {
        FetchUtil.recordPerfCounter("ipc", "datavisool_register_http_request");
        await invoke<void>("datavisool_register_http_request", {
            request_id: requestId,
        });
    }

    private static async cancelDesktopRequest(requestId: number): Promise<void> {
        FetchUtil.recordPerfCounter("ipc", "datavisool_cancel_http_request");
        await invoke<void>("datavisool_cancel_http_request", {
            request_id: requestId,
        });
    }

    private static async unregisterDesktopRequest(requestId: number): Promise<void> {
        FetchUtil.recordPerfCounter("ipc", "datavisool_unregister_http_request");
        await invoke<void>("datavisool_unregister_http_request", {
            request_id: requestId,
        });
    }

    private static raceWithAbort<T>(
        signal: AbortSignal | undefined,
        task: Promise<T>,
        abortedValue: T,
        onAbort?: () => void,
    ): Promise<T> {
        if (!signal) {
            return task;
        }
        if (signal.aborted) {
            onAbort?.();
            return Promise.resolve(abortedValue);
        }

        let abortListener: (() => void) | null = null;
        const abortPromise = new Promise<T>((resolve) => {
            abortListener = () => {
                try {
                    onAbort?.();
                } finally {
                    resolve(abortedValue);
                }
            };
            signal.addEventListener("abort", abortListener, {once: true});
        });

        task.finally(() => {
            if (abortListener) {
                signal.removeEventListener("abort", abortListener);
            }
        }).catch(() => undefined);

        return Promise.race([task, abortPromise]);
    }

    private static async invokeDesktopHttpRequest(
        config: AxiosRequestConfig,
        abortedValue: HttpResponse | null,
    ): Promise<HttpResponse | null> {
        const signal = config.signal as AbortSignal | undefined;
        let requestId: number | null = null;
        let requestRegistered = false;

        try {
            if (signal) {
                requestId = FetchUtil.createDesktopRequestId();
                try {
                    await FetchUtil.registerDesktopRequest(requestId);
                    requestRegistered = true;
                } catch (error) {
                    console.warn("桌面端请求注册失败，取消能力可能不可用:", error);
                }
            }

            if (signal?.aborted) {
                if (requestRegistered && requestId !== null) {
                    await FetchUtil.unregisterDesktopRequest(requestId).catch(() => undefined);
                }
                return abortedValue;
            }

            const headers: Record<string, string> | undefined = config.headers
                ? (config.headers as any)
                : undefined;
            const body = FetchUtil.normalizeBody(config.data);
            FetchUtil.recordPerfCounter("ipc", "datavisool_http_request");
            const requestTask = invoke<HttpResponse | null>("datavisool_http_request", {
                req: {
                    url: config.url,
                    method: (config.method || "GET").toString().toUpperCase(),
                    headers,
                    body,
                    requestId: requestId ?? undefined,
                },
            });

            let onAbort: (() => void) | undefined;
            if (requestId !== null) {
                const currentRequestId = requestId;
                onAbort = () => {
                    void FetchUtil.cancelDesktopRequest(currentRequestId).catch(() => undefined);
                };
            }

            return await FetchUtil.raceWithAbort(
                signal,
                requestTask,
                abortedValue,
                onAbort,
            );
        } catch (error) {
            if (requestRegistered && requestId !== null) {
                await FetchUtil.unregisterDesktopRequest(requestId).catch(() => undefined);
            }
            throw error;
        }
    }

    /**
     * GET请求
     * @param url
     * @param options
     */
    static async get<T>(url: string, options: FetchRequestInit = {}): Promise<T> {
        const config: AxiosRequestConfig = {
            method: 'GET',
            headers: options.headers as any,
            signal: options.signal as any,
        };
        const response = await FetchUtil.request(url, config);
        return responseHandler(response, {
            suppressErrorMessage: options.suppressErrorMessage,
        }) as T;
    }

    /**
     * POST请求
     * @param url
     * @param data
     * @param options
     */
    static async post<T>(url: string, data: any, options: FetchRequestInit = {}): Promise<T> {
        const config: AxiosRequestConfig = {
            method: 'POST',
            headers: options.headers as any || {},
            signal: options.signal as any,
        };

        // 默认'Content-Type': 'application/json'
        if (!config.headers) {
            config.headers = {};
        }
        
        // 兼容 RequestInit 的 headers 可能为 Headers 对象的情况，虽然这里假定是 Record<string, string>
        // 但为了安全起见，如果是 Headers 对象需要处理，不过 AuthFetchUtil 传入的是 Record。
        // 简单处理：如果 config.headers 没有 Content-Type，则设置默认值。
        const headers = config.headers as Record<string, any>;
        if (!headers['Content-Type']) {
            headers['Content-Type'] = 'application/json';
        }

        const contentType = headers['Content-Type'];
        
        if (contentType === 'application/json') {
            config.data = data;
        } else if (contentType === 'application/x-www-form-urlencoded') {
            const params = new URLSearchParams();
            for (const [key, value] of Object.entries(data)) {
                params.append(key, value as string);
            }
            config.data = params;
        } else if (contentType === 'multipart/form-data') {
            // multipart/form-data 不能直接设置 Content-Type，因为需要由浏览器自动生成
            delete headers['Content-Type'];
            if (data instanceof FormData) {
                config.data = data;
            } else {
                const formData = new FormData();
                data.values().forEach((value: any, key: string) => {
                    formData.append(key, value as Blob | string);
                })
                config.data = formData;
            }
        } else {
            // 默认处理为 text/plain
            config.data = data.toString();
        }
        
        const response = await FetchUtil.request(url, config);
        return responseHandler(response, {
            suppressErrorMessage: options.suppressErrorMessage,
        }) as T;
    }

    /**
     * 请求的标准返回结果必须包含code、msg字段，data字段可选，且code为200时，data为有效数据
     * @param url
     * @param config
     * @private
     */
    private static async request(url: string, config: AxiosRequestConfig): Promise<HttpResponse> {
        config.url = url;
        if (url.startsWith('/api/'))
            FetchUtil.recordPerfCounter("api", url.split('?')[0]);
        // 强制返回 blob 以便于后续手动处理 Content-Type 逻辑，保持与 fetch 逻辑一致
        config.responseType = 'blob';

        try {
            if (config.signal && (config.signal as AbortSignal).aborted) {
                return {code: 499, msg: '请求已取消', data: null};
            }
            if (FetchUtil.isTauri() && config.url.startsWith('/api/')) {
                const response = await FetchUtil.invokeDesktopHttpRequest(
                    config,
                    {code: 499, msg: '请求已取消', data: null},
                );
                return response ?? {code: 499, msg: '请求已取消', data: null};
            }

            const response = await axios(config);
            const contentType = response.headers['content-type'];
            const blob = response.data as Blob;

            if (contentType && contentType.includes('application/json')) {
                const text = await blob.text();
                // 某些情况下接口可能返回空，或者返回非 JSON 格式
                try {
                    const res = JSON.parse(text);
                    if (typeof res === 'object' && 'code' in res && 'msg' in res)
                        return res;
                    else
                        return {code: 200, msg: '操作成功', data: res};
                } catch (e) {
                    // JSON 解析失败，作为普通文本或对象返回
                    return {code: 200, msg: '操作成功', data: text};
                }
            } else if (contentType && (contentType.includes('text/html') || contentType.includes('text/plain'))) {
                const text = await blob.text();
                return {code: 200, msg: '操作成功', data: text}
            } else {
                return {code: 200, msg: '操作成功', data: blob}
            }
        } catch (error: any) {
            if ((config.signal && (config.signal as AbortSignal).aborted)
                || error?.code === 'ERR_CANCELED'
                || error?.name === 'CanceledError') {
                return {code: 499, msg: '请求已取消', data: null};
            }
            if (FetchUtil.isTauri()) {
                console.error('请求错误:', {url: config.url, method: config.method, error});
            }
            const response = error.response;
            return {
                code: response?.status ?? 500, 
                msg: response?.statusText ?? (error.message || 'Unknown Error'), 
                data: null
            };
        }
    }

    /**
     * 返回原生结果，不做任何封装
     * @param url
     * @param options
     * @private
     */
    private static async requestNativeResultOutcome(url: string, options: RequestInit = {}): Promise<NativeRequestTestResult> {
        const config: AxiosRequestConfig = {
            method: options.method as any || 'GET',
            headers: options.headers as any,
            data: options.body, // 注意：fetch 的 body 在这里对应 axios 的 data
            url: url,
            responseType: 'blob',
            signal: options.signal as any,
        };

        try {
            if (options.signal?.aborted) {
                return {ok: false, error: '请求已取消'};
            }
            if (FetchUtil.isTauri() && typeof config.url === 'string' && config.url.startsWith('/api/')) {
                const response = await FetchUtil.invokeDesktopHttpRequest(config, null);
                if (response == null || response.code === 499) {
                    return {ok: false, error: '请求已取消'};
                }
                return {ok: true, data: response};
            }

            const response = await axios(config);
            const data = await FetchUtil.readNativeResponseData(
                response.data,
                response.headers['content-type'],
            );
            return {ok: true, data};
        } catch (error: any) {
            if (options.signal?.aborted
                || error?.code === 'ERR_CANCELED'
                || error?.name === 'CanceledError') {
                return {ok: false, error: '请求已取消'};
            }
            console.error('请求错误:', error);
            const failedResponse = error?.response;
            if (failedResponse) {
                try {
                    const body = await FetchUtil.readNativeResponseData(
                        failedResponse.data,
                        failedResponse.headers?.['content-type'],
                    );
                    return {
                        ok: false,
                        error: `HTTP ${failedResponse.status}${failedResponse.statusText ? ` ${failedResponse.statusText}` : ''}`,
                        status: failedResponse.status,
                        response: body,
                    };
                } catch (parseError: any) {
                    const body = failedResponse.data instanceof Blob
                        ? await failedResponse.data.text()
                        : undefined;
                    const reason = parseError?.message || '响应 JSON 无法解析';
                    return {
                        ok: false,
                        error: `HTTP ${failedResponse.status}，响应 JSON 无法解析：${reason}`,
                        status: failedResponse.status,
                        response: body,
                    };
                }
            }

            const message = error?.message || '请求失败';
            return {
                ok: false,
                error: error instanceof SyntaxError
                    ? `响应 JSON 无法解析：${message}`
                    : message,
            };
        }
    }

    private static async readNativeResponseData(data: unknown, contentType?: string): Promise<unknown> {
        if (!(data instanceof Blob)) {
            return data;
        }

        if (contentType?.includes('application/json')) {
            return JSON.parse(await data.text());
        }
        if (contentType?.includes('text/html') || contentType?.includes('text/plain')) {
            return data.text();
        }
        return data;
    }

    private static async requestNativeResult(url: string, options: RequestInit = {}): Promise<any> {
        const result = await FetchUtil.requestNativeResultOutcome(url, options);
        return result.ok ? result.data : null;
    }

    private static isTauri(): boolean {
        return typeof window !== 'undefined' && (('__TAURI_INTERNALS__' in window) || ('__TAURI__' in window));
    }

    private static normalizeBody(input: any): any {
        if (input == null) return null;
        if (typeof URLSearchParams !== 'undefined' && input instanceof URLSearchParams) {
            const obj: Record<string, string> = {};
            input.forEach((value, key) => {
                obj[key] = value;
            });
            return obj;
        }
        if (typeof FormData !== 'undefined' && input instanceof FormData) {
            return null;
        }
        return input;
    }

    /**
     * 请求前置处理，返回处理后的url和options
     * @param url
     * @param method
     * @param headers
     * @param params
     * @private
     */
    private static doRequestBefore(url: string, method: string, headers: Record<string, string>, params: FormData | Record<string, any>): {
        url: string,
        options: RequestInit
    } {
        const options: RequestInit = {
            method: method.toUpperCase(),
            headers: headers,
        };
        if (params) {
            if (method.toUpperCase() === 'GET') {
                if (params && Object.keys(params).length > 0) {
                    const queryString = new URLSearchParams(params as Record<string, string>).toString();
                    if (queryString) {
                        const fragmentIndex = url.indexOf("#");
                        const fragment = fragmentIndex >= 0 ? url.slice(fragmentIndex) : "";
                        const requestUrl = fragmentIndex >= 0 ? url.slice(0, fragmentIndex) : url;
                        const separator = requestUrl.includes("?")
                            ? (requestUrl.endsWith("?") || requestUrl.endsWith("&") ? "" : "&")
                            : "?";
                        url = `${requestUrl}${separator}${queryString}${fragment}`;
                    }
                }
            } else {
                // 如果没有设置 Content-Type，默认为 application/json
                if (!options.headers) options.headers = {};
                if (!('Content-Type' in (options.headers as any))) {
                    (options.headers as any)['Content-Type'] = 'application/json';
                }
                
                // 这里我们不需要像 fetch 那样手动 stringify body，
                // 但为了保持 doRequestBefore 的返回类型一致（供 requestNativeResult 使用），
                // 我们暂时保留 fetch 的风格，但在调用 request/requestNativeResult 时再适配。
                // 不过 FetchUtil.post 已经处理了 body 构建。
                // 这里的 doRequestBefore 主要是为了构建参数。
                
                // 为了兼容 requestNativeResult 中直接使用 options.body
                const contentType = (options.headers as any)['Content-Type'];
                if (contentType === 'application/json') {
                    options.body = JSON.stringify(params);
                } else {
                     // 其他类型暂不处理，假设 params 已经合适或者会在 post 中处理
                     // 但 requestNativeResult 直接用 options.body
                     // 如果是 axios，requestNativeResult 需要把 body 赋给 data
                     // 如果是 json string，axios 会再 stringify 一次吗？
                     // 如果 data 是 string，axios 默认发送 string。
                     options.body = params as any;
                }
            }
        }
        return {url, options};
    }

    /**
     * 执行自定义请求，请求参数均通过外部指定，返回结果必须为HttpResponse对象，必须包含code、msg字段，data字段可选
     * @param url
     * @param method
     * @param headers
     * @param params
     */
    static async doRequest(url: string, method: string, headers: Record<string, string>, params: FormData | Record<string, any>): Promise<HttpResponse> {
        // doRequestBefore 构建了 url 和 options (包含 headers 和 body)
        // 我们需要把 options 转换回 axios config
        const reqInfo = FetchUtil.doRequestBefore(url, method, headers, params);
        
        // 此时 reqInfo.options.body 可能是 JSON string 或者原始对象（如果 doRequestBefore 没转的话）
        // 上面的 doRequestBefore 逻辑里，如果是 json 则转了 string。
        // 如果我们传 string 给 axios data，axios 会直接发出去。
        // 如果 Content-Type 是 application/json，axios 发送 string 是没问题的。
        
        const config: AxiosRequestConfig = {
            method: reqInfo.options.method as any,
            headers: reqInfo.options.headers as any,
            data: reqInfo.options.body
        };
        
        return FetchUtil.request(reqInfo.url, config);
    }

    /**
     * 执行自定义请求，请求参数均通过外部指定，返回结果为接口原生结果
     * @param url
     * @param method
     * @param headers
     * @param params
     */
    static async doRequestNativeResult(url: string, method: string, headers: Record<string, string>, params: FormData | Record<string, any>): Promise<object> {
        const reqInfo = FetchUtil.doRequestBefore(url, method, headers, params);
        return FetchUtil.requestNativeResult(reqInfo.url, reqInfo.options);
    }

    /** API 编辑面板使用，保留 HTTP、网络和解析错误供测试结果区显示。 */
    static async doRequestNativeResultForTest(url: string, method: string, headers: Record<string, string>, params: FormData | Record<string, any>): Promise<NativeRequestTestResult> {
        const reqInfo = FetchUtil.doRequestBefore(url, method, headers, params);
        const result = await FetchUtil.requestNativeResultOutcome(reqInfo.url, reqInfo.options);
        if (!result.ok
            || !FetchUtil.isTauri()
            || !reqInfo.url.startsWith('/api/')
            || !result.data
            || typeof result.data !== 'object'
            || !('code' in result.data)
            || !('msg' in result.data)) {
            return result;
        }

        const response = result.data as HttpResponse;
        if (response.code === 200) {
            return result;
        }
        return {
            ok: false,
            error: response.msg || `请求失败（${response.code}）`,
            status: response.code,
            response: response.data,
        };
    }


}
