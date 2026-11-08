/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use crate::{ConnectionConfig, DriverError, ErrorKind};
use std::{collections::HashMap, net::Ipv6Addr, time::Duration};
use tiberius::{AuthMethod, Config, EncryptionLevel};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_QUERY_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const DEFAULT_PORT: u16 = 1433;

#[derive(Clone)]
pub(super) struct SqlServerConfig {
    pub(super) client: Config,
    pub(super) target: ConnectionTarget,
    pub(super) connect_timeout: Duration,
    pub(super) query_timeout: Option<Duration>,
    pub(super) socket_timeout: Option<Duration>,
    #[cfg(test)]
    pub(super) authentication: AuthMethod,
    #[cfg(test)]
    pub(super) tls_options: Option<JdbcTlsOptions>,
}

#[derive(Clone, Debug)]
pub(super) struct JdbcTlsOptions {
    pub(super) encryption: EncryptionLevel,
    pub(super) trust_server_certificate: bool,
    pub(super) trust_server_certificate_ca: Option<String>,
    pub(super) hostname_in_certificate: Option<String>,
}

impl JdbcTlsOptions {
    fn apply(&self, client: &mut Config) {
        client.encryption(self.encryption);
        if self.trust_server_certificate {
            client.trust_cert();
        } else if let Some(ca) = &self.trust_server_certificate_ca {
            client.trust_cert_ca(ca);
        }
        if let Some(hostname) = &self.hostname_in_certificate {
            client.hostname_in_certificate(hostname);
        }
    }
}

#[derive(Clone)]
pub(super) struct ConnectionTarget {
    pub(super) host: String,
    pub(super) port: Option<u16>,
    pub(super) instance: Option<String>,
}

impl SqlServerConfig {
    pub(super) fn from_connection_config(config: &ConnectionConfig) -> Result<Self, DriverError> {
        match config
            .url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
        {
            Some(url) => Self::from_jdbc_url(config, url),
            None => Self::from_fields(config),
        }
    }

    fn from_fields(config: &ConnectionConfig) -> Result<Self, DriverError> {
        let (host, instance, embedded_port) = parse_server_name(config.host.trim())?;
        if host.is_empty() {
            return Err(configuration_error("SQL Server 主机不能为空"));
        }
        let port = parse_port(&config.port, "port")?.or(embedded_port);
        let instance = if port.is_some() { None } else { instance };
        let mut client = Config::new();
        client.host(&host);
        if let Some(port) = port {
            client.port(port);
        }
        if !config.database.trim().is_empty() {
            client.database(&config.database);
        }
        let authentication = AuthMethod::sql_server(&config.username, &config.password);
        #[cfg(test)]
        let test_authentication = authentication.clone();
        client.authentication(authentication);

        Ok(Self {
            client,
            target: ConnectionTarget {
                host,
                port,
                instance,
            },
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            query_timeout: Some(DEFAULT_QUERY_TIMEOUT),
            socket_timeout: None,
            #[cfg(test)]
            authentication: test_authentication,
            #[cfg(test)]
            tls_options: None,
        })
    }

    fn from_jdbc_url(config: &ConnectionConfig, url: &str) -> Result<Self, DriverError> {
        let parsed = ParsedJdbcUrl::parse(url)?;
        reject_unsupported_properties(&parsed.properties)?;
        let mut target = parsed.target;
        // Resolve explicit ports before instance names. JDBC ignores the
        // instance entirely when a port is supplied, including PRELOGIN checks.
        if let Some(port) = property(&parsed.properties, &["portnumber", "port"])? {
            let port = parse_port_value(&port, "portNumber")?;
            merge_port(
                &mut target.port,
                Some(port),
                "SQL Server URL authority 与 portNumber 配置冲突",
            )?;
        }
        if let Some(server) = property(&parsed.properties, &["servername", "server name"])? {
            let (host, instance, port) = parse_server_name(&server)?;
            merge_host(
                &mut target.host,
                host,
                "SQL Server URL authority 与 serverName 配置冲突",
            )?;
            merge_port(
                &mut target.port,
                port,
                "SQL Server serverName 与端口配置冲突",
            )?;
            if target.port.is_none() {
                merge_instance(&mut target.instance, instance)?;
            }
        }

        if target.port.is_some() {
            target.instance = None;
        } else if let Some(instance) =
            property(&parsed.properties, &["instancename", "instance name"])?
        {
            let instance = instance.trim().to_string();
            if instance.is_empty() {
                return Err(configuration_error("SQL Server instanceName 不能为空"));
            }
            merge_instance(&mut target.instance, Some(instance))?;
        }

        if target.host.trim().is_empty() {
            return Err(configuration_error("SQL Server JDBC URL 中缺少主机"));
        }

        let mut client = Config::new();
        client.host(&target.host);
        if let Some(port) = target.port {
            client.port(port);
        }
        // Named instances are resolved through SSRP before connecting. Keep the
        // instance on target only; Tiberius receives the resolved explicit port.
        if let Some(database) = property(
            &parsed.properties,
            &["database", "databasename", "initial catalog"],
        )? {
            client.database(database);
        } else if !config.database.trim().is_empty() {
            client.database(&config.database);
        }

        let username = property(&parsed.properties, &["uid", "username", "user", "user id"])?
            .unwrap_or_else(|| config.username.clone());
        let password = property(&parsed.properties, &["password", "pwd"])?
            .unwrap_or_else(|| config.password.clone());
        let authentication = AuthMethod::sql_server(username, password);
        #[cfg(test)]
        let test_authentication = authentication.clone();
        client.authentication(authentication);

        if let Some(packet_size) = property(&parsed.properties, &["packetsize", "packet size"])? {
            let packet_size = packet_size
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|size| (512..=32767).contains(size))
                .ok_or_else(|| {
                    configuration_error("SQL Server packetSize 必须在 512..=32767 之间")
                })?;
            client.packet_size(packet_size);
        }

        let connect_timeout = property(&parsed.properties, &["logintimeout", "login timeout"])?
            .map(|value| parse_timeout(&value, "loginTimeout"))
            .transpose()?
            .unwrap_or(DEFAULT_CONNECT_TIMEOUT);
        let query_timeout = property(&parsed.properties, &["querytimeout", "query timeout"])?
            .map(|value| parse_query_timeout(&value))
            .transpose()?
            .unwrap_or(Some(DEFAULT_QUERY_TIMEOUT));
        let socket_timeout = property(&parsed.properties, &["sockettimeout", "socket timeout"])?
            .map(|value| parse_socket_timeout(&value))
            .transpose()?
            .flatten();

        if let Some(integrated_security) = property(
            &parsed.properties,
            &["integratedsecurity", "integrated security"],
        )? {
            if parse_bool(&integrated_security, "integratedSecurity")? {
                return Err(configuration_error(
                    "当前 SQL Server adapter 不支持 integratedSecurity 身份验证",
                ));
            }
        }
        if let Some(application_intent) = property(
            &parsed.properties,
            &["applicationintent", "application intent"],
        )? {
            if !matches!(
                application_intent.trim().to_ascii_lowercase().as_str(),
                "readonly" | "readwrite"
            ) {
                return Err(configuration_error(
                    "SQL Server applicationIntent 只支持 ReadOnly 或 ReadWrite",
                ));
            }
        }
        if let Some(multi_subnet_failover) = property(
            &parsed.properties,
            &["multisubnetfailover", "multi subnet failover"],
        )? {
            if parse_bool(&multi_subnet_failover, "multiSubnetFailover")? {
                return Err(configuration_error(
                    "当前 SQL Server adapter 不支持 multiSubnetFailover=true",
                ));
            }
        }

        #[cfg(test)]
        let tls_options = apply_jdbc_client_options(&mut client, &parsed.properties)?;
        #[cfg(not(test))]
        apply_jdbc_client_options(&mut client, &parsed.properties)?;

        Ok(Self {
            client,
            target,
            connect_timeout,
            query_timeout,
            socket_timeout,
            #[cfg(test)]
            authentication: test_authentication,
            #[cfg(test)]
            tls_options: Some(tls_options),
        })
    }
}

struct ParsedJdbcUrl {
    target: ConnectionTarget,
    properties: HashMap<String, String>,
}

impl ParsedJdbcUrl {
    fn parse(url: &str) -> Result<Self, DriverError> {
        const PREFIX: &str = "jdbc:sqlserver://";
        if !url
            .get(..PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PREFIX))
        {
            return Err(configuration_error(
                "SQL Server URL 必须以 jdbc:sqlserver:// 开头",
            ));
        }
        let body = &url[PREFIX.len()..];
        let (authority, property_text) = body.split_once(';').unwrap_or((body, ""));
        let (host, instance, port) = if authority.trim().is_empty() {
            (String::new(), None, None)
        } else {
            parse_server_address(authority)?
        };
        let properties = parse_properties(property_text)?;
        Ok(Self {
            target: ConnectionTarget {
                host,
                port,
                instance,
            },
            properties,
        })
    }
}

fn parse_server_name(value: &str) -> Result<(String, Option<String>, Option<u16>), DriverError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(configuration_error("SQL Server serverName 不能为空"));
    }

    // An unbracketed IPv6 serverName is a complete address, not a host:port
    // pair. A port can be supplied separately with portNumber.
    if !value.contains('\\') && value.parse::<Ipv6Addr>().is_ok() {
        return Ok((value.to_string(), None, None));
    }
    parse_server_address(value)
}

fn merge_host(target: &mut String, host: String, conflict: &str) -> Result<(), DriverError> {
    if host.is_empty() {
        return Ok(());
    }
    if !target.is_empty() && !target.eq_ignore_ascii_case(&host) {
        return Err(configuration_error(conflict));
    }
    *target = host;
    Ok(())
}

fn merge_port(
    target: &mut Option<u16>,
    port: Option<u16>,
    conflict: &str,
) -> Result<(), DriverError> {
    if let Some(port) = port {
        if target.is_some_and(|existing| existing != port) {
            return Err(configuration_error(conflict));
        }
        *target = Some(port);
    }
    Ok(())
}

fn merge_instance(
    target: &mut Option<String>,
    instance: Option<String>,
) -> Result<(), DriverError> {
    if let Some(instance) = instance {
        if target
            .as_deref()
            .is_some_and(|existing| !existing.eq_ignore_ascii_case(&instance))
        {
            return Err(configuration_error(
                "SQL Server URL 中的 instanceName 配置冲突",
            ));
        }
        *target = Some(instance);
    }
    Ok(())
}

fn apply_jdbc_client_options(
    client: &mut Config,
    properties: &HashMap<String, String>,
) -> Result<JdbcTlsOptions, DriverError> {
    let encryption = match property(properties, &["encrypt"])? {
        Some(value) if value.eq_ignore_ascii_case("strict") => {
            return Err(configuration_error(
                "SQL Server encrypt=strict 需要 TDS 8.0 支持，当前 adapter 不支持",
            ));
        }
        Some(value) if value == "DANGER_PLAINTEXT" => EncryptionLevel::NotSupported,
        Some(value) => match parse_bool(&value, "encrypt")? {
            true => EncryptionLevel::Required,
            false => EncryptionLevel::Off,
        },
        None => EncryptionLevel::Required,
    };
    let trust_server_certificate = property(properties, &["trustservercertificate"])?
        .map(|value| parse_bool(&value, "trustServerCertificate"))
        .transpose()?
        .unwrap_or(false);
    let trust_server_certificate_ca = property(properties, &["trustservercertificateca"])?;
    if trust_server_certificate && trust_server_certificate_ca.is_some() {
        return Err(configuration_error(
            "trustServerCertificate 与 trustServerCertificateCA 不能同时配置",
        ));
    }
    let hostname_in_certificate = property(
        properties,
        &["hostnameincertificate", "hostname in certificate"],
    )?;
    if let Some(application_name) = property(properties, &["applicationname", "application name"])?
    {
        client.application_name(application_name);
    }
    if let Some(workstation_id) = property(properties, &["workstationid", "workstation id"])? {
        client.client_name(workstation_id);
    }
    if let Some(application_intent) =
        property(properties, &["applicationintent", "application intent"])?
    {
        client.readonly(application_intent.trim().eq_ignore_ascii_case("readonly"));
    }
    client.multi_subnet_failover(false);
    let options = JdbcTlsOptions {
        encryption,
        trust_server_certificate,
        trust_server_certificate_ca,
        hostname_in_certificate,
    };
    options.apply(client);
    Ok(options)
}

fn parse_server_address(input: &str) -> Result<(String, Option<String>, Option<u16>), DriverError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(configuration_error("SQL Server 主机不能为空"));
    }

    let (host, remainder) = if let Some(bracketed) = input.strip_prefix('[') {
        let end = bracketed
            .find(']')
            .ok_or_else(|| configuration_error("SQL Server IPv6 主机格式无效"))?;
        (&bracketed[..end], &bracketed[end + 1..])
    } else {
        let split = input.find('\\').or_else(|| input.rfind(':'));
        match split {
            Some(index) => (&input[..index], &input[index..]),
            None => (input, ""),
        }
    };
    let host = host.trim_matches(['[', ']']).trim().to_string();
    if host.is_empty() {
        return Err(configuration_error("SQL Server 主机不能为空"));
    }

    let mut remainder = remainder;
    let mut instance = None;
    if let Some(value) = remainder.strip_prefix('\\') {
        let split = value.find(':').unwrap_or(value.len());
        let name = value[..split].trim();
        if name.is_empty() {
            return Err(configuration_error("SQL Server instanceName 不能为空"));
        }
        instance = Some(name.to_string());
        remainder = &value[split..];
    }

    let port = if remainder.is_empty() {
        None
    } else {
        let raw = remainder
            .strip_prefix(':')
            .ok_or_else(|| configuration_error("SQL Server 主机地址格式无效"))?;
        Some(parse_port_value(raw, "port")?)
    };
    Ok((host, instance, port))
}

fn parse_properties(input: &str) -> Result<HashMap<String, String>, DriverError> {
    let chars: Vec<char> = input.chars().collect();
    let mut position = 0;
    let mut properties = HashMap::new();
    while position < chars.len() {
        skip_whitespace(&chars, &mut position);
        if position == chars.len() {
            break;
        }
        if chars[position] == ';' {
            position += 1;
            continue;
        }

        let key_start = position;
        while position < chars.len() && chars[position] != '=' && chars[position] != ';' {
            position += 1;
        }
        if position == chars.len() || chars[position] != '=' {
            return Err(configuration_error(
                "SQL Server JDBC URL 参数必须为 key=value",
            ));
        }
        let key = normalize_key(&chars[key_start..position].iter().collect::<String>());
        if key.is_empty() {
            return Err(configuration_error("SQL Server JDBC URL 参数名不能为空"));
        }
        position += 1;
        skip_whitespace(&chars, &mut position);
        let value = if position < chars.len() && chars[position] == '{' {
            position += 1;
            let mut value = String::new();
            let mut closed = false;
            while position < chars.len() {
                let ch = chars[position];
                if ch == '}' {
                    if chars.get(position + 1) == Some(&'}') {
                        value.push('}');
                        position += 2;
                    } else {
                        position += 1;
                        closed = true;
                        break;
                    }
                } else {
                    value.push(ch);
                    position += 1;
                }
            }
            if !closed {
                return Err(configuration_error(
                    "SQL Server JDBC URL 的花括号参数未闭合",
                ));
            }
            skip_whitespace(&chars, &mut position);
            if position < chars.len() && chars[position] != ';' {
                return Err(configuration_error(
                    "SQL Server JDBC URL 参数花括号后只能跟分号或结尾",
                ));
            }
            value
        } else {
            let start = position;
            while position < chars.len() && chars[position] != ';' {
                position += 1;
            }
            chars[start..position]
                .iter()
                .collect::<String>()
                .trim()
                .to_string()
        };
        if properties.insert(key.clone(), value).is_some() {
            return Err(configuration_error(&format!(
                "SQL Server JDBC URL 参数重复：{key}"
            )));
        }
        if position < chars.len() {
            position += 1;
        }
    }
    Ok(properties)
}

fn skip_whitespace(chars: &[char], position: &mut usize) {
    while *position < chars.len() && chars[*position].is_whitespace() {
        *position += 1;
    }
}

fn reject_unsupported_properties(properties: &HashMap<String, String>) -> Result<(), DriverError> {
    const SUPPORTED: &[&str] = &[
        "application name",
        "applicationname",
        "applicationintent",
        "database",
        "databasename",
        "initial catalog",
        "encrypt",
        "hostname in certificate",
        "hostnameincertificate",
        "instancename",
        "instance name",
        "integrated security",
        "integratedsecurity",
        "login timeout",
        "logintimeout",
        "multi subnet failover",
        "multisubnetfailover",
        "packet size",
        "packetsize",
        "password",
        "port",
        "portnumber",
        "pwd",
        "query timeout",
        "querytimeout",
        "server name",
        "servername",
        "socket timeout",
        "sockettimeout",
        "trustservercertificate",
        "trustservercertificateca",
        "uid",
        "user",
        "user id",
        "username",
        "workstation id",
        "workstationid",
    ];
    for key in properties.keys() {
        if !SUPPORTED.contains(&key.as_str()) {
            return Err(configuration_error(&format!(
                "当前 SQL Server adapter 不支持 JDBC 参数：{key}"
            )));
        }
    }
    Ok(())
}

fn property(
    properties: &HashMap<String, String>,
    aliases: &[&str],
) -> Result<Option<String>, DriverError> {
    let mut found: Option<&str> = None;
    for alias in aliases {
        if let Some(value) = properties.get(&normalize_key(alias)) {
            if let Some(previous) = found {
                if previous != value {
                    return Err(configuration_error("SQL Server JDBC URL 的同类参数值冲突"));
                }
            } else {
                found = Some(value);
            }
        }
    }
    Ok(found.map(str::to_string))
}

fn normalize_key(key: &str) -> String {
    key.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn parse_port(value: &str, name: &str) -> Result<Option<u16>, DriverError> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    parse_port_value(value, name).map(Some)
}

fn parse_port_value(value: &str, name: &str) -> Result<u16, DriverError> {
    value
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| configuration_error(&format!("SQL Server {name} 必须是 1..=65535")))
}

fn parse_timeout(value: &str, name: &str) -> Result<Duration, DriverError> {
    let seconds = value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)
        .ok_or_else(|| configuration_error(&format!("SQL Server {name} 必须是正整数秒")))?;
    Ok(Duration::from_secs(seconds))
}

fn parse_query_timeout(value: &str) -> Result<Option<Duration>, DriverError> {
    let seconds = value.trim().parse::<i128>().map_err(|_| {
        configuration_error("SQL Server queryTimeout 必须是正整数秒、0 或 -1（不限时）")
    })?;
    match seconds {
        -1 | 0 => Ok(None),
        seconds if seconds > 0 && seconds <= u64::MAX as i128 => {
            Ok(Some(Duration::from_secs(seconds as u64)))
        }
        _ => Err(configuration_error(
            "SQL Server queryTimeout 必须是正整数秒、0 或 -1（不限时）",
        )),
    }
}

fn parse_socket_timeout(value: &str) -> Result<Option<Duration>, DriverError> {
    let millis = value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|millis| *millis <= i32::MAX as u64)
        .ok_or_else(|| {
            configuration_error("SQL Server socketTimeout 必须是 0..=2147483647 毫秒，0 表示不限时")
        })?;
    Ok((millis != 0).then(|| Duration::from_millis(millis)))
}

fn parse_bool(value: &str, name: &str) -> Result<bool, DriverError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" => Ok(true),
        "false" | "no" => Ok(false),
        _ => Err(configuration_error(&format!(
            "SQL Server {name} 参数值无效"
        ))),
    }
}

fn configuration_error(message: &str) -> DriverError {
    DriverError::new(ErrorKind::Configuration, message)
}

#[cfg(test)]
#[path = "sql_server_config_tests.rs"]
mod tests;
