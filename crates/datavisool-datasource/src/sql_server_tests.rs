/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use super::{
    config::SqlServerConfig, connect_with_routing, parse_instance_response, query,
    resolve_named_instance_candidates, test_connection, ConnectFailure,
};
use crate::{ConnectionConfig, ErrorKind};
use std::{
    net::{SocketAddr, TcpListener},
    time::Duration,
};
use tiberius::{error::Error as TiberiusError, AuthMethod, EncryptionLevel};

#[test]
fn jdbc_url_accepts_named_instance_properties_and_timeouts() {
    let config = ConnectionConfig {
        database: "fallback_db".into(),
        username: "fallback_user".into(),
        password: "fallback_password".into(),
        url: Some(
            concat!(
                "jdbc:sqlserver://db.example.com\\REPORTS;",
                "databaseName=warehouse;user=analyst;password={p@ss;word};",
                "encrypt=true;trustServerCertificate=false;loginTimeout=4;queryTimeout=7"
            )
            .into(),
        ),
        ..Default::default()
    };
    let parsed = SqlServerConfig::from_connection_config(&config).unwrap();

    assert_eq!(parsed.target.host, "db.example.com");
    assert_eq!(parsed.target.instance.as_deref(), Some("REPORTS"));
    assert_eq!(parsed.target.port, None);
    assert_eq!(parsed.connect_timeout, Duration::from_secs(4));
    assert_eq!(parsed.query_timeout, Some(Duration::from_secs(7)));
}

#[test]
fn jdbc_url_server_name_properties_accept_ipv6_instance_and_port() {
    let config = ConnectionConfig {
        url: Some(
            "jdbc:sqlserver://;serverName=3ffe:8311:eeee:f70f:0:5eae:10.203.31.9\\REPORTS;portNumber=51433;instanceName=reports"
                .into(),
        ),
        ..Default::default()
    };
    let parsed = SqlServerConfig::from_connection_config(&config).unwrap();

    assert_eq!(parsed.target.host, "3ffe:8311:eeee:f70f:0:5eae:10.203.31.9");
    assert_eq!(parsed.target.instance, None);
    assert_eq!(parsed.target.port, Some(51433));
}

#[test]
fn jdbc_url_server_name_property_accepts_hostname_without_authority() {
    let config = ConnectionConfig {
        url: Some("jdbc:sqlserver://;serverName=db.example.com;portNumber=1433".into()),
        ..Default::default()
    };
    let parsed = SqlServerConfig::from_connection_config(&config).unwrap();

    assert_eq!(parsed.target.host, "db.example.com");
    assert_eq!(parsed.target.port, Some(1433));
}

#[test]
fn jdbc_url_unescapes_braced_credentials_for_final_client_authentication() {
    let config = ConnectionConfig {
        url: Some(
            concat!(
                "jdbc:sqlserver://;",
                "serverName=3ffe:8311:eeee:f70f:0:5eae:10.203.31.9\\REPORTS;",
                "portNumber=51433;uid=analyst;",
                "password={ pass;\"{}} word };databaseName=warehouse;",
                "packetSize=8192;encrypt=true;trustServerCertificate=false;",
                "loginTimeout=4;queryTimeout=7"
            )
            .into(),
        ),
        ..Default::default()
    };
    let parsed = SqlServerConfig::from_connection_config(&config).unwrap();

    assert_eq!(
        parsed.authentication,
        AuthMethod::sql_server("analyst", " pass;\"{} word ")
    );
    assert_eq!(parsed.client.get_packet_size(), Some(8192));
    assert_eq!(parsed.target.port, Some(51433));
    assert_eq!(parsed.target.instance, None);
}

#[test]
fn jdbc_url_unescapes_braced_password_closing_braces_and_semicolons() {
    for (encoded, expected) in [("{p}}word}", "p}word"), ("{p}};word}", "p};word")] {
        let config = ConnectionConfig {
            url: Some(format!(
                "jdbc:sqlserver://db.example.com;uid=analyst;password={encoded}"
            )),
            ..Default::default()
        };
        let parsed = SqlServerConfig::from_connection_config(&config).unwrap();

        assert_eq!(
            parsed.authentication,
            AuthMethod::sql_server("analyst", expected)
        );
    }
}

#[test]
fn jdbc_url_preserves_tls_options_and_rejects_conflicting_trust_settings() {
    let config = ConnectionConfig {
        url: Some(
            concat!(
                "jdbc:sqlserver://;serverName=db.example.com;encrypt=false;",
                "trustServerCertificate=false;trustServerCertificateCA=server-ca.pem;",
                "hostnameInCertificate=db-cert.example.com"
            )
            .into(),
        ),
        ..Default::default()
    };
    let parsed = SqlServerConfig::from_connection_config(&config).unwrap();
    let tls_options = parsed
        .tls_options
        .expect("JDBC URL should capture TLS options");
    assert!(matches!(tls_options.encryption, EncryptionLevel::Off));
    assert!(!tls_options.trust_server_certificate);
    assert_eq!(
        tls_options.trust_server_certificate_ca.as_deref(),
        Some("server-ca.pem")
    );
    assert_eq!(
        tls_options.hostname_in_certificate.as_deref(),
        Some("db-cert.example.com")
    );

    let conflicting = ConnectionConfig {
        url: Some(
            concat!(
                "jdbc:sqlserver://db.example.com;trustServerCertificate=true;",
                "trustServerCertificateCA=server-ca.pem"
            )
            .into(),
        ),
        ..Default::default()
    };
    assert_eq!(
        SqlServerConfig::from_connection_config(&conflicting)
            .err()
            .expect("conflicting trust settings should fail")
            .kind,
        ErrorKind::Configuration
    );
}

#[test]
fn jdbc_url_rejects_unrecognized_options_instead_of_ignoring_them() {
    let config = ConnectionConfig {
        url: Some("jdbc:sqlserver://db.example.com;authentication=ActiveDirectoryPassword".into()),
        ..Default::default()
    };
    let error = SqlServerConfig::from_connection_config(&config)
        .err()
        .expect("unrecognized property should fail");
    assert_eq!(error.kind, ErrorKind::Configuration);
    assert!(error.message.contains("不支持 JDBC 参数"));
}

#[test]
fn named_instance_response_checks_header_and_returns_tcp_port() {
    let payload = b"ServerName;db;InstanceName;REPORTS;IsClustered;No;Version;16.0;tcp;51433;;";
    let mut response = vec![0x05];
    response.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    response.extend_from_slice(payload);

    assert_eq!(
        parse_instance_response(&response, "reports").unwrap(),
        51433
    );
    assert_eq!(
        parse_instance_response(&response, "other")
            .unwrap_err()
            .kind,
        ErrorKind::Connection
    );
    assert_eq!(
        parse_instance_response(&response[1..], "reports")
            .unwrap_err()
            .kind,
        ErrorKind::Connection
    );
}

#[test]
fn named_instance_response_requires_identity_and_parses_transport_fields() {
    for (payload, expected_port) in [
        ("ServerName;db;IsClustered;No;Version;16.0;tcp;51433;;", None),
        ("ServerName;db;tcp;51433;InstanceName;OTHER;IsClustered;No;Version;16.0;;", None),
        ("ServerName;db;InstanceName;REPORTS;IsClustered;No;Version;16.0;tcp;51433;tcp;51434;;", None),
        ("ServerName;db;InstanceName;REPORTS;IsClustered;No;Version;16.0;tcp;;", None),
        ("ServerName;db;InstanceName;REPORTS;IsClustered;No;Version;16.0;tcp;0;;", None),
        ("ServerName;db;InstanceName;REPORTS;IsClustered;No;Version;16.0;bv;item;group;tcp;12345;org;;", None),
        ("ServerName;tcp;InstanceName;REPORTS;IsClustered;No;Version;16.0;tcp;51433;;", Some(51433)),
        ("ServerName;db;InstanceName;REPORTS;IsClustered;No;Version;16.0;bv;item;group;tcp;12345;org;tcp;51433;;", Some(51433)),
    ] {
        let mut response = vec![0x05];
        response.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        response.extend_from_slice(payload.as_bytes());
        let actual = parse_instance_response(&response, "reports");
        match expected_port {
            Some(port) => assert_eq!(actual.unwrap(), port, "{payload}"),
            None => assert_eq!(actual.unwrap_err().kind, ErrorKind::Connection, "{payload}"),
        }
    }
}

#[tokio::test]
async fn routing_reconnects_once_and_rebuilds_original_connection_options() {
    let source = ConnectionConfig {
        url: Some(
            concat!(
                "jdbc:sqlserver://primary.example.com\\REPORTS;uid=analyst;password=secret;",
                "databaseName=warehouse;packetSize=8192;encrypt=true;",
                "trustServerCertificate=false;hostnameInCertificate=db-cert.example.com;",
                "applicationName=analytics;applicationIntent=ReadOnly;",
                "loginTimeout=3;queryTimeout=7;socketTimeout=25"
            )
            .into(),
        ),
        ..Default::default()
    };
    let mut initial = SqlServerConfig::from_connection_config(&source).unwrap();
    initial.target.port = Some(51433);
    initial.client.port(51433);

    struct Attempt {
        host: String,
        port: Option<u16>,
        network_address: Option<SocketAddr>,
        client_addr: String,
        instance: Option<String>,
        client_instance_is_none: bool,
        client_has_database: bool,
        client_has_application_name: bool,
        client_has_readonly_intent: bool,
        packet_size: Option<u32>,
        authentication: AuthMethod,
        tls: Option<super::config::JdbcTlsOptions>,
        handshake_timeout: Option<Duration>,
        command_timeout: Option<Duration>,
        socket_timeout: Option<Duration>,
    }

    let mut attempts = Vec::new();
    let result: Result<(), ConnectFailure> = connect_with_routing(
        initial,
        Some("192.0.2.14:51433".parse().unwrap()),
        Duration::from_secs(3),
        Some(Duration::from_secs(7)),
        |attempt, network_address| {
            let index = attempts.len();
            let client_debug = format!("{:?}", attempt.client);
            attempts.push(Attempt {
                host: attempt.target.host,
                port: attempt.target.port,
                network_address,
                client_addr: attempt.client.get_addr(),
                instance: attempt.target.instance,
                client_instance_is_none: client_debug.contains("instance_name: None"),
                client_has_database: client_debug.contains("database: Some(\"warehouse\")"),
                client_has_application_name: client_debug
                    .contains("application_name: Some(\"analytics\")"),
                client_has_readonly_intent: client_debug.contains("readonly: true"),
                packet_size: attempt.client.get_packet_size(),
                authentication: attempt.authentication,
                tls: attempt.tls_options,
                handshake_timeout: attempt.client.get_handshake_timeout(),
                command_timeout: attempt.client.get_command_timeout(),
                socket_timeout: attempt.socket_timeout,
            });
            async move {
                if index == 0 {
                    Err(TiberiusError::Routing {
                        host: "redirect.example.net".into(),
                        port: 15433,
                    })
                } else {
                    Err(TiberiusError::Routing {
                        host: "unexpected-third.example.net".into(),
                        port: 16433,
                    })
                }
            }
        },
    )
    .await;

    assert!(matches!(
        result,
        Err(ConnectFailure::Tiberius(TiberiusError::Routing { ref host, port: 16433 }))
            if host == "unexpected-third.example.net"
    ));
    assert_eq!(
        attempts.len(),
        2,
        "the second redirect must not be followed"
    );
    assert_eq!(attempts[0].host, "primary.example.com");
    assert_eq!(attempts[0].port, Some(51433));
    assert_eq!(
        attempts[0].network_address,
        Some("192.0.2.14:51433".parse().unwrap())
    );
    assert_eq!(attempts[0].client_addr, "primary.example.com:51433");
    assert_eq!(attempts[0].instance.as_deref(), Some("REPORTS"));
    assert_eq!(attempts[1].network_address, None);
    assert_eq!(attempts[1].host, "redirect.example.net");
    assert_eq!(attempts[1].port, Some(15433));
    assert_eq!(attempts[1].client_addr, "redirect.example.net:15433");
    assert_eq!(attempts[1].instance, None);
    for attempt in &attempts {
        assert!(attempt.client_instance_is_none);
        assert!(attempt.client_has_database);
        assert!(attempt.client_has_application_name);
        assert!(attempt.client_has_readonly_intent);
        assert_eq!(attempt.packet_size, Some(8192));
        assert_eq!(
            attempt.authentication,
            AuthMethod::sql_server("analyst", "secret")
        );
        assert_eq!(attempt.handshake_timeout, Some(Duration::from_secs(3)));
        assert_eq!(attempt.command_timeout, Some(Duration::from_secs(7)));
        assert_eq!(attempt.socket_timeout, Some(Duration::from_millis(25)));
        let tls = attempt
            .tls
            .as_ref()
            .expect("TLS options must survive routing");
        assert!(matches!(tls.encryption, EncryptionLevel::Required));
        assert!(!tls.trust_server_certificate);
        assert_eq!(
            tls.hostname_in_certificate.as_deref(),
            Some("db-cert.example.com")
        );
    }
}

#[tokio::test]
async fn routing_reconnect_returns_success_from_redirect() {
    let source = ConnectionConfig {
        host: "primary.example.com".into(),
        ..Default::default()
    };
    let mut initial = SqlServerConfig::from_connection_config(&source).unwrap();
    initial.target.port = Some(1433);
    initial.client.port(1433);
    let mut attempts = Vec::new();

    let result: Result<&str, ConnectFailure> = connect_with_routing(
        initial,
        None,
        Duration::from_secs(5),
        Some(Duration::from_secs(9)),
        |attempt, network_address| {
            let index = attempts.len();
            attempts.push((attempt.target.host, attempt.target.port, network_address));
            async move {
                if index == 0 {
                    Err(TiberiusError::Routing {
                        host: "redirect.example.net".into(),
                        port: 15433,
                    })
                } else {
                    Ok("connected")
                }
            }
        },
    )
    .await;

    assert_eq!(result.unwrap(), "connected");
    assert_eq!(attempts.len(), 2);
    assert_eq!(
        attempts[0],
        ("primary.example.com".into(), Some(1433), None)
    );
    assert_eq!(
        attempts[1],
        ("redirect.example.net".into(), Some(15433), None)
    );
}

#[tokio::test]
async fn non_routing_connect_error_does_not_retry() {
    let source = ConnectionConfig {
        host: "primary.example.com".into(),
        ..Default::default()
    };
    let mut initial = SqlServerConfig::from_connection_config(&source).unwrap();
    initial.target.port = Some(1433);
    initial.client.port(1433);
    let mut attempts = 0;

    let result: Result<(), ConnectFailure> = connect_with_routing(
        initial,
        None,
        Duration::from_secs(5),
        Some(Duration::from_secs(9)),
        |_attempt, _network_address| {
            attempts += 1;
            async {
                Err(TiberiusError::Io {
                    kind: std::io::ErrorKind::ConnectionRefused,
                    message: "connection refused".into(),
                })
            }
        },
    )
    .await;

    assert!(matches!(
        result,
        Err(ConnectFailure::Tiberius(TiberiusError::Io { kind, .. }))
            if kind == std::io::ErrorKind::ConnectionRefused
    ));
    assert_eq!(attempts, 1);
}

#[tokio::test]
async fn named_instance_resolver_uses_ipv6_dns_candidate_family() {
    let browser = match tokio::net::UdpSocket::bind("[::1]:0").await {
        Ok(socket) => socket,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::Unsupported
            ) =>
        {
            return;
        }
        Err(error) => panic!("could not bind IPv6 loopback UDP socket: {error}"),
    };
    let browser_port = browser.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let mut request = [0; 64];
        let (size, peer) = browser.recv_from(&mut request).await.unwrap();
        assert_eq!(&request[..size], b"\x04REPORTS\0");

        let payload = b"ServerName;db;InstanceName;REPORTS;IsClustered;No;Version;16.0;tcp;51433;;";
        let mut response = vec![0x05];
        response.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        response.extend_from_slice(payload);
        browser.send_to(&response, peer).await.unwrap();
    });

    // This models a hostname without a colon whose DNS lookup returned ::1.
    let candidate: SocketAddr = format!("[::1]:{browser_port}").parse().unwrap();
    let target = resolve_named_instance_candidates(
        vec![candidate],
        "REPORTS",
        tokio::time::Instant::now() + Duration::from_secs(3),
    )
    .await
    .unwrap();
    server.await.unwrap();
    assert_eq!(target, "[::1]:51433".parse().unwrap());
}

#[tokio::test]
async fn connection_refusal_returns_connection_error_without_hanging() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = ConnectionConfig {
        host: "127.0.0.1".into(),
        port: port.to_string(),
        username: "user".into(),
        password: "secret".into(),
        ..Default::default()
    };
    let error = test_connection(config).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Connection);
}

#[tokio::test]
async fn socket_timeout_interrupts_stalled_tiberius_handshake() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        for _ in 0..3 {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    for query_timeout in ["7", "0", "-1"] {
        let config = ConnectionConfig {
            url: Some(format!(
                "jdbc:sqlserver://127.0.0.1:{port};encrypt=false;loginTimeout=3;queryTimeout={query_timeout};socketTimeout=50"
            )),
            ..Default::default()
        };

        let start = tokio::time::Instant::now();
        let result =
            tokio::time::timeout(Duration::from_millis(400), test_connection(config)).await;
        let error = result
            .expect("socket timeout should fire before loginTimeout")
            .expect_err("a server that never answers prelogin must fail");
        assert_eq!(error.kind, ErrorKind::Connection);
        assert!(error.message.contains("socket read timed out"));
        assert!(start.elapsed() < Duration::from_millis(400));
    }
    server.abort();
}

#[tokio::test]
#[ignore = "requires a SQL Server configured through DATAVISOOL_SQLSERVER_TEST_* environment variables"]
async fn real_sql_server_connection_and_query_smoke_test() {
    let config = ConnectionConfig {
        host: std::env::var("DATAVISOOL_SQLSERVER_TEST_HOST").unwrap_or_default(),
        port: std::env::var("DATAVISOOL_SQLSERVER_TEST_PORT").unwrap_or_default(),
        database: std::env::var("DATAVISOOL_SQLSERVER_TEST_DATABASE").unwrap_or_default(),
        username: std::env::var("DATAVISOOL_SQLSERVER_TEST_USERNAME").unwrap_or_default(),
        password: std::env::var("DATAVISOOL_SQLSERVER_TEST_PASSWORD").unwrap_or_default(),
        url: std::env::var("DATAVISOOL_SQLSERVER_TEST_URL").ok(),
    };
    test_connection(config.clone())
        .await
        .expect("SQL Server connection should succeed");
    let result = query(config, "SELECT CAST(1 AS INT) AS smoke".into())
        .await
        .expect("SQL Server smoke query should succeed");
    assert_eq!(result[0]["smoke"], 1);
}
