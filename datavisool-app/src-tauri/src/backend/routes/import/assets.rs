/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
/// 导入图片资源文件，并补齐 image 表中的资源记录。
async fn import_images<R: Read + std::io::Seek>(
    app_state: &AppState,
    tx: &mut Transaction<'_, Sqlite>,
    file_staging: &mut ImportFileStaging,
    archive: &mut ZipArchive<R>,
    images: &[AssetBlobRef],
) -> Result<(), String> {
    for item in images {
        let blob = item.blob.trim().to_string();
        if blob.is_empty() {
            continue;
        }
        // 图片采用 blob 去重存储，业务目录只保存可直接访问的同名文件。
        let bytes = read_zip_bytes(archive, &blob)?;
        let blob_filename = parse_blob_filename(&blob).ok_or_else(|| "导入失败：blob 路径不合法".to_string())?;

        let abs = app_state.static_storage_dir().join("images").join(&blob_filename);
        file_staging.stage_blob_and_link(&app_state.static_storage_dir().join("blobs"), &blob_filename, &bytes, &abs)?;

        let exists = sqlx::query("SELECT id FROM image WHERE deleted = 0 AND url = ?1 LIMIT 1")
            .bind(&blob_filename)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| format!("导入失败：查询图片失败（{e}）"))?
            .is_some();
        if !exists {
            let name_to_store = if item.name.trim().is_empty() {
                blob_filename.clone()
            } else {
                item.name.trim().to_string()
            };
            sqlx::query("INSERT INTO image (url, name, deleted) VALUES (?1, ?2, 0)")
                .bind(&blob_filename)
                .bind(name_to_store)
                .execute(&mut **tx)
                .await
                .map_err(|e| format!("导入失败：保存图片信息失败（{e}）"))?;
        }
    }
    Ok(())
}

/// 导入视频资源文件，并补齐 video 表中的资源记录。
async fn import_videos<R: Read + std::io::Seek>(
    app_state: &AppState,
    tx: &mut Transaction<'_, Sqlite>,
    file_staging: &mut ImportFileStaging,
    archive: &mut ZipArchive<R>,
    videos: &[AssetBlobRef],
) -> Result<(), String> {
    for item in videos {
        let blob = item.blob.trim().to_string();
        if blob.is_empty() {
            continue;
        }
        // 先暂存文件，再写数据库；外层事务失败时文件暂存器负责回滚。
        let bytes = read_zip_bytes(archive, &blob)?;
        let blob_filename = parse_blob_filename(&blob).ok_or_else(|| "导入失败：blob 路径不合法".to_string())?;

        let abs = app_state.static_storage_dir().join("videos").join(&blob_filename);
        file_staging.stage_blob_and_link(&app_state.static_storage_dir().join("blobs"), &blob_filename, &bytes, &abs)?;

        let exists = sqlx::query("SELECT id FROM video WHERE deleted = 0 AND url = ?1 LIMIT 1")
            .bind(&blob_filename)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| format!("导入失败：查询视频失败（{e}）"))?
            .is_some();
        if !exists {
            let name_to_store = if item.name.trim().is_empty() {
                blob_filename.clone()
            } else {
                item.name.trim().to_string()
            };
            sqlx::query("INSERT INTO video (name, url, size, type, deleted) VALUES (?1, ?2, ?3, NULL, 0)")
                .bind(name_to_store)
                .bind(&blob_filename)
                .bind(bytes.len() as i64)
                .execute(&mut **tx)
                .await
                .map_err(|e| format!("导入失败：保存视频信息失败（{e}）"))?;
        }
    }
    Ok(())
}

/// 导入字体资源文件，并补齐 font 表中的资源记录。
async fn import_fonts<R: Read + std::io::Seek>(
    app_state: &AppState,
    tx: &mut Transaction<'_, Sqlite>,
    file_staging: &mut ImportFileStaging,
    archive: &mut ZipArchive<R>,
    fonts: &[AssetBlobRef],
) -> Result<(), String> {
    for item in fonts {
        let blob = item.blob.trim().to_string();
        if blob.is_empty() {
            continue;
        }
        let bytes = read_zip_bytes(archive, &blob)?;
        let blob_filename = parse_blob_filename(&blob).ok_or_else(|| "导入失败：blob 路径不合法".to_string())?;
        let abs = app_state.static_storage_dir().join("font").join(&blob_filename);
        file_staging.stage_blob_and_link(&app_state.static_storage_dir().join("blobs"), &blob_filename, &bytes, &abs)?;

        let exists = sqlx::query("SELECT id FROM font WHERE deleted = 0 AND url = ?1 LIMIT 1")
            .bind(&blob_filename)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| format!("导入失败：查询字体失败（{e}）"))?
            .is_some();
        if exists {
            continue;
        }
        let name_to_store = if item.name.trim().is_empty() {
            blob_filename.clone()
        } else {
            item.name.trim().to_string()
        };
        sqlx::query("INSERT INTO font (name, url, deleted) VALUES (?1, ?2, 0)")
            .bind(name_to_store)
            .bind(&blob_filename)
            .execute(&mut **tx)
            .await
            .map_err(|e| format!("导入失败：保存字体失败（{e}）"))?;
    }
    Ok(())
}

/// 导入远程组件资源，兼容单文件组件和组件 zip 包。
async fn import_components<R: Read + std::io::Seek>(
    app_state: &AppState,
    tx: &mut Transaction<'_, Sqlite>,
    file_staging: &mut ImportFileStaging,
    archive: &mut ZipArchive<R>,
    components: &[AssetBlobRef],
    asset_url_context: &mut ImportedAssetUrlContext,
) -> Result<(), String> {
    for item in components {
        let blob = item.blob.trim().to_string();
        if blob.is_empty() {
            continue;
        }
        let bytes = read_zip_bytes(archive, &blob)?;
        let mut blob_filename = parse_blob_filename(&blob).ok_or_else(|| "导入失败：blob 路径不合法".to_string())?;
        // zip 组件包需要展开到独立 package_id 目录，普通组件直接写入 components。
        if is_zip_filename(&blob_filename) {
            import_component_package(app_state, tx, file_staging, item, &blob_filename, &bytes, asset_url_context).await?;
            continue;
        }

        let mut component_bytes = bytes.clone();
        let lower_filename = blob_filename.to_ascii_lowercase();
        let extension = if lower_filename.ends_with(".mjs") { Some("mjs") } else if lower_filename.ends_with(".js") { Some("js") } else { None };
        if let Some(extension) = extension {
            let source = std::str::from_utf8(&bytes).map_err(|_| format!("导入失败：服务器组件 {} 的脚本不是 UTF-8", item.name))?;
            // TODO(legacy-compat): 停止支持 LCP 项目包后，移除旧单文件组件转换和原文件留存。
            if let Some(converted) = legacy_component::convert_entry(source).map_err(|e| format!("导入失败：服务器组件 {} 无法静态转换（{e}）", item.name))? {
                let original_name = format!("{}.{extension}", datavisool_archive::sha256_hex(&bytes));
                file_staging.stage_write_if_missing(&app_state.static_storage_dir().join("blobs").join(original_name), &bytes)?;
                component_bytes = converted.into_bytes();
                blob_filename = format!("{}.{extension}", datavisool_archive::sha256_hex(&component_bytes));
                if let Some(web_path) = build_import_static_path("components", &item.original) {
                    asset_url_context.static_url_map.insert(web_path, to_asset_url(&app_state.static_storage_dir().join("components").join(&blob_filename)));
                }
            }
        }
        let abs = app_state.static_storage_dir().join("components").join(&blob_filename);
        file_staging.stage_blob_and_link(&app_state.static_storage_dir().join("blobs"), &blob_filename, &component_bytes, &abs)?;

        let exists = sqlx::query("SELECT id FROM remote_component WHERE deleted = 0 AND url = ?1 LIMIT 1")
            .bind(&blob_filename)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| format!("导入失败：查询服务器组件失败（{e}）"))?
            .is_some();
        if exists {
            continue;
        }
        let name_to_store = if item.name.trim().is_empty() {
            blob_filename.clone()
        } else {
            item.name.trim().to_string()
        };
        sqlx::query("INSERT INTO remote_component (component_name, url, deleted) VALUES (?1, ?2, 0)")
            .bind(name_to_store)
            .bind(&blob_filename)
            .execute(&mut **tx)
            .await
            .map_err(|e| format!("导入失败：保存服务器组件失败（{e}）"))?;
    }
    Ok(())
}

/// 导入组件 zip 包，解压到预留目录并写入 remote_component 记录。
async fn import_component_package(
    app_state: &AppState,
    tx: &mut Transaction<'_, Sqlite>,
    file_staging: &mut ImportFileStaging,
    item: &AssetBlobRef,
    blob_filename: &str,
    bytes: &[u8],
    asset_url_context: &mut ImportedAssetUrlContext,
) -> Result<(), String> {
    // 先扫描入口文件，确保组件包可用后再提交目录和数据库记录。
    let entry = scan_component_package_entry(bytes, component_entry_hint(&item.original).as_deref())
        .map_err(|e| format!("导入失败：组件压缩包不合法（{e}）"))?;
    let package = asset_url_context
        .component_package_reservations
        .get_mut(&component_package_reservation_key(item, blob_filename))
        .ok_or_else(|| "导入失败：组件包目录未分配".to_string())?;
    if let Err(e) = extract_component_package_zip(bytes, &package.temp_dir) {
        package.rollback();
        return Err(format!("导入失败：解压服务器组件失败（{e}）"));
    }
    // TODO(legacy-compat): 停止支持 LCP 项目包后，移除旧组件包转换和原包留存。
    let converted = legacy_component::convert_package(&package.temp_dir, &entry)
        .map_err(|e| format!("导入失败：服务器组件 {} 无法静态转换（{e}）", item.name))?;
    if converted {
        let original_name = format!("{}.zip", datavisool_archive::sha256_hex(bytes));
        file_staging.stage_write_if_missing(&app_state.static_storage_dir().join("blobs").join(original_name), bytes)?;
    }
    if let Err(e) = package.commit() {
        package.rollback();
        return Err(format!("导入失败：保存服务器组件文件失败（{e}）"));
    }

    let package_id = package.package_id.clone();
    let stored_url = format!("static/components/{}/{}", package_id, entry);
    let exists = match sqlx::query("SELECT id FROM remote_component WHERE deleted = 0 AND url = ?1 LIMIT 1")
        .bind(&stored_url)
        .fetch_optional(&mut **tx)
        .await
    {
        Ok(row) => row.is_some(),
        Err(e) => {
            package.rollback();
            return Err(format!("导入失败：查询服务器组件失败（{e}）"));
        }
    };
    if exists {
        return Ok(());
    }
    let name_to_store = if item.name.trim().is_empty() {
        file_name_string(&entry).unwrap_or_else(|| blob_filename.to_string())
    } else {
        item.name.trim().to_string()
    };
    sqlx::query("INSERT INTO remote_component (component_name, url, deleted) VALUES (?1, ?2, 0)")
        .bind(name_to_store)
        .bind(&stored_url)
        .execute(&mut **tx)
        .await
        .map_err(|e| {
            package.rollback();
            format!("导入失败：保存服务器组件失败（{e}）")
        })?;
    Ok(())
}

/// 扫描组件 zip 包，选择最可能的 .js/.mjs 入口文件。
fn scan_component_package_entry(zip_bytes: &[u8], hint: Option<&str>) -> Result<String, String> {
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes)).map_err(|e| format!("zip open failed: {e}"))?;
    let normalized_hint = hint.map(|s| s.trim().replace('\\', "/"));
    let mut best: Option<(i32, usize)> = None;

    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(|e| format!("zip read entry failed: {e}"))?;
        if file.is_dir() {
            continue;
        }
        let Some(rel) = safe_component_zip_rel_path(file.name()) else {
            continue;
        };
        let lower = rel.to_ascii_lowercase();
        if !lower.ends_with(".js") && !lower.ends_with(".mjs") {
            continue;
        }
        // 优先使用导出时记录的入口提示；缺失时按路径形态推断入口。
        let priority = match normalized_hint.as_deref() {
            Some(hint) if rel == hint => 100,
            Some(hint) if file_name_string(&rel).as_deref() == Some(hint) => 90,
            Some(hint) if file_name_string(&rel).as_deref() == file_name_string(hint).as_deref() => 80,
            _ => component_entry_priority(&lower),
        };
        match best {
            Some((best_priority, _)) if best_priority >= priority => {}
            _ => best = Some((priority, i)),
        }
    }

    let Some((_, idx)) = best else {
        return Err("zip has no .js or .mjs entry".to_string());
    };
    let file = archive.by_index(idx).map_err(|e| format!("zip read entry failed: {e}"))?;
    safe_component_zip_rel_path(file.name()).ok_or_else(|| "zip entry path is not safe".to_string())
}

/// 为候选组件入口路径计算优先级。
fn component_entry_priority(path: &str) -> i32 {
    let depth = path.split('/').count();
    if depth == 1 {
        return 30;
    }
    if path.starts_with("assets/") || path.starts_with("chunks/") {
        return 10;
    }
    if path.contains("/assets/") || path.contains("/chunks/") {
        return 15;
    }
    20
}

/// 将组件 zip 安全解压到指定目录。
fn extract_component_package_zip(zip_bytes: &[u8], target_dir: &Path) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes)).map_err(|e| format!("zip open failed: {e}"))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| format!("zip read entry failed: {e}"))?;
        if file.is_dir() {
            continue;
        }
        // 过滤绝对路径、父级跳转和系统隐藏文件，避免 zip-slip 写出目标目录。
        let Some(rel) = safe_component_zip_rel_path(file.name()) else {
            continue;
        };
        let out_path = target_dir.join(&rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("zip create dir failed: {e}"))?;
        }
        let mut out_file = std::fs::File::create(&out_path).map_err(|e| format!("zip create file failed: {e}"))?;
        std::io::copy(&mut file, &mut out_file).map_err(|e| format!("zip write file failed: {e}"))?;
    }
    Ok(())
}

/// 规范化组件 zip 内部路径，只保留安全的相对路径。
fn safe_component_zip_rel_path(name: &str) -> Option<String> {
    let normalized = name.replace('\\', "/");
    if normalized.starts_with("__MACOSX/") || normalized.ends_with("/.DS_Store") {
        return None;
    }
    let path = Path::new(&normalized);
    if path.is_absolute() {
        return None;
    }
    let mut out = Vec::<String>::new();
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::Normal(s) => out.push(s.to_string_lossy().to_string()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join("/"))
    }
}

/// 导入动态脚本，分别处理远程脚本和本地脚本 blob。
async fn import_dynamic_scripts<R: Read + std::io::Seek>(
    app_state: &AppState,
    tx: &mut Transaction<'_, Sqlite>,
    file_staging: &mut ImportFileStaging,
    archive: &mut ZipArchive<R>,
    scripts: &[DynamicScriptRef],
    default_project_id: i64,
    project_id_map: Option<&HashMap<String, i64>>,
) -> Result<(), String> {
    for item in scripts {
        let target_project_id = resolve_import_dynamic_script_project_id(item, default_project_id, project_id_map)?;
        let ty = item.ty.trim().to_ascii_uppercase();
        // REMOTE 脚本只保存 URL，不需要从导出包读取文件内容。
        if ty == "REMOTE" {
            let url = item.url.clone().unwrap_or_default();
            if url.trim().is_empty() {
                continue;
            }
            let exists = match target_project_id {
                Some(project_id) => {
                    sqlx::query("SELECT id FROM dynamic_script WHERE deleted = 0 AND type = 'REMOTE' AND url = ?1 AND project_id = ?2 LIMIT 1")
                        .bind(&url)
                        .bind(project_id.to_string())
                        .fetch_optional(&mut **tx)
                        .await
                        .map_err(|e| format!("导入失败：查询动态脚本失败（{e}）"))?
                        .is_some()
                }
                None => {
                    sqlx::query("SELECT id FROM dynamic_script WHERE deleted = 0 AND type = 'REMOTE' AND url = ?1 AND project_id IS NULL LIMIT 1")
                        .bind(&url)
                        .fetch_optional(&mut **tx)
                        .await
                        .map_err(|e| format!("导入失败：查询动态脚本失败（{e}）"))?
                        .is_some()
                }
            };
            if exists {
                continue;
            }
            sqlx::query("INSERT INTO dynamic_script (name, url, type, project_id, deleted) VALUES (?1, ?2, 'REMOTE', ?3, 0)")
                .bind(item.name.trim().to_string())
                .bind(&url)
                .bind(target_project_id.map(|value| value.to_string()))
                .execute(&mut **tx)
                .await
                .map_err(|e| format!("导入失败：保存动态脚本失败（{e}）"))?;
            continue;
        }

        let blob = item.blob.clone().unwrap_or_default();
        if blob.trim().is_empty() {
            continue;
        }
        let bytes = read_zip_bytes(archive, &blob)?;
        let blob_filename = parse_blob_filename(&blob).ok_or_else(|| "导入失败：blob 路径不合法".to_string())?;
        let abs = app_state.static_storage_dir().join("lib").join(&blob_filename);
        file_staging.stage_blob_and_link(&app_state.static_storage_dir().join("blobs"), &blob_filename, &bytes, &abs)?;

        let exists = match target_project_id {
            Some(project_id) => {
                sqlx::query("SELECT id FROM dynamic_script WHERE deleted = 0 AND type = 'LOCAL' AND url = ?1 AND project_id = ?2 LIMIT 1")
                    .bind(&blob_filename)
                    .bind(project_id.to_string())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| format!("导入失败：查询动态脚本失败（{e}）"))?
                    .is_some()
            }
            None => sqlx::query("SELECT id FROM dynamic_script WHERE deleted = 0 AND type = 'LOCAL' AND url = ?1 AND project_id IS NULL LIMIT 1")
                .bind(&blob_filename)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| format!("导入失败：查询动态脚本失败（{e}）"))?
                .is_some(),
        };
        if exists {
            continue;
        }
        sqlx::query("INSERT INTO dynamic_script (name, url, type, project_id, deleted) VALUES (?1, ?2, 'LOCAL', ?3, 0)")
            .bind(item.name.trim().to_string())
            .bind(&blob_filename)
            .bind(target_project_id.map(|value| value.to_string()))
            .execute(&mut **tx)
            .await
            .map_err(|e| format!("导入失败：保存动态脚本失败（{e}）"))?;
    }
    Ok(())
}

/// 解析导入脚本的归属项目，支持全局脚本和项目级脚本。
fn resolve_import_dynamic_script_project_id(
    item: &DynamicScriptRef,
    default_project_id: i64,
    project_id_map: Option<&HashMap<String, i64>>,
) -> Result<Option<i64>, String> {
    let scope = item.scope.trim().to_ascii_uppercase();
    if scope == "GLOBAL" {
        return Ok(None);
    }

    if scope == "PROJECT" {
        let owner_source_project_id = item
            .owner_source_project_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "导入失败：项目级动态脚本缺少 ownerSourceProjectId".to_string())?;
        let project_id_map = project_id_map.ok_or_else(|| "导入失败：项目级动态脚本缺少项目映射".to_string())?;
        let project_id = project_id_map
            .get(owner_source_project_id)
            .copied()
            .ok_or_else(|| format!("导入失败：动态脚本归属项目缺失（{owner_source_project_id}）"))?;
        return Ok(Some(project_id));
    }

    Ok(Some(default_project_id))
}

/// 导入 3D 模型包文件，并补齐 three_d_model 表记录。
async fn import_models<R: Read + std::io::Seek>(
    app_state: &AppState,
    tx: &mut Transaction<'_, Sqlite>,
    file_staging: &mut ImportFileStaging,
    archive: &mut ZipArchive<R>,
    models: &[ModelPackageRef],
) -> Result<(), String> {
    for model in models {
        let package_id = model.package_id.trim().to_string();
        let entry = model.entry.trim().replace('\\', "/");
        if package_id.is_empty() || entry.is_empty() {
            continue;
        }
        let root = app_state.static_storage_dir().join("models").join(&package_id);
        let mut staged_entry_size = None;
        let mut staged_entry_exists = false;
        // 模型包通常包含多个资源文件，只有入口文件存在时才写入数据库记录。
        for f in &model.files {
            let rel = f.path.trim().replace('\\', "/");
            let blob = f.blob.trim().to_string();
            if rel.is_empty() || blob.is_empty() {
                continue;
            }
            let bytes = read_zip_bytes(archive, &blob)?;
            if rel == entry {
                staged_entry_exists = true;
                staged_entry_size = Some(bytes.len() as i64);
            }
            let dst = root.join(rel);
            file_staging.stage_write_if_missing(&dst, &bytes)?;
        }

        let entry_abs = root.join(&entry);
        let entry_will_exist = entry_abs.exists() || staged_entry_exists;
        if entry_will_exist {
            let url_key = format!("{}/{}", package_id, entry);
            let exists = sqlx::query("SELECT id FROM three_d_model WHERE deleted = 0 AND url = ?1 LIMIT 1")
                .bind(&url_key)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| format!("导入失败：查询 3D 模型失败（{e}）"))?
                .is_some();
            if !exists {
                let size = std::fs::metadata(&entry_abs).ok().map(|m| m.len() as i64).or(staged_entry_size);
                let name_to_store = if model.name.trim().is_empty() {
                    // Simple filename extraction
                    let p = entry.replace('\\', "/");
                    p.rsplit('/').next().unwrap_or(&p).to_string()
                } else {
                    model.name.trim().to_string()
                };
                sqlx::query("INSERT INTO three_d_model (name, url, size, type, is_compressed, deleted) VALUES (?1, ?2, ?3, NULL, 1, 0)")
                    .bind(name_to_store)
                    .bind(&url_key)
                    .bind(size)
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| format!("导入失败：保存 3D 模型信息失败（{e}）"))?;
            }
        }
    }
    Ok(())
}

/// 尝试从 zip 中读取文本文件，缺失时返回 None。
fn try_read_zip_text<R: Read + std::io::Seek>(archive: &mut ZipArchive<R>, path: &str) -> Result<Option<String>, String> {
    let p = normalize_zip_path_lenient(path);
    datavisool_archive::try_read_zip_text(archive, path).map_err(|e| format!("导入失败：读取 {p} 失败（{e}）"))
}

/// 从 zip 中读取必需的文本文件，并统一导入错误文案。
fn read_zip_text<R: Read + std::io::Seek>(archive: &mut ZipArchive<R>, path: &str) -> Result<String, String> {
    let p = normalize_zip_path_lenient(path);
    datavisool_archive::read_zip_text(archive, path).map_err(|e| format!("导入失败：读取 {p} 失败（{e}）"))
}

/// 从 zip 中读取必需的二进制文件，并统一导入错误文案。
fn read_zip_bytes<R: Read + std::io::Seek>(archive: &mut ZipArchive<R>, path: &str) -> Result<Vec<u8>, String> {
    let p = normalize_zip_path_lenient(path);
    datavisool_archive::read_zip_bytes(archive, path).map_err(|e| format!("导入失败：读取 {p} 失败（{e}）"))
}
