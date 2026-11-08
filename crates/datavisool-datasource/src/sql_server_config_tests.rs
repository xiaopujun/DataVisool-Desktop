/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use super::*;

fn from_url(url: &str) -> Result<SqlServerConfig, DriverError> {
    SqlServerConfig::from_connection_config(&ConnectionConfig {
        url: Some(url.to_string()),
        ..Default::default()
    })
}

#[test]
fn explicit_port_ignores_instance_names_in_authority_and_properties() {
    for url in [
        "jdbc:sqlserver://db\\REPORTS:1433;instanceName=OTHER",
        "jdbc:sqlserver://db\\REPORTS;portNumber=1433;instanceName=OTHER",
        "jdbc:sqlserver://;serverName=db\\REPORTS;portNumber=1433;instanceName=OTHER",
        "jdbc:sqlserver://db\\REPORTS;serverName=db\\OTHER;portNumber=1433",
    ] {
        let parsed = from_url(url).unwrap();
        assert_eq!(parsed.target.port, Some(1433), "{url}");
        assert!(parsed.target.instance.is_none(), "{url}");
        // Inspect the actual driver configuration, not just the network target:
        // a leftover instance would still be sent in the PRELOGIN packet.
        assert!(
            format!("{:?}", parsed.client).contains("instance_name: None"),
            "the driver must not validate an ignored instance: {url}"
        );
    }
}

#[test]
fn named_instance_is_retained_when_no_port_is_given() {
    let parsed = from_url("jdbc:sqlserver://db\\REPORTS").unwrap();
    assert_eq!(parsed.target.instance.as_deref(), Some("REPORTS"));
    assert!(parsed.target.port.is_none());
    assert!(from_url("jdbc:sqlserver://db\\REPORTS;instanceName=OTHER").is_err());
}

#[test]
fn explicit_form_port_also_ignores_embedded_instance() {
    let parsed = SqlServerConfig::from_connection_config(&ConnectionConfig {
        host: "db\\REPORTS".into(),
        port: "1433".into(),
        ..Default::default()
    })
    .unwrap();
    assert!(parsed.target.instance.is_none());
    assert_eq!(parsed.target.port, Some(1433));
}

#[test]
fn socket_and_query_timeouts_are_independent() {
    for (properties, query_seconds, socket_millis) in [
        ("", 30, None),
        ("socketTimeout=1000", 30, Some(1000)),
        ("queryTimeout=7;socketTimeout=1000", 7, Some(1000)),
        ("queryTimeout=7;socketTimeout=0", 7, None),
    ] {
        let parsed = from_url(&format!("jdbc:sqlserver://db;{properties}")).unwrap();
        assert_eq!(parsed.connect_timeout, DEFAULT_CONNECT_TIMEOUT);
        assert_eq!(
            parsed.query_timeout,
            Some(Duration::from_secs(query_seconds))
        );
        assert_eq!(
            parsed.socket_timeout,
            socket_millis.map(Duration::from_millis)
        );
    }
}

#[test]
fn form_fields_preserve_complete_unbracketed_ipv6_hosts() {
    for (host, expected) in [
        ("::1", "::1"),
        ("2001:db8::1", "2001:db8::1"),
        ("2001:db8::abcd", "2001:db8::abcd"),
        ("[::1]", "::1"),
        ("[2001:db8::abcd]", "2001:db8::abcd"),
    ] {
        for port in ["1433", ""] {
            let parsed = SqlServerConfig::from_connection_config(&ConnectionConfig {
                host: host.into(),
                port: port.into(),
                ..Default::default()
            })
            .unwrap();
            assert_eq!(parsed.target.host, expected, "{host}:{port}");
            assert_eq!(
                parsed.target.host.parse::<Ipv6Addr>().unwrap(),
                expected.parse::<Ipv6Addr>().unwrap()
            );
            assert_eq!(parsed.target.port, (!port.is_empty()).then_some(1433));
            assert!(parsed.target.instance.is_none());
        }
    }
}

#[test]
fn unlimited_query_timeout_keeps_login_and_socket_deadlines() {
    for value in ["0", "-1", "+0", "-0"] {
        let parsed = from_url(&format!(
            "jdbc:sqlserver://db;queryTimeout={value};loginTimeout=3;socketTimeout=25"
        ))
        .unwrap();
        assert_eq!(parsed.query_timeout, None);
        assert_eq!(parsed.connect_timeout, Duration::from_secs(3));
        assert_eq!(parsed.socket_timeout, Some(Duration::from_millis(25)));
    }
    assert_eq!(
        from_url("jdbc:sqlserver://db;queryTimeout=1")
            .unwrap()
            .query_timeout,
        Some(Duration::from_secs(1))
    );
}

#[test]
fn invalid_query_timeouts_are_still_configuration_errors() {
    for value in ["-2", "abc", "", "18446744073709551616"] {
        let error = from_url(&format!("jdbc:sqlserver://db;queryTimeout={value}"))
            .err()
            .expect("invalid query timeout should fail configuration");
        assert_eq!(error.kind, ErrorKind::Configuration);
        assert!(error.message.contains("queryTimeout"));
    }
}

#[test]
fn socket_timeout_rejects_invalid_or_out_of_range_milliseconds() {
    for value in ["-1", "abc", "2147483648", "18446744073709551615"] {
        let error = from_url(&format!("jdbc:sqlserver://db;socketTimeout={value}"))
            .err()
            .expect("invalid milliseconds should fail configuration");
        assert_eq!(error.kind, ErrorKind::Configuration);
        assert!(error.message.contains("socketTimeout"));
    }
}
