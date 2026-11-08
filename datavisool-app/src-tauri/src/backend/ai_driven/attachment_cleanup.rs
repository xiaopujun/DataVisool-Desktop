/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 未认领 Agent 临时附件的安全过期清理。

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::SqlitePool;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AttachmentCleanupOutcome {
    pub deleted: u64,
    pub failed: u64,
}

/// 将到期的 temporary 附件标为 expired，并只删除受控附件目录中的对应文件。
///
/// 数据库状态先进入 expired；文件删除失败时保留该状态，下一次清理会继续重试。
pub(crate) async fn cleanup_expired_agent_attachments(pool: &SqlitePool, resource_root: &Path) -> Result<AttachmentCleanupOutcome, String> {
    let now = current_time_ms()?;
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT attachment_id, status, relative_path FROM ai_agent_attachment \
         WHERE (status = 'temporary' AND expire_at <= ?1) OR status = 'expired' \
         ORDER BY created_at ASC",
    )
    .bind(now)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("query expired Agent attachments failed: {error}"))?;

    let mut outcome = AttachmentCleanupOutcome::default();
    for (attachment_id, status, relative_path) in rows {
        if status == "temporary" {
            let updated = sqlx::query(
                "UPDATE ai_agent_attachment SET status = 'expired', updated_at = ?1 \
                 WHERE attachment_id = ?2 AND status = 'temporary' AND expire_at <= ?1",
            )
            .bind(now)
            .bind(&attachment_id)
            .execute(pool)
            .await
            .map_err(|error| format!("mark Agent attachment expired failed: {error}"))?;
            if updated.rows_affected() == 0 {
                continue;
            }
        }

        let target = match resolve_attachment_target(resource_root, &relative_path) {
            Ok(target) => target,
            Err(error) => {
                outcome.failed += 1;
                tracing::warn!(attachment_id, relative_path, %error, "拒绝清理不安全的 Agent 附件路径");
                continue;
            }
        };
        match fs::remove_file(&target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                outcome.failed += 1;
                tracing::warn!(attachment_id, path = %target.display(), %error, "Agent 临时附件文件清理失败");
                continue;
            }
        }
        let updated = sqlx::query(
            "UPDATE ai_agent_attachment SET status = 'deleted', updated_at = ?1 \
             WHERE attachment_id = ?2 AND status = 'expired'",
        )
        .bind(now)
        .bind(&attachment_id)
        .execute(pool)
        .await
        .map_err(|error| format!("mark Agent attachment deleted failed: {error}"))?;
        if updated.rows_affected() == 1 {
            outcome.deleted += 1;
        }
    }
    Ok(outcome)
}

fn resolve_attachment_target(resource_root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("relative path contains an unsafe component".to_string());
    }
    let components = relative.components().collect::<Vec<_>>();
    if components.len() < 3 || components[0].as_os_str() != "ai-driven" || components[1].as_os_str() != "attachments" {
        return Err("relative path is outside the Agent attachment directory".to_string());
    }

    let target = resource_root.join(relative);
    let root_metadata = match fs::symlink_metadata(resource_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(target),
        Err(error) => return Err(format!("inspect resource root failed: {error}")),
    };
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err("resource root must be a real directory".to_string());
    }

    let mut current = resource_root.to_path_buf();
    for component in components {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!("attachment path contains a symbolic link: {}", current.display()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(target),
            Err(error) => return Err(format!("inspect attachment path failed: {error}")),
        }
    }

    let canonical_root = fs::canonicalize(resource_root).map_err(|error| format!("canonicalize resource root failed: {error}"))?;
    let canonical_target = fs::canonicalize(&target).map_err(|error| format!("canonicalize attachment failed: {error}"))?;
    if !canonical_target.starts_with(canonical_root) {
        return Err("attachment path escapes the resource root".to_string());
    }
    Ok(canonical_target)
}

fn current_time_ms() -> Result<i64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(elapsed.as_millis()).map_err(|_| "current timestamp exceeds SQLite INTEGER".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn attachment_cleanup_path_is_restricted_to_agent_attachments() {
        let root = Path::new("/tmp/datavisool-resource-root-that-does-not-exist");
        assert!(resolve_attachment_target(root, "ai-driven/attachments/1/session/file.png").is_ok());
        assert!(resolve_attachment_target(root, "static/project/file.png").is_err());
        assert!(resolve_attachment_target(root, "ai-driven/attachments/../secret.png").is_err());
    }

    #[tokio::test]
    async fn deletes_only_expired_unclaimed_files_in_the_whitelist() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE ai_agent_attachment (
                attachment_id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                expire_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );",
        )
        .execute(&pool)
        .await
        .unwrap();

        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("datavisool-expired-attachment-{}-{nonce}", std::process::id()));
        let expired = root.join("ai-driven/attachments/1/session/expired.png");
        let claimed = root.join("ai-driven/attachments/1/session/claimed.png");
        let business = root.join("static/project/keep.png");
        for path in [&expired, &claimed, &business] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"image").unwrap();
        }
        let now = current_time_ms().unwrap();
        for (id, status, path) in [
            ("expired", "temporary", "ai-driven/attachments/1/session/expired.png"),
            ("claimed", "claimed", "ai-driven/attachments/1/session/claimed.png"),
            ("unsafe", "temporary", "static/project/keep.png"),
        ] {
            sqlx::query(
                "INSERT INTO ai_agent_attachment (attachment_id, status, relative_path, expire_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, 1, 1)",
            )
            .bind(id)
            .bind(status)
            .bind(path)
            .bind(now - 1)
            .execute(&pool)
            .await
            .unwrap();
        }

        let outcome = cleanup_expired_agent_attachments(&pool, &root).await.unwrap();
        assert_eq!(outcome, AttachmentCleanupOutcome { deleted: 1, failed: 1 });
        assert!(!expired.exists());
        assert!(claimed.exists());
        assert!(business.exists());
        let statuses: Vec<(String, String)> = sqlx::query_as("SELECT attachment_id, status FROM ai_agent_attachment ORDER BY attachment_id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(
            statuses,
            vec![
                ("claimed".to_string(), "claimed".to_string()),
                ("expired".to_string(), "deleted".to_string()),
                ("unsafe".to_string(), "expired".to_string()),
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn counts_deleted_only_when_this_cleanup_wins_the_status_update() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE ai_agent_attachment (
                attachment_id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                expire_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE cleanup_race_guard (won INTEGER NOT NULL);
            INSERT INTO cleanup_race_guard (won) VALUES (0);
            CREATE TRIGGER simulate_other_cleanup_winning
            BEFORE UPDATE OF status ON ai_agent_attachment
            WHEN OLD.attachment_id = 'race' AND OLD.status = 'expired' AND NEW.status = 'deleted'
                 AND (SELECT won FROM cleanup_race_guard) = 0
            BEGIN
                UPDATE cleanup_race_guard SET won = 1;
                UPDATE ai_agent_attachment SET status = 'deleted' WHERE attachment_id = OLD.attachment_id;
                SELECT RAISE(IGNORE);
            END;",
        )
        .execute(&pool)
        .await
        .unwrap();

        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("datavisool-attachment-cleanup-race-{}-{nonce}", std::process::id()));
        let file = root.join("ai-driven/attachments/1/session/race.png");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"image").unwrap();
        sqlx::query(
            "INSERT INTO ai_agent_attachment (attachment_id, status, relative_path, expire_at, created_at, updated_at) \
             VALUES ('race', 'expired', 'ai-driven/attachments/1/session/race.png', 1, 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = cleanup_expired_agent_attachments(&pool, &root).await.unwrap();
        assert_eq!(outcome, AttachmentCleanupOutcome { deleted: 0, failed: 0 });
        assert!(!file.exists());
        let status: String = sqlx::query_scalar("SELECT status FROM ai_agent_attachment WHERE attachment_id = 'race'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "deleted");
        fs::remove_dir_all(root).unwrap();
    }
}
