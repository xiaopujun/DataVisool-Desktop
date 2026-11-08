/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 模型附件数据源边界。

use std::future::Future;
use std::path::{Component, Path};
use std::pin::Pin;

use sqlx::Row;

use crate::app_state::AppState;
use crate::backend::ai_driven::model_invocation::types::ModelInvocationError;
use crate::db::DbState;

/// 附件元数据读取异步结果。
pub(crate) type AttachmentMetadataFuture<'a> = Pin<Box<dyn Future<Output = Result<StoredAttachmentMetadata, ModelInvocationError>> + Send + 'a>>;
/// 附件文件读取异步结果。
pub(crate) type AttachmentBytesFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<u8>, ModelInvocationError>> + Send + 'a>>;
/// 附件状态更新异步结果。
pub(crate) type AttachmentTouchFuture<'a> = Pin<Box<dyn Future<Output = Result<(), ModelInvocationError>> + Send + 'a>>;

/// 附件数据源。
pub(crate) trait AttachmentStore: Send + Sync {
    /// 读取图片附件元数据。
    fn load_image_attachment_metadata<'a>(
        &'a self,
        project_id: i64,
        thread_id: &'a str,
        turn_id: &'a str,
        attachment_id: &'a str,
    ) -> AttachmentMetadataFuture<'a>;

    /// 读取图片附件文件内容。
    fn read_attachment_bytes<'a>(&'a self, metadata: &'a StoredAttachmentMetadata) -> AttachmentBytesFuture<'a>;

    /// 标记附件被模型调用使用。
    fn touch_attachment<'a>(&'a self, attachment_id: &'a str, used_at: i64) -> AttachmentTouchFuture<'a>;
}

/// 已从存储层读取的附件元数据。
#[derive(Clone, Debug)]
pub(crate) struct StoredAttachmentMetadata {
    /// 图片 MIME 类型。
    pub mime_type: String,
    /// 存储记录中的文件 SHA-256。
    pub sha256: String,
    /// 文件相对存储路径。
    pub relative_path: String,
    /// 存储记录中的文件大小。
    pub byte_size: i64,
    /// 图片宽度。
    pub width: Option<i64>,
    /// 图片高度。
    pub height: Option<i64>,
    /// 附件状态。
    pub status: String,
    /// 过期时间戳。
    pub expire_at: i64,
}

/// 基于桌面端数据库和资源目录的附件数据源。
pub(crate) struct DbAttachmentStore<'a> {
    db: &'a DbState,
    app_state: &'a AppState,
}

impl<'a> DbAttachmentStore<'a> {
    /// 创建真实附件数据源。
    pub(crate) fn new(db: &'a DbState, app_state: &'a AppState) -> Self {
        Self { db, app_state }
    }
}

impl AttachmentStore for DbAttachmentStore<'_> {
    fn load_image_attachment_metadata<'a>(
        &'a self,
        project_id: i64,
        thread_id: &'a str,
        turn_id: &'a str,
        attachment_id: &'a str,
    ) -> AttachmentMetadataFuture<'a> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT attachment_id, mime_type, sha256, relative_path, byte_size, width, height, status, expire_at \
                 FROM ai_agent_attachment \
                 WHERE attachment_id = ?1 AND project_id = ?2 AND claimed_thread_id = ?3 \
                   AND claimed_turn_id = ?4 AND status = 'claimed' \
                 LIMIT 1",
            )
            .bind(attachment_id)
            .bind(project_id)
            .bind(thread_id)
            .bind(turn_id)
            .fetch_optional(&self.db.pool)
            .await
            .map_err(|e| ModelInvocationError::validation(format!("查询图片附件失败：{e}")))?
            .ok_or_else(|| ModelInvocationError::validation("图片附件不存在或不属于当前 Turn"))?;

            Ok(StoredAttachmentMetadata {
                mime_type: row
                    .try_get("mime_type")
                    .map_err(|e| ModelInvocationError::validation(format!("读取附件 MIME 失败：{e}")))?,
                sha256: row
                    .try_get("sha256")
                    .map_err(|e| ModelInvocationError::validation(format!("读取附件哈希失败：{e}")))?,
                relative_path: row
                    .try_get("relative_path")
                    .map_err(|e| ModelInvocationError::validation(format!("读取附件路径失败：{e}")))?,
                byte_size: row
                    .try_get("byte_size")
                    .map_err(|e| ModelInvocationError::validation(format!("读取附件大小失败：{e}")))?,
                width: row
                    .try_get("width")
                    .map_err(|e| ModelInvocationError::validation(format!("读取附件宽度失败：{e}")))?,
                height: row
                    .try_get("height")
                    .map_err(|e| ModelInvocationError::validation(format!("读取附件高度失败：{e}")))?,
                status: row
                    .try_get("status")
                    .map_err(|e| ModelInvocationError::validation(format!("读取附件状态失败：{e}")))?,
                expire_at: row
                    .try_get("expire_at")
                    .map_err(|e| ModelInvocationError::validation(format!("读取附件过期时间失败：{e}")))?,
            })
        })
    }

    fn read_attachment_bytes<'a>(&'a self, metadata: &'a StoredAttachmentMetadata) -> AttachmentBytesFuture<'a> {
        Box::pin(async move {
            let absolute_path = resolve_relative_attachment_path(&self.app_state.resource_storage_root(), &metadata.relative_path)?;
            std::fs::read(&absolute_path).map_err(|e| ModelInvocationError::validation(format!("读取图片附件文件失败：{e}")))
        })
    }

    fn touch_attachment<'a>(&'a self, attachment_id: &'a str, used_at: i64) -> AttachmentTouchFuture<'a> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE ai_agent_attachment \
                 SET last_used_at = ?2, updated_at = ?2 \
                 WHERE attachment_id = ?1 AND status = 'claimed'",
            )
            .bind(attachment_id)
            .bind(used_at)
            .execute(&self.db.pool)
            .await
            .map_err(|e| ModelInvocationError::validation(format!("更新附件使用时间失败：{e}")))?;
            Ok(())
        })
    }
}

/// 将数据库中的相对路径解析到资源存储根目录下。
fn resolve_relative_attachment_path(root: &Path, relative_path: &str) -> Result<std::path::PathBuf, ModelInvocationError> {
    let relative = Path::new(relative_path);
    if relative.is_absolute() || relative.components().any(|component| matches!(component, Component::ParentDir)) {
        return Err(ModelInvocationError::validation("图片附件路径不合法"));
    }
    let canonical_root = std::fs::canonicalize(root).map_err(|error| ModelInvocationError::validation(format!("附件存储根目录不可用：{error}")))?;
    let canonical_target = std::fs::canonicalize(canonical_root.join(relative))
        .map_err(|error| ModelInvocationError::validation(format!("图片附件路径不可用：{error}")))?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(ModelInvocationError::validation("图片附件路径越过存储根目录"));
    }
    Ok(canonical_target)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::resolve_relative_attachment_path;

    fn test_root(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("datavisool-attachment-path-{label}-{nonce}"))
    }

    #[test]
    fn resolves_only_existing_files_inside_canonical_root() {
        let root = test_root("inside");
        fs::create_dir_all(root.join("images")).unwrap();
        fs::write(root.join("images/reference.png"), b"image").unwrap();

        let resolved = resolve_relative_attachment_path(&root, "images/reference.png").unwrap();
        assert!(resolved.starts_with(fs::canonicalize(&root).unwrap()));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = test_root("root");
        let outside = test_root("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.png"), b"secret").unwrap();
        symlink(&outside, root.join("escape")).unwrap();

        let error = resolve_relative_attachment_path(&root, "escape/secret.png").unwrap_err();
        assert!(error.message.contains("越过存储根目录"));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
