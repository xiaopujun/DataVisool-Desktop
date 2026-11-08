/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 以预先打开的 Workspace 根目录为能力边界的安全文件访问。

use std::io::{Read, Write};
use std::path::Path;

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, OpenOptionsMaybeDirExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, File, OpenOptions};

use crate::backend::ai_driven::ai_task_execution::workspace::{WorkspaceError, WorkspaceRelativePath, WorkspaceResult, WORKSPACE_TEMP_MARKER};

pub(super) struct WorkspaceRoot {
    root: Dir,
}

impl WorkspaceRoot {
    pub(super) fn open(path: &Path) -> WorkspaceResult<Self> {
        let metadata = std::fs::symlink_metadata(path).map_err(|error| storage("inspectWorkspaceCapabilityRoot", path, error))?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(WorkspaceError::PathOutOfScope(path.display().to_string()));
        }
        let root = Dir::open_ambient_dir(path, ambient_authority()).map_err(|error| storage("openWorkspaceCapabilityRoot", path, error))?;
        Ok(Self { root })
    }

    pub(super) fn read_file(&self, path: &WorkspaceRelativePath, max_bytes: u64) -> WorkspaceResult<Vec<u8>> {
        let (parent, name) = self.open_parent(path, false)?;
        let file = open_file_nofollow(&parent, name, false, false)?;
        ensure_regular_file(&file, path)?;
        let size = file
            .metadata()
            .map_err(|error| storage("inspectWorkspaceFileHandle", Path::new(path.as_str()), error))?
            .len();
        if size > max_bytes {
            return Err(WorkspaceError::ResourceLimit(format!("file exceeds {max_bytes} bytes: {path}")));
        }
        let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        file.take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| storage("readWorkspaceFile", Path::new(path.as_str()), error))?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
            return Err(WorkspaceError::ResourceLimit(format!("file exceeds {max_bytes} bytes: {path}")));
        }
        Ok(bytes)
    }

    pub(super) fn try_read_file(&self, path: &WorkspaceRelativePath, max_bytes: u64) -> WorkspaceResult<Option<Vec<u8>>> {
        let Some((parent, name)) = self.try_open_parent(path, false)? else {
            return Ok(None);
        };
        match parent.symlink_metadata(name) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(WorkspaceError::PathOutOfScope(path.to_string())),
            Ok(_) => self.read_file(path, max_bytes).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(storage("inspectWorkspaceFile", Path::new(path.as_str()), error)),
        }
    }

    pub(super) fn write_file_atomically(&self, path: &WorkspaceRelativePath, bytes: &[u8], sequence: u64) -> WorkspaceResult<()> {
        let (parent, name) = self.open_parent(path, true)?;
        reject_symlink_if_exists(&parent, name, path)?;
        let temp_name = format!(".{name}{WORKSPACE_TEMP_MARKER}{}-{sequence}", std::process::id());
        let mut temp = open_file_nofollow(&parent, &temp_name, true, true)?;
        temp.write_all(bytes)
            .and_then(|_| temp.sync_all())
            .map_err(|error| storage("writeWorkspaceTemporaryFile", Path::new(path.as_str()), error))?;
        drop(temp);
        if let Err(error) = parent.rename(&temp_name, &parent, name) {
            let _ = parent.remove_file(&temp_name);
            return Err(storage("replaceWorkspaceFile", Path::new(path.as_str()), error));
        }
        sync_directory(&parent, path)
    }

    pub(super) fn remove_file(&self, path: &WorkspaceRelativePath) -> WorkspaceResult<()> {
        let (parent, name) = self.open_parent(path, false)?;
        reject_symlink_if_exists(&parent, name, path)?;
        let file = open_file_nofollow(&parent, name, false, false)?;
        ensure_regular_file(&file, path)?;
        drop(file);
        parent
            .remove_file(name)
            .map_err(|error| storage("removeWorkspaceFile", Path::new(path.as_str()), error))?;
        sync_directory(&parent, path)
    }

    pub(super) fn cleanup_temporary_files(&self, path: &WorkspaceRelativePath) -> WorkspaceResult<()> {
        let Some((parent, name)) = self.try_open_parent(path, false)? else {
            return Ok(());
        };
        let prefix = format!(".{name}{WORKSPACE_TEMP_MARKER}");
        let entries = parent
            .entries()
            .map_err(|error| storage("listWorkspaceTemporaryFiles", Path::new(path.as_str()), error))?;
        let mut removed = false;
        for entry in entries {
            let entry = entry.map_err(|error| storage("readWorkspaceTemporaryFile", Path::new(path.as_str()), error))?;
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else { continue };
            if !file_name.starts_with(&prefix) {
                continue;
            }
            let metadata = parent
                .symlink_metadata(file_name)
                .map_err(|error| storage("inspectWorkspaceTemporaryFile", Path::new(path.as_str()), error))?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(WorkspaceError::PathOutOfScope(path.to_string()));
            }
            parent
                .remove_file(file_name)
                .map_err(|error| storage("removeWorkspaceTemporaryFile", Path::new(path.as_str()), error))?;
            removed = true;
        }
        if removed {
            sync_directory(&parent, path)?;
        }
        Ok(())
    }

    fn open_parent<'a>(&self, path: &'a WorkspaceRelativePath, create: bool) -> WorkspaceResult<(Dir, &'a str)> {
        self.try_open_parent(path, create)?.ok_or_else(|| {
            storage(
                "openWorkspacePathParent",
                Path::new(path.as_str()),
                std::io::Error::new(std::io::ErrorKind::NotFound, "workspace path parent does not exist"),
            )
        })
    }

    fn try_open_parent<'a>(&self, path: &'a WorkspaceRelativePath, create: bool) -> WorkspaceResult<Option<(Dir, &'a str)>> {
        let mut components = path.as_str().split('/').collect::<Vec<_>>();
        let name = components.pop().expect("validated Workspace path has a final component");
        let mut current = self
            .root
            .try_clone()
            .map_err(|error| storage("cloneWorkspaceCapabilityRoot", Path::new(path.as_str()), error))?;
        for component in components {
            match current.symlink_metadata(component) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(WorkspaceError::PathOutOfScope(path.to_string()));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
                    current
                        .create_dir(component)
                        .map_err(|error| storage("createWorkspaceDirectory", Path::new(path.as_str()), error))?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(storage("inspectWorkspacePathParent", Path::new(path.as_str()), error)),
            }
            let directory_file = open_directory_nofollow(&current, component, path)?;
            current = Dir::from_std_file(directory_file.into_std());
        }
        Ok(Some((current, name)))
    }
}

fn open_directory_nofollow(parent: &Dir, name: &str, logical_path: &WorkspaceRelativePath) -> WorkspaceResult<File> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).maybe_dir(true);
    let file = parent
        .open_with(name, &options)
        .map_err(|error| storage("openWorkspaceDirectory", Path::new(logical_path.as_str()), error))?;
    let metadata = file
        .metadata()
        .map_err(|error| storage("inspectWorkspaceDirectoryHandle", Path::new(logical_path.as_str()), error))?;
    if !metadata.is_dir() {
        return Err(WorkspaceError::PathOutOfScope(logical_path.to_string()));
    }
    Ok(file)
}

fn open_file_nofollow(parent: &Dir, name: &str, create_new: bool, write: bool) -> WorkspaceResult<File> {
    let mut options = OpenOptions::new();
    options
        .read(!write)
        .write(write)
        .create_new(create_new)
        .follow(FollowSymlinks::No);
    parent
        .open_with(name, &options)
        .map_err(|error| storage("openWorkspaceFile", Path::new(name), error))
}

fn reject_symlink_if_exists(parent: &Dir, name: &str, logical_path: &WorkspaceRelativePath) -> WorkspaceResult<()> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(WorkspaceError::PathOutOfScope(logical_path.to_string())),
        Ok(metadata) if !metadata.is_file() => Err(WorkspaceError::PathOutOfScope(logical_path.to_string())),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(storage("inspectWorkspaceFile", Path::new(logical_path.as_str()), error)),
    }
}

fn ensure_regular_file(file: &File, path: &WorkspaceRelativePath) -> WorkspaceResult<()> {
    let metadata = file
        .metadata()
        .map_err(|error| storage("inspectWorkspaceFileHandle", Path::new(path.as_str()), error))?;
    if !metadata.is_file() {
        return Err(WorkspaceError::PathOutOfScope(path.to_string()));
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn sync_directory(directory: &Dir, path: &WorkspaceRelativePath) -> WorkspaceResult<()> {
    directory
        .try_clone()
        .map(Dir::into_std_file)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| storage("syncWorkspaceDirectory", Path::new(path.as_str()), error))
}

#[cfg(target_os = "windows")]
fn sync_directory(_directory: &Dir, _path: &WorkspaceRelativePath) -> WorkspaceResult<()> {
    Ok(())
}

fn storage(operation: &'static str, path: &Path, error: std::io::Error) -> WorkspaceError {
    WorkspaceError::Storage { operation, path: path.to_path_buf(), message: error.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("datavisool-workspace-root-{}-{}", std::process::id(), TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)))
    }

    #[test]
    fn reads_writes_and_removes_only_relative_regular_files() {
        let root = root();
        fs::create_dir_all(&root).unwrap();
        let capability = WorkspaceRoot::open(&root).unwrap();
        let path = WorkspaceRelativePath::new("docs/overview.md").unwrap();
        assert_eq!(capability.try_read_file(&path, 100).unwrap(), None);
        capability.write_file_atomically(&path, b"overview", 1).unwrap();
        assert_eq!(capability.read_file(&path, 100).unwrap(), b"overview");
        capability.remove_file(&path).unwrap();
        assert!(!root.join("docs/overview.md").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn atomic_write_uses_new_temporary_marker() {
        let root = root();
        fs::create_dir_all(root.join("docs")).unwrap();
        let temporary = root.join(format!("docs/.overview.md.datavisool-tmp-{}-42", std::process::id()));
        fs::write(&temporary, b"existing").unwrap();
        let capability = WorkspaceRoot::open(&root).unwrap();
        let path = WorkspaceRelativePath::new("docs/overview.md").unwrap();
        assert!(capability.write_file_atomically(&path, b"new", 42).is_err());
        assert_eq!(fs::read(&temporary).unwrap(), b"existing");
        assert!(!root.join("docs/overview.md").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_cleans_current_temporary_files_for_target_only() {
        let root = root();
        fs::create_dir_all(root.join("docs")).unwrap();
        let current = root.join("docs/.overview.md.datavisool-tmp-1-1");
        let legacy = root.join("docs/.overview.md.lcp-tmp-1-1");
        let unrelated = root.join("docs/.other.md.lcp-tmp-1-1");
        for path in [&current, &legacy, &unrelated] {
            fs::write(path, b"temporary").unwrap();
        }
        let capability = WorkspaceRoot::open(&root).unwrap();
        capability.cleanup_temporary_files(&WorkspaceRelativePath::new("docs/overview.md").unwrap()).unwrap();
        assert!(!current.exists());
        assert!(legacy.exists());
        assert!(unrelated.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_link_file_and_parent() {
        use std::os::unix::fs::symlink;

        let root = root();
        let outside = root.with_extension("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret"), "secret").unwrap();
        symlink(outside.join("secret"), root.join("file-link")).unwrap();
        symlink(&outside, root.join("dir-link")).unwrap();
        let capability = WorkspaceRoot::open(&root).unwrap();
        assert!(capability
            .read_file(&WorkspaceRelativePath::new("file-link").unwrap(), 100)
            .is_err());
        assert!(capability
            .read_file(&WorkspaceRelativePath::new("dir-link/secret").unwrap(), 100)
            .is_err());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }
}
