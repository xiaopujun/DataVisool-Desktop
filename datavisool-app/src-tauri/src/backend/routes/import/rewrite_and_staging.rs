/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
/// 根据 manifest 资源表构建静态资源 URL 重写上下文。
fn build_imported_asset_url_context(app_state: &AppState, assets: &ManifestAssets) -> Result<ImportedAssetUrlContext, String> {
    let static_root = app_state.static_storage_dir();
    let mut static_url_map = HashMap::new();
    let mut component_package_reservations = HashMap::new();

    build_asset_blob_url_map(&mut static_url_map, "images", &static_root.join("images"), &assets.images)?;
    build_asset_blob_url_map(&mut static_url_map, "videos", &static_root.join("videos"), &assets.videos)?;
    build_component_blob_url_map(&mut static_url_map, &static_root.join("components"), &assets.components, &mut component_package_reservations)?;

    for model in &assets.models {
        let package_id = model.package_id.trim();
        let entry = model.entry.trim().replace('\\', "/");
        if package_id.is_empty() || entry.is_empty() {
            continue;
        }
        let Some(web_path) = build_import_static_path("models", &model.original) else {
            continue;
        };
        let abs = static_root.join("models").join(package_id).join(&entry);
        static_url_map.insert(web_path, to_asset_url(&abs));
    }

    Ok(ImportedAssetUrlContext { static_url_map, component_package_reservations })
}

/// 根据普通资源 blob 引用构建 URL 映射。
fn build_asset_blob_url_map(
    static_url_map: &mut HashMap<String, String>,
    asset_type: &str,
    target_root: &Path,
    items: &[AssetBlobRef],
) -> Result<(), String> {
    for item in items {
        let Some(web_path) = build_import_static_path(asset_type, &item.original) else {
            continue;
        };
        let blob_filename = parse_blob_filename(&item.blob).ok_or_else(|| "导入失败：blob 路径不合法".to_string())?;
        static_url_map.insert(web_path, to_asset_url(&target_root.join(blob_filename)));
    }
    Ok(())
}

/// 根据组件资源 blob 引用构建 URL 映射。
fn build_component_blob_url_map(
    static_url_map: &mut HashMap<String, String>,
    target_root: &Path,
    items: &[AssetBlobRef],
    component_package_reservations: &mut HashMap<String, ReservedComponentPackage>,
) -> Result<(), String> {
    for item in items {
        let blob_filename = parse_blob_filename(&item.blob).ok_or_else(|| "导入失败：blob 路径不合法".to_string())?;
        if is_zip_filename(&blob_filename) {
            let Some(source_package_id) = file_stem_string(&blob_filename) else {
                continue;
            };
            let package = reserve_component_package(target_root, &source_package_id, ".import")
                .map_err(|e| format!("导入失败：分配服务器组件目录失败（{e}）"))?;
            let package_id = package.package_id.clone();
            component_package_reservations.insert(component_package_reservation_key(item, &blob_filename), package);
            let Some(entry_rel) = component_entry_hint(&item.original) else {
                continue;
            };
            let target = to_asset_url(&target_root.join(&package_id).join(&entry_rel));
            for web_path in component_import_static_paths(&item.original, &package_id, &entry_rel) {
                static_url_map.insert(web_path, target.clone());
            }
            continue;
        }

        let Some(web_path) = build_import_static_path("components", &item.original) else {
            continue;
        };
        static_url_map.insert(web_path, to_asset_url(&target_root.join(blob_filename)));
    }
    Ok(())
}

/// 生成组件包预留目录的去重 key。
fn component_package_reservation_key(item: &AssetBlobRef, blob_filename: &str) -> String {
    format!("{}\n{}\n{}", blob_filename.trim(), item.original.trim(), item.name.trim())
}

/// 生成组件包导入后需要支持的静态路径别名。
fn component_import_static_paths(original: &str, package_id: &str, entry_rel: &str) -> Vec<String> {
    let mut out = Vec::new();
    push_unique_static_path(&mut out, build_import_static_path("components", original));
    push_unique_static_path(&mut out, Some(format!("/static/components/{package_id}/{entry_rel}")));
    if let Some(name) = file_name_string(entry_rel) {
        push_unique_static_path(&mut out, Some(format!("/static/components/{name}")));
    }
    out
}

/// 向静态路径列表追加去重后的路径。
fn push_unique_static_path(out: &mut Vec<String>, value: Option<String>) {
    let Some(value) = value else {
        return;
    };
    let Some(normalized) = normalize_imported_static_url(&value) else {
        return;
    };
    if !out.contains(&normalized) {
        out.push(normalized);
    }
}

/// 根据资源类型和原始路径构建导入后的静态路径。
fn build_import_static_path(asset_type: &str, original: &str) -> Option<String> {
    let raw = original.trim().replace('\\', "/");
    if raw.is_empty() {
        return None;
    }

    let prefixed = if raw.starts_with("/static/") {
        raw
    } else if raw.starts_with("static/") {
        format!("/{raw}")
    } else {
        format!("/static/{asset_type}/{raw}")
    };

    normalize_imported_static_url(&prefixed)
}

/// 归一化导入包中的静态资源 URL。
fn normalize_imported_static_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let without_hash = trimmed.split('#').next().unwrap_or("");
    let without_query = without_hash.split('?').next().unwrap_or("");
    // 旧桌面版会把本机绝对路径保存为 asset URL；只提取包内映射所需的静态资源相对路径。
    // TODO(legacy-compat): 确认当前项目包不再使用本机 asset URL 后，移除旧地址识别；保留当前静态路径映射。
    let legacy_asset_path = without_query.strip_prefix("asset://localhost/")
        .or_else(|| without_query.strip_prefix("http://asset.localhost/"));
    let normalized = if let Some(encoded) = legacy_asset_path {
        let decoded = urlencoding::decode(encoded).ok()?.replace('\\', "/");
        let start = decoded.rfind("/static/")?;
        decoded[start..].to_string()
    } else {
        urlencoding::decode(without_query).unwrap_or_else(|_| without_query.into()).replace('\\', "/")
    };
    if normalized.starts_with("/static/images/")
        || normalized.starts_with("/static/videos/")
        || normalized.starts_with("/static/components/")
        || normalized.starts_with("/static/models/")
    {
        Some(normalized)
    } else {
        None
    }
}

/// 静态数据值属于用户内容，即使含有类似项目结构的字段也不能改写数据源。
fn is_inside_static_value(path: &[String]) -> bool {
    path.windows(2).any(|pair| (pair[0] == "data" || pair[0] == "triggerSource" || pair[0] == "dataConfig") && pair[1] == "staticData")
}

/// 判断当前 JSON 路径是否位于组件数据字段。
fn is_component_data_path(path: &[String]) -> bool {
    let n = path.len();
    n >= 4 && path[n - 4] == "layerManager" && path[n - 3] == "elemConfigs" && path[n - 1] == "data"
        && !is_inside_static_value(path)
}

/// 判断当前 JSON 路径是否位于全局变量触发源字段。
fn is_global_variable_trigger_source_path(path: &[String]) -> bool {
    let n = path.len();
    n >= 4 && path[n - 4] == "globalVariableManager" && path[n - 3] == "globalVariables" && path[n - 1] == "triggerSource"
        && !is_inside_static_value(path)
}

/// 判断当前 JSON 路径是否位于 GIS 数据配置字段。
fn is_gis_data_config_path(path: &[String], parent: &Map<String, Value>) -> bool {
    path.last().is_some_and(|key| key == "dataConfig")
        && !is_inside_static_value(path)
        && path.iter().any(|key| key == "elemConfigs")
        && path.iter().any(|key| key == "sources")
        && parent.get("sourceType").and_then(Value::as_str) == Some("geojson")
        && parent.get("dataMode").and_then(Value::as_str) == Some("dataConfig")
}

/// 判断当前 JSON 路径是否位于大屏引用样式字段。
fn is_screen_reference_style_path(path: &[String], parent: &Map<String, Value>) -> bool {
    path.last().is_some_and(|key| key == "style")
        && parent
            .get("base")
            .and_then(Value::as_object)
            .and_then(|base| base.get("type"))
            .and_then(Value::as_str)
            == Some("ScreenReference")
}

/// 重写大屏引用中的项目 ID。
fn rewrite_screen_reference_project_id(value: &mut Value, project_id_map: Option<&HashMap<String, i64>>) -> Result<(), String> {
    let Some(project_id_map) = project_id_map else {
        return Ok(());
    };
    let Value::Object(style) = value else {
        return Ok(());
    };
    let Some(source_screen_id) = style
        .get("screenId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let Some(new_project_id) = project_id_map.get(source_screen_id) else {
        return Err(format!("导入失败：项目引用缺失（{source_screen_id}）"));
    };
    style.insert("screenId".to_string(), Value::String(new_project_id.to_string()));
    Ok(())
}

/// 重写已识别的数据源引用；无法映射的非空引用被清空，避免误用本机同 ID 数据源。
fn rewrite_datasource_reference(
    value: &mut Value,
    source_type: &str,
    field: &str,
    id_field: &str,
    id_map: &HashMap<String, i64>,
    ref_key_map: &HashMap<String, i64>,
) -> bool {
    let Some(config) = value.as_object_mut() else { return false };
    if config.get("sourceType").and_then(Value::as_str) != Some(source_type) { return false; }
    let Some(reference) = config.get_mut(field).and_then(Value::as_object_mut) else { return false };
    let ref_key = reference.get("refKey").and_then(Value::as_str).unwrap_or("").trim();
    let origin_id = reference.get(id_field).and_then(Value::as_str).unwrap_or("").trim();
    if ref_key.is_empty() && origin_id.is_empty() { return false; }
    let mapped = ref_key_map.get(ref_key).or_else(|| id_map.get(origin_id)).copied();
    if let Some(id) = mapped {
        reference.insert(id_field.to_string(), Value::String(id.to_string()));
        false
    } else {
        reference.insert(id_field.to_string(), Value::String(String::new()));
        reference.insert("refKey".to_string(), Value::String(String::new()));
        true
    }
}

/// 从 zip 中读取 manifest 声明的所有项目节点 JSON。
fn read_manifest_project_nodes<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    manifest_projects: &[ManifestProjectNode],
) -> Result<Vec<ImportedProjectNode>, String> {
    let mut seen_ids = HashSet::new();
    let mut items = Vec::with_capacity(manifest_projects.len());
    for node in manifest_projects {
        let source_project_id = node.source_project_id.trim().to_string();
        if source_project_id.is_empty() {
            return Err("导入失败：导出包缺少 projects[].sourceProjectId".to_string());
        }
        if !seen_ids.insert(source_project_id.clone()) {
            return Err(format!("导入失败：导出包 projects[].sourceProjectId 重复（{source_project_id}）"));
        }
        let data_path = node.data_path.trim();
        if data_path.is_empty() {
            return Err(format!("导入失败：导出包缺少项目数据路径（{source_project_id}）"));
        }
        items.push(ImportedProjectNode {
            source_project_id,
            name: node.name.trim().to_string(),
            source_pid: node
                .source_pid
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            is_entry: node.is_entry,
            project_json: read_zip_text(archive, data_path)?,
        });
    }
    Ok(items)
}

/// 从 manifest 的 blob 路径中提取真实文件名。
fn parse_blob_filename(blob_path: &str) -> Option<String> {
    let name = Path::new(blob_path).file_name()?.to_str()?.trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// 判断文件名是否是 zip 包，用于区分普通组件文件和组件包。
fn is_zip_filename(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
}

/// 提取文件主名，主要用于为导入资源生成兜底显示名称。
fn file_stem_string(name: &str) -> Option<String> {
    let stem = Path::new(name).file_stem()?.to_str()?.trim().to_string();
    if stem.is_empty() {
        None
    } else {
        Some(stem)
    }
}

/// 从路径字符串中提取文件名，并过滤空白结果。
fn file_name_string(path: &str) -> Option<String> {
    let name = Path::new(path).file_name()?.to_str()?.trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// 从导出时记录的原始组件 URL 推断组件包入口文件。
fn component_entry_hint(original: &str) -> Option<String> {
    let raw = original.trim().trim_start_matches('/').replace('\\', "/");
    if raw.is_empty() {
        return None;
    }
    if let Some(idx) = raw.rfind("static/components/") {
        let after = &raw[idx + "static/components/".len()..];
        let parts = after.split('/').filter(|part| !part.trim().is_empty()).collect::<Vec<_>>();
        if parts.len() >= 2 {
            return Some(parts[1..].join("/"));
        }
    }
    Some(raw)
}

/// 将底层文件系统错误转换为用户可读的导入错误。
fn map_fs_error(e: FsError) -> String {
    match e {
        FsError::CreateDir(s) => format!("导入失败：创建目录失败（{s}）"),
        FsError::Write(s) => format!("导入失败：写入资源失败（{s}）"),
        FsError::HardLink(s) => format!("导入失败：写入资源失败（{s}）"),
        FsError::Copy(s) => format!("导入失败：写入资源失败（{s}）"),
    }
}

/// 导入资源文件的事务性暂存器。
#[derive(Debug)]
struct ImportFileStaging {
    staging_dir: PathBuf,
    writes: Vec<StagedFsWrite>,
    finalized: bool,
}

impl ImportFileStaging {
    /// 创建导入暂存目录，并初始化待提交写入列表。
    fn new(static_root: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(static_root).map_err(|e| format!("导入失败：创建资源目录失败（{e}）"))?;
        let staging_dir = create_unique_import_staging_dir(static_root)?;
        Ok(Self { staging_dir, writes: Vec::new(), finalized: false })
    }

    /// 暂存 blob 内容，并在提交时链接或复制到具体资源目录。
    fn stage_blob_and_link(&mut self, blob_store_dir: &Path, blob_filename: &str, bytes: &[u8], target_abs: &Path) -> Result<(), String> {
        let temp_blob = self.next_temp_path("blob");
        std::fs::write(&temp_blob, bytes).map_err(|e| map_fs_error(FsError::Write(e.to_string())))?;
        self.writes.push(StagedFsWrite::BlobAndLink {
            temp_blob,
            blob_abs: blob_store_dir.join(blob_filename),
            target_abs: target_abs.to_path_buf(),
            created_blob: false,
            created_target: false,
        });
        Ok(())
    }

    /// 暂存普通文件写入，目标文件不存在时才落盘。
    fn stage_write_if_missing(&mut self, target_abs: &Path, bytes: &[u8]) -> Result<(), String> {
        let temp_file = self.next_temp_path("file");
        std::fs::write(&temp_file, bytes).map_err(|e| map_fs_error(FsError::Write(e.to_string())))?;
        self.writes
            .push(StagedFsWrite::WriteIfMissing { temp_file, target_abs: target_abs.to_path_buf(), created_target: false });
        Ok(())
    }

    /// 按记录顺序提交所有暂存写入。
    fn commit(&mut self) -> Result<(), String> {
        for write in &mut self.writes {
            write.commit()?;
        }
        let _ = std::fs::remove_dir_all(&self.staging_dir);
        Ok(())
    }

    /// 标记暂存器已完成，避免 Drop 回滚已经确认的文件。
    fn finalize(&mut self) {
        self.finalized = true;
    }

    /// 按写入的逆序回滚，尽量恢复提交前的文件系统状态。
    fn rollback(&self) {
        for write in self.writes.iter().rev() {
            write.rollback();
        }
        let _ = std::fs::remove_dir_all(&self.staging_dir);
    }

    /// 为单个暂存写入生成稳定的临时文件路径。
    fn next_temp_path(&self, prefix: &str) -> PathBuf {
        self.staging_dir.join(format!("{prefix}-{}", self.writes.len()))
    }
}

/// 在静态资源根目录下创建唯一导入暂存目录。
fn create_unique_import_staging_dir(static_root: &Path) -> Result<PathBuf, String> {
    let base = format!(".import-staging-{}", current_datetime_suffix());
    create_unique_import_staging_dir_with_base(static_root, &base)
}

/// 按基础目录名尝试创建暂存目录，冲突时追加序号。
fn create_unique_import_staging_dir_with_base(static_root: &Path, base: &str) -> Result<PathBuf, String> {
    for index in 0..1000 {
        let dir_name = if index == 0 { base.to_string() } else { format!("{}-{}", base, index) };
        let staging_dir = static_root.join(dir_name);
        match std::fs::create_dir(&staging_dir) {
            Ok(_) => return Ok(staging_dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("导入失败：创建资源暂存目录失败（{e}）")),
        }
    }
    Err("导入失败：创建资源暂存目录失败（名称冲突过多）".to_string())
}

impl Drop for ImportFileStaging {
    fn drop(&mut self) {
        // 未 finalize 的导入说明事务失败或提前退出，需要自动清理文件。
        if !self.finalized {
            self.rollback();
        }
    }
}

/// 一条可提交、可回滚的文件系统写入操作。
#[derive(Debug)]
enum StagedFsWrite {
    BlobAndLink { temp_blob: PathBuf, blob_abs: PathBuf, target_abs: PathBuf, created_blob: bool, created_target: bool },
    WriteIfMissing { temp_file: PathBuf, target_abs: PathBuf, created_target: bool },
}

impl StagedFsWrite {
    /// 执行单条文件写入，并记录本次提交创建了哪些目标。
    fn commit(&mut self) -> Result<(), String> {
        match self {
            StagedFsWrite::BlobAndLink { temp_blob, blob_abs, target_abs, created_blob, created_target } => {
                // blob 存储只写入一次；资源目录通过硬链接复用 blob，失败时退化为复制。
                ensure_parent_dir(blob_abs)?;
                if !blob_abs.exists() {
                    std::fs::rename(&temp_blob, &blob_abs).map_err(|e| map_fs_error(FsError::Write(e.to_string())))?;
                    *created_blob = true;
                }
                ensure_parent_dir(target_abs)?;
                if !target_abs.exists() {
                    if std::fs::hard_link(&blob_abs, &target_abs).is_err() {
                        std::fs::copy(&blob_abs, &target_abs).map_err(|e| map_fs_error(FsError::Copy(e.to_string())))?;
                    }
                    *created_target = true;
                }
                let _ = std::fs::remove_file(temp_blob);
            }
            StagedFsWrite::WriteIfMissing { temp_file, target_abs, created_target } => {
                ensure_parent_dir(target_abs)?;
                if !target_abs.exists() {
                    std::fs::rename(&temp_file, &target_abs).map_err(|e| map_fs_error(FsError::Write(e.to_string())))?;
                    *created_target = true;
                }
                let _ = std::fs::remove_file(temp_file);
            }
        }
        Ok(())
    }

    /// 删除本次提交创建的文件，避免失败导入遗留孤儿资源。
    fn rollback(&self) {
        match self {
            StagedFsWrite::BlobAndLink { temp_blob, blob_abs, target_abs, created_blob, created_target } => {
                if *created_target {
                    let _ = std::fs::remove_file(target_abs);
                }
                if *created_blob {
                    let _ = std::fs::remove_file(blob_abs);
                }
                let _ = std::fs::remove_file(temp_blob);
            }
            StagedFsWrite::WriteIfMissing { temp_file, target_abs, created_target } => {
                if *created_target {
                    let _ = std::fs::remove_file(target_abs);
                }
                let _ = std::fs::remove_file(temp_file);
            }
        }
    }
}

/// 确保目标文件的父目录存在。
fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| map_fs_error(FsError::CreateDir(e.to_string())))?;
        }
    }
    Ok(())
}
