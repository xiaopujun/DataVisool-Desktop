/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {isTauri} from '@tauri-apps/api/core';

let installed = false;
let alertQueue: Promise<void> = Promise.resolve();

/**
 * 将动态脚本的 alert 接到 Tauri 原生消息框，并强制串行显示。
 *
 * WKWebView 连续显示原生 JavaScript alert 时可能遗留无法关闭的窗口；
 * Tauri dialog 的 Promise 会在窗口关闭后完成，因此可作为可靠的队列边界。
 */
export function installNativeScriptAlertBridge(): void {
    if (installed || !isTauri())
        return;

    installed = true;
    window.alert = (value?: unknown) => {
        const content = value === undefined ? '' : String(value);
        alertQueue = alertQueue
            .catch(() => undefined)
            .then(async () => {
                const {message} = await import('@tauri-apps/plugin-dialog');
                await message(content, {title: 'DataVisool', kind: 'info'});
            })
            .catch((error) => {
                console.error('动态脚本原生提示框显示失败:', error);
            });
    };
}
