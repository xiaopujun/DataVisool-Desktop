/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
    use super::*;
    use crate::app_state::AppState;
    use crate::db::DbState;
    use datavisool_archive::{build_export_zip, ZipEntryBytes};
    use datavisool_codec::encrypt_with_passphrase_v1;
    use datavisool_project_format::{
        CommonApiRef, CommonDatabaseRef, EncryptedOrPlain, ExportManifest, ManifestAssets, ManifestCommonDataSources, ManifestProject,
        ManifestProjectNode,
    };
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::Row;
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    async fn build_test_env() -> (DbState, AppState) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory failed");

        datavisool_db::run_base_migrations(&pool).await.expect("run base migrations failed");
        datavisool_db::run_custom_migrations(&pool, sqlx::migrate!("./migrations_desktop"))
            .await
            .expect("run desktop migrations failed");

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("datavisool-import-route-test-{unique}"));
        std::fs::create_dir_all(root.join("static")).expect("create temp static dir failed");
        let app_state = AppState::new_for_test(root.clone(), root.clone());

        (DbState { pool }, app_state)
    }

    fn write_zip_to_temp(zip_bytes: &[u8], name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{name}-{unique}.zip"));
        std::fs::write(&path, zip_bytes).expect("write zip file failed");
        path
    }

    #[test]
    fn create_unique_import_staging_dir_adds_suffix_when_base_exists() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("datavisool-import-staging-test-{unique}"));
        std::fs::create_dir_all(&root).expect("create temp root failed");
        let base = ".import-staging-20260515123456789";
        std::fs::create_dir(root.join(base)).expect("create base staging dir failed");

        let staging_dir = create_unique_import_staging_dir_with_base(&root, base).expect("create suffixed staging dir failed");

        assert_eq!(staging_dir.file_name().and_then(|name| name.to_str()), Some(".import-staging-20260515123456789-1"));
        assert!(staging_dir.is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    fn build_encrypted_package_zip(inner_zip: &[u8], passphrase: &str) -> Vec<u8> {
        build_encrypted_package_zip_with_format(inner_zip, passphrase, "light-chaser-project")
    }

    fn build_encrypted_package_zip_with_format(inner_zip: &[u8], passphrase: &str, format: &str) -> Vec<u8> {
        let envelope = datavisool_codec::encrypt_envelope_v1(inner_zip, passphrase).expect("encrypt inner zip failed");
        let payload = serde_json::to_vec(&envelope).expect("serialize encrypted payload failed");
        let manifest = serde_json::json!({
            "format": format,
            "formatVersion": 4,
            "generator": "test",
            "generatedAtMs": 0,
            "encrypted": true,
            "encryption": {
                "mode": "package",
                "algorithm": "passphrase-v1",
                "payloadPath": "payload.enc"
            },
            "payload": {
                "path": "payload.enc",
                "sha256": datavisool_archive::sha256_hex(&payload),
                "size": payload.len()
            }
        });
        let cursor = Cursor::new(Vec::<u8>::new());
        let mut zip = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("manifest.json", options)
            .expect("start encrypted manifest failed");
        zip.write_all(&serde_json::to_vec(&manifest).expect("serialize encrypted manifest failed"))
            .expect("write encrypted manifest failed");
        zip.start_file("payload.enc", options).expect("start encrypted payload failed");
        zip.write_all(&payload).expect("write encrypted payload failed");
        zip.finish().expect("finish encrypted package failed").into_inner()
    }

    fn build_v2_manifest() -> ExportManifest {
        ExportManifest {
            format: "light-chaser-project".to_string(),
            format_version: 2,
            generator: "test".to_string(),
            generated_at_ms: 0,
            project: ManifestProject {
                id: "entry".to_string(), name: "入口项目".to_string(), data_path: "project/project.json".to_string()
            },
            projects: vec![],
            assets: ManifestAssets::default(),
            common_datasource: ManifestCommonDataSources::default(),
            files: vec![],
            warnings_path: None,
        }
    }

    fn build_v2_manifest_with_common_datasource(
        common_apis: EncryptedOrPlain<Vec<CommonApiRef>>,
        databases: EncryptedOrPlain<Vec<CommonDatabaseRef>>,
    ) -> ExportManifest {
        let mut manifest = build_v2_manifest();
        manifest.common_datasource = ManifestCommonDataSources { common_apis, databases };
        manifest
    }

    fn build_v3_manifest(dynamic_scripts: Vec<DynamicScriptRef>) -> ExportManifest {
        build_v3_manifest_with_projects(
            "A1",
            vec![
                manifest_project_node("A", None, false),
                manifest_project_node("A1", Some("A"), true),
                manifest_project_node("A1C", Some("A1"), false),
                manifest_project_node("B", None, false),
                manifest_project_node("B1", Some("B"), false),
            ],
            dynamic_scripts,
        )
    }

    fn manifest_project_node(source_project_id: &str, source_pid: Option<&str>, is_entry: bool) -> ManifestProjectNode {
        ManifestProjectNode {
            source_project_id: source_project_id.to_string(),
            name: source_project_id.to_string(),
            source_pid: source_pid.map(ToString::to_string),
            data_path: format!("projects/{source_project_id}/project.json"),
            is_entry,
        }
    }

    fn build_v3_manifest_with_projects(
        entry_project_id: &str,
        projects: Vec<ManifestProjectNode>,
        dynamic_scripts: Vec<DynamicScriptRef>,
    ) -> ExportManifest {
        let entry_project = projects
            .iter()
            .find(|item| item.source_project_id == entry_project_id)
            .expect("entry project should exist in manifest projects");
        ExportManifest {
            format: "light-chaser-project".to_string(),
            format_version: 3,
            generator: "test".to_string(),
            generated_at_ms: 0,
            project: ManifestProject {
                id: entry_project.source_project_id.clone(),
                name: entry_project.name.clone(),
                data_path: entry_project.data_path.clone(),
            },
            projects,
            assets: ManifestAssets { dynamic_scripts, ..ManifestAssets::default() },
            common_datasource: ManifestCommonDataSources::default(),
            files: vec![],
            warnings_path: None,
        }
    }

    fn build_v3_manifest_with_common_datasource(
        entry_project_id: &str,
        projects: Vec<ManifestProjectNode>,
        common_apis: EncryptedOrPlain<Vec<CommonApiRef>>,
        databases: EncryptedOrPlain<Vec<CommonDatabaseRef>>,
    ) -> ExportManifest {
        let mut manifest = build_v3_manifest_with_projects(entry_project_id, projects, vec![]);
        manifest.common_datasource = ManifestCommonDataSources { common_apis, databases };
        manifest
    }

    fn encrypt_common_apis_payload(common_apis: &[CommonApiRef], passphrase: &str) -> String {
        let payload = serde_json::to_vec(common_apis).expect("serialize commonApis failed");
        encrypt_with_passphrase_v1(&payload, passphrase).expect("encrypt commonApis failed")
    }

    fn encrypt_databases_payload(databases: &[CommonDatabaseRef], passphrase: &str) -> String {
        let payload = serde_json::to_vec(databases).expect("serialize databases failed");
        encrypt_with_passphrase_v1(&payload, passphrase).expect("encrypt databases failed")
    }

    fn build_v3_graph_zip(manifest: &ExportManifest, project_jsons: &[(&str, &str)], extra_entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut owned_entries: Vec<(String, Vec<u8>)> = project_jsons
            .iter()
            .map(|(project_id, project_json)| (format!("projects/{project_id}/project.json"), project_json.as_bytes().to_vec()))
            .collect();
        owned_entries.extend(extra_entries.iter().map(|(path, bytes)| (path.to_string(), bytes.to_vec())));
        let zip_entries: Vec<ZipEntryBytes<'_>> = owned_entries
            .iter()
            .map(|(path, bytes)| ZipEntryBytes { path: path.as_str(), bytes: bytes.as_slice() })
            .collect();
        build_export_zip(manifest, zip_entries, &[]).expect("build v3 zip failed")
    }

    async fn load_imported_project_graph(db: &DbState) -> (HashMap<String, i64>, HashMap<String, Option<i64>>, HashMap<String, String>) {
        let project_rows = sqlx::query("SELECT id, name, pid, data_json FROM project WHERE deleted = 0 ORDER BY name ASC")
            .fetch_all(&db.pool)
            .await
            .expect("query projects failed");

        let mut project_ids = HashMap::new();
        let mut project_pids = HashMap::new();
        let mut project_jsons = HashMap::new();
        for row in project_rows {
            let id: i64 = row.try_get("id").expect("read project id failed");
            let name: String = row.try_get("name").expect("read project name failed");
            let pid: Option<i64> = row.try_get("pid").expect("read project pid failed");
            let data_json: String = row.try_get("data_json").expect("read project data failed");
            project_ids.insert(name.clone(), id);
            project_pids.insert(name.clone(), pid);
            project_jsons.insert(name, data_json);
        }

        (project_ids, project_pids, project_jsons)
    }

    #[tokio::test]
    async fn import_project_zip_impl_keeps_v2_manifest_compatible() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let project_json = r#"{"layerManager":{"elemConfigs":{"screen":{"base":{"type":"ScreenReference"},"style":{"screenId":"9"}}}}}"#;
        let manifest = build_v2_manifest();
        let zip_bytes = build_export_zip(&manifest, [ZipEntryBytes { path: "project/project.json", bytes: project_json.as_bytes() }], &[])
            .expect("build v2 zip failed");
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-v2");

        import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "")
            .await
            .expect("import v2 project failed");

        let row = sqlx::query("SELECT pid, data_json FROM project WHERE id = ?1")
            .bind(target_project_id)
            .fetch_one(&db.pool)
            .await
            .expect("query imported target failed");
        let pid: Option<i64> = row.try_get("pid").expect("read pid failed");
        let data_json: String = row.try_get("data_json").expect("read data json failed");
        assert_eq!(pid, None);
        assert_eq!(data_json, project_json);

        let total_projects: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM project WHERE deleted = 0")
            .fetch_one(&db.pool)
            .await
            .expect("count projects failed");
        assert_eq!(total_projects, 1);
    }

    #[tokio::test]
    async fn imports_current_plain_and_encrypted_packages() {
        let (db, app_state) = build_test_env().await;
        let target = sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('target', '', '0', NULL, '{}', '', 0)")
            .execute(&db.pool).await.unwrap().last_insert_rowid();
        let data = r#"{"layerManager":{"elemConfigs":{}}}"#;
        let mut manifest = build_v2_manifest();
        manifest.format = datavisool_project_format::PROJECT_FORMAT.to_string();
        let inner = build_export_zip(&manifest, [ZipEntryBytes {path: "project/project.json", bytes: data.as_bytes()}], &[]).unwrap();
        let encrypted = build_encrypted_package_zip_with_format(&inner, "test-passphrase", datavisool_project_format::PROJECT_FORMAT);
        for (name, bytes, passphrase) in [("current-plain", inner, ""), ("current-encrypted", encrypted, "test-passphrase")] {
            let path = write_zip_to_temp(&bytes, name);
            import_project_zip_impl(&db, &app_state, target, path.to_str().unwrap(), passphrase).await.unwrap();
            let saved: String = sqlx::query_scalar("SELECT data_json FROM project WHERE id = ?1").bind(target).fetch_one(&db.pool).await.unwrap();
            assert_eq!(saved, data);
            let _ = std::fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn import_project_zip_impl_accepts_encrypted_package_and_keeps_plain_package_compatible() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let project_json = r#"{"layerManager":{"elemConfigs":{}}}"#;
        let manifest = build_v2_manifest_with_common_datasource(
            EncryptedOrPlain::Plain(vec![CommonApiRef {
                origin_id: "api-1".to_string(),
                ref_key: "common-api-ref-key-1".to_string(),
                name: "加密包 API".to_string(),
                url: "https://example.com/secure".to_string(),
                method: "GET".to_string(),
                params: "{}".to_string(),
                headers: "{\"Authorization\":\"Bearer token\"}".to_string(),
            }]),
            EncryptedOrPlain::Plain(vec![]),
        );
        let inner_zip = build_export_zip(&manifest, [ZipEntryBytes { path: "project/project.json", bytes: project_json.as_bytes() }], &[])
            .expect("build inner zip failed");
        let encrypted_zip = build_encrypted_package_zip(&inner_zip, "package-secret");
        let zip_path = write_zip_to_temp(&encrypted_zip, "desktop-import-encrypted-package");

        let missing = import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "")
            .await
            .expect_err("encrypted package should require passphrase");
        assert_eq!(missing, "请求参数错误：缺少导入口令（该项目包已整体加密）");

        import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "package-secret")
            .await
            .expect("import encrypted package failed");

        let data_json: String = sqlx::query_scalar("SELECT data_json FROM project WHERE id = ?1")
            .bind(target_project_id)
            .fetch_one(&db.pool)
            .await
            .expect("query imported project json failed");
        assert_eq!(data_json, project_json);

        let imported_api_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM common_api WHERE deleted = 0 AND ref_key = 'common-api-ref-key-1'")
            .fetch_one(&db.pool)
            .await
            .expect("count imported api failed");
        assert_eq!(imported_api_count, 1);
    }

    #[tokio::test]
    async fn import_project_zip_impl_rewrites_web_static_urls_to_asset_urls() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let project_json = serde_json::json!({
            "layerManager": {
                "elemConfigs": {
                    "img": {
                        "base": {"type": "BaseImage"},
                        "style": {"type": "local", "localUrl": "/static/images/image.png"}
                    },
                    "video": {
                        "base": {"type": "BaseVideo"},
                        "style": {"src": "/static/videos/video.mp4"}
                    },
                    "model": {
                        "base": {"type": "ThreeModel"},
                        "style": {"source": {"url": "/static/models/hash001/scene.gltf"}}
                    }
                }
            }
        })
        .to_string();

        let mut manifest = build_v2_manifest();
        manifest.assets.images =
            vec![AssetBlobRef { name: "image".to_string(), original: "image.png".to_string(), blob: "blobs/image.png".to_string() }];
        manifest.assets.videos =
            vec![AssetBlobRef { name: "video".to_string(), original: "video.mp4".to_string(), blob: "blobs/video.mp4".to_string() }];
        manifest.assets.models = vec![ModelPackageRef {
            name: "scene".to_string(),
            original: "hash001/scene.gltf".to_string(),
            package_id: "hash001".to_string(),
            entry: "scene.gltf".to_string(),
            files: vec![
                datavisool_project_format::ModelFileRef { path: "scene.gltf".to_string(), blob: "models/hash001/scene.gltf".to_string() },
                datavisool_project_format::ModelFileRef { path: "scene.bin".to_string(), blob: "models/hash001/scene.bin".to_string() },
            ],
        }];

        let zip_bytes = build_export_zip(
            &manifest,
            [
                ZipEntryBytes { path: "project/project.json", bytes: project_json.as_bytes() },
                ZipEntryBytes { path: "blobs/image.png", bytes: b"image-bytes" },
                ZipEntryBytes { path: "blobs/video.mp4", bytes: b"video-bytes" },
                ZipEntryBytes {
                    path: "models/hash001/scene.gltf",
                    bytes: br#"{"asset":{"version":"2.0"},"buffers":[{"uri":"scene.bin","byteLength":11}]}"#,
                },
                ZipEntryBytes { path: "models/hash001/scene.bin", bytes: b"scene-bytes" },
            ],
            &[],
        )
        .expect("build v2 asset zip failed");
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-static-url-rewrite");

        import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "")
            .await
            .expect("import v2 project with static assets failed");

        let imported_project_json: String = sqlx::query("SELECT data_json FROM project WHERE id = ?1")
            .bind(target_project_id)
            .fetch_one(&db.pool)
            .await
            .expect("query imported project failed")
            .try_get("data_json")
            .expect("read imported project json failed");
        let imported_value: Value = serde_json::from_str(&imported_project_json).expect("parse imported json failed");

        let expected_image_url = to_asset_url(&app_state.static_storage_dir().join("images").join("image.png"));
        let expected_video_url = to_asset_url(&app_state.static_storage_dir().join("videos").join("video.mp4"));
        let expected_model_url = to_asset_url(&app_state.static_storage_dir().join("models").join("hash001").join("scene.gltf"));

        assert_eq!(imported_value["layerManager"]["elemConfigs"]["img"]["style"]["localUrl"], expected_image_url);
        assert_eq!(imported_value["layerManager"]["elemConfigs"]["video"]["style"]["src"], expected_video_url);
        assert_eq!(imported_value["layerManager"]["elemConfigs"]["model"]["style"]["source"]["url"], expected_model_url);
    }

    #[tokio::test]
    async fn import_project_zip_impl_rolls_back_db_and_staged_files_when_late_step_fails() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前项目', '', '0', NULL, ?1, '', 0)")
                .bind("original-json")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let mut manifest = build_v2_manifest_with_common_datasource(
            EncryptedOrPlain::Plain(vec![CommonApiRef {
                origin_id: "rollback-api".to_string(),
                ref_key: String::new(),
                name: "回滚 API".to_string(),
                url: "https://example.com/rollback".to_string(),
                method: "GET".to_string(),
                params: "{}".to_string(),
                headers: "{}".to_string(),
            }]),
            EncryptedOrPlain::Plain(vec![]),
        );
        manifest.assets.images = vec![AssetBlobRef {
            name: "atomic image".to_string(),
            original: "atomic-image.png".to_string(),
            blob: "blobs/atomic-image.png".to_string(),
        }];

        let zip_bytes = build_export_zip(
            &manifest,
            [
                ZipEntryBytes { path: "project/project.json", bytes: b"{" },
                ZipEntryBytes { path: "blobs/atomic-image.png", bytes: b"image-bytes" },
            ],
            &[],
        )
        .expect("build rollback zip failed");
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-rollback");

        let err = import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "")
            .await
            .expect_err("import should fail on invalid project json");
        assert!(err.starts_with("导入失败：项目 JSON 解析失败"), "unexpected error: {err}");

        let common_api_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM common_api WHERE deleted = 0 AND url = 'https://example.com/rollback'")
            .fetch_one(&db.pool)
            .await
            .expect("count common_api failed");
        let image_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM image WHERE deleted = 0 AND url = 'atomic-image.png'")
            .fetch_one(&db.pool)
            .await
            .expect("count images failed");
        let project_json: String = sqlx::query_scalar("SELECT data_json FROM project WHERE id = ?1")
            .bind(target_project_id)
            .fetch_one(&db.pool)
            .await
            .expect("query project json failed");

        assert_eq!(common_api_count, 0);
        assert_eq!(image_count, 0);
        assert_eq!(project_json, "original-json");
        assert!(!app_state.static_storage_dir().join("images").join("atomic-image.png").exists());
        assert!(!app_state.static_storage_dir().join("blobs").join("atomic-image.png").exists());
    }

    #[tokio::test]
    async fn import_project_zip_impl_upserts_common_datasources_and_rewrites_project_json_for_v2() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let existing_api_id =
            sqlx::query("INSERT INTO common_api (name, url, method, params, headers, ref_key, deleted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)")
                .bind("旧 API")
                .bind("https://old.example.com/weather")
                .bind("POST")
                .bind("{\"old\":true}")
                .bind("{\"Authorization\":\"old-token\"}")
                .bind("api-ref-key-101")
                .execute(&db.pool)
                .await
                .expect("seed common_api failed")
                .last_insert_rowid();

        let existing_database_id = sqlx::query(
            "INSERT INTO common_database (name, type, ip, port, database_name, username, password, url, des, ref_key, deleted) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0)",
        )
        .bind("旧数据库")
        .bind("PostgreSQL")
        .bind("10.0.0.1")
        .bind("5432")
        .bind("old_demo")
        .bind("postgres")
        .bind("old-secret")
        .bind("postgres://postgres:old-secret@10.0.0.1:5432/old_demo")
        .bind("old desktop db")
        .bind("db-ref-key-202")
        .execute(&db.pool)
        .await
        .expect("seed common_database failed")
        .last_insert_rowid();

        let manifest = build_v2_manifest_with_common_datasource(
            EncryptedOrPlain::Plain(vec![CommonApiRef {
                origin_id: "legacy-api".to_string(),
                ref_key: "api-ref-key-101".to_string(),
                name: "天气 API".to_string(),
                url: "https://example.com/weather".to_string(),
                method: "GET".to_string(),
                params: "{\"city\":\"shanghai\"}".to_string(),
                headers: "{\"Authorization\":\"token\"}".to_string(),
            }]),
            EncryptedOrPlain::Plain(vec![CommonDatabaseRef {
                origin_id: "legacy-db".to_string(),
                ref_key: "db-ref-key-202".to_string(),
                ty: "MySQL".to_string(),
                ip: "127.0.0.1".to_string(),
                port: "3306".to_string(),
                database_name: "demo".to_string(),
                username: "root".to_string(),
                password: "secret".to_string(),
                url: "mysql://root:secret@127.0.0.1:3306/demo".to_string(),
                des: "desktop db".to_string(),
                allow_sql_script: true,
            }]),
        );
        let project_json = serde_json::json!({
            "layerManager": {
                "elemConfigs": {
                    "componentCommonApiByRefKey": {
                        "data": {
                            "sourceType": "commonApi",
                            "commonApi": {
                                "commonApiId": "stale-common-api-id",
                                "refKey": "api-ref-key-101"
                            }
                        }
                    },
                    "componentDatabaseByRefKey": {
                        "data": {
                            "sourceType": "database",
                            "database": {
                                "targetDb": "stale-database-id",
                                "refKey": "db-ref-key-202"
                            }
                        }
                    },
                    "componentCommonApiLegacy": {
                        "data": {
                            "sourceType": "commonApi",
                            "commonApi": {
                                "commonApiId": "legacy-api"
                            }
                        }
                    },
                    "componentDatabaseLegacy": {
                        "data": {
                            "sourceType": "database",
                            "database": {
                                "targetDb": "legacy-db"
                            }
                        }
                    },
                    "loopContainer": {
                        "managerData": {
                            "layerManager": {
                                "elemConfigs": {
                                    "nestedComponentCommonApi": {
                                        "data": {
                                            "sourceType": "commonApi",
                                            "commonApi": {
                                                "commonApiId": "legacy-api"
                                            }
                                        }
                                    },
                                    "nestedGis": {
                                        "style": {
                                            "sources": [
                                                {
                                                    "id": "gis-common-api",
                                                    "sourceType": "geojson",
                                                    "dataMode": "dataConfig",
                                                    "dataConfig": {
                                                        "sourceType": "commonApi",
                                                        "commonApi": {
                                                            "commonApiId": "legacy-api"
                                                        }
                                                    }
                                                },
                                                {
                                                    "id": "gis-database",
                                                    "sourceType": "geojson",
                                                    "dataMode": "dataConfig",
                                                    "dataConfig": {
                                                        "sourceType": "database",
                                                        "database": {
                                                            "targetDb": "legacy-db"
                                                        }
                                                    }
                                                }
                                            ]
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "globalVariableManager": {
                "globalVariables": {
                    "gvCommonApi": {
                        "triggerSource": {
                            "sourceType": "commonApi",
                            "commonApi": {
                                "commonApiId": "legacy-api"
                            }
                        }
                    },
                    "gvDatabase": {
                        "triggerSource": {
                            "sourceType": "database",
                            "database": {
                                "targetDb": "legacy-db"
                            }
                        }
                    }
                }
            }
        })
        .to_string();
        let zip_bytes = build_export_zip(&manifest, [ZipEntryBytes { path: "project/project.json", bytes: project_json.as_bytes() }], &[])
            .expect("build v2 commonDatasource zip failed");
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-v2-common-datasource");

        import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "")
            .await
            .expect("import v2 commonDatasource project failed");

        let common_api_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM common_api WHERE deleted = 0")
            .fetch_one(&db.pool)
            .await
            .expect("count common_api failed");
        let common_database_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM common_database WHERE deleted = 0")
            .fetch_one(&db.pool)
            .await
            .expect("count common_database failed");
        assert_eq!(common_api_count, 1);
        assert_eq!(common_database_count, 1);

        let common_api_row = sqlx::query("SELECT ref_key, name, url, method, params, headers FROM common_api WHERE id = ?1")
            .bind(existing_api_id)
            .fetch_one(&db.pool)
            .await
            .expect("query common_api failed");
        let common_api_ref_key: Option<String> = common_api_row.try_get("ref_key").expect("read api ref_key failed");
        let common_api_name: Option<String> = common_api_row.try_get("name").expect("read api name failed");
        let common_api_url: String = common_api_row.try_get("url").expect("read api url failed");
        let common_api_method: String = common_api_row.try_get("method").expect("read api method failed");
        let common_api_params: Option<String> = common_api_row.try_get("params").expect("read api params failed");
        let common_api_headers: Option<String> = common_api_row.try_get("headers").expect("read api headers failed");
        assert_eq!(common_api_ref_key.unwrap_or_default(), "api-ref-key-101");
        assert_eq!(common_api_name.unwrap_or_default(), "天气 API");
        assert_eq!(common_api_url, "https://example.com/weather");
        assert_eq!(common_api_method, "GET");
        assert_eq!(common_api_params.unwrap_or_default(), "{\"city\":\"shanghai\"}");
        assert_eq!(common_api_headers.unwrap_or_default(), "{\"Authorization\":\"token\"}");

        let common_database_row = sqlx::query(
            "SELECT ref_key, name, type, ip, port, database_name, username, password, url, des, allow_sql_script \
             FROM common_database WHERE id = ?1",
        )
        .bind(existing_database_id)
        .fetch_one(&db.pool)
        .await
        .expect("query common_database failed");
        let common_database_ref_key: Option<String> = common_database_row.try_get("ref_key").expect("read database ref_key failed");
        let common_database_name: Option<String> = common_database_row.try_get("name").expect("read database name failed");
        let common_database_type: String = common_database_row.try_get("type").expect("read database type failed");
        let common_database_ip: String = common_database_row.try_get("ip").expect("read database ip failed");
        let common_database_port: String = common_database_row.try_get("port").expect("read database port failed");
        let common_database_database_name: String = common_database_row.try_get("database_name").expect("read database_name failed");
        let common_database_username: String = common_database_row.try_get("username").expect("read database username failed");
        let common_database_password: String = common_database_row.try_get("password").expect("read database password failed");
        let common_database_url: Option<String> = common_database_row.try_get("url").expect("read database url failed");
        let common_database_des: Option<String> = common_database_row.try_get("des").expect("read database des failed");
        let common_database_allow_sql_script: i64 = common_database_row
            .try_get("allow_sql_script")
            .expect("read database allow_sql_script failed");
        assert_eq!(common_database_ref_key.unwrap_or_default(), "db-ref-key-202");
        assert_eq!(common_database_name.unwrap_or_default(), "demo");
        assert_eq!(common_database_type, "MySQL");
        assert_eq!(common_database_ip, "127.0.0.1");
        assert_eq!(common_database_port, "3306");
        assert_eq!(common_database_database_name, "demo");
        assert_eq!(common_database_username, "root");
        assert_eq!(common_database_password, "secret");
        assert_eq!(common_database_url.unwrap_or_default(), "mysql://root:secret@127.0.0.1:3306/demo");
        assert_eq!(common_database_des.unwrap_or_default(), "desktop db");
        assert_eq!(common_database_allow_sql_script, 1);

        let imported_project_row = sqlx::query("SELECT data_json FROM project WHERE id = ?1")
            .bind(target_project_id)
            .fetch_one(&db.pool)
            .await
            .expect("query imported project failed");
        let imported_project_json: String = imported_project_row
            .try_get("data_json")
            .expect("read imported project json failed");
        let imported_value: Value = serde_json::from_str(&imported_project_json).expect("parse imported project json failed");

        assert_eq!(
            imported_value["layerManager"]["elemConfigs"]["componentCommonApiByRefKey"]["data"]["commonApi"]["commonApiId"],
            Value::String(existing_api_id.to_string())
        );
        assert_eq!(
            imported_value["layerManager"]["elemConfigs"]["componentDatabaseByRefKey"]["data"]["database"]["targetDb"],
            Value::String(existing_database_id.to_string())
        );
        assert_eq!(
            imported_value["layerManager"]["elemConfigs"]["componentCommonApiLegacy"]["data"]["commonApi"]["commonApiId"],
            Value::String(existing_api_id.to_string())
        );
        assert_eq!(
            imported_value["layerManager"]["elemConfigs"]["componentDatabaseLegacy"]["data"]["database"]["targetDb"],
            Value::String(existing_database_id.to_string())
        );
        assert_eq!(
            imported_value["layerManager"]["elemConfigs"]["loopContainer"]["managerData"]["layerManager"]["elemConfigs"]["nestedComponentCommonApi"]
                ["data"]["commonApi"]["commonApiId"],
            Value::String(existing_api_id.to_string())
        );
        assert_eq!(
            imported_value["layerManager"]["elemConfigs"]["loopContainer"]["managerData"]["layerManager"]["elemConfigs"]["nestedGis"]["style"]
                ["sources"][0]["dataConfig"]["commonApi"]["commonApiId"],
            Value::String(existing_api_id.to_string())
        );
        assert_eq!(
            imported_value["layerManager"]["elemConfigs"]["loopContainer"]["managerData"]["layerManager"]["elemConfigs"]["nestedGis"]["style"]
                ["sources"][1]["dataConfig"]["database"]["targetDb"],
            Value::String(existing_database_id.to_string())
        );
        assert_eq!(
            imported_value["globalVariableManager"]["globalVariables"]["gvCommonApi"]["triggerSource"]["commonApi"]["commonApiId"],
            Value::String(existing_api_id.to_string())
        );
        assert_eq!(
            imported_value["globalVariableManager"]["globalVariables"]["gvDatabase"]["triggerSource"]["database"]["targetDb"],
            Value::String(existing_database_id.to_string())
        );
    }
