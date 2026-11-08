/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

#[tokio::test]
async fn missing_datasources_are_cleared_and_reported_even_when_local_ids_exist() {
    let (db, app_state) = build_test_env().await;
    let project_id = sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('target', '', '0', NULL, '{}', '', 0)")
        .execute(&db.pool).await.unwrap().last_insert_rowid();
    sqlx::query("INSERT INTO common_api (id, name, url, method, deleted) VALUES (42, 'local', 'https://local.invalid', 'GET', 0)")
        .execute(&db.pool).await.unwrap();
    let project = serde_json::json!({
        "layerManager": {"elemConfigs": {
            "api": {"data": {"sourceType": "commonApi", "commonApi": {"commonApiId": "42", "refKey": "missing"}}},
            "loop": {"managerData": {"layerManager": {"elemConfigs": {
                "db": {"data": {"sourceType": "database", "database": {"targetDb": "99", "refKey": "missing-db"}}}
            }}}},
            "map": {"style": {"sources": [{"sourceType": "geojson", "dataMode": "dataConfig", "dataConfig": {"sourceType": "commonApi", "commonApi": {"commonApiId": "88"}}}]}},
            "text": {"style": {"commonApiId": "42", "refKey": "keep"}, "data": {"sourceType": "static", "staticData": {"value": {"layerManager": {"elemConfigs": {"user": {"data": {"sourceType": "commonApi", "commonApi": {"commonApiId": "42"}}}}}}}}}
        }},
        "globalVariableManager": {"globalVariables": {"v": {"triggerSource": {"sourceType": "database", "database": {"targetDb": "12"}}}}}
    });
    let json = project.to_string();
    let zip = build_export_zip(&build_v2_manifest(), [ZipEntryBytes {path: "project/project.json", bytes: json.as_bytes()}], &[]).unwrap();
    let path = write_zip_to_temp(&zip, "missing-datasources");
    let response = import_project_from_path(&db, &app_state, serde_json::json!({"projectId": project_id.to_string(), "path": path})).await.unwrap();
    assert_eq!(response.code, 200);
    assert_eq!(response.data["warnings"].as_array().unwrap().len(), 4);
    let stored: String = sqlx::query_scalar("SELECT data_json FROM project WHERE id = ?1").bind(project_id).fetch_one(&db.pool).await.unwrap();
    let v: Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(v["layerManager"]["elemConfigs"]["api"]["data"]["commonApi"], serde_json::json!({"commonApiId":"", "refKey":""}));
    assert_eq!(v["layerManager"]["elemConfigs"]["loop"]["managerData"]["layerManager"]["elemConfigs"]["db"]["data"]["database"]["targetDb"], "");
    assert_eq!(v["layerManager"]["elemConfigs"]["map"]["style"]["sources"][0]["dataConfig"]["commonApi"]["commonApiId"], "");
    assert_eq!(v["globalVariableManager"]["globalVariables"]["v"]["triggerSource"]["database"]["targetDb"], "");
    assert_eq!(v["layerManager"]["elemConfigs"]["text"]["style"]["commonApiId"], "42");
    assert_eq!(v["layerManager"]["elemConfigs"]["text"]["data"]["staticData"]["value"]["layerManager"]["elemConfigs"]["user"]["data"]["commonApi"]["commonApiId"], "42");
    assert!(response.data["warnings"].to_string().contains("sources.0.dataConfig"));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn missing_datasources_are_reported_across_project_graph() {
    let (db, app_state) = build_test_env().await;
    let project_id = sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('target', '', '0', NULL, '{}', '', 0)")
        .execute(&db.pool).await.unwrap().last_insert_rowid();
    let manifest = build_v3_manifest_with_projects("A", vec![manifest_project_node("A", None, true), manifest_project_node("B", Some("A"), false)], vec![]);
    let project = r#"{"layerManager":{"elemConfigs":{"api":{"data":{"sourceType":"commonApi","commonApi":{"commonApiId":"42"}}}}}}"#;
    let zip = build_v3_graph_zip(&manifest, &[("A", project), ("B", project)], &[]);
    let path = write_zip_to_temp(&zip, "missing-graph-datasources");
    let warnings = import_project_zip_impl(&db, &app_state, project_id, path.to_str().unwrap(), "").await.unwrap();
    assert_eq!(warnings.len(), 2);
    assert_ne!(warnings[0], warnings[1]);
    let stored: Vec<String> = sqlx::query_scalar("SELECT data_json FROM project WHERE deleted = 0").fetch_all(&db.pool).await.unwrap();
    assert_eq!(stored.len(), 2);
    for json in stored {
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["layerManager"]["elemConfigs"]["api"]["data"]["commonApi"]["commonApiId"], "");
    }
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn poster_export_zip_can_be_imported_and_rewritten() {
    let (db, app_state) = build_test_env().await;
    let target = sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('target', '', '0', NULL, '{}', '', 0)")
        .execute(&db.pool).await.unwrap().last_insert_rowid();
    let images = app_state.static_storage_dir().join("images");
    std::fs::create_dir_all(&images).unwrap();
    std::fs::write(images.join("my poster+.png"), b"poster bytes").unwrap();
    let project = serde_json::json!({"layerManager":{"elemConfigs":{"video":{"base":{"type":"BaseVideo"},"style":{"poster":"/static/images/my%20poster%2B.png?size=1#top"}}}}});
    let path = app_state.files_dir.join("poster-export.zip");
    let response = crate::backend::routes::export::export_project_to_path(&db, &app_state, serde_json::json!({
        "id": target.to_string(), "name": "poster", "projectJson": project.to_string(),
        "images": ["my poster+.png"], "savePath": path
    })).await.unwrap();
    assert_eq!(response.code, 200, "{}", response.msg);
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let mut image = Vec::new();
    archive.by_name("blobs/my poster+.png").unwrap().read_to_end(&mut image).unwrap();
    assert_eq!(image, b"poster bytes");
    drop(archive);
    import_project_zip_impl(&db, &app_state, target, path.to_str().unwrap(), "").await.unwrap();
    let stored: String = sqlx::query_scalar("SELECT data_json FROM project WHERE id = ?1").bind(target).fetch_one(&db.pool).await.unwrap();
    let value: Value = serde_json::from_str(&stored).unwrap();
    let expected = format!("{}?size=1#top", to_asset_url(&images.join("my poster+.png")));
    assert_eq!(value["layerManager"]["elemConfigs"]["video"]["style"]["poster"], expected);
}
