/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::{collections::HashMap, collections::HashSet};

use datavisool_archive::{normalize_zip_path_lenient, ArchiveError};
use datavisool_codec::{decrypt_envelope_v1, decrypt_with_passphrase_v1, CodecError};
use datavisool_fs::FsError;
use datavisool_project_format::{
    validate_manifest, AssetBlobRef, CommonApiRef, CommonDatabaseRef, DynamicScriptRef, EncryptedEnvelopeV1, ExportManifest, ManifestAssets,
    ManifestError, ManifestProjectNode, ModelPackageRef,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use sqlx::{Row, Sqlite, Transaction};
use zip::ZipArchive;

use crate::app_state::AppState;
use crate::backend::types::{err, err_with_detail, ok, parse_i64_param, HttpResponse};
use crate::backend::utils::{current_datetime_suffix, reserve_component_package, to_asset_url, ReservedComponentPackage};
use crate::db::DbState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// 从本地路径导入项目的请求体。
struct ImportFromPathReq {
    project_id: Option<String>,
    path: Option<String>,
    secret_passphrase: Option<String>,
}

#[derive(Debug, Clone)]
/// 导入包中的项目节点。
struct ImportedProjectNode {
    source_project_id: String,
    name: String,
    source_pid: Option<String>,
    is_entry: bool,
    project_json: String,
}

#[derive(Debug, Clone, Default)]
/// 公共数据源导入后的 ID/refKey 映射上下文。
struct ImportedDatasourceContext {
    common_api_id_map: HashMap<String, i64>,
    database_id_map: HashMap<String, i64>,
    common_api_ref_key_map: HashMap<String, i64>,
    database_ref_key_map: HashMap<String, i64>,
}

#[derive(Debug, Default)]
/// 静态资源导入后的 URL 重写上下文。
struct ImportedAssetUrlContext {
    static_url_map: HashMap<String, String>,
    component_package_reservations: HashMap<String, ReservedComponentPackage>,
}

impl ImportedAssetUrlContext {
    /// 将导入包中的旧静态资源 URL 重写为当前应用可访问的 URL。
    fn rewrite_static_url(&self, raw: &str) -> Option<String> {
        let normalized = normalize_imported_static_url(raw)?;
        self.static_url_map.get(&normalized).map(|target| {
            let suffix = raw.find(['?', '#']).map(|index| &raw[index..]).unwrap_or("");
            format!("{target}{suffix}")
        })
    }

    /// 将已预留的组件包目录切换为正式目录。
    fn finalize_component_packages(&mut self) {
        for package in self.component_package_reservations.values_mut() {
            package.finalize();
        }
    }
}

/// 从本地 zip 文件路径导入项目。
///
/// 所有导入逻辑下沉到 `import_project_zip_impl`，这里只负责请求解析和错误映射。
pub(crate) async fn import_project_from_path(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: ImportFromPathReq = serde_json::from_value(body).map_err(|e| format!("import.importProjectFromPath: invalid body: {e}"))?;

    let project_id = req.project_id.unwrap_or_default().trim().to_string();
    if project_id.is_empty() {
        return Ok(err(400, "请求参数错误：缺少 projectId"));
    }
    let project_id_num = match parse_i64_param(&project_id, "projectId") {
        Ok(project_id_num) => project_id_num,
        Err(response) => return Ok(response),
    };

    let zip_path = req.path.unwrap_or_default().trim().to_string();
    if zip_path.is_empty() {
        return Ok(err(400, "请求参数错误：缺少文件路径"));
    }

    let secret_passphrase = req.secret_passphrase.unwrap_or_default().trim().to_string();
    let result = import_project_zip_impl(db, app_state, project_id_num, &zip_path, &secret_passphrase).await;
    match result {
        Ok(warnings) => Ok(ok(serde_json::json!({"warnings": warnings}))),
        Err(e) if e.starts_with("请求参数错误：") => {
            tracing::warn!("import_project_from_path bad request: {}", e);
            Ok(err(400, e))
        }
        Err(e) => {
            tracing::error!("import_project_from_path failed: {}", e);
            Ok(err_with_detail(500, "导入失败", e))
        }
    }
}

/// 导入项目 zip 的内部入口。
///
/// 负责打开文件、创建 zip reader，并进入统一的 archive 导入流程。
async fn import_project_zip_impl(db: &DbState, app_state: &AppState, project_id: i64, zip_path: &str, secret_passphrase: &str) -> Result<Vec<String>, String> {
    let file = File::open(zip_path).map_err(|e| format!("导入失败：无法读取压缩包（{e}）"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("导入失败：压缩包格式不合法（{e}）"))?;

    import_project_archive(db, app_state, project_id, &mut archive, secret_passphrase).await
}

/// 导入项目 archive。
///
/// 会先读取 manifest 判断是否为包级加密格式，再分流到明文或解密导入。
async fn import_project_archive<R: Read + std::io::Seek>(
    db: &DbState,
    app_state: &AppState,
    project_id: i64,
    archive: &mut ZipArchive<R>,
    secret_passphrase: &str,
) -> Result<Vec<String>, String> {
    if let Some(manifest_json) = try_read_zip_text(archive, "manifest.json")? {
        let manifest_value: serde_json::Value =
            serde_json::from_str(&manifest_json).map_err(|e| format!("导入失败：manifest.json 解析失败（{e}）"))?;
        let format = manifest_value.get("format").and_then(Value::as_str).unwrap_or_default();
        if !datavisool_project_format::is_supported_project_format(format) {
            return Err("导入失败：不支持的导出包格式".to_string());
        }
        if is_encrypted_project_package_manifest(&manifest_value) {
            let inner_zip = decrypt_project_package_payload(archive, &manifest_value, secret_passphrase)?;
            let mut inner_archive = ZipArchive::new(Cursor::new(inner_zip)).map_err(|e| format!("导入失败：解密后的压缩包格式不合法（{e}）"))?;
            return import_plain_project_archive(db, app_state, project_id, &mut inner_archive, secret_passphrase).await;
        }
        return import_plain_project_archive_with_manifest_value(db, app_state, project_id, archive, manifest_value, secret_passphrase).await;
    }

    Err("导入失败：未找到 manifest.json，不支持旧版本格式".to_string())
}

/// 导入明文项目 archive。
async fn import_plain_project_archive<R: Read + std::io::Seek>(
    db: &DbState,
    app_state: &AppState,
    project_id: i64,
    archive: &mut ZipArchive<R>,
    secret_passphrase: &str,
) -> Result<Vec<String>, String> {
    if let Some(manifest_json) = try_read_zip_text(archive, "manifest.json")? {
        let manifest_value: serde_json::Value =
            serde_json::from_str(&manifest_json).map_err(|e| format!("导入失败：manifest.json 解析失败（{e}）"))?;
        return import_plain_project_archive_with_manifest_value(db, app_state, project_id, archive, manifest_value, secret_passphrase).await;
    }

    Err("导入失败：未找到 manifest.json，不支持旧版本格式".to_string())
}

/// 使用已读取的 manifest 导入明文项目 archive。
///
/// 这里是导入主流程，会导入公共数据、项目节点和静态资源，并在失败时回滚。
async fn import_plain_project_archive_with_manifest_value<R: Read + std::io::Seek>(
    db: &DbState,
    app_state: &AppState,
    project_id: i64,
    archive: &mut ZipArchive<R>,
    manifest_value: Value,
    secret_passphrase: &str,
) -> Result<Vec<String>, String> {
    let format = manifest_value
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    if !datavisool_project_format::is_supported_project_format(&format) {
        return Err("导入失败：不支持的导出包格式".to_string());
    }

    let manifest: ExportManifest = serde_json::from_value(manifest_value).map_err(|e| format!("导入失败：manifest.json 解析失败（{e}）"))?;

    if let Err(e) = validate_manifest(&manifest) {
        let msg = match e {
            ManifestError::InvalidFormat => "导入失败：不支持的导出包格式".to_string(),
            ManifestError::MissingFormatVersion => "导入失败：导出包版本过旧，请重新导出".to_string(),
            ManifestError::FormatVersionTooOld(_) => "导入失败：导出包版本过旧，请重新导出".to_string(),
            ManifestError::FormatVersionTooNew(_) => "导入失败：导出包版本过新，请升级应用".to_string(),
            ManifestError::MissingProjectId => "导入失败：导出包缺少 project.id".to_string(),
            ManifestError::MissingProjectName => "导入失败：导出包缺少 project.name".to_string(),
            ManifestError::MissingProjects => "导入失败：导出包缺少 projects 节点".to_string(),
            ManifestError::MissingProjectNodeSourceProjectId(index) => {
                format!("导入失败：导出包缺少 projects[{index}].sourceProjectId")
            }
            ManifestError::MissingProjectNodeName(index) => {
                format!("导入失败：导出包缺少 projects[{index}].name")
            }
            ManifestError::MissingProjectNodeDataPath(index) => {
                format!("导入失败：导出包缺少 projects[{index}].dataPath")
            }
            ManifestError::MissingEntryProjectNode => "导入失败：导出包缺少入口项目节点".to_string(),
            ManifestError::MultipleEntryProjectNodes(count) => {
                format!("导入失败：导出包存在多个入口项目节点（{count}）")
            }
            ManifestError::EntryProjectIdMismatch => "导入失败：导出包入口项目与 projects 定义不一致".to_string(),
        };
        return Err(msg);
    }

    datavisool_archive::verify_manifest_files(archive, &manifest.files).map_err(|e| match e {
        ArchiveError::Verify(s) => {
            if let Some(p) = s.strip_prefix("size mismatch (").and_then(|v| v.strip_suffix(')')) {
                format!("导入失败：文件大小校验失败（{}）", p)
            } else if let Some(p) = s.strip_prefix("sha256 mismatch (").and_then(|v| v.strip_suffix(')')) {
                format!("导入失败：文件校验失败（{}）", p)
            } else if let Some(p) = s.strip_prefix("missing (").and_then(|v| v.strip_suffix(')')) {
                format!("导入失败：文件缺失（{}）", p)
            } else {
                format!("导入失败：文件校验失败（{}）", s)
            }
        }
        _ => format!("导入失败：文件校验失败（{e}）"),
    })?;
    if manifest.files.is_empty() {
        datavisool_archive::verify_manifest_references_exist(archive, &manifest).map_err(|e| match e {
            ArchiveError::Verify(s) => {
                if let Some(p) = s.strip_prefix("missing (").and_then(|v| v.strip_suffix(')')) {
                    format!("导入失败：文件缺失（{}）", p)
                } else {
                    format!("导入失败：文件缺失（{}）", s)
                }
            }
            _ => format!("导入失败：文件校验失败（{e}）"),
        })?;
    }
    ensure_secret_passphrase_for_encrypted_manifest(&manifest, secret_passphrase)?;
    let common_apis = parse_manifest_common_apis(&manifest, secret_passphrase)?;
    let databases = parse_manifest_common_databases(&manifest, secret_passphrase)?;
    let mut asset_url_context = build_imported_asset_url_context(app_state, &manifest.assets)?;
    let mut warnings = Vec::new();
    if manifest.format_version >= 3 && !manifest.projects.is_empty() {
        // v3 使用项目图结构，先读取所有项目节点，再统一建立源 ID 到新 ID 的映射。
        let project_nodes = read_manifest_project_nodes(archive, &manifest.projects)?;
        let mut tx = begin_import_transaction(db).await?;
        // 文件先写入 staging 目录，数据库提交成功后再发布到正式静态目录。
        let mut file_staging = ImportFileStaging::new(&app_state.static_storage_dir())?;
        let datasource_context = import_common_datasources(&mut tx, &common_apis, &databases).await?;
        import_manifest_v3(
            app_state,
            &mut tx,
            project_id,
            archive,
            manifest.assets,
            project_nodes,
            &datasource_context,
            &mut asset_url_context,
            &mut file_staging,
            &mut warnings,
        )
        .await?;
        finish_import_transaction(tx, &mut file_staging, &mut asset_url_context).await?;
        return Ok(warnings);
    }

    // TODO(legacy-compat): 停止支持旧项目包时，确认当前格式不再使用 v2 后移除此导入分支。
    let data_path = if manifest.project.data_path.trim().is_empty() {
        "project/project.json".to_string()
    } else {
        manifest.project.data_path
    };
    let project_json = read_zip_text(archive, &data_path)?;
    let mut tx = begin_import_transaction(db).await?;
    // v2/兼容路径同样使用 staging，避免导入失败时留下半成品资源文件。
    let mut file_staging = ImportFileStaging::new(&app_state.static_storage_dir())?;
    let datasource_context = import_common_datasources(&mut tx, &common_apis, &databases).await?;
    import_manifest(
        app_state,
        &mut tx,
        project_id,
        archive,
        manifest.assets,
        &project_json,
        &datasource_context,
        &mut asset_url_context,
        &mut file_staging,
        &mut warnings,
    )
    .await?;
    finish_import_transaction(tx, &mut file_staging, &mut asset_url_context).await?;
    Ok(warnings)
}

/// 判断 manifest 是否为包级加密格式。
fn is_encrypted_project_package_manifest(manifest_value: &Value) -> bool {
    manifest_value.get("encrypted").and_then(|value| value.as_bool()) == Some(true)
        && manifest_value
            .get("encryption")
            .and_then(|value| value.get("mode"))
            .and_then(|value| value.as_str())
            == Some("package")
}

/// 解密包级加密项目 payload。
fn decrypt_project_package_payload<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    manifest_value: &Value,
    secret_passphrase: &str,
) -> Result<Vec<u8>, String> {
    if secret_passphrase.trim().is_empty() {
        return Err("请求参数错误：缺少导入口令（该项目包已整体加密）".to_string());
    }
    let algorithm = manifest_value
        .get("encryption")
        .and_then(|value| value.get("algorithm"))
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if algorithm != "passphrase-v1" {
        return Err("导入失败：不支持的项目包加密算法".to_string());
    }
    let payload_path = manifest_value
        .get("encryption")
        .and_then(|value| value.get("payloadPath"))
        .and_then(|value| value.as_str())
        .unwrap_or("payload.enc")
        .trim();
    if payload_path.is_empty() {
        return Err("导入失败：加密项目包缺少 payloadPath".to_string());
    }
    let payload = datavisool_archive::read_zip_bytes(archive, payload_path).map_err(|e| format!("导入失败：读取加密 payload 失败（{e}）"))?;
    if let Some(expected_sha) = manifest_value
        .get("payload")
        .and_then(|value| value.get("sha256"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
    {
        let actual_sha = datavisool_archive::sha256_hex(&payload);
        if actual_sha != expected_sha {
            return Err("导入失败：加密 payload 校验失败".to_string());
        }
    }
    let envelope: EncryptedEnvelopeV1 = serde_json::from_slice(&payload).map_err(|e| format!("导入失败：加密 payload 解析失败（{e}）"))?;
    decrypt_envelope_v1(&envelope, secret_passphrase).map_err(|err| match err {
        CodecError::AesDecrypt => "请求参数错误：导入口令错误".to_string(),
        other => format!("导入失败：解密项目包失败（{other}）"),
    })
}

/// 开启导入事务。
async fn begin_import_transaction(db: &DbState) -> Result<Transaction<'_, Sqlite>, String> {
    db.pool
        .begin()
        .await
        .map_err(|e| format!("导入失败：开启数据库事务失败（{e}）"))
}

/// 提交导入事务并发布暂存文件。
async fn finish_import_transaction(
    tx: Transaction<'_, Sqlite>,
    file_staging: &mut ImportFileStaging,
    asset_url_context: &mut ImportedAssetUrlContext,
) -> Result<(), String> {
    file_staging.commit()?;
    tx.commit().await.map_err(|e| format!("导入失败：提交数据库事务失败（{e}）"))?;
    file_staging.finalize();
    asset_url_context.finalize_component_packages();
    Ok(())
}

/// 导入 v2/兼容格式 manifest。
async fn import_manifest<R: Read + std::io::Seek>(
    app_state: &AppState,
    tx: &mut Transaction<'_, Sqlite>,
    project_id: i64,
    archive: &mut ZipArchive<R>,
    assets: ManifestAssets,
    project_json: &str,
    datasource_context: &ImportedDatasourceContext,
    asset_url_context: &mut ImportedAssetUrlContext,
    file_staging: &mut ImportFileStaging,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    import_fonts(app_state, tx, file_staging, archive, &assets.fonts).await?;
    import_components(app_state, tx, file_staging, archive, &assets.components, asset_url_context).await?;
    import_dynamic_scripts(app_state, tx, file_staging, archive, &assets.dynamic_scripts, project_id, None).await?;
    import_images(app_state, tx, file_staging, archive, &assets.images).await?;
    import_videos(app_state, tx, file_staging, archive, &assets.videos).await?;
    import_models(app_state, tx, file_staging, archive, &assets.models).await?;

    update_project_json(tx, project_id, project_json, datasource_context, None, asset_url_context, warnings).await
}

/// 导入 v3 项目图格式 manifest。
async fn import_manifest_v3<R: Read + std::io::Seek>(
    app_state: &AppState,
    tx: &mut Transaction<'_, Sqlite>,
    target_project_id: i64,
    archive: &mut ZipArchive<R>,
    assets: ManifestAssets,
    project_nodes: Vec<ImportedProjectNode>,
    datasource_context: &ImportedDatasourceContext,
    asset_url_context: &mut ImportedAssetUrlContext,
    file_staging: &mut ImportFileStaging,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    import_fonts(app_state, tx, file_staging, archive, &assets.fonts).await?;
    import_components(app_state, tx, file_staging, archive, &assets.components, asset_url_context).await?;
    import_images(app_state, tx, file_staging, archive, &assets.images).await?;
    import_videos(app_state, tx, file_staging, archive, &assets.videos).await?;
    import_models(app_state, tx, file_staging, archive, &assets.models).await?;

    let entry_source_project_id = project_nodes
        .iter()
        .find(|node| node.is_entry)
        .map(|node| node.source_project_id.clone())
        .ok_or_else(|| "导入失败：导出包缺少入口项目节点".to_string())?;

    let node_map: HashMap<String, ImportedProjectNode> = project_nodes
        .iter()
        .cloned()
        .map(|node| (node.source_project_id.clone(), node))
        .collect();

    let project_id_map = build_import_project_id_map(tx, target_project_id, &entry_source_project_id, &node_map).await?;

    rewrite_and_persist_project_graph_json(tx, &project_nodes, &project_id_map, datasource_context, asset_url_context, warnings).await?;
    import_dynamic_scripts(app_state, tx, file_staging, archive, &assets.dynamic_scripts, target_project_id, Some(&project_id_map)).await
}

/// 更新目标项目的 project_json。
async fn update_project_json(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: i64,
    project_json: &str,
    datasource_context: &ImportedDatasourceContext,
    project_id_map: Option<&HashMap<String, i64>>,
    asset_url_context: &ImportedAssetUrlContext,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let normalized_project_json = normalize_imported_project_json(project_json)?;
    let mut project_warnings = Vec::new();
    let final_project_json = rewrite_imported_project_json(&normalized_project_json, datasource_context, project_id_map, asset_url_context, &mut project_warnings)?;
    warnings.extend(project_warnings.into_iter().map(|warning| format!("项目 {project_id}：{warning}")));
    let changed = sqlx::query("UPDATE project SET data_json = ?2 WHERE id = ?1 AND deleted = 0")
        .bind(project_id)
        .bind(final_project_json)
        .execute(&mut **tx)
        .await
        .map_err(|e| format!("导入失败：更新项目数据失败（{e}）"))?
        .rows_affected();
    if changed == 0 {
        return Err("导入失败：目标项目不存在或已删除".to_string());
    }
    Ok(())
}

/// 更新导入项目的父项目 id。
async fn update_project_pid(tx: &mut Transaction<'_, Sqlite>, project_id: i64, pid: Option<i64>) -> Result<(), String> {
    let changed = sqlx::query("UPDATE project SET pid = ?2 WHERE id = ?1 AND deleted = 0")
        .bind(project_id)
        .bind(pid)
        .execute(&mut **tx)
        .await
        .map_err(|e| format!("导入失败：更新项目层级失败（{e}）"))?
        .rows_affected();
    if changed == 0 {
        return Err("导入失败：目标项目不存在或已删除".to_string());
    }
    Ok(())
}

/// 创建导入得到的新项目记录。
async fn create_imported_project(tx: &mut Transaction<'_, Sqlite>, name: &str, pid: Option<i64>) -> Result<i64, String> {
    let name_to_store = if name.trim().is_empty() {
        "未命名项目".to_string()
    } else {
        name.trim().to_string()
    };

    let result = sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES (?1, '', '0', ?2, '{}', '', 0)")
        .bind(name_to_store)
        .bind(pid)
        .execute(&mut **tx)
        .await
        .map_err(|e| format!("导入失败：创建项目失败（{e}）"))?;

    Ok(result.last_insert_rowid())
}

/// 建立导入源项目 ID 到当前数据库项目 ID 的映射。
async fn build_import_project_id_map(
    tx: &mut Transaction<'_, Sqlite>,
    target_project_id: i64,
    entry_source_project_id: &str,
    node_map: &HashMap<String, ImportedProjectNode>,
) -> Result<HashMap<String, i64>, String> {
    let mut project_id_map = HashMap::new();
    project_id_map.insert(entry_source_project_id.to_string(), target_project_id);

    let mut remaining: HashSet<String> = node_map
        .keys()
        .filter(|source_project_id| source_project_id.as_str() != entry_source_project_id)
        .cloned()
        .collect();

    while !remaining.is_empty() {
        let mut progress = false;
        let current_round: Vec<String> = remaining.iter().cloned().collect();
        for source_project_id in current_round {
            let Some(node) = node_map.get(&source_project_id) else {
                return Err(format!("导入失败：项目节点缺失（{source_project_id}）"));
            };

            let parent_project_id = match node.source_pid.as_deref() {
                Some(parent_source_project_id) => match project_id_map.get(parent_source_project_id) {
                    Some(project_id) => Some(*project_id),
                    None => continue,
                },
                None => None,
            };

            let new_project_id = create_imported_project(tx, &node.name, parent_project_id).await?;
            project_id_map.insert(source_project_id.clone(), new_project_id);
            remaining.remove(&source_project_id);
            progress = true;
        }

        if !progress {
            let unresolved: Vec<String> = remaining.into_iter().collect();
            return Err(format!("导入失败：项目结构不完整，无法恢复这些项目：{}", unresolved.join(", ")));
        }
    }

    let entry_parent_project_id = node_map
        .get(entry_source_project_id)
        .and_then(|node| node.source_pid.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|source_project_id| {
            project_id_map
                .get(source_project_id)
                .copied()
                .ok_or_else(|| format!("导入失败：入口项目父级缺失（{source_project_id}）"))
        })
        .transpose()?;

    update_project_pid(tx, target_project_id, entry_parent_project_id).await?;
    Ok(project_id_map)
}

/// 重写项目图中的跨项目引用并持久化到数据库。
async fn rewrite_and_persist_project_graph_json(
    tx: &mut Transaction<'_, Sqlite>,
    project_nodes: &[ImportedProjectNode],
    project_id_map: &HashMap<String, i64>,
    datasource_context: &ImportedDatasourceContext,
    asset_url_context: &ImportedAssetUrlContext,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    for node in project_nodes {
        let target_project_id = *project_id_map
            .get(&node.source_project_id)
            .ok_or_else(|| format!("导入失败：缺少项目映射（{}）", node.source_project_id))?;
        update_project_json(tx, target_project_id, &node.project_json, datasource_context, Some(project_id_map), asset_url_context, warnings).await?;
    }
    Ok(())
}

/// 校验加密 manifest 是否提供了导入口令。
fn ensure_secret_passphrase_for_encrypted_manifest(manifest: &ExportManifest, secret_passphrase: &str) -> Result<(), String> {
    // TODO(legacy-compat): 移除历史字段级加密支持时，同步移除此检查；保留整包加密检查。
    let needs_passphrase = matches!(manifest.common_datasource.common_apis, datavisool_project_format::EncryptedOrPlain::Encrypted(_))
        || matches!(manifest.common_datasource.databases, datavisool_project_format::EncryptedOrPlain::Encrypted(_));

    if needs_passphrase && secret_passphrase.trim().is_empty() {
        return Err("请求参数错误：缺少导入口令（该项目包包含加密数据源）".to_string());
    }

    Ok(())
}

/// 解析 manifest 中的公共 API 引用。
fn parse_manifest_common_apis(manifest: &ExportManifest, secret_passphrase: &str) -> Result<Vec<CommonApiRef>, String> {
    let common_apis = match read_manifest_common_apis_field(manifest) {
        ManifestCommonApisField::Plain(common_apis) => common_apis,
        ManifestCommonApisField::Encrypted(payload) => {
            let decrypted = decrypt_import_secret_payload(payload, secret_passphrase, "commonApis")?;
            let parsed: Vec<CommonApiRef> = serde_json::from_slice(&decrypted).map_err(|e| format!("导入失败：解析 commonApis 失败（{e}）"))?;
            return Ok(normalize_common_apis(&parsed));
        }
    };

    Ok(normalize_common_apis(common_apis))
}
