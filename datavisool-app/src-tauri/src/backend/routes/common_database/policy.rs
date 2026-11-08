/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

use super::{DataSource, StringOrNumber};

/// 校验数据库显示名称、端口和描述，供新增和更新接口共用。
pub(super) fn validate(data: &DataSource) -> Option<&'static str> {
    if data
        .name
        .as_deref()
        .unwrap_or_default()
        .chars()
        .all(|character| character.is_whitespace() || character == '\u{feff}')
    {
        return Some("请输入数据库显示名称");
    }

    let port = match &data.port {
        Some(StringOrNumber::Str(value)) => value.parse::<u16>().ok(),
        Some(StringOrNumber::Num(value)) => u16::try_from(*value).ok(),
        None => None,
    };
    if !matches!(port, Some(1..=u16::MAX)) {
        return Some("端口必须在 1 至 65535 之间");
    }
    // 与浏览器 maxlength 使用相同的 UTF-16 单位。
    if data.des.as_ref().is_some_and(|value| value.encode_utf16().count() > 500) {
        return Some("描述不能超过 500 个字符");
    }
    None
}

pub(super) fn generated_url(ty: &str, ip: &str, port: &str, database: &str, username: &str, password: &str) -> String {
    let scheme = if ty.eq_ignore_ascii_case("MySQL") {
        "mysql"
    } else if ty.eq_ignore_ascii_case("PostgresSQL") || ty.eq_ignore_ascii_case("PostgreSQL") {
        "postgres"
    } else {
        return String::new();
    };
    format!(
        "{}://{}:{}@{}:{}/{}",
        scheme,
        urlencoding::encode(username),
        urlencoding::encode(password),
        ip,
        port,
        database
    )
}

/// 旧版本把自动生成的 URL 保存在库中；只在完全匹配时识别，避免误删自定义 URL。
pub(super) fn custom_url(url: Option<&str>, ty: &str, ip: &str, port: &str, database: &str, username: &str, password: &str) -> String {
    let value = url.unwrap_or_default().trim();
    if value == generated_url(ty, ip, port, database, username, password) {
        String::new()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn data(port: serde_json::Value, des: &str) -> DataSource {
        serde_json::from_value(json!({"name": "sample", "port": port, "des": des})).unwrap()
    }

    #[test]
    fn rejects_missing_or_whitespace_names_but_accepts_original_valid_text() {
        for name in [None, Some(""), Some("   "), Some("\t"), Some("\u{3000}"), Some("\u{feff}"), Some("\u{0085}")] {
            let mut value = json!({"port": "3306"});
            if let Some(name) = name {
                value["name"] = json!(name);
            }
            let parsed: DataSource = serde_json::from_value(value).unwrap();
            assert_eq!(validate(&parsed), Some("请输入数据库显示名称"));
        }

        let mut parsed = data(json!(3306), "ok");
        parsed.name = Some("  saved name  ".to_string());
        assert_eq!(validate(&parsed), None);
        assert_eq!(parsed.name.as_deref(), Some("  saved name  "));
    }

    #[test]
    fn validates_port_and_description_at_the_write_boundary() {
        for port in [json!(0), json!(65536), json!("-1"), json!("1.5"), json!(null)] {
            assert!(validate(&data(port, "ok")).is_some());
        }
        for port in [json!(1), json!("65535")] {
            assert!(validate(&data(port, &"a".repeat(500))).is_none());
        }
        assert!(validate(&data(json!(3306), &"😀".repeat(251))).is_some());
        assert!(validate(&data(json!(3306), &"😀".repeat(250))).is_none());
    }

    #[test]
    fn recognizes_only_exact_legacy_generated_urls() {
        let generated = generated_url("MySQL", "127.0.0.1", "3306", "db", "user", "p@ss");
        assert_eq!(custom_url(Some(&generated), "MySQL", "127.0.0.1", "3306", "db", "user", "p@ss"), "");
        let custom = format!("{generated}?ssl-mode=required");
        assert_eq!(custom_url(Some(&custom), "MySQL", "127.0.0.1", "3306", "db", "user", "p@ss"), custom);
        assert_eq!(custom_url(Some(&generated), "MySQL", "127.0.0.1", "3307", "db", "user", "p@ss"), generated);
    }
}
