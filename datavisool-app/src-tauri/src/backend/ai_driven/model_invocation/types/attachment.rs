/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 模型附件输入和解析结果类型。

use serde::{Deserialize, Serialize};

/// 图片附件用途。
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ModelAttachmentRole {
    /// 用户输入区上传的提示图片。
    PromptImage,
    /// 画布视觉截图。
    CanvasSnapshot,
}

/// 已读取并校验过的图片附件。
#[derive(Clone, Debug)]
pub(crate) struct ModelResolvedAttachment {
    /// 附件主键。
    pub attachment_id: String,
    /// 图片 MIME 类型。
    pub mime_type: String,
    /// 图片原始字节。
    pub bytes: Vec<u8>,
    /// 图片字节大小。
    pub byte_size: u64,
    /// 图片宽度。
    pub width: Option<u32>,
    /// 图片高度。
    pub height: Option<u32>,
    /// 图片 SHA-256 哈希。
    pub sha256: String,
}
