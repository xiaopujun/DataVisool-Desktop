/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
/// 导出字体资源。
async fn export_fonts(
    db: &DbState,
    app_state: &AppState,
    font_names: Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<(Vec<AssetBlobRef>, Vec<ZipEntry>), String> {
    if font_names.is_empty() {
        return Ok((vec![], vec![]));
    }

    let mut query = String::from("SELECT name, url FROM font WHERE deleted = 0 AND name IN (");
    for i in 0..font_names.len() {
        if i > 0 {
            query.push(',');
        }
        query.push('?');
        query.push_str(&(i + 1).to_string());
    }
    query.push(')');

    let mut q = sqlx::query(&query);
    for n in &font_names {
        q = q.bind(n);
    }

    let rows = q
        .fetch_all(&db.pool)
        .await
        .map_err(|e| format!("export.exportFonts: query failed: {e}"))?;

    let mut items: Vec<AssetBlobRef> = Vec::new();
    let mut entries: Vec<ZipEntry> = Vec::new();

    for row in rows {
        let name: String = row.try_get("name").map_err(|e| e.to_string())?;
        let url: String = row.try_get("url").map_err(|e| e.to_string())?;

        // Data URL 没有稳定文件名，需要按内容 hash 生成 blob 名称。
        if let Some((mime, bytes)) = parse_data_url(&url) {
            let ext = mime_to_ext(&mime, "woff2");
            let sha = sha256_hex(&bytes);
            let blob_path = format!("blobs/{}.{}", sha, ext);
            items.push(AssetBlobRef { name: name.clone(), original: name.clone(), blob: blob_path.clone() });
            entries.push(ZipEntry { path: blob_path, bytes });
            continue;
        }

        let file_name = file_name_from_url(&url);
        if file_name.trim().is_empty() || file_name.starts_with("http://") || file_name.starts_with("https://") {
            warnings.push(format!("【字体缺失】{}", name));
            continue;
        }

        let p1 = app_state.static_storage_dir().join("font").join(&file_name);
        let p2 = app_state.files_dir.join("static").join("font").join(&file_name);
        let bytes = match std::fs::read(if p1.exists() { &p1 } else { &p2 }) {
            Ok(b) => b,
            Err(_) => {
                warnings.push(format!("【字体缺失】{}", file_name));
                continue;
            }
        };

        // 普通字体文件沿用现有文件名，通常已经是内容 hash。
        let blob_path = format!("blobs/{}", file_name);
        items.push(AssetBlobRef { name: name.clone(), original: name.clone(), blob: blob_path.clone() });
        entries.push(ZipEntry { path: blob_path, bytes });
    }

    Ok((items, entries))
}

/// 导出远程组件资源。
///
/// 单文件组件直接复制，组件包会重新压缩成 zip 后写入 blobs/components。
async fn export_components(
    db: &DbState,
    app_state: &AppState,
    component_files: Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<(Vec<AssetBlobRef>, Vec<ZipEntry>), String> {
    if component_files.is_empty() {
        return Ok((vec![], vec![]));
    }

    let mut items: Vec<AssetBlobRef> = Vec::new();
    let mut entries: Vec<ZipEntry> = Vec::new();

    for raw_file in component_files {
        let raw_file = raw_file.trim().to_string();
        if raw_file.is_empty() {
            continue;
        }
        let lookup_keys = component_lookup_keys(&raw_file);
        let mut row = None;
        for key in &lookup_keys {
            row = sqlx::query(
                "SELECT component_name, url FROM remote_component \
                 WHERE deleted = 0 AND (url = ?1 OR url LIKE '%' || ?1) \
                 ORDER BY update_time DESC LIMIT 1",
            )
            .bind(key)
            .fetch_optional(&db.pool)
            .await
            .map_err(|e| format!("export.exportComponents: query failed: {e}"))?;
            if row.is_some() {
                break;
            }
        }

        let Some(row) = row else {
            warnings.push(format!("【组件缺失】{}", raw_file));
            continue;
        };

        let component_name: Option<String> = row.try_get("component_name").map_err(|e| e.to_string())?;
        let url: String = row.try_get("url").map_err(|e| e.to_string())?;
        let path = resolve_component_path(app_state, &url);
        // 组件包以目录为单位重新压缩，确保导入后目录结构完整。
        if let Some(package) = component_package_from_url(app_state, &url, &path) {
            let bytes = match zip_component_package_dir(&package.dir) {
                Ok(b) => b,
                Err(_) => {
                    warnings.push(format!("【组件缺失】{}", raw_file));
                    continue;
                }
            };
            let blob_path = format!("blobs/components/{}.zip", package.package_id);
            items.push(AssetBlobRef {
                name: component_name.unwrap_or_else(|| raw_file.clone()),
                original: normalize_component_manifest_original(&url, &package),
                blob: blob_path.clone(),
            });
            entries.push(ZipEntry { path: blob_path, bytes });
            continue;
        }

        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => {
                warnings.push(format!("【组件缺失】{}", raw_file));
                continue;
            }
        };

        let file_name = file_name_from_url(&url);
        let blob_path = format!("blobs/{}", file_name);
        items.push(AssetBlobRef {
            name: component_name.unwrap_or_else(|| raw_file.clone()),
            original: component_manifest_original_from_lookup(&url, &raw_file),
            blob: blob_path.clone(),
        });
        entries.push(ZipEntry { path: blob_path, bytes });
    }

    Ok((items, entries))
}

/// 根据前端传入的组件引用生成数据库查找 key。
fn component_lookup_keys(raw: &str) -> Vec<String> {
    let normalized = normalize_component_ref(raw);
    let mut keys = Vec::new();

    push_unique_component_key(&mut keys, normalized.clone());
    if let Some(rel) = component_rel_after_static_marker(&normalized) {
        push_unique_component_key(&mut keys, format!("static/components/{rel}"));
        push_unique_component_key(&mut keys, rel);
    }
    let file_name = file_name_from_url(&normalized);
    push_unique_component_key(&mut keys, file_name);

    keys
}

/// 向组件查找 key 列表追加去重后的 key。
fn push_unique_component_key(keys: &mut Vec<String>, value: String) {
    let key = value.trim().trim_start_matches('/').to_string();
    if key.is_empty() || keys.contains(&key) {
        return;
    }
    keys.push(key);
}

/// 归一化组件引用路径。
fn normalize_component_ref(raw: &str) -> String {
    let mut value = raw.trim().to_string();
    if let Some(stripped) = value.strip_prefix("http://asset.localhost/") {
        value = stripped.to_string();
    } else if let Some(stripped) = value.strip_prefix("asset://localhost/") {
        value = stripped.to_string();
    }

    let decoded = urlencoding::decode(&value).map(|v| v.into_owned()).unwrap_or(value);
    decoded
        .split('?')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .replace('\\', "/")
}

/// 从 `static/components/` 后提取组件相对路径。
fn component_rel_after_static_marker(normalized: &str) -> Option<String> {
    let marker = "static/components/";
    let idx = normalized.rfind(marker)?;
    let rel = normalized[idx + marker.len()..].trim_start_matches('/').trim();
    if rel.is_empty() {
        None
    } else {
        Some(rel.to_string())
    }
}

/// 根据查找输入生成 manifest 中保留的组件原始引用。
fn component_manifest_original_from_lookup(url: &str, raw: &str) -> String {
    let normalized = normalize_component_ref(raw);
    if let Some(rel) = component_rel_after_static_marker(&normalized) {
        format!("static/components/{rel}")
    } else {
        url.trim().trim_start_matches('/').replace('\\', "/")
    }
}

/// 导出动态脚本资源。
///
/// 远程脚本只记录 URL，本地脚本会复制为 blob。
async fn export_dynamic_scripts(
    db: &DbState,
    app_state: &AppState,
    project_ids: &[String],
    warnings: &mut Vec<String>,
) -> Result<(Vec<DynamicScriptRef>, Vec<ZipEntry>), String> {
    let normalized_project_ids = normalize_text_vec(project_ids.to_vec());
    let rows = if normalized_project_ids.is_empty() {
        sqlx::query(
            "SELECT name, url, type, project_id \
             FROM dynamic_script \
             WHERE deleted = 0 AND project_id IS NULL \
             ORDER BY update_time DESC",
        )
        .fetch_all(&db.pool)
        .await
        .map_err(|e| format!("export.exportDynamicScripts: query globals failed: {e}"))?
    } else {
        let mut query = String::from(
            "SELECT name, url, type, project_id \
             FROM dynamic_script \
             WHERE deleted = 0 AND (project_id IS NULL OR CAST(project_id AS TEXT) IN (",
        );
        for i in 0..normalized_project_ids.len() {
            if i > 0 {
                query.push(',');
            }
            query.push('?');
            query.push_str(&(i + 1).to_string());
        }
        query.push_str(")) ORDER BY update_time DESC");

        let mut q = sqlx::query(&query);
        for project_id in &normalized_project_ids {
            q = q.bind(project_id);
        }
        q.fetch_all(&db.pool)
            .await
            .map_err(|e| format!("export.exportDynamicScripts: query failed: {e}"))?
    };

    let mut items: Vec<DynamicScriptRef> = Vec::with_capacity(rows.len());
    let mut entries: Vec<ZipEntry> = Vec::new();
    let mut emitted_blob_paths = std::collections::HashSet::new();

    for row in rows {
        let name: String = row.try_get("name").map_err(|e| e.to_string())?;
        let url: String = row.try_get("url").map_err(|e| e.to_string())?;
        let ty: String = row.try_get("type").map_err(|e| e.to_string())?;
        let project_id: Option<String> = row.try_get("project_id").map_err(|e| e.to_string())?;
        let owner_source_project_id = project_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let scope = if owner_source_project_id.is_some() {
            "PROJECT".to_string()
        } else {
            "GLOBAL".to_string()
        };

        // 远程脚本不写入 zip，只记录 URL；本地脚本才需要复制为 blob。
        if ty.to_ascii_uppercase() == "REMOTE" || url.starts_with("http") {
            items.push(DynamicScriptRef {
                name,
                ty: "REMOTE".to_string(),
                scope,
                owner_source_project_id,
                original: None,
                blob: None,
                url: Some(url),
            });
            continue;
        }

        if let Some((mime, bytes)) = parse_data_url(&url) {
            let ext = mime_to_ext(&mime, "js");
            let sha = sha256_hex(&bytes);
            let blob_path = format!("blobs/{}.{}", sha, ext);
            let safe_name = sanitize_download_file_name(&name, "DataVisoolProject");
            items.push(DynamicScriptRef {
                name,
                ty: "LOCAL".to_string(),
                scope,
                owner_source_project_id,
                original: Some(format!("{}.{}", safe_name, ext)),
                blob: Some(blob_path.clone()),
                url: None,
            });
            if emitted_blob_paths.insert(blob_path.clone()) {
                entries.push(ZipEntry { path: blob_path, bytes });
            }
            continue;
        }

        let file_name = file_name_from_url(&url);
        if file_name.trim().is_empty() {
            warnings.push(format!("【三方脚本缺失】{}", name));
            continue;
        }
        let p1 = app_state.static_storage_dir().join("lib").join(&file_name);
        let p2 = app_state.files_dir.join("static").join("lib").join(&file_name);
        let bytes = match std::fs::read(if p1.exists() { &p1 } else { &p2 }) {
            Ok(b) => b,
            Err(_) => {
                warnings.push(format!("【三方脚本缺失】{}", file_name));
                continue;
            }
        };

        let blob_path = format!("blobs/{}", file_name);
        items.push(DynamicScriptRef {
            name,
            ty: "LOCAL".to_string(),
            scope,
            owner_source_project_id,
            original: Some(file_name),
            blob: Some(blob_path.clone()),
            url: None,
        });
        if emitted_blob_paths.insert(blob_path.clone()) {
            entries.push(ZipEntry { path: blob_path, bytes });
        }
    }

    Ok((items, entries))
}

/// 导出图片资源。
async fn export_images(
    db: &DbState,
    app_state: &AppState,
    image_files: Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<(Vec<AssetBlobRef>, Vec<ZipEntry>), String> {
    if image_files.is_empty() {
        return Ok((vec![], vec![]));
    }

    let mut items: Vec<AssetBlobRef> = Vec::new();
    let mut entries: Vec<ZipEntry> = Vec::new();

    for file in image_files {
        let file = file.trim().to_string();
        if file.is_empty() {
            continue;
        }
        let image_name =
            sqlx::query_scalar::<_, String>("SELECT COALESCE(name, '') FROM image WHERE deleted = 0 AND url = ?1 ORDER BY update_time DESC LIMIT 1")
                .bind(&file)
                .fetch_optional(&db.pool)
                .await
                .ok()
                .flatten()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| file.clone());

        // data URL 图片直接按内容 hash 写入 blob，避免导出超长 original 路径。
        if file.starts_with("data:") {
            if let Some((mime, bytes)) = parse_data_url(&file) {
                let ext = mime_to_ext(&mime, "bin");
                let sha = sha256_hex(&bytes);
                let blob_path = format!("blobs/{}.{}", sha, ext);
                items.push(AssetBlobRef { name: image_name, original: file.clone(), blob: blob_path.clone() });
                entries.push(ZipEntry { path: blob_path, bytes });
            } else {
                warnings.push(format!("【图片缺失】{}", file));
            }
            continue;
        }

        // 普通图片从当前静态资源目录读取。
        let file_name = file_name_from_url(&file);
        let p1 = app_state.static_storage_dir().join("images").join(&file_name);
        if let Ok(bytes) = std::fs::read(&p1) {
            let blob_path = format!("blobs/{}", file_name);
            items.push(AssetBlobRef { name: image_name, original: file.clone(), blob: blob_path.clone() });
            entries.push(ZipEntry { path: blob_path, bytes });
        } else {
            warnings.push(format!("【图片缺失】{}", file));
        }
    }

    Ok((items, entries))
}

/// 导出视频资源。
async fn export_videos(
    db: &DbState,
    app_state: &AppState,
    video_files: Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<(Vec<AssetBlobRef>, Vec<ZipEntry>), String> {
    if video_files.is_empty() {
        return Ok((vec![], vec![]));
    }

    let mut items: Vec<AssetBlobRef> = Vec::new();
    let mut entries: Vec<ZipEntry> = Vec::new();

    for file in video_files {
        let file = file.trim().to_string();
        if file.is_empty() {
            continue;
        }

        let row = sqlx::query("SELECT name, url FROM video WHERE deleted = 0 AND url LIKE '%' || ?1 ORDER BY update_time DESC LIMIT 1")
            .bind(&file)
            .fetch_optional(&db.pool)
            .await
            .map_err(|e| format!("export.exportVideos: query failed: {e}"))?;

        let Some(row) = row else {
            warnings.push(format!("【视频缺失】{}", file));
            continue;
        };

        let name: String = row.try_get("name").map_err(|e| e.to_string())?;
        let url: String = row.try_get("url").map_err(|e| e.to_string())?;
        let file_name = file_name_from_url(&url);
        if file_name.trim().is_empty() {
            warnings.push(format!("【视频缺失】{}", name));
            continue;
        }

        let p1 = app_state.static_storage_dir().join("videos").join(&file_name);
        let p2 = app_state.files_dir.join("static").join("videos").join(&file_name);
        let bytes = match std::fs::read(if p1.exists() { &p1 } else { &p2 }) {
            Ok(b) => b,
            Err(_) => {
                warnings.push(format!("【视频缺失】{}", file_name));
                continue;
            }
        };

        let blob_path = format!("blobs/{}", file_name);
        items.push(AssetBlobRef { name, original: file.clone(), blob: blob_path.clone() });
        entries.push(ZipEntry { path: blob_path, bytes });
    }

    Ok((items, entries))
}

/// 导出 3D 模型资源。
///
/// 标准模型包会保留 models/{packageId}/ 下的相对文件结构。
async fn export_models(
    db: &DbState,
    app_state: &AppState,
    model_files: Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<(Vec<ModelPackageRef>, Vec<ZipEntry>), String> {
    if model_files.is_empty() {
        return Ok((vec![], vec![]));
    }

    let mut items: Vec<ModelPackageRef> = Vec::new();
    let mut entries: Vec<ZipEntry> = Vec::new();

    let models_dir = app_state.static_storage_dir().join("models");

    for file in model_files {
        let key = file.trim().replace('\\', "/");
        if key.is_empty() {
            continue;
        }

        let row = sqlx::query("SELECT name, url FROM three_d_model WHERE deleted = 0 AND url LIKE '%' || ?1 ORDER BY update_time DESC LIMIT 1")
            .bind(&key)
            .fetch_optional(&db.pool)
            .await
            .map_err(|e| format!("export.exportModels: query failed: {e}"))?;

        let (name, url) = if let Some(row) = row {
            let name: String = row.try_get("name").map_err(|e| e.to_string())?;
            let url: String = row.try_get("url").map_err(|e| e.to_string())?;
            (name, url)
        } else {
            (key.clone(), key.clone())
        };

        let normalized_url = url.trim().replace('\\', "/");
        if normalized_url.is_empty() {
            warnings.push(format!("【模型缺失】{}", key));
            continue;
        }

        // 模型资源按 models/{packageId}/{entry} 结构导出。
        let parts: Vec<&str> = normalized_url.split('/').collect();
        let (package_hash, entry_rel) = if parts.len() >= 2 {
            let entry = parts.last().unwrap();
            let folder = parts[parts.len() - 2];
            (folder.to_string(), entry.to_string())
        } else {
            let entry = parts.last().unwrap_or(&normalized_url.as_str()).to_string();
            ("root".to_string(), entry)
        };

        let model_folder = models_dir.join(&package_hash);
        if !model_folder.is_dir() {
            warnings.push(format!("【模型缺失】{}", normalized_url));
            continue;
        }

        // 标准模型包保留目录内所有相对路径，确保材质、贴图等依赖可一并导入。
        let rel_files = collect_dir_rel_bytes(&model_folder)?;
        let mut manifest_files: Vec<ModelFileRef> = Vec::new();

        for (rel, bytes) in rel_files {
            let rel_norm = datavisool_archive::normalize_rel_path_strict(&rel).map_err(|_| "invalid path".to_string())?;
            let zip_path = format!("models/{}/{}", package_hash, rel_norm);
            entries.push(ZipEntry { path: zip_path.clone(), bytes });
            manifest_files.push(ModelFileRef { path: rel_norm, blob: zip_path });
        }

        items.push(ModelPackageRef { name, original: key.clone(), package_id: package_hash, entry: entry_rel, files: manifest_files });
    }

    Ok((items, entries))
}

/// 将组件 URL 解析成本地文件路径。
fn resolve_component_path(app_state: &AppState, url: &str) -> PathBuf {
    let s = url.trim();
    if s.starts_with('/') || s.contains(':') {
        return PathBuf::from(s);
    }
    if s.contains('/') || s.contains('\\') {
        let p1 = app_state.resource_storage_root().join(s);
        if p1.exists() {
            return p1;
        }
        return app_state.files_dir.join(s);
    }
    let p1 = app_state.static_storage_dir().join("components").join(s);
    if p1.exists() {
        return p1;
    }
    app_state.files_dir.join("static").join("components").join(s)
}

/// 组件包导出上下文。
struct ComponentPackageExport {
    dir: PathBuf,
    package_id: String,
    entry_rel: String,
}

/// 根据组件 URL 判断它是否属于组件包，并解析包目录。
fn component_package_from_url(app_state: &AppState, url: &str, entry_path: &Path) -> Option<ComponentPackageExport> {
    let normalized = url.trim().trim_start_matches('/').replace('\\', "/");
    let marker = "static/components/";
    let after_marker = normalized.rfind(marker).map(|idx| &normalized[idx + marker.len()..])?;
    let parts = after_marker
        .split('/')
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }

    let package_id = parts[0].to_string();
    let entry_rel = parts[1..].join("/");
    if package_id.is_empty() || entry_rel.is_empty() {
        return None;
    }

    // 优先使用新静态资源目录；找不到时从入口文件路径反推出包根目录。
    let components_root = app_state.static_storage_dir().join("components");
    let preferred = components_root.join(&package_id);
    let dir = if preferred.is_dir() {
        preferred
    } else {
        component_package_dir_from_entry_path(entry_path, &entry_rel)?
    };
    if !dir.is_dir() {
        return None;
    }

    Some(ComponentPackageExport { dir, package_id, entry_rel })
}

/// 根据入口文件路径和入口相对路径反推出组件包根目录。
fn component_package_dir_from_entry_path(entry_path: &Path, entry_rel: &str) -> Option<PathBuf> {
    let depth = entry_rel.split('/').filter(|part| !part.trim().is_empty()).count();
    if depth == 0 {
        return None;
    }
    let mut dir = entry_path.to_path_buf();
    for _ in 0..depth {
        dir = dir.parent()?.to_path_buf();
    }
    Some(dir)
}

/// 归一化组件包在 manifest 中的 original 字段。
fn normalize_component_manifest_original(url: &str, package: &ComponentPackageExport) -> String {
    let normalized = url.trim().trim_start_matches('/').replace('\\', "/");
    if normalized.contains("static/components/") {
        normalized
    } else {
        format!("static/components/{}/{}", package.package_id, package.entry_rel)
    }
}

/// 将组件包目录重新压缩为 zip 字节。
fn zip_component_package_dir(dir: &Path) -> Result<Vec<u8>, String> {
    let mut files = Vec::new();
    collect_component_package_files(dir, dir, &mut files)?;
    if files.is_empty() {
        return Err("component package is empty".to_string());
    }

    let cursor = Cursor::new(Vec::<u8>::new());
    let mut writer = ZipWriter::new(cursor);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (rel, abs) in files {
        writer
            .start_file(rel, options)
            .map_err(|e| format!("zip start file failed: {e}"))?;
        let bytes = std::fs::read(&abs).map_err(|e| format!("zip read file failed: {e}"))?;
        writer.write_all(&bytes).map_err(|e| format!("zip write file failed: {e}"))?;
    }
    let cursor = writer.finish().map_err(|e| format!("zip finish failed: {e}"))?;
    Ok(cursor.into_inner())
}

/// 收集组件包目录下的文件。
fn collect_component_package_files(root: &Path, current: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    for entry in std::fs::read_dir(current).map_err(|e| format!("read dir failed: {e}"))? {
        let entry = entry.map_err(|e| format!("read dir entry failed: {e}"))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| format!("read file type failed: {e}"))?;
        if file_type.is_dir() {
            collect_component_package_files(root, &path, out)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|e| format!("strip package root failed: {e}"))?
            .to_string_lossy()
            .replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        out.push((rel, path));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(())
}
