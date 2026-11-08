/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("create_dir_all failed: {0}")]
    CreateDir(String),
    #[error("write failed: {0}")]
    Write(String),
    #[error("hard_link failed: {0}")]
    HardLink(String),
    #[error("copy failed: {0}")]
    Copy(String),
}

pub fn ensure_blob_and_link(
    blob_store_dir: &Path,
    blob_filename: &str,
    bytes: &[u8],
    target_abs: &Path,
) -> Result<(), FsError> {
    let blob_abs = blob_store_dir.join(blob_filename);

    if let Some(parent) = blob_abs.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| FsError::CreateDir(e.to_string()))?;
        }
    }
    if !blob_abs.exists() {
        std::fs::write(&blob_abs, bytes).map_err(|e| FsError::Write(e.to_string()))?;
    }

    if let Some(parent) = target_abs.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| FsError::CreateDir(e.to_string()))?;
        }
    }
    if target_abs.exists() {
        return Ok(());
    }
    if let Err(_e) = std::fs::hard_link(&blob_abs, target_abs) {
        std::fs::copy(&blob_abs, target_abs).map_err(|err| FsError::Copy(err.to_string()))?;
        return Ok(());
    }
    Ok(())
}

pub fn ensure_write_if_missing(target_abs: &Path, bytes: &[u8]) -> Result<(), FsError> {
    if let Some(parent) = target_abs.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| FsError::CreateDir(e.to_string()))?;
        }
    }
    if target_abs.exists() {
        return Ok(());
    }
    std::fs::write(target_abs, bytes).map_err(|e| FsError::Write(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn ensure_blob_and_link_writes_blob_and_target() {
        let dir = tempfile::tempdir().unwrap();
        let blob_store = dir.path().join("blobs");
        let target = dir.path().join("out").join("x.bin");
        ensure_blob_and_link(&blob_store, "x.bin", b"abc", &target).unwrap();

        assert_eq!(fs::read(blob_store.join("x.bin")).unwrap(), b"abc");
        assert_eq!(fs::read(&target).unwrap(), b"abc");
    }

    #[test]
    fn ensure_blob_and_link_target_exists_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let blob_store = dir.path().join("blobs");
        let target = dir.path().join("out").join("x.bin");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"old").unwrap();

        ensure_blob_and_link(&blob_store, "x.bin", b"new", &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert_eq!(fs::read(blob_store.join("x.bin")).unwrap(), b"new");
    }

    #[test]
    fn ensure_blob_and_link_create_dir_error() {
        let dir = tempfile::tempdir().unwrap();
        let blob_store = dir.path().join("blobs");
        fs::write(&blob_store, b"x").unwrap();
        let target = dir.path().join("out").join("x.bin");

        let err = ensure_blob_and_link(&blob_store, "x.bin", b"abc", &target).unwrap_err();
        match err {
            FsError::CreateDir(_) => {}
            _ => panic!("unexpected error: {err:?}"),
        }
    }

    #[test]
    fn ensure_write_if_missing_writes_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out").join("a.txt");
        ensure_write_if_missing(&target, b"hi").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"hi");
    }
}
