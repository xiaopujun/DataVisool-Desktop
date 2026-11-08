/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI驱动后端能力模块。

pub(crate) mod agent;
pub(crate) mod agent_context;
pub(crate) mod ai_task_execution;
pub(crate) mod attachment_cleanup;
pub(crate) mod component_docs;
pub(crate) mod domain;
pub(crate) mod memory;
pub(crate) mod model;
pub(crate) mod model_invocation;
pub(crate) mod provider;
pub(crate) mod provider_model_catalog;
pub(crate) mod ripgrep;
pub(crate) mod security;
pub(crate) mod settings;
pub(crate) mod task;
pub(crate) mod thread;
pub(crate) mod time;
pub(crate) mod tools;

pub(crate) use attachment_cleanup::cleanup_expired_agent_attachments;
