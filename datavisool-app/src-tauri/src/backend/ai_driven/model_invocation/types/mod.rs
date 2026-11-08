/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 模型调用层的统一输入、输出和错误类型。

mod attachment;
mod error;
mod runtime;

pub(crate) use attachment::{ModelAttachmentRole, ModelResolvedAttachment};
pub(crate) use error::ModelInvocationError;
pub(crate) use runtime::{ModelRuntimeCapabilities, ModelRuntimeConfig};
