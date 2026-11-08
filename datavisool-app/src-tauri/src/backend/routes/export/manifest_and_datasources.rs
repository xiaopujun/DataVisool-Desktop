/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use datavisool_archive::sha256_hex;
use datavisool_archive::ZipEntryBytes;
use datavisool_asset::{mime_to_ext, parse_data_url};
use datavisool_codec::{encrypt_envelope_v1, maybe_decompress_project_json};
use datavisool_project_format::{
    AssetBlobRef, CommonApiRef, CommonDatabaseRef, DynamicScriptRef, ExportManifest, ManifestAssets, ManifestCommonDataSources, ManifestProject,
    ManifestProjectNode, ModelFileRef, ModelPackageRef,
};
use serde::Deserialize;
use serde_json::Value;
use sqlx::Row;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::app_state::AppState;
use crate::backend::types::{err, err_with_detail, ok, HttpResponse};
use crate::backend::utils::sanitize_download_file_name;
use crate::db::DbState;

/// 项目导出请求体。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportProjectReq {
    id: Option<String>,
    name: Option<String>,
    secret_mode: Option<String>,
    secret_passphrase: Option<String>,
    projects: Option<Vec<ExportProjectNodeReq>>,
    fonts: Option<Vec<String>>,
    images: Option<Vec<String>>,
    videos: Option<Vec<String>>,
    models: Option<Vec<String>>,
    components: Option<Vec<String>>,
    common_api_ids: Option<Vec<String>>,
    database_ids: Option<Vec<String>>,
    project_json: Option<String>,
}

/// v3 项目图导出中的单个项目节点请求。
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ExportProjectNodeReq {
    source_project_id: Option<String>,
    name: Option<String>,
    source_pid: Option<String>,
    is_entry: Option<bool>,
    fonts: Option<Vec<String>>,
    images: Option<Vec<String>>,
    videos: Option<Vec<String>>,
    models: Option<Vec<String>>,
    components: Option<Vec<String>>,
    common_api_ids: Option<Vec<String>>,
    database_ids: Option<Vec<String>>,
    project_json: Option<String>,
}

/// 待写入 zip 的文件项。
struct ZipEntry {
    path: String,
    bytes: Vec<u8>,
}

/// 归一化后的导出项目节点。
#[derive(Clone)]
struct PreparedExportProjectNode {
    source_project_id: String,
    name: String,
    source_pid: Option<String>,
    is_entry: bool,
    fonts: Vec<String>,
    images: Vec<String>,
    videos: Vec<String>,
    models: Vec<String>,
    components: Vec<String>,
    common_api_ids: Vec<String>,
    database_ids: Vec<String>,
    data_path: String,
    project_json: String,
}

/// 裁剪字符串数组并去重。
fn normalize_text_vec(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() || out.contains(&trimmed) {
            continue;
        }
        out.push(trimmed);
    }
    out
}

/// 将请求体中的项目节点归一化为导出流程内部结构。
fn prepare_export_project_nodes(raw_nodes: Vec<ExportProjectNodeReq>) -> Result<Vec<PreparedExportProjectNode>, String> {
    if raw_nodes.is_empty() {
        return Ok(vec![]);
    }

    let mut prepared = Vec::with_capacity(raw_nodes.len());
    let mut seen_ids = std::collections::HashSet::new();
    let mut entry_count = 0usize;

    for raw in raw_nodes {
        let source_project_id = raw.source_project_id.unwrap_or_default().trim().to_string();
        if source_project_id.is_empty() {
            return Err("请求参数错误：projects[].sourceProjectId 不能为空".to_string());
        }
        if !seen_ids.insert(source_project_id.clone()) {
            return Err(format!("请求参数错误：projects[].sourceProjectId 重复：{}", source_project_id));
        }

        let name = raw.name.unwrap_or_default().trim().to_string();
        if name.is_empty() {
            return Err(format!("请求参数错误：projects[{}].name 不能为空", source_project_id));
        }

        let payload = raw.project_json.unwrap_or_default();
        if payload.trim().is_empty() {
            return Err(format!("请求参数错误：projects[{}].projectJson 不能为空", source_project_id));
        }

        let project_json = decompress_project_json(&payload).map_err(|e| format!("导出失败：{e}"))?;
        let is_entry = raw.is_entry.unwrap_or(false);
        if is_entry {
            entry_count += 1;
        }

        prepared.push(PreparedExportProjectNode {
            data_path: format!("projects/{}/project.json", source_project_id),
            source_project_id,
            name,
            source_pid: raw
                .source_pid
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            is_entry,
            fonts: normalize_text_vec(raw.fonts.unwrap_or_default()),
            images: normalize_text_vec(raw.images.unwrap_or_default()),
            videos: normalize_text_vec(raw.videos.unwrap_or_default()),
            models: normalize_text_vec(raw.models.unwrap_or_default()),
            components: normalize_text_vec(raw.components.unwrap_or_default()),
            common_api_ids: normalize_text_vec(raw.common_api_ids.unwrap_or_default()),
            database_ids: normalize_text_vec(raw.database_ids.unwrap_or_default()),
            project_json,
        });
    }

    if entry_count > 1 {
        return Err("请求参数错误：projects[] 只能有一个入口项目".to_string());
    }

    Ok(prepared)
}

/// 判断是否需要使用 v3 项目图导出格式。
fn should_use_project_graph_export(project_nodes: &[PreparedExportProjectNode]) -> bool {
    !project_nodes.is_empty() && (project_nodes.len() > 1 || project_nodes.iter().any(|node| node.source_pid.as_deref().is_some()))
}

/// 导出项目并直接返回 zip 字节的 base64。
pub(crate) async fn export_project(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: ExportProjectReq = serde_json::from_value(body).map_err(|e| format!("export.exportProject: invalid body: {e}"))?;

    let (file_name, bytes) = match build_export_zip(db, app_state, req).await {
        Ok(v) => v,
        Err(e) if e.starts_with("请求参数错误：") => {
            tracing::warn!("export_project bad request: {}", e);
            return Ok(err(400, e));
        }
        Err(e) => {
            tracing::error!("export_project failed: {}", e);
            return Ok(err_with_detail(500, "导出失败", e));
        }
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);

    Ok(ok(serde_json::json!({
        "fileName": file_name,
        "base64": b64
    })))
}

/// 导出项目到指定本地路径。
pub(crate) async fn export_project_to_path(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ExportToPathReq {
        #[serde(flatten)]
        base: ExportProjectReq,
        save_path: Option<String>,
    }

    let req: ExportToPathReq = serde_json::from_value(body).map_err(|e| format!("export.exportProjectToPath: invalid body: {e}"))?;

    let save_path = req.save_path.unwrap_or_default().trim().to_string();
    if save_path.is_empty() {
        return Ok(err(400, "请求参数错误：缺少保存路径"));
    }

    let (file_name, bytes) = match build_export_zip(db, app_state, req.base).await {
        Ok(v) => v,
        Err(e) if e.starts_with("请求参数错误：") => {
            tracing::warn!("export_project_to_path bad request: {}", e);
            return Ok(err(400, e));
        }
        Err(e) => {
            tracing::error!("export_project_to_path failed: {}", e);
            return Ok(err_with_detail(500, "导出失败", e));
        }
    };

    if let Some(parent) = Path::new(&save_path).parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::error!("export_project_to_path create_dir_all failed: {}", e);
            }
        }
    }
    if let Err(e) = std::fs::write(&save_path, bytes) {
        tracing::error!("export_project_to_path write failed: {}", e);
        return Ok(err_with_detail(500, "导出失败：写入文件失败", e.to_string()));
    }

    Ok(ok(serde_json::json!({
        "fileName": file_name,
        "path": save_path
    })))
}

/// 构建项目导出 zip。
///
/// 这是导出主流程：组装 manifest、收集资源、写入 zip，并按 secretMode 决定是否整体加密。
async fn build_export_zip(db: &DbState, app_state: &AppState, req: ExportProjectReq) -> Result<(String, Vec<u8>), String> {
    let secret_mode = req.secret_mode.as_deref().unwrap_or("plain").trim();
    let secret_passphrase = req.secret_passphrase.as_deref().unwrap_or("").trim();
    if !matches!(secret_mode, "" | "plain" | "passphrase-v1") {
        return Err(format!("请求参数错误：不支持的 secretMode: {secret_mode}"));
    }
    if secret_mode == "passphrase-v1" && secret_passphrase.is_empty() {
        return Err("请求参数错误：加密导出缺少导出口令".to_string());
    }
    // v3 项目图导出通过 projects[] 表达多项目依赖；老协议则继续使用单项目字段。
    let project_nodes = prepare_export_project_nodes(req.projects.clone().unwrap_or_default())?;
    let use_project_graph_export = should_use_project_graph_export(&project_nodes);

    let entry_node = if use_project_graph_export || !project_nodes.is_empty() {
        let entry_nodes: Vec<&PreparedExportProjectNode> = project_nodes.iter().filter(|node| node.is_entry).collect();
        match entry_nodes.len() {
            0 => {
                if use_project_graph_export {
                    return Err("请求参数错误：projects[] 缺少入口项目".to_string());
                }
                project_nodes.first()
            }
            1 => entry_nodes.first().copied(),
            _ => return Err("请求参数错误：projects[] 只能有一个入口项目".to_string()),
        }
    } else {
        None
    };

    let project_id = entry_node
        .map(|node| node.source_project_id.clone())
        .unwrap_or_else(|| req.id.clone().unwrap_or_default().trim().to_string());
    if project_id.is_empty() {
        return Err("请求参数错误：缺少项目 id".to_string());
    }

    let project_name = req.name.clone().unwrap_or_else(|| {
        entry_node
            .map(|node| node.name.clone())
            .unwrap_or_else(|| "DataVisoolProject".to_string())
    });
    let file_name = format!("{}.zip", sanitize_download_file_name(&project_name, "DataVisoolProject"));

    let mut warnings: Vec<String> = Vec::new();
    let mut entries: Vec<ZipEntry> = Vec::new();

    // 单项目导出使用请求顶层资源列表；项目图导出合并所有项目节点的资源列表。
    let merged_fonts = if project_nodes.is_empty() {
        normalize_text_vec(req.fonts.unwrap_or_default())
    } else {
        normalize_text_vec(project_nodes.iter().flat_map(|node| node.fonts.clone()).collect::<Vec<_>>())
    };
    let merged_images = if project_nodes.is_empty() {
        normalize_text_vec(req.images.unwrap_or_default())
    } else {
        normalize_text_vec(project_nodes.iter().flat_map(|node| node.images.clone()).collect::<Vec<_>>())
    };
    let merged_videos = if project_nodes.is_empty() {
        normalize_text_vec(req.videos.unwrap_or_default())
    } else {
        normalize_text_vec(project_nodes.iter().flat_map(|node| node.videos.clone()).collect::<Vec<_>>())
    };
    let merged_models = if project_nodes.is_empty() {
        normalize_text_vec(req.models.unwrap_or_default())
    } else {
        normalize_text_vec(project_nodes.iter().flat_map(|node| node.models.clone()).collect::<Vec<_>>())
    };
    let merged_components = if project_nodes.is_empty() {
        normalize_text_vec(req.components.unwrap_or_default())
    } else {
        normalize_text_vec(
            project_nodes
                .iter()
                .flat_map(|node| node.components.clone())
                .collect::<Vec<_>>(),
        )
    };
    let merged_common_api_ids = if project_nodes.is_empty() {
        normalize_text_vec(req.common_api_ids.unwrap_or_default())
    } else {
        normalize_text_vec(
            project_nodes
                .iter()
                .flat_map(|node| node.common_api_ids.clone())
                .collect::<Vec<_>>(),
        )
    };
    let merged_database_ids = if project_nodes.is_empty() {
        normalize_text_vec(req.database_ids.unwrap_or_default())
    } else {
        normalize_text_vec(
            project_nodes
                .iter()
                .flat_map(|node| node.database_ids.clone())
                .collect::<Vec<_>>(),
        )
    };

    // 公共数据源包含敏感信息，因此明文导出时直接拒绝，避免泄露密钥或密码。
    let common_apis = export_common_apis(db, merged_common_api_ids, &mut warnings).await?;
    let databases = export_common_databases(db, merged_database_ids, &mut warnings).await?;
    if matches!(secret_mode, "" | "plain") && (!common_apis.is_empty() || !databases.is_empty()) {
        return Err("请求参数错误：导出包含公共 API 或数据库时必须启用整包加密".to_string());
    }
    let common_apis_field = common_apis.into();
    let databases_field = databases.into();

    let (fonts, font_entries) = export_fonts(db, app_state, merged_fonts, &mut warnings).await?;
    entries.extend(font_entries);

    let (components, comp_entries) = export_components(db, app_state, merged_components, &mut warnings).await?;
    entries.extend(comp_entries);

    let dynamic_script_project_ids = if use_project_graph_export {
        project_nodes
            .iter()
            .map(|node| node.source_project_id.clone())
            .collect::<Vec<_>>()
    } else {
        vec![project_id.clone()]
    };
    let (dynamic_scripts, script_entries) = export_dynamic_scripts(db, app_state, &dynamic_script_project_ids, &mut warnings).await?;
    entries.extend(script_entries);

    let (images, img_entries) = export_images(db, app_state, merged_images, &mut warnings).await?;
    entries.extend(img_entries);

    let (videos, video_entries) = export_videos(db, app_state, merged_videos, &mut warnings).await?;
    entries.extend(video_entries);

    let (models, model_entries) = export_models(db, app_state, merged_models, &mut warnings).await?;
    entries.extend(model_entries);

    // v3 manifest 使用 projects[] 保存项目图；v2 manifest 保持单 project/project.json。
    let (manifest_project, manifest_projects, format_version) = if use_project_graph_export {
        let mut manifest_projects = Vec::with_capacity(project_nodes.len());
        for node in &project_nodes {
            entries.push(ZipEntry { path: node.data_path.clone(), bytes: node.project_json.as_bytes().to_vec() });
            manifest_projects.push(ManifestProjectNode {
                source_project_id: node.source_project_id.clone(),
                name: node.name.clone(),
                source_pid: node.source_pid.clone(),
                data_path: node.data_path.clone(),
                is_entry: node.is_entry,
            });
        }
        let entry = entry_node.expect("entry node should exist for graph export");
        (
            ManifestProject { id: entry.source_project_id.clone(), name: project_name.clone(), data_path: entry.data_path.clone() },
            manifest_projects,
            3,
        )
    } else {
        let project_json = if let Some(entry) = entry_node {
            entry.project_json.clone()
        } else {
            let payload = req.project_json.unwrap_or_default();
            if payload.trim().is_empty() {
                return Err("请求参数错误：项目数据为空".to_string());
            }
            decompress_project_json(&payload).map_err(|e| format!("导出失败：{e}"))?
        };
        entries.push(ZipEntry { path: "project/project.json".to_string(), bytes: project_json.as_bytes().to_vec() });
        (
            ManifestProject { id: project_id.clone(), name: project_name.clone(), data_path: "project/project.json".to_string() },
            vec![],
            2,
        )
    };

    let generated_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis() as u64;
    let manifest = ExportManifest {
        format: datavisool_project_format::PROJECT_FORMAT.to_string(),
        format_version,
        generator: "datavisool-desktop".to_string(),
        generated_at_ms,
        project: manifest_project,
        projects: manifest_projects,
        assets: ManifestAssets { images, fonts, videos, models, components, dynamic_scripts },
        common_datasource: ManifestCommonDataSources {
            common_apis: common_apis_field,
            databases: databases_field,
            ..ManifestCommonDataSources::default()
        },
        files: vec![],
        warnings_path: if warnings.is_empty() { None } else { Some("err.log".to_string()) },
    };

    let inner_zip = datavisool_archive::build_export_zip(&manifest, entries.iter().map(|e| ZipEntryBytes { path: &e.path, bytes: &e.bytes }), &warnings)
        .map_err(|e| format!("导出失败：{e}"))?;
    // 加密导出采用外层 zip 包装，manifest 只暴露 payload 元信息。
    let bytes = if secret_mode == "passphrase-v1" {
        build_encrypted_export_package_zip(&inner_zip, secret_passphrase, generated_at_ms)?
    } else {
        inner_zip
    };
    Ok((file_name, bytes))
}

/// 将明文导出内容包装成加密导出包。
fn build_encrypted_export_package_zip(inner_zip: &[u8], secret_passphrase: &str, generated_at_ms: u64) -> Result<Vec<u8>, String> {
    // payload.enc 是完整明文导出 zip 的加密信封，导入端需先解密再走普通导入流程。
    let envelope = encrypt_envelope_v1(inner_zip, secret_passphrase).map_err(|e| format!("导出失败：加密项目包失败（{e}）"))?;
    let payload = serde_json::to_vec(&envelope).map_err(|e| format!("导出失败：加密项目包序列化失败（{e}）"))?;
    let manifest = serde_json::json!({
        "format": datavisool_project_format::PROJECT_FORMAT,
        "formatVersion": 4,
        "generator": "datavisool-desktop",
        "generatedAtMs": generated_at_ms,
        "encrypted": true,
        "encryption": {
            "mode": "package",
            "algorithm": "passphrase-v1",
            "payloadPath": "payload.enc"
        },
        "payload": {
            "path": "payload.enc",
            "sha256": sha256_hex(&payload),
            "size": payload.len()
        }
    });

    let cursor = Cursor::new(Vec::<u8>::new());
    let mut zip = ZipWriter::new(cursor);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("manifest.json", options)
        .map_err(|e| format!("导出失败：写入加密 manifest 失败（{e}）"))?;
    let manifest_json = serde_json::to_vec(&manifest).map_err(|e| format!("导出失败：生成加密 manifest 失败（{e}）"))?;
    zip.write_all(&manifest_json)
        .map_err(|e| format!("导出失败：写入加密 manifest 失败（{e}）"))?;
    zip.start_file("payload.enc", options)
        .map_err(|e| format!("导出失败：写入加密 payload 失败（{e}）"))?;
    zip.write_all(&payload)
        .map_err(|e| format!("导出失败：写入加密 payload 失败（{e}）"))?;
    let cursor = zip.finish().map_err(|e| format!("导出失败：完成加密压缩包失败（{e}）"))?;
    Ok(cursor.into_inner())
}

/// 从 URL 或路径中提取文件名。
fn file_name_from_url(s: &str) -> String {
    let mut raw = s.trim().to_string();
    // 统一去掉桌面端两种资源协议前缀，避免把协议本身当作文件名的一部分。
    if let Some(stripped) = raw.strip_prefix("http://asset.localhost/") {
        raw = stripped.to_string();
    } else if let Some(stripped) = raw.strip_prefix("asset://localhost/") {
        raw = stripped.to_string();
    }

    // 桌面端资源路径通常是 URL 编码的绝对路径，这里先解码再做文件名提取。
    let decoded = urlencoding::decode(&raw).map(|v| v.into_owned()).unwrap_or(raw);
    // 去掉 query/hash，保证导出时 key 只使用稳定的文件名部分。
    let without_query = decoded
        .split('?')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .to_string();

    let normalized = without_query.replace('\\', "/");
    normalized.rsplit('/').next().unwrap_or("").trim().to_string()
}

/// 递归收集目录下所有文件的相对路径和字节。
fn collect_dir_rel_bytes(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    fn walk(cur: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
        let rd = std::fs::read_dir(cur).map_err(|e| e.to_string())?;
        for entry in rd {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out)?;
                continue;
            }
            if !path.is_file() {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            if rel.trim().is_empty() {
                continue;
            }
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            out.push((rel, bytes));
        }
        Ok(())
    }

    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

/// 导出公共 API 引用。
async fn export_common_apis(db: &DbState, common_api_ids: Vec<String>, warnings: &mut Vec<String>) -> Result<Vec<CommonApiRef>, String> {
    let mut normalized_ids: Vec<String> = Vec::new();
    for id in common_api_ids {
        let trimmed = id.trim().to_string();
        if trimmed.is_empty() || normalized_ids.contains(&trimmed) {
            continue;
        }
        normalized_ids.push(trimmed);
    }

    if normalized_ids.is_empty() {
        return Ok(vec![]);
    }

    let mut query = String::from(
        "SELECT id, ref_key, name, url, method, params, headers \
         FROM common_api WHERE deleted = 0 AND CAST(id AS TEXT) IN (",
    );
    for i in 0..normalized_ids.len() {
        if i > 0 {
            query.push(',');
        }
        query.push('?');
        query.push_str(&(i + 1).to_string());
    }
    query.push(')');

    let mut q = sqlx::query(&query);
    for id in &normalized_ids {
        q = q.bind(id);
    }

    let rows = q
        .fetch_all(&db.pool)
        .await
        .map_err(|e| format!("export.exportCommonApis: query failed: {e}"))?;

    let mut by_id = std::collections::HashMap::<String, CommonApiRef>::new();
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("export.exportCommonApis: read id failed: {e}"))?;
        let ref_key: Option<String> = row
            .try_get("ref_key")
            .map_err(|e| format!("export.exportCommonApis: read ref_key failed: {e}"))?;
        let name: Option<String> = row
            .try_get("name")
            .map_err(|e| format!("export.exportCommonApis: read name failed: {e}"))?;
        let url: String = row
            .try_get("url")
            .map_err(|e| format!("export.exportCommonApis: read url failed: {e}"))?;
        let method: String = row
            .try_get("method")
            .map_err(|e| format!("export.exportCommonApis: read method failed: {e}"))?;
        let params: Option<String> = row
            .try_get("params")
            .map_err(|e| format!("export.exportCommonApis: read params failed: {e}"))?;
        let headers: Option<String> = row
            .try_get("headers")
            .map_err(|e| format!("export.exportCommonApis: read headers failed: {e}"))?;

        by_id.insert(
            id.to_string(),
            CommonApiRef {
                origin_id: id.to_string(),
                ref_key: ref_key.unwrap_or_default(),
                name: name.unwrap_or_default(),
                url,
                method,
                params: params.unwrap_or_else(|| "{}".to_string()),
                headers: headers.unwrap_or_else(|| "{}".to_string()),
            },
        );
    }

    let mut result = Vec::new();
    for id in normalized_ids {
        match by_id.remove(&id) {
            Some(item) => result.push(item),
            None => warnings.push(format!("【公共API缺失】{}", id)),
        }
    }

    Ok(result)
}

/// 导出公共数据库引用。
async fn export_common_databases(db: &DbState, database_ids: Vec<String>, warnings: &mut Vec<String>) -> Result<Vec<CommonDatabaseRef>, String> {
    let mut normalized_ids: Vec<String> = Vec::new();
    for id in database_ids {
        let trimmed = id.trim().to_string();
        if trimmed.is_empty() || normalized_ids.contains(&trimmed) {
            continue;
        }
        normalized_ids.push(trimmed);
    }

    if normalized_ids.is_empty() {
        return Ok(vec![]);
    }

    let mut query = String::from(
        "SELECT id, ref_key, type, ip, port, database_name, username, password, url, des, allow_sql_script \
         FROM common_database WHERE deleted = 0 AND CAST(id AS TEXT) IN (",
    );
    for i in 0..normalized_ids.len() {
        if i > 0 {
            query.push(',');
        }
        query.push('?');
        query.push_str(&(i + 1).to_string());
    }
    query.push(')');

    let mut q = sqlx::query(&query);
    for id in &normalized_ids {
        q = q.bind(id);
    }

    let rows = q
        .fetch_all(&db.pool)
        .await
        .map_err(|e| format!("export.exportCommonDatabases: query failed: {e}"))?;

    let mut by_id = std::collections::HashMap::<String, CommonDatabaseRef>::new();
    for row in rows {
        let id: i64 = row
            .try_get("id")
            .map_err(|e| format!("export.exportCommonDatabases: read id failed: {e}"))?;
        let ref_key: Option<String> = row
            .try_get("ref_key")
            .map_err(|e| format!("export.exportCommonDatabases: read ref_key failed: {e}"))?;
        let ty: String = row
            .try_get("type")
            .map_err(|e| format!("export.exportCommonDatabases: read type failed: {e}"))?;
        let ip: String = row
            .try_get("ip")
            .map_err(|e| format!("export.exportCommonDatabases: read ip failed: {e}"))?;
        let port: String = row
            .try_get("port")
            .map_err(|e| format!("export.exportCommonDatabases: read port failed: {e}"))?;
        let database_name: String = row
            .try_get("database_name")
            .map_err(|e| format!("export.exportCommonDatabases: read database_name failed: {e}"))?;
        let username: String = row
            .try_get("username")
            .map_err(|e| format!("export.exportCommonDatabases: read username failed: {e}"))?;
        let password: String = row
            .try_get("password")
            .map_err(|e| format!("export.exportCommonDatabases: read password failed: {e}"))?;
        let url: Option<String> = row
            .try_get("url")
            .map_err(|e| format!("export.exportCommonDatabases: read url failed: {e}"))?;
        let des: Option<String> = row
            .try_get("des")
            .map_err(|e| format!("export.exportCommonDatabases: read des failed: {e}"))?;
        let allow_sql_script: i64 = row
            .try_get("allow_sql_script")
            .map_err(|e| format!("export.exportCommonDatabases: read allow_sql_script failed: {e}"))?;

        by_id.insert(
            id.to_string(),
            CommonDatabaseRef {
                origin_id: id.to_string(),
                ref_key: ref_key.unwrap_or_default(),
                ty,
                ip,
                port,
                database_name,
                username,
                password,
                url: url.unwrap_or_default(),
                des: des.unwrap_or_default(),
                allow_sql_script: allow_sql_script != 0,
            },
        );
    }

    let mut result = Vec::new();
    for id in normalized_ids {
        match by_id.remove(&id) {
            Some(item) => result.push(item),
            None => warnings.push(format!("【数据库缺失】{}", id)),
        }
    }

    Ok(result)
}

/// 解压或透传前端传来的 projectJson。
fn decompress_project_json(payload: &str) -> Result<String, String> {
    maybe_decompress_project_json(payload).map_err(|e| e.to_string())
}
