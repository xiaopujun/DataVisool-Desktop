/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use crate::ConnectionConfig;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Configuration,
    Connection,
    Query,
}

#[derive(Debug)]
pub struct DriverError {
    pub kind: ErrorKind,
    pub message: String,
}

impl DriverError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn sanitize(mut self, config: &ConnectionConfig) -> Self {
        let mut secrets = vec![config.password.clone()];
        let password_pattern = password_pattern();
        if let Some(url) = config
            .url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
        {
            for captures in password_pattern.captures_iter(url) {
                for index in 1..=4 {
                    if let Some(secret) = captures.get(index) {
                        secrets.push(secret.as_str().to_owned());
                        secrets.push(secret.as_str().replace("}}", "}"));
                    }
                }
            }
            if let Ok(parsed) = url::Url::parse(url.strip_prefix("jdbc:").unwrap_or(url)) {
                if let Some(password) = parsed.password() {
                    secrets.push(password.to_owned());
                    if let Ok(decoded) = urlencoding::decode(password) {
                        secrets.push(decoded.into_owned());
                    }
                }
            }
            self.message = self.message.replace(url, "[数据库连接串]");
        }
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        for secret in secrets.into_iter().filter(|secret| !secret.is_empty()) {
            self.message = self.message.replace(&secret, "***");
            self.message = self
                .message
                .replace(urlencoding::encode(&secret).as_ref(), "***");
        }
        self.message = password_pattern
            .replace_all(&self.message, "password=***")
            .into_owned();
        self
    }
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}

impl std::error::Error for DriverError {}

fn password_pattern() -> &'static regex::Regex {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    PATTERN.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)(?:password|pwd)\s*=\s*(?:\{((?:[^}]|}})*)\}|"([^"]*)"|'([^']*)'|([^;\s&]+))"#,
        )
        .expect("constant password pattern")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_from_fields_and_jdbc_properties_are_redacted() {
        let config = ConnectionConfig {
            password: "field password".into(),
            url: Some("jdbc:sqlserver://db;password={url;secret}}value}".into()),
            ..Default::default()
        };
        let error = DriverError::new(ErrorKind::Connection,
            "field password field%20password url;secret}value password=other-secret; connection failed")
            .sanitize(&config);
        assert!(!error.message.contains("field password"));
        assert!(!error.message.contains("field%20password"));
        assert!(!error.message.contains("url;secret"));
        assert!(!error.message.contains("other-secret"));
        assert!(error.message.contains("connection failed"));
    }

    #[test]
    fn connection_debug_never_exposes_credentials_or_url() {
        let config = ConnectionConfig {
            password: "secret".into(),
            url: Some("private-url".into()),
            ..Default::default()
        };
        let debug = format!("{config:?}");
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("private-url"));
    }
}
