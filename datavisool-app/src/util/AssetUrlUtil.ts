/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {convertFileSrc} from "@tauri-apps/api/core";

/**
 * 统一处理资源路径在“桌面端协议 / 本地绝对路径 / 浏览器可加载 URL”之间的转换。
 *
 * 这个工具类主要解决三类问题：
 * 1. 不同平台下本地文件路径格式不同（Windows 盘符、Unix 根目录）；
 * 2. 桌面端后端返回的是资源协议 URL，而组件渲染时需要的是可真正加载的地址；
 * 3. 模型这类多文件资源会包含相对路径子资源，需要基于主资源路径做再次解析。
 *
 * 这里不做业务层判断，也不校验文件是否真实存在，只负责“路径与 URL 的归一化和转换”。
 */
export default class AssetUrlUtil {
    private static readonly ASSET_HTTP_PREFIX = 'http://asset.localhost/';
    private static readonly ASSET_SCHEME_PREFIX = 'asset://localhost/';

    /**
     * 判断当前是否运行在 Tauri 桌面端环境。
     *
     * 之所以同时判断两个全局标记，是因为不同版本或不同注入方式下，
     * 可能存在 `__TAURI_INTERNALS__` 或 `__TAURI__` 任意一种。
     *
     * @returns `true` 表示当前在桌面端；否则视为普通浏览器环境
     */
    public static isTauri() {
        return typeof window !== 'undefined' && (('__TAURI_INTERNALS__' in window) || ('__TAURI__' in window));
    }

    /**
     * 判断一个 URL 是否属于桌面端资源协议地址。
     *
     * 当前项目里桌面端资源 URL 主要有两种形式：
     * - Windows: `http://asset.localhost/...`
     * - macOS / Linux: `asset://localhost/...`
     *
     * @param url 待判断的 URL
     * @returns 是否匹配桌面端资源协议前缀
     */
    public static isAssetLikeUrl(url?: string | null) {
        const value = (url || '').trim();
        return value.startsWith(AssetUrlUtil.ASSET_HTTP_PREFIX) || value.startsWith(AssetUrlUtil.ASSET_SCHEME_PREFIX);
    }

    /**
     * 把桌面端资源 URL 解码回本地绝对路径。
     *
     * 例如：
     * - `asset://localhost/%2FUsers%2Fa%2Fdemo.png` -> `/Users/a/demo.png`
     * - `http://asset.localhost/C%3A%2Fwork%2Fa.png` -> `C:/work/a.png`
     *
     * 如果输入并不是桌面端资源 URL，也会尽量原样返回。
     * 如果 URL 解码失败，则返回去掉协议前缀后的原始内容，避免中断后续流程。
     *
     * @param url 桌面端资源 URL
     * @returns 解码后的本地绝对路径，或失败时的兜底原值
     */
    public static decodeAssetLikeUrl(url?: string | null) {
        const value = (url || '').trim();
        if (!value) return value;
        let encoded = value;
        if (value.startsWith(AssetUrlUtil.ASSET_HTTP_PREFIX))
            encoded = value.slice(AssetUrlUtil.ASSET_HTTP_PREFIX.length);
        else if (value.startsWith(AssetUrlUtil.ASSET_SCHEME_PREFIX))
            encoded = value.slice(AssetUrlUtil.ASSET_SCHEME_PREFIX.length);
        try {
            return decodeURIComponent(encoded);
        } catch {
            return encoded;
        }
    }

    /**
     * 判断字符串是否为 Windows 绝对路径。
     *
     * 支持：
     * - 盘符路径：`C:\a\b` / `C:/a/b`
     * - UNC 路径：`\\server\share\file`
     *
     * @param value 待判断字符串
     * @returns 是否为 Windows 绝对路径
     */
    private static isWindowsAbsolutePath(value: string) {
        return /^[A-Za-z]:[\\/]/.test(value) || value.startsWith('\\\\');
    }

    /**
     * 判断字符串是否为 Unix 风格绝对路径。
     *
     * 这里没有简单使用 `startsWith('/')`，而是限定为当前项目实际会遇到的几个常见系统根目录，
     * 用来尽量避免把浏览器里的 `/static/...` 误判为本地文件系统路径。
     *
     * @param value 待判断字符串
     * @returns 是否为 Unix 风格绝对路径
     */
    private static isUnixAbsolutePath(value: string) {
        return /^\/(Users|home|var|private|opt|Volumes|tmp|Applications|System|Library|data)\//.test(value);
    }

    /**
     * 统一判断一个字符串是否为本地绝对路径。
     *
     * @param value 待判断字符串
     * @returns 是否为 Windows 或 Unix 本地绝对路径
     */
    private static isLocalAbsolutePath(value: string) {
        return AssetUrlUtil.isWindowsAbsolutePath(value) || AssetUrlUtil.isUnixAbsolutePath(value);
    }

    /**
     * 判断字符串是否为浏览器语义下的站内路径或相对路径。
     *
     * 例如：
     * - `/static/images/a.png`
     * - `./textures/a.png`
     * - `../bin/scene.bin`
     *
     * @param value 待判断字符串
     * @returns 是否为 Web 路径
     */
    private static isWebRelativePath(value: string) {
        return value.startsWith('/') || value.startsWith('./') || value.startsWith('../');
    }

    /**
     * 判断字符串是否已经属于网络可加载地址。
     *
     * 这里把以下协议都视为“已经可直接交给浏览器加载”：
     * - `http://`
     * - `https://`
     * - `data:`
     * - `blob:`
     *
     * @param value 待判断字符串
     * @returns 是否为网络可加载 URL
     */
    private static isNetworkLikeUrl(value: string) {
        return value.startsWith('http://') || value.startsWith('https://') || value.startsWith('data:') || value.startsWith('blob:');
    }

    /**
     * 统一把本地路径里的分隔符规范成 `/`。
     *
     * 这个方法只做字符串层面的归一化，不负责判断路径是否存在。
     *
     * @param value 原始路径
     * @returns 统一分隔符后的路径
     */
    private static normalizeLocalPath(value: string) {
        return value.replace(/\\/g, '/');
    }

    /**
     * 去掉 URL 或路径上的 query 与 hash 部分。
     *
     * 对本地文件定位来说，`?` 和 `#` 后面的内容不参与真实文件路径解析。
     *
     * @param value 原始 URL 或路径
     * @returns 去掉 query/hash 后的主体部分
     */
    private static stripQueryAndHash(value: string) {
        return value.split('#')[0].split('?')[0];
    }

    /**
     * 基于主资源所在目录，解析一个相对路径子资源。
     *
     * 典型场景是模型资源：
     * - 主文件：`/a/models/demo/scene.gltf`
     * - 子资源：`../textures/base.png`
     * - 结果：`/a/models/textures/base.png`
     *
     * 这个方法只处理路径拼接规则，不做文件存在性检查。
     *
     * @param baseDir 主资源所在目录，要求是本地绝对路径
     * @param relativePath 子资源相对路径
     * @returns 拼接后的本地绝对路径
     */
    private static resolveRelativePath(baseDir: string, relativePath: string) {
        const normalizedBase = AssetUrlUtil.normalizeLocalPath(baseDir);
        const normalizedRelative = AssetUrlUtil.normalizeLocalPath(relativePath);

        const windowsRootMatch = normalizedBase.match(/^[A-Za-z]:\//);
        const isUnix = normalizedBase.startsWith('/');
        const root = windowsRootMatch?.[0] ?? (isUnix ? '/' : '');

        const baseBody = windowsRootMatch
            ? normalizedBase.slice(windowsRootMatch[0].length)
            : (isUnix ? normalizedBase.slice(1) : normalizedBase);

        const segments = baseBody.split('/').filter(Boolean);
        for (const part of normalizedRelative.split('/')) {
            if (!part || part === '.') continue;
            if (part === '..') {
                segments.pop();
                continue;
            }
            segments.push(part);
        }
        return `${root}${segments.join('/')}`;
    }

    /**
     * 尽可能把一个资源值还原为本地绝对路径。
     *
     * 支持两种输入：
     * - 桌面端资源 URL：先解码回本地绝对路径
     * - 本地绝对路径：直接做规范化
     *
     * 对普通网络 URL 或浏览器相对路径，这里不会强行处理，直接返回空字符串，
     * 表示当前无法安全地把它视作本地文件路径。
     *
     * @param url 原始资源值
     * @returns 本地绝对路径；无法解析时返回空字符串
     */
    private static toLocalPath(url?: string | null) {
        const value = (url || '').trim();
        if (!value) return value;

        if (AssetUrlUtil.isAssetLikeUrl(value))
            return AssetUrlUtil.normalizeLocalPath(AssetUrlUtil.stripQueryAndHash(AssetUrlUtil.decodeAssetLikeUrl(value)));

        if (AssetUrlUtil.isLocalAbsolutePath(value))
            return AssetUrlUtil.normalizeLocalPath(AssetUrlUtil.stripQueryAndHash(value));

        return '';
    }

    /**
     * 把“桌面端资源 URL”或“本地绝对路径”转换成当前环境下更适合加载的地址。
     *
     * 主要规则：
     * - 如果已经是桌面端资源 URL，则在 Tauri 环境下转成 `convertFileSrc(...)`
     * - 如果已经是网络 URL，则原样返回
     * - 如果是本地绝对路径，则在 Tauri 环境下转成 `convertFileSrc(...)`
     *
     * 这个方法偏向“桌面端协议与本地路径的桥接”，不会主动把 `/static/...` 这类浏览器路径补全成完整 origin。
     *
     * @param path 原始路径或 URL
     * @returns 当前环境可用的资源地址
     */
    public static toAssetUrl(path?: string | null) {
        const value = (path || '').trim();
        if (!value) return value;
        if (AssetUrlUtil.isAssetLikeUrl(value)) {
            if (!AssetUrlUtil.isTauri()) return value;
            const decodedPath = AssetUrlUtil.decodeAssetLikeUrl(value);
            try {
                return convertFileSrc(decodedPath);
            } catch {
                return value;
            }
        }
        if (AssetUrlUtil.isNetworkLikeUrl(value))
            return value;
        if (value.startsWith('asset://') || value.startsWith('tauri://'))
            return value;
        if (!AssetUrlUtil.isTauri())
            return value;
        if (!AssetUrlUtil.isLocalAbsolutePath(value))
            return value;
        return convertFileSrc(value);
    }

    /**
     * 把任意资源引用尽量转换成“最终可直接渲染”的 URL。
     *
     * 与 `toAssetUrl()` 相比，这里更偏向组件渲染入口使用：
     * - 先尝试处理桌面端资源 URL / 本地绝对路径
     * - 如果还是普通 Web 相对路径，则基于 `window.location.origin` 补全
     * - 如果本身已经是网络地址，则直接返回
     *
     * 适用场景：
     * - `<img src>`
     * - `<video src>`
     * - 模型主文件 URL
     *
     * @param url 原始资源值
     * @returns 尽可能可直接加载的 URL
     */
    public static toLoadableUrl(url?: string | null) {
        const value = (url || '').trim();
        if (!value) return value;

        const assetOrLocal = AssetUrlUtil.toAssetUrl(value);
        if (assetOrLocal !== value)
            return assetOrLocal;

        if (AssetUrlUtil.isLocalAbsolutePath(value))
            return value;

        if (AssetUrlUtil.isNetworkLikeUrl(value) || value.startsWith('asset://') || value.startsWith('tauri://'))
            return value;

        if (AssetUrlUtil.isWebRelativePath(value) && typeof window !== 'undefined')
            return new URL(value, window.location.origin).toString();

        return value;
    }

    /**
     * 从主资源 URL 中推导其所在目录，并返回该目录对应的可加载地址。
     *
     * 典型用法是给模型加载器设置 `resourcePath`：
     * - 主文件：`asset://localhost/.../models/hash/scene.gltf`
     * - 结果目录：`.../models/hash/`
     *
     * 只有当输入值能够还原成本地绝对路径时，这个方法才会生效。
     *
     * @param url 主资源 URL 或本地绝对路径
     * @returns 主资源目录的可加载 URL；失败时返回空字符串
     */
    public static toResourceDirectoryUrl(url?: string | null) {
        const localPath = AssetUrlUtil.toLocalPath(url);
        if (!localPath)
            return '';

        const lastSlashIndex = localPath.lastIndexOf('/');
        if (lastSlashIndex < 0)
            return '';

        const baseDir = localPath.slice(0, lastSlashIndex + 1);
        return AssetUrlUtil.toLoadableUrl(baseDir);
    }

    /**
     * 基于主资源地址，解析一个子资源请求 URL。
     *
     * 典型场景是 glTF / 纹理 / bin 文件：
     * - 主资源：`scene.gltf`
     * - 子资源请求：`scene.bin`、`textures/a.png`、`../shared/b.jpg`
     * - 需要把这些相对路径转换成当前平台可加载的真实地址
     *
     * 处理顺序如下：
     * - 如果子资源本身已是 `http/https/data/blob/asset/tauri`，直接返回
     * - 如果子资源是站内绝对路径 `/static/...`，按浏览器可加载地址处理
     * - 如果子资源是相对路径，则基于主资源本地目录进行拼接
     *
     * @param baseUrl 主资源 URL 或本地绝对路径
     * @param requestUrl 子资源原始请求值
     * @returns 解析后的可加载地址；无法解析时返回原值
     */
    public static resolveSubResourceUrl(baseUrl?: string | null, requestUrl?: string | null) {
        const request = (requestUrl || '').trim();
        if (!request) return request;

        if (AssetUrlUtil.isNetworkLikeUrl(request) || request.startsWith('asset://') || request.startsWith('tauri://'))
            return request;

        if (request.startsWith('/')) {
            const absolute = AssetUrlUtil.toLoadableUrl(request);
            return absolute || request;
        }

        const localBasePath = AssetUrlUtil.toLocalPath(baseUrl);
        if (!localBasePath)
            return request;

        const lastSlashIndex = localBasePath.lastIndexOf('/');
        if (lastSlashIndex < 0)
            return request;

        const baseDir = localBasePath.slice(0, lastSlashIndex + 1);
        const resolvedPath = AssetUrlUtil.resolveRelativePath(baseDir, request);
        return AssetUrlUtil.toLoadableUrl(resolvedPath) || request;
    }
}
