/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use super::*;

use crate::ConnectionConfig;
use serde_json::Value;

fn config_with_url(url: &str) -> ConnectionConfig {
    ConnectionConfig {
        username: "scott".into(),
        password: "tiger".into(),
        url: Some(url.into()),
        ..Default::default()
    }
}

#[test]
fn default_connection_descriptor_keeps_oracle_service_name_and_ipv6_brackets() {
    let config = ConnectionConfig {
        host: "[2001:db8::1]".into(),
        port: "1521".into(),
        database: "XEPDB1".into(),
        username: "scott".into(),
        password: "tiger".into(),
        ..Default::default()
    };
    let descriptor = oracle_config::driver_config(&config)
        .unwrap()
        .get_connect_descriptor();
    assert!(descriptor.contains("(HOST=[2001:db8::1])"), "{descriptor}");
    assert!(descriptor.contains("(SERVICE_NAME=XEPDB1)"), "{descriptor}");
}

#[test]
fn default_service_names_with_driver_special_characters_are_not_truncated() {
    for service_name in ["APP-PDB", "APP$PDB", "APP#PDB"] {
        let config = ConnectionConfig {
            host: "db.example".into(),
            port: "1521".into(),
            database: service_name.into(),
            username: "scott".into(),
            password: "tiger".into(),
            ..Default::default()
        };
        let descriptor = oracle_config::driver_config(&config)
            .unwrap()
            .get_connect_descriptor();
        assert!(
            descriptor.contains(&format!("(SERVICE_NAME={service_name})")),
            "{descriptor}"
        );
    }
}

#[test]
fn custom_easy_connect_urls_reject_service_names_the_driver_would_truncate() {
    for service_name in ["APP-PDB", "APP$PDB", "APP#PDB"] {
        for url in [
            format!("db.example:1521/{service_name}"),
            format!("jdbc:oracle:thin:@//db.example:1521/{service_name}"),
            format!("jdbc:oracle:thin:@tcps://db.example:2484/{service_name}"),
        ] {
            let error = oracle_config::connect_string(&config_with_url(&url)).unwrap_err();
            assert_eq!(error.kind, ErrorKind::Configuration, "{url}");
            assert!(error.message.contains("描述符"), "{url}: {}", error.message);
        }
    }
}

#[test]
fn custom_easy_connect_urls_preserve_complete_instance_names_in_driver_descriptor() {
    for (url, expected_instance, protocol, port, server_type) in [
        (
            "jdbc:oracle:thin:@//[2001:db8::1]:1521/APP/INST_1",
            "INST_1",
            "tcp",
            "1521",
            None,
        ),
        (
            "jdbc:oracle:thin:@tcps://[2001:db8::1]:2484/APP:DEDICATED/INST.1",
            "INST.1",
            "tcps",
            "2484",
            Some("dedicated"),
        ),
    ] {
        let descriptor = oracle_config::driver_config(&config_with_url(url))
            .unwrap()
            .get_connect_descriptor();
        let descriptor_lowercase = descriptor.to_ascii_lowercase();
        assert!(descriptor.contains("(SERVICE_NAME=APP)"), "{descriptor}");
        assert!(
            descriptor.contains(&format!("(INSTANCE_NAME={expected_instance})")),
            "{descriptor}"
        );
        assert!(descriptor.contains("[2001:db8::1]"), "{descriptor}");
        assert!(
            descriptor_lowercase.contains(&format!("(protocol={protocol})")),
            "{descriptor}"
        );
        assert!(
            descriptor_lowercase.contains(&format!("(port={port})")),
            "{descriptor}"
        );
        if let Some(server_type) = server_type {
            assert!(
                descriptor_lowercase.contains(&format!("(server={server_type})")),
                "{descriptor}"
            );
        }
    }
}

#[test]
fn custom_easy_connect_urls_reject_truncated_or_ignored_instance_suffixes() {
    for instance_segment in ["APP-PDB", "APP$PDB", "APP#PDB"] {
        for url in [
            format!("db.example:1521/APP/{instance_segment}"),
            format!("jdbc:oracle:thin:@//db.example:1521/APP/{instance_segment}"),
        ] {
            let error = oracle_config::driver_config(&config_with_url(&url))
                .err()
                .expect("unsafe instance name should be rejected");
            assert_eq!(error.kind, ErrorKind::Configuration, "{url}");
            assert!(error.message.contains("描述符"), "{url}: {}", error.message);
        }
    }

    for url in [
        "db.example:1521/APP/INST/EXTRA",
        "jdbc:oracle:thin:@tcps://db.example:2484/APP:DEDICATED/INST/EXTRA",
    ] {
        let error = oracle_config::driver_config(&config_with_url(url))
            .err()
            .expect("trailing instance path should be rejected");
        assert_eq!(error.kind, ErrorKind::Configuration, "{url}");
        assert!(error.message.contains("描述符"), "{url}: {}", error.message);
    }
}

#[test]
fn jdbc_service_and_web_legacy_sid_urls_are_supported() {
    assert_eq!(
        oracle_config::connect_string(&config_with_url(
            "jdbc:oracle:thin:@//db.example:1521/XEPDB1"
        ))
        .unwrap(),
        "//db.example:1521/XEPDB1"
    );
    assert_eq!(
        oracle_config::connect_string(&config_with_url(
            "jdbc:oracle:thin:@:db.example:1521:ORCL"
        ))
        .unwrap(),
        "(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST=db.example)(PORT=1521))(CONNECT_DATA=(SID=ORCL)))"
    );
    let sid_descriptor =
        oracle_config::driver_config(&config_with_url("jdbc:oracle:thin:@db.example:1521:ORCL"))
            .unwrap()
            .get_connect_descriptor();
    assert!(sid_descriptor.contains("(SID=ORCL)"), "{sid_descriptor}");
    assert!(!sid_descriptor.contains("SERVICE_NAME"), "{sid_descriptor}");
}

#[test]
fn oracle_descriptor_and_jdbc_sid_are_supported_but_oci_is_rejected() {
    let descriptor = "(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST=db)(PORT=1521))(CONNECT_DATA=(SERVICE_NAME=APP-PDB)))";
    assert_eq!(
        oracle_config::connect_string(&config_with_url(descriptor)).unwrap(),
        descriptor
    );
    let parsed_descriptor = oracle_config::driver_config(&config_with_url(descriptor))
        .unwrap()
        .get_connect_descriptor();
    assert!(
        parsed_descriptor.contains("(SERVICE_NAME=APP-PDB)"),
        "{parsed_descriptor}"
    );
    assert_eq!(
        oracle_config::connect_string(&config_with_url("jdbc:oracle:thin:@db:1521:ORCL")).unwrap(),
        "(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST=db)(PORT=1521))(CONNECT_DATA=(SID=ORCL)))"
    );
    let error = oracle_config::connect_string(&config_with_url("jdbc:oracle:oci:@db:1521/APP"))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Configuration);
    assert!(error.message.contains("Thin"));

    let embedded_credentials = oracle_config::connect_string(&config_with_url(
        "jdbc:oracle:thin:scott/tiger@//db:1521/APP",
    ))
    .unwrap_err();
    assert!(embedded_credentials.message.contains("独立字段"));
    assert!(!embedded_credentials.message.contains("scott"));
    assert!(!embedded_credentials.message.contains("tiger"));
}

#[test]
fn oracle_net_descriptor_preserves_special_instance_names() {
    for instance_name in ["APP-PDB", "APP$PDB", "APP#PDB"] {
        let descriptor = format!(
            "(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST=db.example)(PORT=1521))(CONNECT_DATA=(SERVICE_NAME=APP)(INSTANCE_NAME={instance_name})))"
        );
        let parsed_descriptor = oracle_config::driver_config(&config_with_url(&descriptor))
            .unwrap()
            .get_connect_descriptor();
        assert!(
            parsed_descriptor.contains(&format!("(INSTANCE_NAME={instance_name})")),
            "{parsed_descriptor}"
        );
    }
}

#[test]
fn default_fields_reject_invalid_port_or_service() {
    let invalid_port = ConnectionConfig {
        host: "db.example".into(),
        port: "70000".into(),
        database: "APP".into(),
        ..Default::default()
    };
    assert_eq!(
        oracle_config::connect_string(&invalid_port)
            .unwrap_err()
            .kind,
        ErrorKind::Configuration
    );
    let invalid_service = ConnectionConfig {
        host: "db.example".into(),
        port: "1521".into(),
        database: "APP SERVICE".into(),
        ..Default::default()
    };
    assert_eq!(
        oracle_config::connect_string(&invalid_service)
            .unwrap_err()
            .kind,
        ErrorKind::Configuration
    );
}

#[test]
fn oracle_number_is_json_number_and_timestamp_keeps_timezone_semantics() {
    let value = oracle_values::number_to_json(oracledb::OracleNumber::default()).unwrap();
    assert!(matches!(value, Value::Number(_)));
    for expected in [
        "12345678901234567890123456789012345678",
        "-98765432109876543210987654321098765432",
    ] {
        let number = expected.parse::<oracledb::OracleNumber>().unwrap();
        let json = oracle_values::number_to_json(number).unwrap();
        assert!(matches!(json, Value::Number(_)));
        assert_eq!(json.to_string(), expected);
    }
    assert_eq!(
        oracle_values::timestamp_to_string(
            oracledb::OracleTimestamp::new_timestamp(2026, 10, 19, 12, 34, 56, 123_000_000),
            "DB_TYPE_TIMESTAMP"
        ),
        "2026-10-19T12:34:56.123000000"
    );
    assert_eq!(
        oracle_values::timestamp_to_string(
            oracledb::OracleTimestamp::new_timestamp_tz(2026, 10, 19, 12, 34, 56, 0, 5, 30),
            "DB_TYPE_TIMESTAMP_TZ"
        ),
        "2026-10-19T12:34:56+05:30"
    );
}

#[tokio::test]
async fn connection_refusal_is_reported_as_connection_error() {
    let config = ConnectionConfig {
        host: "127.0.0.1".into(),
        port: "1".into(),
        database: "APP".into(),
        username: "scott".into(),
        password: "tiger".into(),
        ..Default::default()
    };
    let error = test_connection(config).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Connection);
}

#[tokio::test]
async fn dropping_request_cancels_its_blocking_worker() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    let started = Arc::new(AtomicBool::new(false));
    let observed_cancel = Arc::new(AtomicBool::new(false));
    let worker_started = Arc::clone(&started);
    let worker_observed = Arc::clone(&observed_cancel);
    let task = tokio::spawn(async move {
        run_blocking(
            ErrorKind::Query,
            Duration::from_secs(30),
            move |cancelled| {
                worker_started.store(true, Ordering::Release);
                while !cancelled.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(2));
                }
                worker_observed.store(true, Ordering::Release);
                Ok(())
            },
        )
        .await
    });

    while !started.load(Ordering::Acquire) {
        tokio::task::yield_now().await;
    }
    task.abort();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !observed_cancel.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("blocking worker should observe cancellation after caller aborts");
}

#[tokio::test]
#[ignore = "requires DATAVISOOL_ORACLE_URL, DATAVISOOL_ORACLE_USERNAME and DATAVISOOL_ORACLE_PASSWORD"]
async fn configured_oracle_smoke_test() {
    let config = ConnectionConfig {
        username: std::env::var("DATAVISOOL_ORACLE_USERNAME").expect("DATAVISOOL_ORACLE_USERNAME is required"),
        password: std::env::var("DATAVISOOL_ORACLE_PASSWORD").expect("DATAVISOOL_ORACLE_PASSWORD is required"),
        url: Some(std::env::var("DATAVISOOL_ORACLE_URL").expect("DATAVISOOL_ORACLE_URL is required")),
        ..Default::default()
    };
    test_connection(config.clone()).await.unwrap();
    let result = query(config, "SELECT 1 AS VALUE FROM DUAL".into())
        .await
        .unwrap();
    assert_eq!(result[0]["VALUE"], 1);
}
