/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
    use super::*;
    use crate::app_state::AppState;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::io::{Cursor, Read};
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::ZipArchive;

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
        let root = std::env::temp_dir().join(format!("datavisool-export-route-test-{unique}"));
        std::fs::create_dir_all(root.join("static")).expect("create temp static dir failed");
        let app_state = AppState::new_for_test(root.clone(), root.clone());

        (DbState { pool }, app_state)
    }

    fn read_manifest_json_from_zip(bytes: &[u8]) -> String {
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor).expect("open zip failed");
        let mut file = archive.by_name("manifest.json").expect("manifest.json missing");
        let mut manifest_json = String::new();
        file.read_to_string(&mut manifest_json).expect("read manifest.json failed");
        manifest_json
    }

    fn read_manifest_from_zip(bytes: Vec<u8>) -> ExportManifest {
        let manifest_json = read_manifest_json_from_zip(&bytes);
        serde_json::from_str(&manifest_json).expect("parse manifest json failed")
    }

    fn decrypt_inner_zip_from_encrypted_package(bytes: &[u8], passphrase: &str) -> Vec<u8> {
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor).expect("open encrypted package failed");
        let manifest_json = read_manifest_json_from_zip(bytes);
        let manifest_value: serde_json::Value = serde_json::from_str(&manifest_json).expect("parse encrypted manifest failed");
        assert_eq!(manifest_value.get("encrypted").and_then(|value| value.as_bool()), Some(true));
        let mut payload = Vec::new();
        archive
            .by_name("payload.enc")
            .expect("payload.enc missing")
            .read_to_end(&mut payload)
            .expect("read payload.enc failed");
        let envelope: datavisool_project_format::EncryptedEnvelopeV1 = serde_json::from_slice(&payload).expect("parse encrypted payload envelope failed");
        datavisool_codec::decrypt_envelope_v1(&envelope, passphrase).expect("decrypt encrypted package failed")
    }

    fn list_zip_paths(bytes: &[u8]) -> Vec<String> {
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor).expect("open zip failed");
        let mut names = Vec::new();
        for idx in 0..archive.len() {
            let file = archive.by_index(idx).expect("open zip entry failed");
            names.push(file.name().to_string());
        }
        names
    }

    #[test]
    fn deserialize_export_project_req_with_secret_mode() {
        let req: ExportProjectReq = serde_json::from_value(serde_json::json!({
            "id": "project-1",
            "name": "Project 1",
            "secretMode": "plain",
            "projectJson": "{}"
        }))
        .expect("deserialize export project req failed");

        assert_eq!(req.id.as_deref(), Some("project-1"));
        assert_eq!(req.secret_mode.as_deref(), Some("plain"));
        assert_eq!(req.project_json.as_deref(), Some("{}"));
    }

    #[test]
    fn deserialize_export_project_req_with_secret_passphrase() {
        let req: ExportProjectReq = serde_json::from_value(serde_json::json!({
            "id": "project-1",
            "name": "Project 1",
            "secretMode": "passphrase-v1",
            "secretPassphrase": "export-secret",
            "projectJson": "{}"
        }))
        .expect("deserialize export project req with secretPassphrase failed");

        assert_eq!(req.secret_mode.as_deref(), Some("passphrase-v1"));
        assert_eq!(req.secret_passphrase.as_deref(), Some("export-secret"));
    }

    #[tokio::test]
    async fn build_export_zip_writes_common_datasource_inside_encrypted_package() {
        let (db, app_state) = build_test_env().await;

        sqlx::query(
            "INSERT INTO common_api (id, ref_key, name, url, method, params, headers, deleted) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
        )
        .bind(101_i64)
        .bind("common-api-ref-key-101")
        .bind("公共API-A")
        .bind("https://example.com/api")
        .bind("GET")
        .bind("{\"k\":\"v\"}")
        .bind("{\"Authorization\":\"token\"}")
        .execute(&db.pool)
        .await
        .expect("insert common_api failed");

        sqlx::query(
            "INSERT INTO common_database \
             (id, ref_key, name, type, ip, port, database_name, username, password, url, des, allow_sql_script, deleted) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, 0)",
        )
        .bind(202_i64)
        .bind("common-database-ref-key-202")
        .bind("数据库-A")
        .bind("MySQL")
        .bind("127.0.0.1")
        .bind("3306")
        .bind("demo")
        .bind("root")
        .bind("secret")
        .bind("")
        .bind("说明")
        .execute(&db.pool)
        .await
        .expect("insert common_database failed");

        let (_file_name, bytes) = build_export_zip(
            &db,
            &app_state,
            ExportProjectReq {
                id: Some("project-1".to_string()),
                name: Some("Project 1".to_string()),
                secret_mode: Some("passphrase-v1".to_string()),
                secret_passphrase: Some("export-secret".to_string()),
                projects: None,
                fonts: None,
                images: None,
                videos: None,
                models: None,
                components: None,
                common_api_ids: Some(vec!["101".to_string()]),
                database_ids: Some(vec!["202".to_string()]),
                project_json: Some("{\"layerManager\":{\"elemConfigs\":{}}}".to_string()),
            },
        )
        .await
        .expect("build export zip failed");

        let outer_manifest_json = read_manifest_json_from_zip(&bytes);
        assert!(outer_manifest_json.contains("\"encrypted\":true"));
        assert!(outer_manifest_json.contains("\"payloadPath\":\"payload.enc\""));
        assert!(!outer_manifest_json.contains("common-api-ref-key-101"));
        assert!(!outer_manifest_json.contains("common-database-ref-key-202"));
        assert!(!outer_manifest_json.contains("Authorization"));
        assert!(!outer_manifest_json.contains("secret"));

        let inner_zip = decrypt_inner_zip_from_encrypted_package(&bytes, "export-secret");
        let manifest_json = read_manifest_json_from_zip(&inner_zip);
        assert!(manifest_json.contains("\"commonDatasource\""));
        assert!(manifest_json.contains("\"commonApis\""));
        assert!(manifest_json.contains("\"databases\""));
        assert!(manifest_json.contains("\"common-api-ref-key-101\""));
        assert!(manifest_json.contains("\"common-database-ref-key-202\""));

        let manifest = read_manifest_from_zip(inner_zip);

        let common_apis = manifest
            .common_datasource
            .common_apis
            .as_plain()
            .expect("commonApis should be plain array");
        assert_eq!(common_apis.len(), 1);
        assert_eq!(common_apis[0].origin_id, "101");
        assert_eq!(common_apis[0].ref_key, "common-api-ref-key-101");
        assert_eq!(common_apis[0].name, "公共API-A");
        assert_eq!(common_apis[0].url, "https://example.com/api");

        let databases = manifest
            .common_datasource
            .databases
            .as_plain()
            .expect("databases should be plain array");
        assert_eq!(databases.len(), 1);
        assert_eq!(databases[0].origin_id, "202");
        assert_eq!(databases[0].ref_key, "common-database-ref-key-202");
        assert_eq!(databases[0].ty, "MySQL");
        assert_eq!(databases[0].database_name, "demo");
        assert_eq!(databases[0].username, "root");
        assert!(databases[0].allow_sql_script);

        let _ = std::fs::remove_dir_all(&app_state.files_dir);
    }

    #[tokio::test]
    async fn build_export_zip_requires_package_encryption_for_each_common_datasource_type() {
        let (db, app_state) = build_test_env().await;

        sqlx::query(
            "INSERT INTO common_api (id, ref_key, name, url, method, params, headers, deleted) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
        )
        .bind(101_i64)
        .bind("common-api-ref-key-101")
        .bind("公共API-A")
        .bind("https://example.com/api")
        .bind("GET")
        .bind("{\"k\":\"v\"}")
        .bind("{\"Authorization\":\"token\"}")
        .execute(&db.pool)
        .await
        .expect("insert common_api failed");

        sqlx::query(
            "INSERT INTO common_database \
             (id, ref_key, name, type, ip, port, database_name, username, password, url, des, deleted) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
        )
        .bind(202_i64)
        .bind("common-database-ref-key-202")
        .bind("数据库-A")
        .bind("MySQL")
        .bind("127.0.0.1")
        .bind("3306")
        .bind("demo")
        .bind("root")
        .bind("secret")
        .bind("mysql://root:secret@127.0.0.1:3306/demo")
        .bind("说明")
        .execute(&db.pool)
        .await
        .expect("insert common_database failed");

        let cases = [(Some(vec!["101".to_string()]), None, "common api"), (None, Some(vec!["202".to_string()]), "database")];
        for (common_api_ids, database_ids, label) in cases {
            let err = build_export_zip(
                &db,
                &app_state,
                ExportProjectReq {
                    id: Some("project-1".to_string()),
                    name: Some("Project 1".to_string()),
                    secret_mode: None,
                    secret_passphrase: None,
                    projects: None,
                    fonts: None,
                    images: None,
                    videos: None,
                    models: None,
                    components: None,
                    common_api_ids,
                    database_ids,
                    project_json: Some("{\"layerManager\":{\"elemConfigs\":{}}}".to_string()),
                },
            )
            .await
            .expect_err(&format!("plain export with {label} should be rejected"));
            assert_eq!(err, "请求参数错误：导出包含公共 API 或数据库时必须启用整包加密");
        }

        let _ = std::fs::remove_dir_all(&app_state.files_dir);
    }

    #[tokio::test]
    async fn build_export_zip_encrypts_package_when_common_apis_are_present() {
        let (db, app_state) = build_test_env().await;

        sqlx::query(
            "INSERT INTO common_api (id, ref_key, name, url, method, params, headers, deleted) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
        )
        .bind(101_i64)
        .bind("common-api-ref-key-101")
        .bind("公共API-A")
        .bind("https://example.com/api")
        .bind("GET")
        .bind("{\"k\":\"v\"}")
        .bind("{\"Authorization\":\"token\"}")
        .execute(&db.pool)
        .await
        .expect("insert common_api failed");

        let (_file_name, bytes) = build_export_zip(
            &db,
            &app_state,
            ExportProjectReq {
                id: Some("project-1".to_string()),
                name: Some("Project 1".to_string()),
                secret_mode: Some("passphrase-v1".to_string()),
                secret_passphrase: Some("export-secret".to_string()),
                projects: None,
                fonts: None,
                images: None,
                videos: None,
                models: None,
                components: None,
                common_api_ids: Some(vec!["101".to_string()]),
                database_ids: None,
                project_json: Some("{\"layerManager\":{\"elemConfigs\":{}}}".to_string()),
            },
        )
        .await
        .expect("build export zip failed");

        let outer_manifest_json = read_manifest_json_from_zip(&bytes);
        assert!(outer_manifest_json.contains("\"encrypted\":true"));
        assert!(!outer_manifest_json.contains("common-api-ref-key-101"));
        assert!(!outer_manifest_json.contains("Authorization"));

        let inner_zip = decrypt_inner_zip_from_encrypted_package(&bytes, "export-secret");
        let manifest = read_manifest_from_zip(inner_zip);
        let common_apis = manifest
            .common_datasource
            .common_apis
            .as_plain()
            .expect("commonApis should be plain inside encrypted package");

        assert_eq!(common_apis.len(), 1);
        assert_eq!(common_apis[0].origin_id, "101");
        assert_eq!(common_apis[0].ref_key, "common-api-ref-key-101");
        let databases = manifest
            .common_datasource
            .databases
            .as_plain()
            .expect("databases should be plain inside encrypted package");
        assert!(databases.is_empty());

        let _ = std::fs::remove_dir_all(&app_state.files_dir);
    }

    #[tokio::test]
    async fn build_export_zip_encrypts_package_when_databases_are_present() {
        let (db, app_state) = build_test_env().await;

        sqlx::query(
            "INSERT INTO common_database \
             (id, ref_key, name, type, ip, port, database_name, username, password, url, des, deleted) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
        )
        .bind(202_i64)
        .bind("common-database-ref-key-202")
        .bind("数据库-A")
        .bind("MySQL")
        .bind("127.0.0.1")
        .bind("3306")
        .bind("demo")
        .bind("root")
        .bind("secret")
        .bind("mysql://root:secret@127.0.0.1:3306/demo")
        .bind("说明")
        .execute(&db.pool)
        .await
        .expect("insert common_database failed");

        let (_file_name, bytes) = build_export_zip(
            &db,
            &app_state,
            ExportProjectReq {
                id: Some("project-1".to_string()),
                name: Some("Project 1".to_string()),
                secret_mode: Some("passphrase-v1".to_string()),
                secret_passphrase: Some("export-secret".to_string()),
                projects: None,
                fonts: None,
                images: None,
                videos: None,
                models: None,
                components: None,
                common_api_ids: None,
                database_ids: Some(vec!["202".to_string()]),
                project_json: Some("{\"layerManager\":{\"elemConfigs\":{}}}".to_string()),
            },
        )
        .await
        .expect("build export zip failed");

        let outer_manifest_json = read_manifest_json_from_zip(&bytes);
        assert!(outer_manifest_json.contains("\"encrypted\":true"));
        assert!(!outer_manifest_json.contains("common-database-ref-key-202"));
        assert!(!outer_manifest_json.contains("mysql://root:secret"));

        let inner_zip = decrypt_inner_zip_from_encrypted_package(&bytes, "export-secret");
        let manifest = read_manifest_from_zip(inner_zip);
        let databases = manifest
            .common_datasource
            .databases
            .as_plain()
            .expect("databases should be plain inside encrypted package");

        assert_eq!(databases.len(), 1);
        assert_eq!(databases[0].origin_id, "202");
        assert_eq!(databases[0].ref_key, "common-database-ref-key-202");
        let common_apis = manifest
            .common_datasource
            .common_apis
            .as_plain()
            .expect("commonApis should be plain inside encrypted package");
        assert!(common_apis.is_empty());

        let _ = std::fs::remove_dir_all(&app_state.files_dir);
    }

    #[tokio::test]
    async fn build_export_zip_keeps_common_datasource_only_inside_encrypted_payload() {
        let (db, app_state) = build_test_env().await;

        sqlx::query(
            "INSERT INTO common_api (id, ref_key, name, url, method, params, headers, deleted) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
        )
        .bind(101_i64)
        .bind("common-api-ref-key-101")
        .bind("公共API-A")
        .bind("https://example.com/api")
        .bind("GET")
        .bind("{\"k\":\"v\"}")
        .bind("{\"Authorization\":\"token\"}")
        .execute(&db.pool)
        .await
        .expect("insert common_api failed");

        sqlx::query(
            "INSERT INTO common_database \
             (id, ref_key, name, type, ip, port, database_name, username, password, url, des, deleted) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
        )
        .bind(202_i64)
        .bind("common-database-ref-key-202")
        .bind("数据库-A")
        .bind("MySQL")
        .bind("127.0.0.1")
        .bind("3306")
        .bind("demo")
        .bind("root")
        .bind("secret")
        .bind("mysql://root:secret@127.0.0.1:3306/demo")
        .bind("说明")
        .execute(&db.pool)
        .await
        .expect("insert common_database failed");

        let (_file_name, bytes) = build_export_zip(
            &db,
            &app_state,
            ExportProjectReq {
                id: Some("project-1".to_string()),
                name: Some("Project 1".to_string()),
                secret_mode: Some("passphrase-v1".to_string()),
                secret_passphrase: Some("export-secret".to_string()),
                projects: None,
                fonts: None,
                images: None,
                videos: None,
                models: None,
                components: None,
                common_api_ids: Some(vec!["101".to_string()]),
                database_ids: Some(vec!["202".to_string()]),
                project_json: Some("{\"layerManager\":{\"elemConfigs\":{}}}".to_string()),
            },
        )
        .await
        .expect("build export zip failed");

        let outer_manifest_json = read_manifest_json_from_zip(&bytes);
        assert!(outer_manifest_json.contains("\"encrypted\":true"));
        assert!(outer_manifest_json.contains("\"payloadPath\":\"payload.enc\""));
        assert!(!outer_manifest_json.contains("common-api-ref-key-101"));
        assert!(!outer_manifest_json.contains("common-database-ref-key-202"));
        assert!(!outer_manifest_json.contains("Authorization"));
        assert!(!outer_manifest_json.contains("mysql://root:secret"));

        let inner_zip = decrypt_inner_zip_from_encrypted_package(&bytes, "export-secret");
        let inner_manifest_json = read_manifest_json_from_zip(&inner_zip);
        assert!(inner_manifest_json.contains("\"commonApis\""));
        assert!(inner_manifest_json.contains("\"databases\""));
        assert!(inner_manifest_json.contains("common-api-ref-key-101"));
        assert!(inner_manifest_json.contains("common-database-ref-key-202"));

        let _ = std::fs::remove_dir_all(&app_state.files_dir);
    }

    #[tokio::test]
    async fn build_export_zip_writes_project_graph_nodes_and_scoped_dynamic_scripts_for_v3() {
        let (db, app_state) = build_test_env().await;

        std::fs::create_dir_all(app_state.static_storage_dir().join("lib")).expect("create static lib dir failed");
        std::fs::write(app_state.static_storage_dir().join("lib").join("global.js"), b"console.log('global');").expect("write global script failed");
        std::fs::write(app_state.static_storage_dir().join("lib").join("project-b1.js"), b"console.log('project-b1');")
            .expect("write project script failed");

        sqlx::query("INSERT INTO dynamic_script (name, url, type, project_id, deleted) VALUES (?1, ?2, ?3, NULL, 0)")
            .bind("全局脚本")
            .bind("global.js")
            .bind("LOCAL")
            .execute(&db.pool)
            .await
            .expect("insert global dynamic script failed");

        sqlx::query("INSERT INTO dynamic_script (name, url, type, project_id, deleted) VALUES (?1, ?2, ?3, ?4, 0)")
            .bind("B1 脚本")
            .bind("project-b1.js")
            .bind("LOCAL")
            .bind("B1")
            .execute(&db.pool)
            .await
            .expect("insert project dynamic script failed");

        let (_file_name, bytes) = build_export_zip(
            &db,
            &app_state,
            ExportProjectReq {
                id: Some("A".to_string()),
                name: Some("入口项目A".to_string()),
                secret_mode: Some("plain".to_string()),
                secret_passphrase: Some(String::new()),
                projects: Some(vec![
                    ExportProjectNodeReq {
                        source_project_id: Some("A".to_string()),
                        name: Some("入口项目A".to_string()),
                        source_pid: None,
                        is_entry: Some(true),
                        fonts: Some(vec!["FontA".to_string()]),
                        images: Some(vec!["image-a.png".to_string()]),
                        videos: None,
                        models: None,
                        components: Some(vec!["remote-a.js".to_string()]),
                        common_api_ids: Some(vec!["101".to_string()]),
                        database_ids: None,
                        project_json: Some("{\"layerManager\":{\"elemConfigs\":{\"screen\":{\"base\":{\"type\":\"ScreenReference\"},\"style\":{\"screenId\":\"B1\"}}}}}".to_string()),
                    },
                    ExportProjectNodeReq {
                        source_project_id: Some("B".to_string()),
                        name: Some("父项目B".to_string()),
                        source_pid: None,
                        is_entry: Some(false),
                        fonts: None,
                        images: None,
                        videos: None,
                        models: None,
                        components: None,
                        common_api_ids: None,
                        database_ids: None,
                        project_json: Some("{\"layerManager\":{\"elemConfigs\":{}}}".to_string()),
                    },
                    ExportProjectNodeReq {
                        source_project_id: Some("B1".to_string()),
                        name: Some("子项目B1".to_string()),
                        source_pid: Some("B".to_string()),
                        is_entry: Some(false),
                        fonts: None,
                        images: None,
                        videos: Some(vec!["video-b1.mp4".to_string()]),
                        models: None,
                        components: None,
                        common_api_ids: None,
                        database_ids: Some(vec!["202".to_string()]),
                        project_json: Some("{\"layerManager\":{\"elemConfigs\":{}}}".to_string()),
                    },
                ]),
                fonts: Some(vec!["FontA".to_string()]),
                images: Some(vec!["image-a.png".to_string()]),
                videos: Some(vec!["video-b1.mp4".to_string()]),
                models: None,
                components: Some(vec!["remote-a.js".to_string()]),
                common_api_ids: Some(vec!["101".to_string()]),
                database_ids: Some(vec!["202".to_string()]),
                project_json: Some("{\"layerManager\":{\"elemConfigs\":{\"screen\":{\"base\":{\"type\":\"ScreenReference\"},\"style\":{\"screenId\":\"B1\"}}}}}".to_string()),
            },
        )
        .await
        .expect("build export zip failed");

        let manifest = read_manifest_from_zip(bytes.clone());
        let zip_paths = list_zip_paths(&bytes);

        assert_eq!(manifest.format, "datavisool-project");
        assert_eq!(manifest.format_version, 3);
        assert_eq!(manifest.project.id, "A");
        assert_eq!(manifest.project.data_path, "projects/A/project.json");
        assert_eq!(manifest.projects.len(), 3);
        assert_eq!(manifest.projects[0].source_project_id, "A");
        assert_eq!(manifest.projects[2].source_pid.as_deref(), Some("B"));
        assert!(zip_paths.contains(&"projects/A/project.json".to_string()));
        assert!(zip_paths.contains(&"projects/B/project.json".to_string()));
        assert!(zip_paths.contains(&"projects/B1/project.json".to_string()));

        let dynamic_scripts = manifest.assets.dynamic_scripts;
        assert_eq!(dynamic_scripts.len(), 2);
        assert!(dynamic_scripts.iter().any(|item| {
            item.name == "全局脚本"
                && item.scope == "GLOBAL"
                && item.owner_source_project_id.is_none()
                && item.blob.as_deref() == Some("blobs/global.js")
        }));
        assert!(dynamic_scripts.iter().any(|item| {
            item.name == "B1 脚本"
                && item.scope == "PROJECT"
                && item.owner_source_project_id.as_deref() == Some("B1")
                && item.blob.as_deref() == Some("blobs/project-b1.js")
        }));

        let _ = std::fs::remove_dir_all(&app_state.files_dir);
    }
