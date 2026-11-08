/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
/// 归一化公共 API 引用列表，去掉空引用和重复 refKey。
fn normalize_common_apis(common_apis: &[CommonApiRef]) -> Vec<CommonApiRef> {
    let mut seen_origin_ids = HashSet::<String>::new();
    let mut out = Vec::<CommonApiRef>::new();

    for item in common_apis {
        let origin_id = item.origin_id.trim().to_string();
        if origin_id.is_empty() {
            continue;
        }
        if !seen_origin_ids.insert(origin_id.clone()) {
            continue;
        }

        out.push(CommonApiRef {
            origin_id,
            ref_key: item.ref_key.trim().to_string(),
            name: item.name.trim().to_string(),
            url: item.url.trim().to_string(),
            method: item.method.trim().to_string(),
            params: item.params.trim().to_string(),
            headers: item.headers.trim().to_string(),
        });
    }

    out
}

/// manifest 中公共 API 字段的明文/加密形态。
// TODO(legacy-compat): 停止支持历史字段级加密项目包后，移除 Encrypted 分支及其解密调用。
enum ManifestCommonApisField<'a> {
    Plain(&'a [CommonApiRef]),
    Encrypted(&'a str),
}

/// 读取 manifest 中公共 API 字段的实际形态。
fn read_manifest_common_apis_field(manifest: &ExportManifest) -> ManifestCommonApisField<'_> {
    match &manifest.common_datasource.common_apis {
        datavisool_project_format::EncryptedOrPlain::Plain(common_apis) => ManifestCommonApisField::Plain(common_apis.as_slice()),
        datavisool_project_format::EncryptedOrPlain::Encrypted(payload) => ManifestCommonApisField::Encrypted(payload.trim()),
    }
}

/// 解析 manifest 中的公共数据库引用。
fn parse_manifest_common_databases(manifest: &ExportManifest, secret_passphrase: &str) -> Result<Vec<CommonDatabaseRef>, String> {
    let databases = match read_manifest_common_databases_field(manifest) {
        ManifestCommonDatabasesField::Plain(databases) => databases,
        ManifestCommonDatabasesField::Encrypted(payload) => {
            let decrypted = decrypt_import_secret_payload(payload, secret_passphrase, "databases")?;
            let parsed: Vec<CommonDatabaseRef> = serde_json::from_slice(&decrypted).map_err(|e| format!("导入失败：解析 databases 失败（{e}）"))?;
            return Ok(normalize_common_databases(&parsed));
        }
    };

    Ok(normalize_common_databases(databases))
}

/// 归一化公共数据库引用列表，去掉空引用和重复 refKey。
fn normalize_common_databases(databases: &[CommonDatabaseRef]) -> Vec<CommonDatabaseRef> {
    let mut seen_origin_ids = HashSet::<String>::new();
    let mut out = Vec::<CommonDatabaseRef>::new();

    for item in databases {
        let origin_id = item.origin_id.trim().to_string();
        if origin_id.is_empty() {
            continue;
        }
        if !seen_origin_ids.insert(origin_id.clone()) {
            continue;
        }

        out.push(CommonDatabaseRef {
            origin_id,
            ref_key: item.ref_key.trim().to_string(),
            ty: item.ty.trim().to_string(),
            ip: item.ip.trim().to_string(),
            port: item.port.trim().to_string(),
            database_name: item.database_name.trim().to_string(),
            username: item.username.trim().to_string(),
            password: item.password.trim().to_string(),
            url: item.url.trim().to_string(),
            des: item.des.trim().to_string(),
            allow_sql_script: item.allow_sql_script,
        });
    }

    out
}

/// manifest 中公共数据库字段的明文/加密形态。
// TODO(legacy-compat): 停止支持历史字段级加密项目包后，移除 Encrypted 分支及其解密调用。
enum ManifestCommonDatabasesField<'a> {
    Plain(&'a [CommonDatabaseRef]),
    Encrypted(&'a str),
}

/// 读取 manifest 中公共数据库字段的实际形态。
fn read_manifest_common_databases_field(manifest: &ExportManifest) -> ManifestCommonDatabasesField<'_> {
    match &manifest.common_datasource.databases {
        datavisool_project_format::EncryptedOrPlain::Plain(databases) => ManifestCommonDatabasesField::Plain(databases.as_slice()),
        datavisool_project_format::EncryptedOrPlain::Encrypted(payload) => ManifestCommonDatabasesField::Encrypted(payload.trim()),
    }
}

/// 解密 manifest 字段级加密 payload。
fn decrypt_import_secret_payload(payload: &str, secret_passphrase: &str, field_name: &str) -> Result<Vec<u8>, String> {
    decrypt_with_passphrase_v1(payload, secret_passphrase).map_err(|err| match err {
        CodecError::AesDecrypt => "请求参数错误：导入口令错误".to_string(),
        other => format!("导入失败：解密 {field_name} 失败（{other}）"),
    })
}

/// 导入公共 API 和公共数据库，并返回新旧引用映射。
async fn import_common_datasources(
    tx: &mut Transaction<'_, Sqlite>,
    common_apis: &[CommonApiRef],
    databases: &[CommonDatabaseRef],
) -> Result<ImportedDatasourceContext, String> {
    let common_api_id_map = import_common_apis(tx, common_apis).await?;
    let database_id_map = import_common_databases(tx, databases).await?;

    Ok(ImportedDatasourceContext {
        common_api_ref_key_map: build_common_api_ref_key_map(common_apis, &common_api_id_map),
        database_ref_key_map: build_common_database_ref_key_map(databases, &database_id_map),
        common_api_id_map,
        database_id_map,
    })
}

/// 导入公共 API。
///
/// 优先按 refKey 匹配已有记录，缺失时新增，再建立 originId/refKey 到新 ID 的映射。
async fn import_common_apis(tx: &mut Transaction<'_, Sqlite>, common_apis: &[CommonApiRef]) -> Result<HashMap<String, i64>, String> {
    let mut mapping = HashMap::<String, i64>::new();

    for item in common_apis {
        let origin_id = item.origin_id.trim().to_string();
        if origin_id.is_empty() {
            continue;
        }

        let url = item.url.trim().to_string();
        let method = item.method.trim().to_string();
        let params = item.params.trim().to_string();
        let headers = item.headers.trim().to_string();
        let name = item.name.trim().to_string();
        let ref_key = item.ref_key.trim().to_string();

        let existing_id = if !ref_key.is_empty() {
            sqlx::query("SELECT id FROM common_api WHERE deleted = 0 AND ref_key = ?1 LIMIT 1")
                .bind(&ref_key)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| format!("导入失败：按 refKey 查询公共API失败（{e}）"))?
        } else {
            sqlx::query(
                "SELECT id FROM common_api \
                 WHERE deleted = 0 AND url = ?1 AND method = ?2 AND COALESCE(params, '') = ?3 AND COALESCE(headers, '') = ?4 \
                 LIMIT 1",
            )
            .bind(&url)
            .bind(&method)
            .bind(&params)
            .bind(&headers)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| format!("导入失败：查询公共API失败（{e}）"))?
        };
        if let Some(row) = existing_id {
            let desktop_id: i64 = row.try_get("id").map_err(|e| format!("导入失败：读取公共API失败（{e}）"))?;
            if !ref_key.is_empty() {
                sqlx::query(
                    "UPDATE common_api \
                     SET name = ?2, url = ?3, method = ?4, params = ?5, headers = ?6, ref_key = ?7 \
                     WHERE id = ?1 AND deleted = 0",
                )
                .bind(desktop_id)
                .bind(&name)
                .bind(&url)
                .bind(&method)
                .bind(&params)
                .bind(&headers)
                .bind(&ref_key)
                .execute(&mut **tx)
                .await
                .map_err(|e| format!("导入失败：按 refKey 更新公共API失败（{e}）"))?;
            }
            mapping.insert(origin_id, desktop_id);
            continue;
        }

        let desktop_id =
            sqlx::query("INSERT INTO common_api (name, url, method, params, headers, ref_key, deleted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)")
                .bind(name)
                .bind(url)
                .bind(method)
                .bind(params)
                .bind(headers)
                .bind(if ref_key.is_empty() { None } else { Some(ref_key) })
                .execute(&mut **tx)
                .await
                .map_err(|e| format!("导入失败：保存公共API失败（{e}）"))?
                .last_insert_rowid();
        mapping.insert(origin_id, desktop_id);
    }

    Ok(mapping)
}

/// 导入公共数据库。
///
/// 优先按 refKey 匹配已有记录，缺失时新增，再建立 originId/refKey 到新 ID 的映射。
async fn import_common_databases(tx: &mut Transaction<'_, Sqlite>, databases: &[CommonDatabaseRef]) -> Result<HashMap<String, i64>, String> {
    let mut mapping = HashMap::<String, i64>::new();

    for item in databases {
        let origin_id = item.origin_id.trim().to_string();
        if origin_id.is_empty() {
            continue;
        }

        let ty = item.ty.trim().to_string();
        let ip = item.ip.trim().to_string();
        let port = item.port.trim().to_string();
        let database_name = item.database_name.trim().to_string();
        let username = item.username.trim().to_string();
        let password = item.password.trim().to_string();
        let url = item.url.trim().to_string();
        let des = item.des.trim().to_string();
        let ref_key = item.ref_key.trim().to_string();
        let allow_sql_script = item.allow_sql_script;
        let name = if database_name.is_empty() {
            origin_id.to_string()
        } else {
            database_name.clone()
        };

        let existing_id = if !ref_key.is_empty() {
            sqlx::query("SELECT id FROM common_database WHERE deleted = 0 AND ref_key = ?1 LIMIT 1")
                .bind(&ref_key)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| format!("导入失败：按 refKey 查询数据库数据源失败（{e}）"))?
        } else {
            sqlx::query(
                "SELECT id FROM common_database \
                 WHERE deleted = 0 AND type = ?1 AND ip = ?2 AND port = ?3 AND database_name = ?4 \
                 AND username = ?5 AND password = ?6 AND COALESCE(url, '') = ?7 \
                 LIMIT 1",
            )
            .bind(&ty)
            .bind(&ip)
            .bind(&port)
            .bind(&database_name)
            .bind(&username)
            .bind(&password)
            .bind(&url)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| format!("导入失败：查询数据库数据源失败（{e}）"))?
        };
        if let Some(row) = existing_id {
            let desktop_id: i64 = row
                .try_get("id")
                .map_err(|e| format!("导入失败：读取数据库数据源失败（{e}）"))?;
            if !ref_key.is_empty() {
                sqlx::query(
                    "UPDATE common_database \
                     SET name = ?2, type = ?3, ip = ?4, port = ?5, database_name = ?6, \
                         username = ?7, password = ?8, url = ?9, des = ?10, ref_key = ?11, \
                         allow_sql_script = ?12, url_is_custom = CASE WHEN ?9 = '' THEN 0 ELSE 1 END \
                     WHERE id = ?1 AND deleted = 0",
                )
                .bind(desktop_id)
                .bind(&name)
                .bind(&ty)
                .bind(&ip)
                .bind(&port)
                .bind(&database_name)
                .bind(&username)
                .bind(&password)
                .bind(&url)
                .bind(&des)
                .bind(&ref_key)
                .bind(bool_to_db_i64(allow_sql_script))
                .execute(&mut **tx)
                .await
                .map_err(|e| format!("导入失败：按 refKey 更新数据库数据源失败（{e}）"))?;
            }
            mapping.insert(origin_id, desktop_id);
            continue;
        }

        let desktop_id = sqlx::query(
            "INSERT INTO common_database (name, type, ip, port, database_name, username, password, url, des, ref_key, allow_sql_script, url_is_custom, deleted) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, CASE WHEN ?8 = '' THEN 0 ELSE 1 END, 0)",
        )
        .bind(name)
        .bind(ty)
        .bind(ip)
        .bind(port)
        .bind(database_name)
        .bind(username)
        .bind(password)
        .bind(url)
        .bind(des)
        .bind(if ref_key.is_empty() { None } else { Some(ref_key) })
        .bind(bool_to_db_i64(allow_sql_script))
        .execute(&mut **tx)
        .await
        .map_err(|e| format!("导入失败：保存数据库数据源失败（{e}）"))?
        .last_insert_rowid();
        mapping.insert(origin_id, desktop_id);
    }

    Ok(mapping)
}

/// 将布尔配置转换为 SQLite 中使用的 0/1。
fn bool_to_db_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

/// 建立公共 API refKey 到当前数据库 ID 的映射。
fn build_common_api_ref_key_map(common_apis: &[CommonApiRef], common_api_id_map: &HashMap<String, i64>) -> HashMap<String, i64> {
    let mut map = HashMap::new();
    for item in common_apis {
        let origin_id = item.origin_id.trim();
        let ref_key = item.ref_key.trim();
        if origin_id.is_empty() || ref_key.is_empty() {
            continue;
        }
        if let Some(desktop_id) = common_api_id_map.get(origin_id) {
            map.insert(ref_key.to_string(), *desktop_id);
        }
    }
    map
}

/// 建立公共数据库 refKey 到当前数据库 ID 的映射。
fn build_common_database_ref_key_map(databases: &[CommonDatabaseRef], database_id_map: &HashMap<String, i64>) -> HashMap<String, i64> {
    let mut map = HashMap::new();
    for item in databases {
        let origin_id = item.origin_id.trim();
        let ref_key = item.ref_key.trim();
        if origin_id.is_empty() || ref_key.is_empty() {
            continue;
        }
        if let Some(desktop_id) = database_id_map.get(origin_id) {
            map.insert(ref_key.to_string(), *desktop_id);
        }
    }
    map
}

/// 重写导入项目 JSON 中的资源 URL 和公共数据源引用。
fn rewrite_imported_project_json(
    project_json: &str,
    datasource_context: &ImportedDatasourceContext,
    project_id_map: Option<&HashMap<String, i64>>,
    asset_url_context: &ImportedAssetUrlContext,
    warnings: &mut Vec<String>,
) -> Result<String, String> {

    let mut value: Value = serde_json::from_str(project_json).map_err(|e| format!("导入失败：项目 JSON 解析失败（{}）", e))?;
    let mut path = Vec::<String>::new();
    rewrite_imported_project_value(&mut value, &mut path, datasource_context, project_id_map, asset_url_context, warnings)?;
    serde_json::to_string(&value).map_err(|e| format!("导入失败：项目 JSON 序列化失败（{e}）"))
}

/// 递归重写导入项目 JSON 值。
fn rewrite_imported_project_value(
    value: &mut Value,
    path: &mut Vec<String>,
    datasource_context: &ImportedDatasourceContext,
    project_id_map: Option<&HashMap<String, i64>>,
    asset_url_context: &ImportedAssetUrlContext,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                path.push(index.to_string());
                rewrite_imported_project_value(item, path, datasource_context, project_id_map, asset_url_context, warnings)?;
                path.pop();
            }
        }
        Value::Object(map) => {
            let keys = map.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                path.push(key.clone());
                let is_component_data = is_component_data_path(path);
                let is_global_variable_trigger_source = is_global_variable_trigger_source_path(path);
                let is_gis_data_config = is_gis_data_config_path(path, map);
                let is_screen_reference = is_screen_reference_style_path(path, map);
                let Some(val) = map.get_mut(&key) else {
                    path.pop();
                    continue;
                };
                if is_component_data || is_global_variable_trigger_source || is_gis_data_config {
                    if rewrite_datasource_reference(val, "commonApi", "commonApi", "commonApiId", &datasource_context.common_api_id_map, &datasource_context.common_api_ref_key_map) {
                        warnings.push(format!("公共 API：{}", path.join(".")));
                    }
                    if rewrite_datasource_reference(val, "database", "database", "targetDb", &datasource_context.database_id_map, &datasource_context.database_ref_key_map) {
                        warnings.push(format!("数据库：{}", path.join(".")));
                    }
                }
                if is_screen_reference {
                    rewrite_screen_reference_project_id(val, project_id_map)?;
                }
                rewrite_imported_project_value(val, path, datasource_context, project_id_map, asset_url_context, warnings)?;
                path.pop();
            }
        }
        Value::String(text) => {
            if let Some(rewritten) = asset_url_context.rewrite_static_url(text) {
                *text = rewritten;
            }
        }
        _ => {}
    }
    Ok(())
}
