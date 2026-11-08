/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
    #[tokio::test]
    async fn import_project_zip_impl_requires_secret_passphrase_for_encrypted_common_datasource() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let common_apis = vec![CommonApiRef {
            origin_id: "encrypted-api".to_string(),
            ref_key: "encrypted-api-ref-key".to_string(),
            name: "加密 API".to_string(),
            url: "https://example.com/encrypted".to_string(),
            method: "GET".to_string(),
            params: "{}".to_string(),
            headers: "{}".to_string(),
        }];
        let manifest = build_v2_manifest_with_common_datasource(
            EncryptedOrPlain::Encrypted(encrypt_common_apis_payload(&common_apis, "import-secret")),
            EncryptedOrPlain::Plain(vec![]),
        );
        let zip_bytes =
            build_export_zip(&manifest, [ZipEntryBytes { path: "project/project.json", bytes: b"{}" }], &[]).expect("build encrypted v2 zip failed");
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-v2-encrypted-missing-secret");

        let err = import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "")
            .await
            .expect_err("import should require secret passphrase");
        assert_eq!(err, "请求参数错误：缺少导入口令（该项目包包含加密数据源）");
    }

    #[tokio::test]
    async fn import_project_zip_impl_rejects_wrong_secret_passphrase_for_encrypted_common_datasource() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let databases = vec![CommonDatabaseRef {
            origin_id: "encrypted-db".to_string(),
            ref_key: "encrypted-db-ref-key".to_string(),
            ty: "MySQL".to_string(),
            ip: "127.0.0.1".to_string(),
            port: "3306".to_string(),
            database_name: "secure_demo".to_string(),
            username: "root".to_string(),
            password: "secret".to_string(),
            url: "mysql://root:secret@127.0.0.1:3306/secure_demo".to_string(),
            des: "encrypted db".to_string(),
            allow_sql_script: true,
        }];
        let manifest = build_v2_manifest_with_common_datasource(
            EncryptedOrPlain::Plain(vec![]),
            EncryptedOrPlain::Encrypted(encrypt_databases_payload(&databases, "import-secret")),
        );
        let zip_bytes = build_export_zip(&manifest, [ZipEntryBytes { path: "project/project.json", bytes: b"{}" }], &[])
            .expect("build encrypted v2 database zip failed");
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-v2-encrypted-wrong-secret");

        let err = import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "wrong-secret")
            .await
            .expect_err("import should reject wrong secret passphrase");
        assert_eq!(err, "请求参数错误：导入口令错误");
    }

    #[tokio::test]
    async fn import_project_from_path_returns_bad_request_when_encrypted_package_lacks_secret_passphrase() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let common_apis = vec![CommonApiRef {
            origin_id: "encrypted-api".to_string(),
            ref_key: "encrypted-api-ref-key".to_string(),
            name: "加密 API".to_string(),
            url: "https://example.com/encrypted".to_string(),
            method: "GET".to_string(),
            params: "{}".to_string(),
            headers: "{}".to_string(),
        }];
        let manifest = build_v2_manifest_with_common_datasource(
            EncryptedOrPlain::Encrypted(encrypt_common_apis_payload(&common_apis, "import-secret")),
            EncryptedOrPlain::Plain(vec![]),
        );
        let zip_bytes = build_export_zip(&manifest, [ZipEntryBytes { path: "project/project.json", bytes: b"{}" }], &[])
            .expect("build encrypted route zip failed");
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-route-missing-secret");

        let response = import_project_from_path(
            &db,
            &app_state,
            serde_json::json!({
                "projectId": target_project_id.to_string(),
                "path": zip_path.to_str().unwrap()
            }),
        )
        .await
        .expect("route should return HttpResponse");

        assert_eq!(response.code, 400);
        assert_eq!(response.msg, "请求参数错误：缺少导入口令（该项目包包含加密数据源）");
    }

    #[tokio::test]
    async fn import_project_from_path_accepts_secret_passphrase_and_imports_encrypted_common_datasource() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let databases = vec![CommonDatabaseRef {
            origin_id: "encrypted-db".to_string(),
            ref_key: "encrypted-db-ref-key".to_string(),
            ty: "MySQL".to_string(),
            ip: "127.0.0.1".to_string(),
            port: "3306".to_string(),
            database_name: "secure_demo".to_string(),
            username: "root".to_string(),
            password: "secret".to_string(),
            url: "mysql://root:secret@127.0.0.1:3306/secure_demo".to_string(),
            des: "encrypted db".to_string(),
            allow_sql_script: true,
        }];
        let manifest = build_v2_manifest_with_common_datasource(
            EncryptedOrPlain::Plain(vec![]),
            EncryptedOrPlain::Encrypted(encrypt_databases_payload(&databases, "import-secret")),
        );
        let project_json = r#"{"layerManager":{"elemConfigs":{"db":{"data":{"sourceType":"database","database":{"targetDb":"encrypted-db"}}}}}}"#;
        let zip_bytes = build_export_zip(&manifest, [ZipEntryBytes { path: "project/project.json", bytes: project_json.as_bytes() }], &[])
            .expect("build encrypted route success zip failed");
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-route-success");

        let response = import_project_from_path(
            &db,
            &app_state,
            serde_json::json!({
                "projectId": target_project_id.to_string(),
                "path": zip_path.to_str().unwrap(),
                "secretPassphrase": "import-secret"
            }),
        )
        .await
        .expect("route should return HttpResponse");

        assert_eq!(response.code, 200);
        assert_eq!(response.data, serde_json::json!({"warnings": []}));

        let imported_db_id: i64 = sqlx::query_scalar("SELECT id FROM common_database WHERE deleted = 0 ORDER BY id ASC LIMIT 1")
            .fetch_one(&db.pool)
            .await
            .expect("query imported db id failed");
        let imported_project_json: String = sqlx::query_scalar("SELECT data_json FROM project WHERE id = ?1")
            .bind(target_project_id)
            .fetch_one(&db.pool)
            .await
            .expect("query imported project json failed");
        let imported_value: Value = serde_json::from_str(&imported_project_json).expect("parse imported project json failed");
        assert_eq!(
            imported_value["layerManager"]["elemConfigs"]["db"]["data"]["database"]["targetDb"],
            Value::String(imported_db_id.to_string())
        );
    }

    #[tokio::test]
    async fn import_project_zip_impl_imports_encrypted_common_datasource_and_rewrites_v3_project_graph() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前入口项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let common_apis = vec![CommonApiRef {
            origin_id: "encrypted-api".to_string(),
            ref_key: "encrypted-api-ref-key".to_string(),
            name: "加密 API".to_string(),
            url: "https://example.com/encrypted".to_string(),
            method: "GET".to_string(),
            params: "{}".to_string(),
            headers: "{}".to_string(),
        }];
        let databases = vec![CommonDatabaseRef {
            origin_id: "encrypted-db".to_string(),
            ref_key: "encrypted-db-ref-key".to_string(),
            ty: "MySQL".to_string(),
            ip: "127.0.0.1".to_string(),
            port: "3306".to_string(),
            database_name: "secure_demo".to_string(),
            username: "root".to_string(),
            password: "secret".to_string(),
            url: "mysql://root:secret@127.0.0.1:3306/secure_demo".to_string(),
            des: "encrypted db".to_string(),
            allow_sql_script: true,
        }];
        let manifest = build_v3_manifest_with_common_datasource(
            "A",
            vec![manifest_project_node("A", None, true), manifest_project_node("B", None, false)],
            EncryptedOrPlain::Encrypted(encrypt_common_apis_payload(&common_apis, "import-secret")),
            EncryptedOrPlain::Encrypted(encrypt_databases_payload(&databases, "import-secret")),
        );
        let zip_bytes = build_v3_graph_zip(
            &manifest,
            &[
                (
                    "A",
                    r#"{
                        "layerManager":{
                            "elemConfigs":{
                                "screen":{"base":{"type":"ScreenReference"},"style":{"screenId":"B"}},
                                "api":{"data":{"sourceType":"commonApi","commonApi":{"commonApiId":"encrypted-api"}}},
                                "db":{"data":{"sourceType":"database","database":{"targetDb":"encrypted-db"}}}
                            }
                        }
                    }"#,
                ),
                ("B", r#"{"layerManager":{"elemConfigs":{"title":{"base":{"type":"Text"}}}}}"#),
            ],
            &[],
        );
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-v3-encrypted-datasource");

        import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "import-secret")
            .await
            .expect("import encrypted v3 project graph failed");

        let (project_ids, _project_pids, project_jsons) = load_imported_project_graph(&db).await;
        let b_id = *project_ids.get("B").expect("B missing");

        let common_api_row = sqlx::query("SELECT id FROM common_api WHERE deleted = 0 ORDER BY id ASC LIMIT 1")
            .fetch_one(&db.pool)
            .await
            .expect("query imported common_api failed");
        let common_api_id: i64 = common_api_row.try_get("id").expect("read common_api id failed");

        let common_database_row = sqlx::query("SELECT id FROM common_database WHERE deleted = 0 ORDER BY id ASC LIMIT 1")
            .fetch_one(&db.pool)
            .await
            .expect("query imported common_database failed");
        let common_database_id: i64 = common_database_row.try_get("id").expect("read common_database id failed");

        let entry_json: Value =
            serde_json::from_str(project_jsons.get("当前入口项目").expect("entry project json missing")).expect("parse entry project json failed");
        assert_eq!(entry_json["layerManager"]["elemConfigs"]["screen"]["style"]["screenId"], Value::String(b_id.to_string()));
        assert_eq!(
            entry_json["layerManager"]["elemConfigs"]["api"]["data"]["commonApi"]["commonApiId"],
            Value::String(common_api_id.to_string())
        );
        assert_eq!(
            entry_json["layerManager"]["elemConfigs"]["db"]["data"]["database"]["targetDb"],
            Value::String(common_database_id.to_string())
        );
    }

    #[tokio::test]
    async fn import_project_zip_impl_restores_project_graph_structure_and_scoped_dynamic_scripts_for_v3() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前入口项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let dynamic_scripts = vec![
            DynamicScriptRef {
                name: "全局远程脚本".to_string(),
                ty: "REMOTE".to_string(),
                scope: "GLOBAL".to_string(),
                owner_source_project_id: None,
                original: None,
                blob: None,
                url: Some("https://example.com/global.js".to_string()),
            },
            DynamicScriptRef {
                name: "B1 项目脚本".to_string(),
                ty: "LOCAL".to_string(),
                scope: "PROJECT".to_string(),
                owner_source_project_id: Some("B1".to_string()),
                original: Some("/scripts/b1-local.js".to_string()),
                blob: Some("blobs/b1-local.js".to_string()),
                url: None,
            },
        ];

        let manifest = build_v3_manifest(dynamic_scripts);
        let a_json = r#"{"layerManager":{"elemConfigs":{"screen":{"base":{"type":"ScreenReference"},"style":{"screenId":"B"}}}}}"#;
        let a1_json = r#"{"layerManager":{"elemConfigs":{"screen":{"base":{"type":"ScreenReference"},"style":{"screenId":"B1"}}}}}"#;
        let a1c_json = r#"{"layerManager":{"elemConfigs":{"title":{"base":{"type":"Text"}}}}}"#;
        let b_json = r#"{"layerManager":{"elemConfigs":{"text":{"base":{"type":"Text"}}}}}"#;
        let b1_json = r#"{"layerManager":{"elemConfigs":{"rect":{"base":{"type":"Rect"}}}}}"#;
        let zip_bytes = build_v3_graph_zip(
            &manifest,
            &[("A", a_json), ("A1", a1_json), ("A1C", a1c_json), ("B", b_json), ("B1", b1_json)],
            &[("blobs/b1-local.js", b"console.log('b1 local script');")],
        );
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-v3");

        import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "")
            .await
            .expect("import v3 project graph failed");

        let project_rows = sqlx::query("SELECT id, name, pid, data_json FROM project WHERE deleted = 0 ORDER BY name ASC")
            .fetch_all(&db.pool)
            .await
            .expect("query projects failed");
        assert_eq!(project_rows.len(), 5);

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

        let a_id = *project_ids.get("A").expect("A missing");
        let b_id = *project_ids.get("B").expect("B missing");
        let b1_id = *project_ids.get("B1").expect("B1 missing");
        let a1c_id = *project_ids.get("A1C").expect("A1C missing");

        assert_eq!(project_ids.get("当前入口项目").copied(), Some(target_project_id));
        assert_eq!(project_pids.get("A").copied().flatten(), None);
        assert_eq!(project_pids.get("B").copied().flatten(), None);
        assert_eq!(project_pids.get("当前入口项目").copied().flatten(), Some(a_id));
        assert_eq!(project_pids.get("A1C").copied().flatten(), Some(target_project_id));
        assert_eq!(project_pids.get("B1").copied().flatten(), Some(b_id));

        let entry_json: Value = serde_json::from_str(project_jsons.get("当前入口项目").unwrap()).expect("parse entry json failed");
        assert_eq!(entry_json["layerManager"]["elemConfigs"]["screen"]["style"]["screenId"], Value::String(b1_id.to_string()));

        let a_project_json: Value = serde_json::from_str(project_jsons.get("A").unwrap()).expect("parse A json failed");
        assert_eq!(a_project_json["layerManager"]["elemConfigs"]["screen"]["style"]["screenId"], Value::String(b_id.to_string()));

        let a1c_json_value: Value = serde_json::from_str(project_jsons.get("A1C").unwrap()).expect("parse A1C json failed");
        assert_eq!(a1c_json_value["layerManager"]["elemConfigs"]["title"]["base"]["type"], Value::String("Text".to_string()));

        let dynamic_rows = sqlx::query("SELECT name, type, url, project_id FROM dynamic_script WHERE deleted = 0 ORDER BY name ASC")
            .fetch_all(&db.pool)
            .await
            .expect("query dynamic scripts failed");
        assert_eq!(dynamic_rows.len(), 2);

        let mut dynamic_scripts = HashMap::new();
        for row in dynamic_rows {
            let name: String = row.try_get("name").expect("read dynamic script name failed");
            let ty: String = row.try_get("type").expect("read dynamic script type failed");
            let url: String = row.try_get("url").expect("read dynamic script url failed");
            let project_id: Option<String> = row.try_get("project_id").expect("read dynamic script project_id failed");
            dynamic_scripts.insert(name, (ty, url, project_id));
        }

        assert_eq!(
            dynamic_scripts.get("全局远程脚本"),
            Some(&("REMOTE".to_string(), "https://example.com/global.js".to_string(), None))
        );
        assert_eq!(
            dynamic_scripts.get("B1 项目脚本"),
            Some(&("LOCAL".to_string(), "b1-local.js".to_string(), Some(b1_id.to_string())))
        );

        assert!(
            app_state.static_storage_dir().join("lib").join("b1-local.js").exists(),
            "project-scoped local script should be written into static/lib"
        );

        assert_ne!(a1c_id, target_project_id);
    }

    #[tokio::test]
    async fn import_project_zip_impl_restores_root_project_reference_to_root_project_for_v3() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前入口项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let manifest =
            build_v3_manifest_with_projects("A", vec![manifest_project_node("A", None, true), manifest_project_node("B", None, false)], vec![]);
        let zip_bytes = build_v3_graph_zip(
            &manifest,
            &[
                (
                    "A",
                    r#"{"layerManager":{"elemConfigs":{"screen":{"base":{"type":"ScreenReference"},"style":{"screenId":"B"}}}}}"#,
                ),
                ("B", r#"{"layerManager":{"elemConfigs":{"text":{"base":{"type":"Text"}}}}}"#),
            ],
            &[],
        );
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-v3-root-to-root");

        import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "")
            .await
            .expect("import root-to-root graph failed");

        let (project_ids, project_pids, project_jsons) = load_imported_project_graph(&db).await;
        assert_eq!(project_ids.len(), 2);
        let b_id = *project_ids.get("B").expect("B missing");
        assert_eq!(project_ids.get("当前入口项目").copied(), Some(target_project_id));
        assert_eq!(project_pids.get("当前入口项目").copied().flatten(), None);
        assert_eq!(project_pids.get("B").copied().flatten(), None);

        let entry_json: Value = serde_json::from_str(project_jsons.get("当前入口项目").unwrap()).expect("parse entry json failed");
        assert_eq!(entry_json["layerManager"]["elemConfigs"]["screen"]["style"]["screenId"], Value::String(b_id.to_string()));
    }

    #[tokio::test]
    async fn import_project_zip_impl_restores_root_project_reference_to_child_project_for_v3() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前入口项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let manifest =
            build_v3_manifest_with_projects("A", vec![manifest_project_node("A", None, true), manifest_project_node("A1", Some("A"), false)], vec![]);
        let zip_bytes = build_v3_graph_zip(
            &manifest,
            &[
                (
                    "A",
                    r#"{"layerManager":{"elemConfigs":{"screen":{"base":{"type":"ScreenReference"},"style":{"screenId":"A1"}}}}}"#,
                ),
                ("A1", r#"{"layerManager":{"elemConfigs":{"title":{"base":{"type":"Text"}}}}}"#),
            ],
            &[],
        );
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-v3-root-to-child");

        import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "")
            .await
            .expect("import root-to-child graph failed");

        let (project_ids, project_pids, project_jsons) = load_imported_project_graph(&db).await;
        assert_eq!(project_ids.len(), 2);
        let a1_id = *project_ids.get("A1").expect("A1 missing");
        assert_eq!(project_pids.get("当前入口项目").copied().flatten(), None);
        assert_eq!(project_pids.get("A1").copied().flatten(), Some(target_project_id));

        let entry_json: Value = serde_json::from_str(project_jsons.get("当前入口项目").unwrap()).expect("parse entry json failed");
        assert_eq!(entry_json["layerManager"]["elemConfigs"]["screen"]["style"]["screenId"], Value::String(a1_id.to_string()));
    }

    #[tokio::test]
    async fn import_project_zip_impl_restores_root_project_reference_to_external_child_for_v3() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前入口项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let manifest = build_v3_manifest_with_projects(
            "A",
            vec![
                manifest_project_node("A", None, true),
                manifest_project_node("B", None, false),
                manifest_project_node("B1", Some("B"), false),
            ],
            vec![],
        );
        let zip_bytes = build_v3_graph_zip(
            &manifest,
            &[
                (
                    "A",
                    r#"{"layerManager":{"elemConfigs":{"screen":{"base":{"type":"ScreenReference"},"style":{"screenId":"B1"}}}}}"#,
                ),
                ("B", r#"{"layerManager":{"elemConfigs":{"text":{"base":{"type":"Text"}}}}}"#),
                ("B1", r#"{"layerManager":{"elemConfigs":{"rect":{"base":{"type":"Rect"}}}}}"#),
            ],
            &[],
        );
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-v3-root-to-external-child");

        import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "")
            .await
            .expect("import root-to-external-child graph failed");

        let (project_ids, project_pids, project_jsons) = load_imported_project_graph(&db).await;
        assert_eq!(project_ids.len(), 3);
        let b_id = *project_ids.get("B").expect("B missing");
        let b1_id = *project_ids.get("B1").expect("B1 missing");
        assert_eq!(project_pids.get("当前入口项目").copied().flatten(), None);
        assert_eq!(project_pids.get("B").copied().flatten(), None);
        assert_eq!(project_pids.get("B1").copied().flatten(), Some(b_id));

        let entry_json: Value = serde_json::from_str(project_jsons.get("当前入口项目").unwrap()).expect("parse entry json failed");
        assert_eq!(entry_json["layerManager"]["elemConfigs"]["screen"]["style"]["screenId"], Value::String(b1_id.to_string()));
    }

    #[tokio::test]
    async fn import_project_zip_impl_keeps_entry_project_as_child_for_v3() {
        let (db, app_state) = build_test_env().await;
        let target_project_id =
            sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('当前入口项目', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool)
                .await
                .expect("insert target project failed")
                .last_insert_rowid();

        let manifest = build_v3_manifest_with_projects(
            "A1",
            vec![manifest_project_node("A", None, false), manifest_project_node("A1", Some("A"), true)],
            vec![],
        );
        let zip_bytes = build_v3_graph_zip(
            &manifest,
            &[
                ("A", r#"{"layerManager":{"elemConfigs":{"title":{"base":{"type":"Text"}}}}}"#),
                ("A1", r#"{"layerManager":{"elemConfigs":{"text":{"base":{"type":"Text"}}}}}"#),
            ],
            &[],
        );
        let zip_path = write_zip_to_temp(&zip_bytes, "desktop-import-v3-entry-child");

        import_project_zip_impl(&db, &app_state, target_project_id, zip_path.to_str().unwrap(), "")
            .await
            .expect("import entry-child graph failed");

        let (project_ids, project_pids, _project_jsons) = load_imported_project_graph(&db).await;
        assert_eq!(project_ids.len(), 2);
        let a_id = *project_ids.get("A").expect("A missing");
        assert_eq!(project_pids.get("A").copied().flatten(), None);
        assert_eq!(project_pids.get("当前入口项目").copied().flatten(), Some(a_id));
    }
