/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use crate::{ConnectionConfig, DriverError, ErrorKind};

pub(super) fn driver_config(config: &ConnectionConfig) -> Result<oracledb::Config, DriverError> {
    if config.username.trim().is_empty() {
        return Err(DriverError::new(
            ErrorKind::Configuration,
            "Oracle用户名不能为空",
        ));
    }
    if config.password.is_empty() {
        return Err(DriverError::new(
            ErrorKind::Configuration,
            "Oracle密码不能为空",
        ));
    }

    let connect_string = connect_string(config)?;
    oracledb::Config::default()
        .set_credentials(config.username.trim(), &config.password)
        .set_connect_string(&connect_string)
        .map_err(|_| DriverError::new(ErrorKind::Configuration, "Oracle连接串格式无效"))
}

pub(super) fn connect_string(config: &ConnectionConfig) -> Result<String, DriverError> {
    if let Some(url) = config
        .url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return normalize_custom_url(url);
    }

    let host = config.host.trim();
    if host.is_empty() || !valid_host(host) {
        return Err(DriverError::new(
            ErrorKind::Configuration,
            "Oracle主机名格式无效",
        ));
    }
    let port = parse_port(&config.port)?;
    let service_name = config.database.trim();
    if !valid_identifier(service_name) {
        return Err(DriverError::new(
            ErrorKind::Configuration,
            "Oracle服务名不能为空或格式无效",
        ));
    }

    let descriptor_host = strip_host_brackets(host);
    Ok(format!(
        "(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST={descriptor_host})(PORT={port}))(CONNECT_DATA=(SERVICE_NAME={service_name})))"
    ))
}

fn normalize_custom_url(url: &str) -> Result<String, DriverError> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("jdbc:oracle:oci:") {
        return Err(DriverError::new(
            ErrorKind::Configuration,
            "桌面端Oracle驱动只支持Thin连接串，请将OCI连接串改为Thin连接串",
        ));
    }

    if lower.starts_with("jdbc:oracle:") {
        let Some((prefix, target)) = url.split_once('@') else {
            return Err(DriverError::new(
                ErrorKind::Configuration,
                "Oracle JDBC连接串缺少@后的连接目标",
            ));
        };
        if !lower.starts_with("jdbc:oracle:thin:") {
            return Err(DriverError::new(
                ErrorKind::Configuration,
                "不支持此Oracle JDBC驱动类型，请使用jdbc:oracle:thin",
            ));
        }
        if !prefix.eq_ignore_ascii_case("jdbc:oracle:thin:") {
            return Err(DriverError::new(
                ErrorKind::Configuration,
                "请将Oracle JDBC连接串中的用户名和密码填写到独立字段",
            ));
        }
        return normalize_jdbc_target(target);
    }

    if url.starts_with('@') {
        return normalize_jdbc_target(&url[1..]);
    }

    if url.starts_with('(') {
        return Ok(url.to_string());
    }
    if is_easy_connect_or_alias(url) {
        validate_easy_connect_path(url)?;
        return Ok(url.to_string());
    }

    Err(DriverError::new(
        ErrorKind::Configuration,
        "Oracle连接串需使用Thin JDBC格式、Oracle Net描述符或主机/端口/服务名格式",
    ))
}

fn normalize_jdbc_target(target: &str) -> Result<String, DriverError> {
    let target = target.trim();
    if target.is_empty() {
        return Err(DriverError::new(
            ErrorKind::Configuration,
            "Oracle JDBC连接串缺少连接目标",
        ));
    }
    if target.starts_with('(') {
        return Ok(target.to_string());
    }
    if target.starts_with("//") || target.contains('/') {
        validate_easy_connect_path(target)?;
        return Ok(target.to_string());
    }

    // The Web URL builder historically emitted @:<host>:<port>:<SID>.
    // Oracle's Thin URL form @<host>:<port>:<SID> uses the same SID mapping.
    let sid_target = target.strip_prefix(':').unwrap_or(target);
    if let Some((host, port, sid)) = split_sid_target(sid_target) {
        let host = strip_host_brackets(host);
        if !valid_host(host) || !valid_identifier(sid) {
            return Err(DriverError::new(
                ErrorKind::Configuration,
                "Oracle SID连接串中的主机或SID格式无效",
            ));
        }
        let port = parse_port(port)?;
        return Ok(format!(
            "(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST={host})(PORT={port}))(CONNECT_DATA=(SID={sid})))"
        ));
    }

    if is_easy_connect_or_alias(target) {
        Ok(target.to_string())
    } else {
        Err(DriverError::new(
            ErrorKind::Configuration,
            "Oracle SID连接串格式无效",
        ))
    }
}

fn validate_easy_connect_path(connect_string: &str) -> Result<(), DriverError> {
    let target = connect_string
        .split_once("://")
        .map(|(_, target)| target)
        .unwrap_or(connect_string);
    let target = target.strip_prefix("//").unwrap_or(target);
    let Some((_, path)) = target.split_once('/') else {
        return Ok(());
    };

    let (service_name, suffix) = split_easy_connect_segment(path);
    let valid_service = valid_easy_connect_name(service_name);
    let valid_suffix = if let Some(instance) = suffix.strip_prefix('/') {
        valid_easy_connect_name(instance)
    } else if let Some(server_and_instance) = suffix.strip_prefix(':') {
        let (server_type, instance) = match server_and_instance.split_once('/') {
            Some((server_type, instance)) => (server_type, Some(instance)),
            None => (server_and_instance, None),
        };
        !server_type.is_empty()
            && server_type.chars().all(|ch| ch.is_ascii_alphabetic())
            && instance.map_or(true, valid_easy_connect_name)
    } else {
        suffix.is_empty()
    };

    if !valid_service || !valid_suffix {
        return Err(DriverError::new(
            ErrorKind::Configuration,
            "Oracle Easy Connect服务名或实例名格式无效，或包含未支持的路径段，请改用完整Oracle Net描述符",
        ));
    }
    Ok(())
}

fn split_easy_connect_segment(value: &str) -> (&str, &str) {
    match value.find(['/', ':']) {
        Some(index) => (&value[..index], &value[index..]),
        None => (value, ""),
    }
}

fn valid_easy_connect_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
}

fn split_sid_target(target: &str) -> Option<(&str, &str, &str)> {
    // Bracketed IPv6 keeps the host's colons inside the first component.
    if let Some(bracket_end) = target.find(']') {
        let host = target.get(..=bracket_end)?;
        let rest = target.get(bracket_end + 1..)?.strip_prefix(':')?;
        let (port, sid) = rest.split_once(':')?;
        if sid.contains(':') {
            return None;
        }
        return Some((host, port, sid));
    }

    let (host_and_port, sid) = target.rsplit_once(':')?;
    let (host, port) = host_and_port.rsplit_once(':')?;
    if host.contains(':') {
        return None;
    }
    Some((host, port, sid))
}

fn is_easy_connect_or_alias(value: &str) -> bool {
    !value.is_empty()
        && !value.chars().any(char::is_whitespace)
        && !value.contains('@')
        && !value.contains(';')
        && !value.contains(')')
        && !value.contains('(')
}

fn valid_host(host: &str) -> bool {
    let host = strip_host_brackets(host);
    !host.is_empty()
        && host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "._:-%".contains(ch))
}

fn strip_host_brackets(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "._$#-".contains(ch))
}

fn parse_port(value: &str) -> Result<u16, DriverError> {
    let port = value
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| {
            DriverError::new(
                ErrorKind::Configuration,
                "Oracle端口必须是1到65535之间的整数",
            )
        })?;
    Ok(port)
}
