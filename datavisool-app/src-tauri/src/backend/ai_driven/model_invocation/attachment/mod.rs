/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 模型调用附件解析模块。

mod resolver;
mod store;

pub(crate) use resolver::ModelAttachmentResolver;
pub(crate) use store::DbAttachmentStore;
#[allow(unused_imports)]
pub(crate) use store::{AttachmentBytesFuture, AttachmentMetadataFuture, AttachmentTouchFuture, StoredAttachmentMetadata};
