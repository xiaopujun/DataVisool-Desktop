/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::net::{IpAddr, ToSocketAddrs};

use serde::Serialize;
use serde_json::{Map, Value};
use url::Url;

use crate::backend::ai_driven::domain::artifact::{ComponentConfigPatch, PageDraftComponent};

use super::{AgentCapabilityGrant, AgentOperation};

/// 由具体参数计算的动作风险等级。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SecurityRisk {
    /// 保留给完全只读的未来 Agent 操作；当前真实写入不会落入该等级。
    #[allow(dead_code)]
    R0,
    R1,
    R2,
    R3,
}

/// 服务端在执行前返回的结构化权限决策。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AuthorizationDecision {
    Allow(SecurityRisk),
    RequireApproval(SecurityRisk),
}

impl AuthorizationDecision {
    pub(crate) fn risk(&self) -> SecurityRisk {
        match self {
            Self::Allow(risk) | Self::RequireApproval(risk) => *risk,
        }
    }
}

/// 可安全返回给模型与前端的稳定安全错误。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SecurityDecisionError {
    code: &'static str,
    message: String,
}

impl SecurityDecisionError {
    pub(crate) fn deny(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    pub(crate) fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for SecurityDecisionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SecurityDecisionError {}

pub(crate) fn authorize_canvas_create(
    grant: &AgentCapabilityGrant,
    components: &[PageDraftComponent],
) -> Result<AuthorizationDecision, SecurityDecisionError> {
    grant.authorize_operation(AgentOperation::CanvasCreate)?;
    grant.authorize_create_count(components.len())?;
    for component in components {
        if let Some(data) = component.config.get("data").and_then(Value::as_object) {
            authorize_data_source(grant, data)?;
        }
    }
    Ok(AuthorizationDecision::RequireApproval(SecurityRisk::R2))
}

pub(crate) fn authorize_canvas_delete(grant: &AgentCapabilityGrant, targets: &[String]) -> Result<AuthorizationDecision, SecurityDecisionError> {
    grant.authorize_operation(AgentOperation::CanvasDelete)?;
    grant.authorize_existing_targets(targets)?;
    Ok(AuthorizationDecision::RequireApproval(SecurityRisk::R3))
}

pub(crate) fn authorize_canvas_patch(
    grant: &AgentCapabilityGrant,
    patches: &[ComponentConfigPatch],
) -> Result<AuthorizationDecision, SecurityDecisionError> {
    grant.authorize_operation(AgentOperation::CanvasPatch)?;
    let targets = patches.iter().map(|patch| patch.component_id.clone()).collect::<Vec<_>>();
    grant.authorize_existing_targets(&targets)?;

    let mut risk = SecurityRisk::R1;
    for patch in patches {
        if let Some(data) = &patch.data_patch {
            risk = max_risk(risk, authorize_data_source(grant, data)?);
        }
        if patch.layer_patch.is_some() || patch.filter_patch.is_some() {
            risk = max_risk(risk, SecurityRisk::R2);
        }
        if patch
            .style_patch
            .as_ref()
            .is_some_and(|style| style.style.keys().any(|key| is_data_mapping_key(key)))
        {
            risk = max_risk(risk, SecurityRisk::R2);
        }
    }
    Ok(match risk {
        SecurityRisk::R0 | SecurityRisk::R1 => AuthorizationDecision::Allow(risk),
        SecurityRisk::R2 | SecurityRisk::R3 => AuthorizationDecision::RequireApproval(risk),
    })
}

fn authorize_data_source(grant: &AgentCapabilityGrant, data: &Map<String, Value>) -> Result<SecurityRisk, SecurityDecisionError> {
    let source_type = data
        .get("sourceType")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| SecurityDecisionError::deny("agent.security.data_source_invalid", "数据源缺少 sourceType"))?;
    grant.authorize_data_source(source_type)?;
    match source_type {
        "static" => Ok(SecurityRisk::R2),
        "api" => {
            let branch = data
                .get("apiData")
                .and_then(Value::as_object)
                .ok_or_else(|| SecurityDecisionError::deny("agent.security.data_source_invalid", "API 数据源缺少 apiData"))?;
            authorize_api_egress(grant, branch)?;
            Ok(SecurityRisk::R3)
        }
        "database" | "commonApi" | "globalVariable" => Ok(SecurityRisk::R3),
        "webSocket" | "mqtt" => Err(SecurityDecisionError::deny(
            "agent.security.network_protocol_forbidden",
            "Agent 默认禁止创建 WebSocket 或 MQTT 外部连接",
        )),
        _ => Err(SecurityDecisionError::deny("agent.security.data_source_forbidden", "不支持的 Agent 数据源")),
    }
}

fn authorize_api_egress(grant: &AgentCapabilityGrant, api: &Map<String, Value>) -> Result<(), SecurityDecisionError> {
    let raw_url = api
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| SecurityDecisionError::deny("agent.security.network_url_invalid", "API 数据源缺少 HTTPS URL"))?;
    let url = Url::parse(raw_url).map_err(|_| SecurityDecisionError::deny("agent.security.network_url_invalid", "API URL 格式无效"))?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(SecurityDecisionError::deny("agent.security.network_url_forbidden", "仅允许不含认证信息的 HTTPS URL"));
    }
    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| SecurityDecisionError::deny("agent.security.network_url_invalid", "API URL 缺少主机名"))?;
    if is_local_or_private_host(&host) {
        return Err(SecurityDecisionError::deny(
            "agent.security.network_private_address",
            "禁止访问本机、内网、链路本地和云元数据地址",
        ));
    }
    grant.authorize_network_host(&host)?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|_| SecurityDecisionError::deny("agent.security.network_dns_failed", "无法安全解析网络目标"))?
        .map(|address| address.ip())
        .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(*address)) {
        return Err(SecurityDecisionError::deny("agent.security.network_private_address", "网络目标解析到了非公网地址"));
    }
    let method = api
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("get")
        .trim()
        .to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS") {
        return Err(SecurityDecisionError::deny(
            "agent.security.network_method_forbidden",
            "Agent 默认只允许 GET、HEAD 或 OPTIONS 请求",
        ));
    }
    Ok(())
}

fn is_data_mapping_key(key: &str) -> bool {
    matches!(
        key,
        "xField" | "yField" | "seriesField" | "angleField" | "colorField" | "sizeField" | "shapeField" | "wordField" | "weightField" | "groupField"
    )
}

fn max_risk(left: SecurityRisk, right: SecurityRisk) -> SecurityRisk {
    if right as u8 > left as u8 {
        right
    } else {
        left
    }
}

fn is_local_or_private_host(host: &str) -> bool {
    if matches!(host, "localhost" | "localhost.localdomain" | "metadata.google.internal") {
        return true;
    }
    host.parse::<IpAddr>().map(|address| !is_public_ip(address)).unwrap_or(false)
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.octets()[0] == 0
                || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
                || (ip.octets()[0] == 169 && ip.octets()[1] == 254))
        }
        IpAddr::V6(ip) => !(ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() || ip.is_unique_local() || ip.is_unicast_link_local()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::backend::ai_driven::tools::context::AgentToolScope;

    use super::*;

    #[test]
    fn rejects_local_and_private_network_targets() {
        for host in ["localhost", "127.0.0.1", "10.0.0.1", "169.254.169.254", "::1", "fc00::1"] {
            assert!(is_local_or_private_host(host), "{host} must be rejected");
        }
        assert!(!is_local_or_private_host("example.com"));
    }

    #[test]
    fn public_ip_filter_rejects_private_ranges() {
        assert!(!is_public_ip("192.168.1.1".parse().unwrap()));
        assert!(!is_public_ip("100.64.0.1".parse().unwrap()));
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn api_egress_rejects_http_and_private_targets_before_dns() {
        let scope = AgentToolScope::new("task", "run", "thread", "project").unwrap();
        let mut grant = AgentCapabilityGrant::for_task(&scope, [], "workspace");
        grant.allowed_data_source_types = BTreeSet::from(["api".to_string()]);
        grant.allowed_network_hosts = BTreeSet::from(["example.com".to_string()]);

        let http = serde_json::json!({
            "sourceType": "api",
            "apiData": { "url": "http://example.com", "method": "get" }
        });
        assert_eq!(
            authorize_data_source(&grant, http.as_object().unwrap()).unwrap_err().code(),
            "agent.security.network_url_forbidden"
        );

        let private = serde_json::json!({
            "sourceType": "api",
            "apiData": { "url": "https://127.0.0.1", "method": "get" }
        });
        assert_eq!(
            authorize_data_source(&grant, private.as_object().unwrap()).unwrap_err().code(),
            "agent.security.network_private_address"
        );
    }
}
