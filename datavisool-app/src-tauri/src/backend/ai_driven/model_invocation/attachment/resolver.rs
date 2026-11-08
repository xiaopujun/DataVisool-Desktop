/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 图片附件解析器，负责把作用域内的附件引用转换为 Rig 可用字节。

use crate::backend::ai_driven::model_invocation::types::{ModelInvocationError, ModelResolvedAttachment};
use crate::backend::ai_driven::time::now_ms;
use crate::backend::utils::sha256_hex;

use super::store::{AttachmentStore, StoredAttachmentMetadata};

pub(crate) struct ModelAttachmentResolver;

impl ModelAttachmentResolver {
    pub(crate) fn new() -> Self {
        Self
    }

    /// 读取已由当前 Turn 认领的图片附件，并校验状态、大小和哈希。
    pub(crate) async fn resolve_image_attachment(
        &self,
        store: &dyn AttachmentStore,
        project_id: &str,
        thread_id: &str,
        turn_id: &str,
        attachment_id: &str,
    ) -> Result<ModelResolvedAttachment, ModelInvocationError> {
        let project_id = parse_i64_id(project_id, "project_id")?;
        let thread_id = normalize_required(thread_id, "thread_id")?;
        let turn_id = normalize_required(turn_id, "turn_id")?;
        let attachment_id = normalize_required(attachment_id, "attachment_id")?;
        let metadata = store
            .load_image_attachment_metadata(project_id, &thread_id, &turn_id, &attachment_id)
            .await?;

        validate_attachment_metadata(&metadata)?;
        let bytes = store.read_attachment_bytes(&metadata).await?;
        if bytes.len() as i64 != metadata.byte_size {
            return Err(ModelInvocationError::validation("图片附件文件大小与记录不一致"));
        }
        let sha256 = sha256_hex(&bytes);
        if sha256 != metadata.sha256 {
            return Err(ModelInvocationError::validation("图片附件文件哈希与记录不一致"));
        }

        store.touch_attachment(&attachment_id, now_ms()).await?;
        Ok(ModelResolvedAttachment {
            attachment_id,
            mime_type: metadata.mime_type,
            byte_size: bytes.len() as u64,
            bytes,
            width: metadata.width.and_then(i64_to_u32),
            height: metadata.height.and_then(i64_to_u32),
            sha256,
        })
    }
}

fn parse_i64_id(value: &str, field: &str) -> Result<i64, ModelInvocationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ModelInvocationError::validation(format!("{field} 不能为空")));
    }
    value
        .parse::<i64>()
        .map_err(|error| ModelInvocationError::validation(format!("{field} 不合法：{error}")))
}

fn normalize_required(value: &str, field: &str) -> Result<String, ModelInvocationError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        Err(ModelInvocationError::validation(format!("{field} 不能为空")))
    } else {
        Ok(value)
    }
}

fn validate_attachment_metadata(metadata: &StoredAttachmentMetadata) -> Result<(), ModelInvocationError> {
    if metadata.status != "claimed" {
        return Err(ModelInvocationError::validation("图片附件状态不可用"));
    }
    if !metadata.mime_type.trim().to_ascii_lowercase().starts_with("image/") {
        return Err(ModelInvocationError::validation("附件不是图片类型"));
    }
    Ok(())
}

fn i64_to_u32(value: i64) -> Option<u32> {
    u32::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::backend::ai_driven::model_invocation::attachment::{AttachmentBytesFuture, AttachmentMetadataFuture, AttachmentTouchFuture};

    struct FakeAttachmentStore {
        metadata: StoredAttachmentMetadata,
        bytes: Vec<u8>,
        touched: Mutex<Vec<String>>,
    }

    impl AttachmentStore for FakeAttachmentStore {
        fn load_image_attachment_metadata<'a>(
            &'a self,
            project_id: i64,
            thread_id: &'a str,
            turn_id: &'a str,
            attachment_id: &'a str,
        ) -> AttachmentMetadataFuture<'a> {
            Box::pin(async move {
                assert_eq!(project_id, 12);
                assert_eq!(thread_id, "thread-1");
                assert_eq!(turn_id, "turn-1");
                assert_eq!(attachment_id, "attachment-1");
                Ok(self.metadata.clone())
            })
        }

        fn read_attachment_bytes<'a>(&'a self, metadata: &'a StoredAttachmentMetadata) -> AttachmentBytesFuture<'a> {
            Box::pin(async move {
                assert_eq!(metadata.relative_path, "static/test.png");
                Ok(self.bytes.clone())
            })
        }

        fn touch_attachment<'a>(&'a self, attachment_id: &'a str, used_at: i64) -> AttachmentTouchFuture<'a> {
            Box::pin(async move {
                assert!(used_at > 0);
                self.touched.lock().expect("touch lock").push(attachment_id.to_string());
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn resolves_scoped_image_and_verifies_integrity() {
        let bytes = b"image-bytes".to_vec();
        let store = FakeAttachmentStore {
            metadata: StoredAttachmentMetadata {
                mime_type: "image/png".to_string(),
                sha256: sha256_hex(&bytes),
                relative_path: "static/test.png".to_string(),
                byte_size: bytes.len() as i64,
                width: Some(640),
                height: Some(480),
                status: "claimed".to_string(),
                expire_at: now_ms() + 60_000,
            },
            bytes,
            touched: Mutex::new(Vec::new()),
        };

        let resolved = ModelAttachmentResolver::new()
            .resolve_image_attachment(&store, "12", "thread-1", "turn-1", "attachment-1")
            .await
            .expect("attachment should resolve");

        assert_eq!(resolved.mime_type, "image/png");
        assert_eq!(resolved.width, Some(640));
        assert_eq!(resolved.height, Some(480));
        assert_eq!(*store.touched.lock().expect("touch lock"), vec!["attachment-1".to_string()]);
    }
}
