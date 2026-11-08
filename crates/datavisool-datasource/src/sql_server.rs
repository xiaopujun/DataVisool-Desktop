/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use crate::{ConnectionConfig, DriverError, ErrorKind};
use std::{
    future::Future,
    io as std_io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use tiberius::Client;
use tokio::{
    net::{lookup_host, TcpStream, UdpSocket},
    task::JoinSet,
    time::{timeout, timeout_at, Instant},
};
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

#[path = "sql_server_config.rs"]
mod config;
#[path = "sql_server_io.rs"]
mod io;
#[path = "sql_server_values.rs"]
mod values;

#[cfg(test)]
#[path = "sql_server_conversion_tests.rs"]
mod conversion_tests;
#[cfg(test)]
#[path = "sql_server_tests.rs"]
mod tests;

use io::SocketTimeoutStream;

const NAMED_INSTANCE_TIMEOUT: Duration = Duration::from_secs(3);
const FAR_FUTURE_QUERY_TIMEOUT: Duration = Duration::from_secs(86400 * 365 * 30);

pub(crate) async fn test_connection(config: ConnectionConfig) -> Result<(), DriverError> {
    let (_client, _query_timeout) = connect(config).await?;
    Ok(())
}

pub(crate) async fn query(
    config: ConnectionConfig,
    statement: String,
) -> Result<serde_json::Value, DriverError> {
    let (mut client, query_timeout) = connect(config).await?;
    let deadline = query_deadline(query_timeout);
    let operation = async move {
        let stream = client.simple_query(statement).await.map_err(|error| {
            DriverError::new(ErrorKind::Query, format!("SQL Server 查询失败：{error}"))
        })?;
        let rows = stream.into_first_result().await.map_err(|error| {
            DriverError::new(ErrorKind::Query, format!("SQL Server 查询失败：{error}"))
        })?;
        values::rows_to_json(rows, deadline).await
    };
    let result = with_query_deadline(deadline, operation).await?;
    Ok(result)
}

fn query_deadline(query_timeout: Option<Duration>) -> Option<Instant> {
    query_timeout.map(|timeout| {
        Instant::now().checked_add(timeout).unwrap_or_else(|| {
            Instant::now()
                .checked_add(FAR_FUTURE_QUERY_TIMEOUT)
                .expect("Tokio's platform-safe 30-year timeout should fit in Instant")
        })
    })
}

async fn with_query_deadline<T>(
    deadline: Option<Instant>,
    operation: impl Future<Output = Result<T, DriverError>>,
) -> Result<T, DriverError> {
    match deadline {
        Some(deadline) => timeout_at(deadline, operation)
            .await
            .map_err(|_| query_timeout_error())?,
        None => operation.await,
    }
}

async fn connect(
    config: ConnectionConfig,
) -> Result<
    (
        Client<Compat<SocketTimeoutStream<TcpStream>>>,
        Option<Duration>,
    ),
    DriverError,
> {
    let mut parsed = config::SqlServerConfig::from_connection_config(&config)?;
    let connect_timeout = parsed.connect_timeout;
    let query_timeout = parsed.query_timeout;
    let connect_result = timeout(connect_timeout, async move {
        let network_address = match (parsed.target.port, parsed.target.instance.as_deref()) {
            (Some(_), _) | (None, None) => None,
            (None, Some(instance)) => {
                Some(resolve_named_instance(&parsed.target.host, instance).await?)
            }
        };
        let port = network_address
            .map(|address| address.port())
            .or(parsed.target.port)
            .unwrap_or(config::DEFAULT_PORT);
        parsed.target.port = Some(port);
        parsed.client.port(port);
        let client = connect_with_routing(
            parsed,
            network_address,
            connect_timeout,
            query_timeout,
            |attempt, network_address| async move {
                let port = attempt.target.port.expect("connection port is resolved");
                let stream = match network_address {
                    Some(address) => TcpStream::connect(address).await,
                    None => TcpStream::connect((attempt.target.host.as_str(), port)).await,
                }
                .map_err(tiberius::error::Error::from)?;
                stream
                    .set_nodelay(true)
                    .map_err(tiberius::error::Error::from)?;
                let stream = SocketTimeoutStream::new(stream, attempt.socket_timeout);
                Client::connect(attempt.client, stream.compat_write()).await
            },
        )
        .await
        .map_err(|error| match error {
            ConnectFailure::Tiberius(error) => DriverError::new(
                ErrorKind::Connection,
                format!("连接 SQL Server 失败：{error}"),
            ),
        })?;
        Ok(client)
    })
    .await
    .map_err(|_| DriverError::new(ErrorKind::Connection, "连接 SQL Server 超时"))?;

    connect_result.map(|client| (client, query_timeout))
}

#[derive(Debug)]
enum ConnectFailure {
    Tiberius(tiberius::error::Error),
}

async fn connect_with_routing<C, F, Fut>(
    mut initial: config::SqlServerConfig,
    initial_network_address: Option<SocketAddr>,
    connect_timeout: Duration,
    query_timeout: Option<Duration>,
    mut connector: F,
) -> Result<C, ConnectFailure>
where
    F: FnMut(config::SqlServerConfig, Option<SocketAddr>) -> Fut,
    Fut: Future<Output = Result<C, tiberius::error::Error>>,
{
    configure_client_timeouts(&mut initial, connect_timeout, query_timeout);
    let retry_template = initial.clone();
    match connector(initial, initial_network_address).await {
        Err(tiberius::error::Error::Routing { host, port }) => {
            let mut retry = retry_template;
            retry.target.host.clone_from(&host);
            retry.target.port = Some(port);
            retry.target.instance = None;
            retry.client.host(&host);
            retry.client.port(port);
            configure_client_timeouts(&mut retry, connect_timeout, query_timeout);
            connector(retry, None)
                .await
                .map_err(ConnectFailure::Tiberius)
        }
        Err(error) => Err(ConnectFailure::Tiberius(error)),
        Ok(client) => Ok(client),
    }
}

fn configure_client_timeouts(
    parsed: &mut config::SqlServerConfig,
    connect_timeout: Duration,
    query_timeout: Option<Duration>,
) {
    if let Some(port) = parsed.target.port {
        parsed.client.port(port);
    }
    parsed.client.handshake_timeout(Some(connect_timeout));
    parsed.client.command_timeout(query_timeout);
}

fn query_timeout_error() -> DriverError {
    DriverError::new(ErrorKind::Query, "SQL Server 查询超时")
}

async fn resolve_named_instance(host: &str, instance: &str) -> Result<SocketAddr, DriverError> {
    resolve_named_instance_at(host, instance, 1434).await
}

async fn resolve_named_instance_at(
    host: &str,
    instance: &str,
    browser_port: u16,
) -> Result<SocketAddr, DriverError> {
    let deadline = Instant::now() + NAMED_INSTANCE_TIMEOUT;
    let candidates = timeout_at(deadline, lookup_host((host, browser_port)))
        .await
        .map_err(|_| named_instance_timeout())?
        .map_err(named_instance_error)?
        .collect::<Vec<_>>();
    resolve_named_instance_candidates(candidates, instance, deadline).await
}

async fn resolve_named_instance_candidates(
    candidates: Vec<SocketAddr>,
    instance: &str,
    deadline: Instant,
) -> Result<SocketAddr, DriverError> {
    if candidates.is_empty() {
        return Err(DriverError::new(
            ErrorKind::Connection,
            "SQL Server 命名实例解析失败：主机没有可用地址",
        ));
    }

    let mut request = Vec::with_capacity(instance.len() + 2);
    request.push(0x04);
    request.extend_from_slice(instance.as_bytes());
    request.push(0);

    let mut requests = JoinSet::new();
    for address in candidates {
        let request = request.clone();
        let instance = instance.to_string();
        requests.spawn(async move {
            query_instance_candidate(address, request, instance, deadline).await
        });
    }

    let mut last_error = None;
    while let Some(result) = requests.join_next().await {
        match result {
            Ok(Ok(address)) => {
                requests.abort_all();
                return Ok(address);
            }
            Ok(Err(error)) => last_error = Some(error),
            Err(error) => {
                last_error = Some(DriverError::new(
                    ErrorKind::Connection,
                    format!("SQL Server 命名实例解析失败：{error}"),
                ));
            }
        }
    }
    Err(last_error.unwrap_or_else(named_instance_timeout))
}

async fn query_instance_candidate(
    address: SocketAddr,
    request: Vec<u8>,
    instance: String,
    deadline: Instant,
) -> Result<SocketAddr, DriverError> {
    let bind_address = match address.ip() {
        IpAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        IpAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
    };
    let result = timeout_at(deadline, async {
        let socket = UdpSocket::bind(bind_address).await?;
        socket.connect(address).await?;
        socket.send(&request).await?;
        let mut response = [0u8; 4096];
        let size = socket.recv(&mut response).await?;
        let port = parse_instance_response(&response[..size], &instance)
            .map_err(|error| std_io::Error::new(std_io::ErrorKind::InvalidData, error.message))?;
        Ok::<_, std_io::Error>(socket_addr_with_port(address, port))
    })
    .await;

    match result {
        Ok(Ok(port)) => Ok(port),
        Ok(Err(error)) => Err(named_instance_error(error)),
        Err(_) => Err(named_instance_timeout()),
    }
}

fn socket_addr_with_port(address: SocketAddr, port: u16) -> SocketAddr {
    match address {
        SocketAddr::V4(mut address) => {
            address.set_port(port);
            address.into()
        }
        SocketAddr::V6(mut address) => {
            address.set_port(port);
            address.into()
        }
    }
}

fn named_instance_error(error: impl std::fmt::Display) -> DriverError {
    DriverError::new(
        ErrorKind::Connection,
        format!("SQL Server 命名实例解析失败：{error}"),
    )
}

fn named_instance_timeout() -> DriverError {
    DriverError::new(ErrorKind::Connection, "SQL Server 命名实例解析超时")
}

fn parse_instance_response(response: &[u8], instance: &str) -> Result<u16, DriverError> {
    if response.len() < 3 || response[0] != 0x05 {
        return Err(DriverError::new(
            ErrorKind::Connection,
            "SQL Server 命名实例响应无效",
        ));
    }
    let payload_len = u16::from_le_bytes([response[1], response[2]]) as usize;
    if payload_len != response.len() - 3 {
        return Err(DriverError::new(
            ErrorKind::Connection,
            "SQL Server 命名实例响应长度无效",
        ));
    }
    let payload = &response[3..];
    let text = std::str::from_utf8(payload)
        .map_err(|_| DriverError::new(ErrorKind::Connection, "SQL Server 命名实例响应无效"))?;
    let fields: Vec<&str> = text.trim_end_matches(';').split(';').collect();
    // SSRP has four mandatory header pairs before any transport information.
    // Validate the instance before considering a port, even in malformed replies.
    let headers = ["ServerName", "InstanceName", "IsClustered", "Version"];
    if fields.len() < 8
        || headers
            .iter()
            .enumerate()
            .any(|(index, name)| !fields[index * 2].eq_ignore_ascii_case(name))
    {
        return Err(DriverError::new(
            ErrorKind::Connection,
            "SQL Server 命名实例响应缺少必要字段",
        ));
    }
    if !fields[3].trim().eq_ignore_ascii_case(instance) {
        return Err(DriverError::new(
            ErrorKind::Connection,
            "SQL Server 命名实例响应与请求不匹配",
        ));
    }
    let mut tcp_port = None;
    let mut transport_fields = &fields[8..];
    while !transport_fields.is_empty() {
        // Banyan VINES is the only transport with five value fields; all other
        // defined SSRP transports have one. Never interpret a value as a key.
        let field_count = if transport_fields[0].eq_ignore_ascii_case("bv") {
            6
        } else {
            2
        };
        if transport_fields.len() < field_count {
            return Err(DriverError::new(
                ErrorKind::Connection,
                "SQL Server 命名实例响应无效",
            ));
        }
        if transport_fields[0].eq_ignore_ascii_case("tcp") {
            let port = transport_fields[1]
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0);
            if port.is_none() || tcp_port.is_some() {
                return Err(DriverError::new(
                    ErrorKind::Connection,
                    "SQL Server 命名实例 TCP 端口无效或重复",
                ));
            }
            tcp_port = port;
        }
        transport_fields = &transport_fields[field_count..];
    }
    tcp_port.ok_or_else(|| {
        DriverError::new(ErrorKind::Connection, "SQL Server 命名实例未返回 TCP 端口")
    })
}
