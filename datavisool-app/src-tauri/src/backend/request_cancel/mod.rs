/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 本地 HTTP 请求取消能力门面。
//!
//! 模块内部负责维护 requestId 到取消广播通道的映射；外部只依赖
//! `HttpRequestCancelRegistry`，避免感知具体存储结构。

mod registry;

pub(crate) use registry::HttpRequestCancelRegistry;
