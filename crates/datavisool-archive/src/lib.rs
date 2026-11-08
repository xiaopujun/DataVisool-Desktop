/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use datavisool_project_format::{ExportManifest, ManifestFile};
use sha2::Digest;
use std::collections::HashSet;
use std::io::{Cursor, Read, Seek, Write};
use zip::write::FileOptions;
use zip::CompressionMethod;
use zip::ZipArchive;
use zip::ZipWriter;

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("invalid zip: {0}")]
    InvalidZip(String),
    #[error("file not found: {0}")]
    NotFound(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("utf8 error: {0}")]
    Utf8(String),
    #[error("json error: {0}")]
    Json(String),
    #[error("verify error: {0}")]
    Verify(String),
    #[error("invalid path")]
    InvalidPath,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let hash = hasher.finalize();
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn normalize_zip_path_lenient(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string()
}

pub fn normalize_zip_path_strict(path: &str) -> Result<String, ArchiveError> {
    let p = normalize_zip_path_lenient(path);
    if p.is_empty() || p.contains("..") {
        return Err(ArchiveError::InvalidPath);
    }
    Ok(p)
}

pub fn normalize_rel_path_strict(path: &str) -> Result<String, ArchiveError> {
    normalize_zip_path_strict(path)
}

pub fn try_read_zip_text<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    path: &str,
) -> Result<Option<String>, ArchiveError> {
    let p = normalize_zip_path_lenient(path);
    match archive.by_name(&p) {
        Ok(mut f) => {
            let mut s = String::new();
            f.read_to_string(&mut s)
                .map_err(|e| ArchiveError::Io(e.to_string()))?;
            Ok(Some(s))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(ArchiveError::InvalidZip(e.to_string())),
    }
}

pub fn read_zip_text<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    path: &str,
) -> Result<String, ArchiveError> {
    let p = normalize_zip_path_lenient(path);
    let mut f = archive
        .by_name(&p)
        .map_err(|e| ArchiveError::NotFound(format!("{p} ({e})")))?;
    let mut s = String::new();
    f.read_to_string(&mut s)
        .map_err(|e| ArchiveError::Io(e.to_string()))?;
    Ok(s)
}

pub fn read_zip_bytes<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    path: &str,
) -> Result<Vec<u8>, ArchiveError> {
    let p = normalize_zip_path_lenient(path);
    let mut f = archive
        .by_name(&p)
        .map_err(|e| ArchiveError::NotFound(format!("{p} ({e})")))?;
    let mut out = Vec::new();
    f.read_to_end(&mut out)
        .map_err(|e| ArchiveError::Io(e.to_string()))?;
    Ok(out)
}

pub fn read_manifest_from_zip<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<ExportManifest, ArchiveError> {
    let manifest_json = read_zip_text(archive, "manifest.json")?;
    serde_json::from_str::<ExportManifest>(&manifest_json)
        .map_err(|e| ArchiveError::Json(e.to_string()))
}

pub fn verify_manifest_files<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    files: &[ManifestFile],
) -> Result<(), ArchiveError> {
    for f in files {
        let p = f.path.trim();
        if p.is_empty() {
            continue;
        }
        let bytes = read_zip_bytes(archive, p)?;
        if f.size != 0 && bytes.len() as u64 != f.size {
            return Err(ArchiveError::Verify(format!("size mismatch ({p})")));
        }
        if !f.sha256.trim().is_empty() {
            let actual = sha256_hex(&bytes);
            if actual != f.sha256.trim().to_ascii_lowercase() {
                return Err(ArchiveError::Verify(format!("sha256 mismatch ({p})")));
            }
        }
    }
    Ok(())
}

pub fn verify_manifest_references_exist<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    manifest: &ExportManifest,
) -> Result<(), ArchiveError> {
    fn verify_path_exists<R: Read + Seek>(
        archive: &mut ZipArchive<R>,
        path: &str,
    ) -> Result<(), ArchiveError> {
        let p = normalize_zip_path_lenient(path);
        match archive.by_name(&p) {
            Ok(_) => Ok(()),
            Err(zip::result::ZipError::FileNotFound) => {
                Err(ArchiveError::Verify(format!("missing ({p})")))
            }
            Err(e) => Err(ArchiveError::InvalidZip(e.to_string())),
        }
    }

    let data_path = if manifest.project.data_path.trim().is_empty() {
        "project/project.json"
    } else {
        manifest.project.data_path.trim()
    };
    verify_path_exists(archive, data_path)?;

    for a in manifest
        .assets
        .images
        .iter()
        .chain(manifest.assets.fonts.iter())
        .chain(manifest.assets.videos.iter())
        .chain(manifest.assets.components.iter())
    {
        let p = a.blob.trim();
        if !p.is_empty() {
            verify_path_exists(archive, p)?;
        }
    }

    for s in &manifest.assets.dynamic_scripts {
        let ty = s.ty.trim().to_ascii_uppercase();
        if ty == "REMOTE" {
            continue;
        }
        let p = s.blob.as_deref().unwrap_or_default().trim();
        if !p.is_empty() {
            verify_path_exists(archive, p)?;
        }
    }

    for m in &manifest.assets.models {
        for f in &m.files {
            let p = f.blob.trim();
            if !p.is_empty() {
                verify_path_exists(archive, p)?;
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct ZipEntryBytes<'a> {
    pub path: &'a str,
    pub bytes: &'a [u8],
}

pub fn build_export_zip<'a, I>(
    manifest: &ExportManifest,
    entries: I,
    warnings: &[String],
) -> Result<Vec<u8>, ArchiveError>
where
    I: IntoIterator<Item = ZipEntryBytes<'a>>,
{
    let cursor = Cursor::new(Vec::<u8>::new());
    let mut zip = ZipWriter::new(cursor);
    let options = FileOptions::<()>::default().compression_method(CompressionMethod::Deflated);

    let mut seen: HashSet<String> = HashSet::new();
    let mut files: Vec<ManifestFile> = Vec::new();
    for e in entries {
        let p = normalize_zip_path_strict(e.path)?;
        if seen.insert(p.clone()) {
            zip.start_file(&p, options)
                .map_err(|err| ArchiveError::InvalidZip(err.to_string()))?;
            zip.write_all(e.bytes)
                .map_err(|err| ArchiveError::Io(err.to_string()))?;
            files.push(ManifestFile {
                path: p.clone(),
                sha256: sha256_hex(e.bytes),
                size: e.bytes.len() as u64,
            });
        }
    }

    let warnings_path = if warnings.is_empty() {
        manifest.warnings_path.clone()
    } else {
        manifest
            .warnings_path
            .clone()
            .or(Some("err.log".to_string()))
    };

    if !warnings.is_empty() {
        let p = "err.log".to_string();
        zip.start_file(&p, options)
            .map_err(|err| ArchiveError::InvalidZip(err.to_string()))?;
        let mut buf: Vec<u8> = Vec::new();
        for w in warnings {
            buf.extend_from_slice(w.as_bytes());
            buf.push(b'\n');
        }
        zip.write_all(&buf)
            .map_err(|err| ArchiveError::Io(err.to_string()))?;
        files.push(ManifestFile {
            path: p,
            sha256: sha256_hex(&buf),
            size: buf.len() as u64,
        });
    }

    let mut out_manifest = ExportManifest {
        format: manifest.format.clone(),
        format_version: manifest.format_version,
        generator: manifest.generator.clone(),
        generated_at_ms: manifest.generated_at_ms,
        project: manifest.project.clone(),
        projects: manifest.projects.clone(),
        assets: manifest.assets.clone(),
        common_datasource: manifest.common_datasource.clone(),
        files,
        warnings_path: warnings_path.clone(),
    };

    zip.start_file("manifest.json", options)
        .map_err(|err| ArchiveError::InvalidZip(err.to_string()))?;
    out_manifest.files.sort_by(|a, b| a.path.cmp(&b.path));
    let json = serde_json::to_vec(&out_manifest).map_err(|e| ArchiveError::Json(e.to_string()))?;
    zip.write_all(&json)
        .map_err(|err| ArchiveError::Io(err.to_string()))?;

    let cursor = zip
        .finish()
        .map_err(|err| ArchiveError::InvalidZip(err.to_string()))?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use datavisool_project_format::{
        ExportManifest, ManifestAssets, ManifestCommonDataSources, ManifestFile, ManifestProject,
    };
    use std::io::Cursor;

    fn base_manifest() -> ExportManifest {
        ExportManifest {
            format: "light-chaser-project".to_string(),
            format_version: 2,
            generator: "test".to_string(),
            generated_at_ms: 0,
            project: ManifestProject {
                id: "1".to_string(),
                name: "p".to_string(),
                data_path: "project/project.json".to_string(),
            },
            projects: vec![],
            assets: ManifestAssets::default(),
            common_datasource: ManifestCommonDataSources::default(),
            files: vec![],
            warnings_path: None,
        }
    }

    #[test]
    fn normalize_zip_path_strict_rejects_invalid() {
        assert!(normalize_zip_path_strict("").is_err());
        assert!(normalize_zip_path_strict("../a").is_err());
        assert!(normalize_zip_path_strict("a/../b").is_err());
        assert_eq!(
            normalize_zip_path_strict("/a/b").unwrap(),
            "a/b".to_string()
        );
        assert_eq!(
            normalize_zip_path_strict("a\\b").unwrap(),
            "a/b".to_string()
        );
    }

    #[test]
    fn build_export_zip_writes_manifest_and_dedup_entries() {
        let manifest = base_manifest();
        let entries = vec![
            ZipEntryBytes {
                path: "blobs/a.txt",
                bytes: b"aaa",
            },
            ZipEntryBytes {
                path: "blobs/a.txt",
                bytes: b"bbb",
            },
            ZipEntryBytes {
                path: "blobs/b.txt",
                bytes: b"ccc",
            },
        ];

        let zip_bytes = build_export_zip(&manifest, entries, &[]).unwrap();
        let cursor = Cursor::new(zip_bytes);
        let mut archive = zip::ZipArchive::new(cursor).unwrap();

        let manifest2 = read_manifest_from_zip(&mut archive).unwrap();
        assert_eq!(manifest2.format, "light-chaser-project");
        assert_eq!(manifest2.files.len(), 2);
        assert_eq!(manifest2.files[0].path, "blobs/a.txt");
        assert_eq!(manifest2.files[1].path, "blobs/b.txt");

        let a_bytes = read_zip_bytes(&mut archive, "blobs/a.txt").unwrap();
        assert_eq!(a_bytes, b"aaa");
        assert_eq!(manifest2.files[0].sha256, sha256_hex(&a_bytes));
        assert_eq!(manifest2.files[0].size, 3);
    }

    #[test]
    fn verify_manifest_files_detects_mismatch() {
        let manifest = base_manifest();
        let entries = vec![ZipEntryBytes {
            path: "blobs/x.bin",
            bytes: b"xyz",
        }];
        let zip_bytes = build_export_zip(&manifest, entries, &[]).unwrap();

        let cursor = Cursor::new(zip_bytes);
        let mut archive = zip::ZipArchive::new(cursor).unwrap();
        let bytes = read_zip_bytes(&mut archive, "blobs/x.bin").unwrap();

        let good = vec![ManifestFile {
            path: "blobs/x.bin".to_string(),
            sha256: sha256_hex(&bytes),
            size: bytes.len() as u64,
        }];
        verify_manifest_files(&mut archive, &good).unwrap();

        let bad = vec![ManifestFile {
            path: "blobs/x.bin".to_string(),
            sha256: "00".to_string(),
            size: bytes.len() as u64,
        }];
        let err = verify_manifest_files(&mut archive, &bad).unwrap_err();
        match err {
            ArchiveError::Verify(s) => assert!(s.contains("sha256 mismatch")),
            _ => panic!("unexpected error: {err:?}"),
        }
    }

    #[test]
    fn verify_manifest_references_exist_detects_missing() {
        let mut manifest = base_manifest();
        manifest.project.data_path = "".to_string();
        manifest
            .assets
            .images
            .push(datavisool_project_format::AssetBlobRef {
                name: "n".to_string(),
                original: "o".to_string(),
                blob: "blobs/missing.bin".to_string(),
            });

        let zip_bytes =
            build_export_zip(&manifest, std::iter::empty::<ZipEntryBytes>(), &[]).unwrap();
        let cursor = Cursor::new(zip_bytes);
        let mut archive = zip::ZipArchive::new(cursor).unwrap();

        let err = verify_manifest_references_exist(&mut archive, &manifest).unwrap_err();
        match err {
            ArchiveError::Verify(s) => assert!(s.contains("missing (")),
            _ => panic!("unexpected error: {err:?}"),
        }
    }
}
