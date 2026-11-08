/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 模型配置、API Key 与附件安全边界。

mod attachment;
mod config;
mod types;

pub(crate) use attachment::{DbAttachmentStore, ModelAttachmentResolver};
pub(crate) use config::{DbModelConfigStore, ModelConfigResolver};
pub(crate) use types::{ModelAttachmentRole, ModelRuntimeCapabilities};
