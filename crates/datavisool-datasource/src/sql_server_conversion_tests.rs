/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use super::{query_deadline, values, with_query_deadline};
use crate::{ConnectionConfig, ErrorKind};
use std::{
    borrow::Cow,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tiberius::{ColumnData, ColumnType, Row};
use tokio::time::Instant;

fn binary_row(bytes: Vec<u8>) -> Row {
    Row::builder()
        .column(
            "payload",
            ColumnType::BigVarBin,
            ColumnData::Binary(Some(Cow::Owned(bytes))),
        )
        .build()
}

#[tokio::test(flavor = "current_thread")]
async fn query_deadline_covers_the_final_large_binary_conversion() {
    let row = binary_row(vec![0xa5; 8 * 1024 * 1024]);
    let deadline = query_deadline(Some(Duration::from_secs(1))).unwrap();
    let operation = async move {
        tokio::time::sleep(Duration::from_millis(990)).await;
        values::rows_to_json(vec![row], Some(deadline)).await
    };

    let error = with_query_deadline(Some(deadline), operation)
        .await
        .expect_err("the conversion must consume the remaining query budget");
    assert_eq!(error.kind, ErrorKind::Query);
    assert!(error.message.contains("查询超时"));
}

#[tokio::test(flavor = "current_thread")]
async fn unlimited_deadline_yields_during_a_large_single_binary_value() {
    let bytes = vec![0xa5; 8 * 1024 * 1024];
    let expected = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    let conversion_finished = Arc::new(AtomicBool::new(false));
    let finished = Arc::clone(&conversion_finished);
    let observed_yield = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&observed_yield);
    let observer = tokio::spawn(async move {
        observed.store(!finished.load(Ordering::Relaxed), Ordering::Relaxed);
    });

    let value = with_query_deadline(None, values::rows_to_json(vec![binary_row(bytes)], None))
        .await
        .expect("None queryTimeout must allow conversion to complete");
    conversion_finished.store(true, Ordering::Relaxed);
    observer.await.unwrap();

    assert!(observed_yield.load(Ordering::Relaxed));
    assert_eq!(value[0]["payload"], expected);
}

#[tokio::test]
async fn normal_conversion_succeeds_before_its_deadline() {
    let row = Row::builder()
        .column("id", ColumnType::Int4, ColumnData::I32(Some(42)))
        .column(
            "name",
            ColumnType::NVarchar,
            ColumnData::String(Some("report".into())),
        )
        .build();
    let deadline = query_deadline(Some(Duration::from_secs(1))).unwrap();

    let value = with_query_deadline(
        Some(deadline),
        values::rows_to_json(vec![row], Some(deadline)),
    )
    .await
    .unwrap();

    assert_eq!(value, serde_json::json!([{"id": 42, "name": "report"}]));
}

#[tokio::test]
async fn large_multibyte_text_preserves_utf8_across_conversion_chunks() {
    // The Chinese character starts at byte 65,535, so the first 64 KiB copy
    // boundary falls inside its UTF-8 encoding.
    let expected = format!("{}中🙂-混合文本-🧪", "a".repeat(65_535));
    assert!(expected.len() > 64 * 1024);
    let row = Row::builder()
        .column(
            "text",
            ColumnType::NVarchar,
            ColumnData::String(Some(expected.clone().into())),
        )
        .build();

    let value = values::rows_to_json(vec![row], None).await.unwrap();

    assert_eq!(value[0]["text"], expected);
}

#[tokio::test]
async fn binary_chunk_boundaries_match_one_shot_standard_base64() {
    for length in [
        0, 1, 2, 3, 65_534, 65_535, 65_536, 131_069, 131_070, 131_071, 131_072,
    ] {
        let bytes = (0..length)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let expected = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        let row = binary_row(bytes);

        let value = values::rows_to_json(vec![row], None)
            .await
            .unwrap_or_else(|error| panic!("conversion failed for {length} bytes: {error}"));

        assert_eq!(
            value[0]["payload"], expected,
            "encoded output differs for {length} input bytes"
        );
    }
}

#[test]
fn enormous_positive_timeout_uses_tokio_compatible_far_future_deadline() {
    let deadline = query_deadline(Some(Duration::from_secs(u64::MAX))).unwrap();
    let remaining = deadline.duration_since(Instant::now());
    assert!(remaining > Duration::from_secs(86400 * 365 * 29));
    assert!(remaining <= Duration::from_secs(86400 * 365 * 30));

    let parsed = super::config::SqlServerConfig::from_connection_config(&ConnectionConfig {
        url: Some("jdbc:sqlserver://db;queryTimeout=18446744073709551615".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(parsed.query_timeout, Some(Duration::from_secs(u64::MAX)));
}
