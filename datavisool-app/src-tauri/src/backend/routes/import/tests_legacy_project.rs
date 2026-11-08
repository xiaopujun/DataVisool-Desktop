/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
#[tokio::test]
async fn imported_lcp_project_is_normalized_before_storage() {
    let (db, app_state) = build_test_env().await;
    let target = sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('target', '', '0', NULL, '{}', '', 0)")
        .execute(&db.pool).await.unwrap().last_insert_rowid();
    let json = serde_json::json!({"layerManager":{"elemConfigs":{
        "loop":{"base":{"type":"LoopContainer"},"style":{"gap":12},"data":{"sourceType":"static","staticData":[1,2]}},
        "table":{"base":{"type":"LcBaseTable"},"style":{"body":{"autoScroll":"true","speed":"5","pauseSeconds":"0.3"}}}
    }}}).to_string();
    let manifest = build_v2_manifest();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(json.as_bytes()).unwrap();
    use base64::Engine;
    let compressed = format!("LCPZ1:gzip:{}", base64::engine::general_purpose::STANDARD.encode(gz.finish().unwrap()));
    let bytes = build_export_zip(&manifest, [ZipEntryBytes {path:"project/project.json", bytes:compressed.as_bytes()}], &[]).unwrap();
    let path = write_zip_to_temp(&bytes, "datavisool-legacy-normalized");
    import_project_zip_impl(&db, &app_state, target, path.to_str().unwrap(), "").await.unwrap();
    let stored: String = sqlx::query_scalar("SELECT data_json FROM project WHERE id = ?1").bind(target).fetch_one(&db.pool).await.unwrap();
    let result: Value = serde_json::from_str(&stored).unwrap();
    let components = &result["layerManager"]["elemConfigs"];
    assert_eq!(components["loop"]["style"]["horizontalGap"], 12);
    assert_eq!(components["loop"]["style"]["verticalGap"], 12);
    assert!(components["loop"]["style"].get("gap").is_none());
    assert_eq!(components["loop"]["data"]["staticData"]["value"], serde_json::json!([1,2]));
    assert_eq!(components["table"]["style"]["body"]["enableCarousel"], true);
    assert_eq!(components["table"]["style"]["body"]["carouselSpeed"], 5.0);
    assert!(components["table"]["style"]["body"].get("autoScroll").is_none());
    let _ = std::fs::remove_file(path);
}

#[test]
fn lcp_compressed_data_is_decoded_at_import() {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(br#"{"layerManager":{}}"#).unwrap();
    use base64::Engine;
    let payload = format!("LCPZ1:gzip:{}", base64::engine::general_purpose::STANDARD.encode(gz.finish().unwrap()));
    assert_eq!(normalize_imported_project_json(&payload).unwrap(), r#"{"layerManager":{}}"#);
}

#[test]
fn imported_static_values_are_not_treated_as_project_configuration() {
    let source = serde_json::json!({"sourceType":"static","staticData":{"value":{
        "sourceType":"static","staticData":[1,2],"base":{"type":"LoopContainer"},"style":{"gap":4}
    }}});
    let output = normalize_imported_project_json(&source.to_string()).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&output).unwrap(), source);
}

#[tokio::test]
async fn imported_legacy_asset_urls_point_to_imported_files() {
    let (db, app_state) = build_test_env().await;
    let target = sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('target', '', '0', NULL, '{}', '', 0)")
        .execute(&db.pool).await.unwrap().last_insert_rowid();
    let mut manifest = build_v2_manifest();
    manifest.assets.images = vec![AssetBlobRef { name: "image".into(), original: "photo.png".into(), blob: "blobs/photo.png".into() }];
    manifest.assets.videos = vec![AssetBlobRef { name: "video".into(), original: "video.mov".into(), blob: "blobs/video.mov".into() }];
    let input = serde_json::json!({
        "image": "asset://localhost/%2FUsers%2Ftest%2FLibrary%2FApplication%20Support%2Flcpdesigner%2F.%2Fstatic%2Fimages%2Fphoto.png",
        "video": "http://asset.localhost/C%3A%5CUsers%5Ctest%5Clcpdesigner%5Cstatic%5Cvideos%5Cvideo.mov",
        "missing": "asset://localhost/%2Fother%2Fstatic%2Fimages%2Fmissing.png",
        "external": "https://example.test/static/images/photo.png"
    }).to_string();
    let zip = build_export_zip(&manifest, [
        ZipEntryBytes {path:"project/project.json", bytes:input.as_bytes()},
        ZipEntryBytes {path:"blobs/photo.png", bytes:b"image content"},
        ZipEntryBytes {path:"blobs/video.mov", bytes:b"video content"},
    ], &[]).unwrap();
    let path = write_zip_to_temp(&zip, "legacy-asset-urls");
    import_project_zip_impl(&db, &app_state, target, path.to_str().unwrap(), "").await.unwrap();
    let stored: String = sqlx::query_scalar("SELECT data_json FROM project WHERE id=?1").bind(target).fetch_one(&db.pool).await.unwrap();
    let value: Value = serde_json::from_str(&stored).unwrap();
    let root = app_state.static_storage_dir();
    assert_eq!(value["image"], to_asset_url(&root.join("images/photo.png")));
    assert_eq!(value["video"], to_asset_url(&root.join("videos/video.mov")));
    assert_eq!(value["missing"], "asset://localhost/%2Fother%2Fstatic%2Fimages%2Fmissing.png");
    assert_eq!(value["external"], "https://example.test/static/images/photo.png");
    assert_eq!(std::fs::read(root.join("images/photo.png")).unwrap(), b"image content");
    assert_eq!(std::fs::read(root.join("videos/video.mov")).unwrap(), b"video content");
    let _ = std::fs::remove_file(path);
}

#[test]
fn legacy_asset_urls_are_decoded_only_for_local_asset_protocols() {
    assert_eq!(normalize_imported_static_url("asset://localhost/%2Ftmp%2Fstatic%2Fimages%2F%E5%9B%BE%20%E7%89%87.png"), Some("/static/images/图 片.png".into()));
    assert_eq!(normalize_imported_static_url("asset://localhost.evil/%2Ftmp%2Fstatic%2Fimages%2Fphoto.png"), None);
    assert_eq!(normalize_imported_static_url("https://example.test/static/images/photo.png"), None);
    assert_eq!(normalize_imported_static_url("asset://localhost/%FF"), None);
}

#[tokio::test]
async fn imports_generic_legacy_components_and_rolls_back_unsupported_ones() {
    const SCRIPT: &str = "(function(R, I, U){class UserComponent extends I.AbstractDesignerDefinition {}; I.BaseInfo; U.ObjectUtil; globalThis.__LIGHT_CHASER_REMOTE_COMPONENT_TEMP__ = UserComponent;})(React, LightChaserInterface, LightChaserUtil);";
    for package in [false, true] {
        for supported in [false, true] {
            let (db, app_state) = build_test_env().await;
            let target = sqlx::query("INSERT INTO project (name, des, state, pid, data_json, cover, deleted) VALUES ('target', '', '0', NULL, '{}', '', 0)")
                .execute(&db.pool).await.unwrap().last_insert_rowid();
            let script = if supported { SCRIPT.to_string() } else { SCRIPT.replace("I.BaseInfo", "I.UnsupportedPanel") };
            let bytes = if package {
                let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
                writer.start_file("entry.js", zip::write::SimpleFileOptions::default()).unwrap();
                writer.write_all(script.as_bytes()).unwrap();
                writer.start_file("resources/custom.txt", zip::write::SimpleFileOptions::default()).unwrap();
                writer.write_all(b"component resource").unwrap();
                writer.finish().unwrap().into_inner()
            } else { script.as_bytes().to_vec() };
            let blob = if package { "blobs/custom.zip" } else { "blobs/custom.js" };
            let original = if package { "old-package/entry.js" } else { "custom.js" };
            let mut manifest = build_v2_manifest();
            manifest.assets.components = vec![AssetBlobRef { name: "user component".into(), original: original.into(), blob: blob.into() }];
            let project_json = serde_json::json!({"componentUrl":format!("/static/components/{original}")}).to_string();
            let zip = build_export_zip(&manifest, [ZipEntryBytes { path:"project/project.json", bytes:project_json.as_bytes() }, ZipEntryBytes {path:blob, bytes:&bytes}], &[]).unwrap();
            let path = write_zip_to_temp(&zip, "legacy-component-import");
            let result = import_project_zip_impl(&db, &app_state, target, path.to_str().unwrap(), "").await;
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM remote_component WHERE deleted=0").fetch_one(&db.pool).await.unwrap();
            if supported {
                result.unwrap();
                assert_eq!(count, 1);
                let url: String = sqlx::query_scalar("SELECT url FROM remote_component WHERE deleted=0").fetch_one(&db.pool).await.unwrap();
                let static_root = app_state.static_storage_dir();
                let entry = if package { static_root.parent().unwrap().join(&url) } else { static_root.join("components").join(&url) };
                let output = std::fs::read_to_string(&entry).unwrap();
                assert!(output.contains("__DATAVISOOL_REMOTE_COMPONENT_TEMP__"));
                assert!(output.contains("DataVisoolSDK, DataVisoolSDK"));
                if package { assert_eq!(std::fs::read(entry.parent().unwrap().join("resources/custom.txt")).unwrap(), b"component resource"); }
                let original_name = format!("{}.{}", datavisool_archive::sha256_hex(&bytes), if package { "zip" } else { "js" });
                assert_eq!(std::fs::read(static_root.join("blobs").join(original_name)).unwrap(), bytes);
                let stored: String = sqlx::query_scalar("SELECT data_json FROM project WHERE id=?1").bind(target).fetch_one(&db.pool).await.unwrap();
                assert!(stored.contains("asset://") || stored.contains("asset.localhost"));
            } else {
                assert!(result.unwrap_err().contains("UnsupportedPanel"));
                assert_eq!(count, 0);
                let stored: String = sqlx::query_scalar("SELECT data_json FROM project WHERE id=?1").bind(target).fetch_one(&db.pool).await.unwrap();
                assert_eq!(stored, "{}");
                let components = app_state.static_storage_dir().join("components");
                if components.exists() { assert_eq!(std::fs::read_dir(components).unwrap().count(), 0); }
            }
            let _ = std::fs::remove_file(path);
        }
    }
}
