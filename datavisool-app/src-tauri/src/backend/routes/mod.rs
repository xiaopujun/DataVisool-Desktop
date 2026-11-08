/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! backend 路由的"业务处理层"（handlers）。
//!
//! 拆分原则：
//! - 一个文件对应一类 API 资源/职责（例如 project、systemAuth）。
//! - 每个 handler 接收"已解析"的参数（body / id），不再关心 url/method 分发。
//! - router 只做分发与参数抽取；DB 查询/写入细节留在 handler 内。

pub(crate) mod ai_driven_attachment;
mod ai_driven_history_common;
pub(crate) mod ai_driven_item;
pub(crate) mod ai_driven_model;
pub(crate) mod ai_driven_settings;
pub(crate) mod ai_driven_task;
pub(crate) mod ai_driven_thread;
pub(crate) mod ai_driven_turn;
pub(crate) mod ai_model;
pub(crate) mod app_config;
pub(crate) mod common_api;
pub(crate) mod common_database;
pub(crate) mod dynamic_script;
pub(crate) mod export;
pub(crate) mod font;
pub(crate) mod image;
pub(crate) mod import;
pub(crate) mod project;
pub(crate) mod release;
pub(crate) mod remote_component;
pub(crate) mod security;
pub(crate) mod system_auth;
pub(crate) mod template;
pub(crate) mod three_d_model;
pub(crate) mod video;
