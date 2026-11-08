/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 本模块放“与具体业务无关”的小工具函数。
//!
//! 这里的目标是：让 router/route handler 里保持更高层语义，
//! 避免把字符串解析、参数裁剪等杂事散落到各处。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use serde_json::Value;

/// 从 URL 最后一段解析数字 id。
pub(crate) fn parse_id_from_tail(url: &str) -> Option<i64> {
    url.rsplit('/')
        .next()
        .and_then(|s| if s.is_empty() { None } else { s.parse::<i64>().ok() })
}

/// 清理错误信息中的密码、连接串密码和大段 base64 数据。
pub(crate) fn sanitize_error_message(msg: &str) -> String {
    let s = redact_url_password(msg, "mysql://");
    let s = redact_url_password(&s, "postgres://");
    let s = redact_kv_password(&s);
    redact_base64_from_text(&s)
}

/// 将调试日志中的 JSON 序列化为可读文本，并强制隐藏敏感字段和二进制附件。
///
/// Agent 调试日志允许保留完整的文本、工具参数和模型返回值，但 API Key、Token、密码
/// 及 Base64 不能写入磁盘。序列化失败时保留错误说明，避免影响正常业务流程。
pub(crate) fn debug_log_json<T: Serialize + ?Sized>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(mut value) => {
            redact_debug_log_value(&mut value);
            serde_json::to_string(&value)
                .unwrap_or_else(|error| format!("[debug payload serialize failed: {}]", sanitize_error_message(&error.to_string())))
        }
        Err(error) => format!("[debug payload unavailable: {}]", sanitize_error_message(&error.to_string())),
    }
}

/// 清理非 JSON 调试文本中的敏感信息；JSON 文本会先按字段名递归脱敏。
pub(crate) fn sanitize_debug_log_text(text: &str) -> String {
    match serde_json::from_str::<Value>(text) {
        Ok(mut value) => {
            redact_debug_log_value(&mut value);
            serde_json::to_string(&value)
                .unwrap_or_else(|error| format!("[debug payload serialize failed: {}]", sanitize_error_message(&error.to_string())))
        }
        Err(_) => sanitize_debug_log_fragment(text),
    }
}

/// 计算字节内容的 SHA-256 十六进制字符串。
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// 清理上传资源文件名中的常见非法字符，空值时使用调用方默认名。
pub(crate) fn sanitize_upload_file_name(input: &str, default_name: &str) -> String {
    let mut out = input.trim().to_string();
    if out.is_empty() {
        out = default_name.to_string();
    }
    out.replace([':', '*', '?', '"', '<', '>', '|'], "_")
}

/// 清理下载或导出文件名中的路径分隔符、控制字符和常见非法字符。
pub(crate) fn sanitize_download_file_name(input: &str, default_name: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        let illegal = matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
        if illegal || ch.is_control() {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let trimmed = out.trim().to_string();
    if trimmed.is_empty() {
        default_name.to_string()
    } else {
        trimmed
    }
}

/// 将外部输入归一成单个安全路径片段，避免路径穿越和空目录名。
pub(crate) fn sanitize_path_segment(input: &str, default_value: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        default_value.to_string()
    } else {
        trimmed.to_string()
    }
}

/// 已预留但尚未最终确认的组件包目录。
#[derive(Debug)]
pub(crate) struct ReservedComponentPackage {
    /// 组件包最终目录名称。
    pub(crate) package_id: String,
    /// 组件包提交后的最终目录。
    pub(crate) final_dir: PathBuf,
    /// 解压或写入期间使用的临时目录。
    pub(crate) temp_dir: PathBuf,
    /// 用于并发占位的预留目录。
    reservation_dir: PathBuf,
    /// 临时目录是否已经移动为最终目录。
    committed: bool,
    /// 调用方是否确认保留该组件包目录。
    finalized: bool,
}

impl ReservedComponentPackage {
    /// 将组件包暂存目录原子移动到最终目录。
    pub(crate) fn commit(&mut self) -> Result<(), String> {
        std::fs::rename(&self.temp_dir, &self.final_dir).map_err(|e| format!("rename component package failed: {e}"))?;
        let _ = std::fs::remove_dir_all(&self.reservation_dir);
        self.committed = true;
        Ok(())
    }

    /// 标记组件包处理已完成，避免 Drop 阶段自动回滚。
    pub(crate) fn finalize(&mut self) {
        self.finalized = true;
    }

    /// 回滚组件包目录，删除已提交目录或尚未提交的临时目录。
    pub(crate) fn rollback(&mut self) {
        if self.committed {
            let _ = std::fs::remove_dir_all(&self.final_dir);
        } else {
            let _ = std::fs::remove_dir_all(&self.temp_dir);
        }
        let _ = std::fs::remove_dir_all(&self.reservation_dir);
        self.finalized = true;
    }
}

impl Drop for ReservedComponentPackage {
    fn drop(&mut self) {
        // 组件包处理失败或提前返回时，依赖 Drop 清理中间目录。
        if !self.finalized {
            self.rollback();
        }
    }
}

/// 为组件包分配一个不会与现有包冲突的最终目录和暂存目录。
pub(crate) fn reserve_component_package(root: &Path, source_id: &str, temp_prefix: &str) -> Result<ReservedComponentPackage, String> {
    let source_id = source_id.trim().trim_start_matches('.');
    if source_id.is_empty() {
        return Err("empty component package id".to_string());
    }
    std::fs::create_dir_all(root).map_err(|e| format!("create component root failed: {e}"))?;
    cleanup_stale_component_package_workdirs(root, temp_prefix);
    let base = format!("{}-{}", source_id, current_datetime_suffix());
    for index in 0..1000 {
        let package_id = if index == 0 { base.clone() } else { format!("{}-{}", base, index) };
        let final_dir = root.join(&package_id);
        let temp_dir = root.join(format!("{temp_prefix}-{package_id}"));
        let reservation_dir = root.join(format!(".reserve-{package_id}"));
        if final_dir.exists() || temp_dir.exists() {
            continue;
        }
        match std::fs::create_dir(&reservation_dir) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("create reservation dir failed: {e}")),
        }
        match std::fs::create_dir(&temp_dir) {
            Ok(_) => {
                return Ok(ReservedComponentPackage { package_id, final_dir, temp_dir, reservation_dir, committed: false, finalized: false });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_dir_all(&reservation_dir);
                continue;
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&reservation_dir);
                return Err(format!("create temp dir failed: {e}"));
            }
        }
    }
    Err("unable to reserve unique component package dir".to_string())
}

/// 将本地文件路径转换为 Tauri asset protocol 可访问 URL。
pub(crate) fn to_asset_url(file_path: &Path) -> String {
    let raw = file_path.to_string_lossy().to_string();
    let encoded = urlencoding::encode(&raw).into_owned();

    #[cfg(target_os = "windows")]
    {
        format!("http://asset.localhost/{encoded}")
    }

    #[cfg(not(target_os = "windows"))]
    {
        format!("asset://localhost/{encoded}")
    }
}

/// 生成精确到毫秒的时间后缀，降低临时目录命名冲突概率。
pub(crate) fn current_datetime_suffix() -> String {
    chrono::Local::now().format("%Y%m%d%H%M%S%3f").to_string()
}

/// 清理 24 小时以前遗留的组件包预留目录和暂存目录。
fn cleanup_stale_component_package_workdirs(root: &Path, temp_prefix: &str) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let Some(cutoff) = SystemTime::now().checked_sub(Duration::from_secs(24 * 60 * 60)) else {
        return;
    };
    let temp_prefix = format!("{temp_prefix}-");
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(".reserve-") && !name.starts_with(&temp_prefix) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        // 只清理遗留目录，避免同名前缀普通文件进入删除分支。
        if !metadata.is_dir() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if modified < cutoff {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// 脱敏 `password=xxx` 这类键值参数中的密码。
fn redact_kv_password(msg: &str) -> String {
    let mut out = msg.to_string();
    for key in ["password=", "passwd=", "pwd="] {
        let mut search_from = 0usize;
        while let Some(pos) = out[search_from..].find(key) {
            let start = search_from + pos + key.len();
            let rest = &out[start..];
            let end_rel = rest
                .find(|c: char| c == '&' || c == ' ' || c == '\n' || c == '\r' || c == '\t')
                .unwrap_or(rest.len());
            let end = start + end_rel;
            out.replace_range(start..end, "***");
            search_from = end.min(out.len());
        }
    }
    out
}

/// 递归脱敏 JSON 中常见的认证字段，同时保留其他诊断内容。
fn redact_debug_log_value(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if is_sensitive_debug_log_key(key) {
                    *child = Value::String("[REDACTED]".to_string());
                } else {
                    redact_debug_log_value(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_debug_log_value(item);
            }
        }
        Value::String(text) => *text = sanitize_debug_log_fragment(text),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// 判断 JSON 字段名是否代表凭据。名称归一化后比较，避免遗漏 camelCase 与 snake_case 写法。
fn is_sensitive_debug_log_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(|character| character.to_lowercase())
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "apikey" | "authorization" | "token" | "accesstoken" | "refreshtoken" | "clientsecret" | "secret" | "password" | "passwd" | "pwd"
    )
}

/// 脱敏普通文本中的连接密码、Base64 与 `key=value` 形式的凭据。
fn sanitize_debug_log_fragment(text: &str) -> String {
    if let Ok(mut embedded_json) = serde_json::from_str::<Value>(text) {
        redact_debug_log_value(&mut embedded_json);
        if let Ok(serialized) = serde_json::to_string(&embedded_json) {
            return serialized;
        }
    }
    let mut sanitized = sanitize_error_message(text);
    for key in [
        "api_key=",
        "apikey=",
        "api-key=",
        "token=",
        "access_token=",
        "access-token=",
        "refresh_token=",
        "refresh-token=",
        "authorization=",
        "authorization:",
        "client_secret=",
        "client-secret=",
    ] {
        sanitized = redact_kv_value(&sanitized, key);
    }
    sanitized
}

/// 脱敏普通文本中某个 `key=value` 或 `key: value` 的值，键名匹配不区分大小写。
fn redact_kv_value(text: &str, key: &str) -> String {
    let mut output = text.to_string();
    let key_lowercase = key.to_ascii_lowercase();
    let mut search_from = 0usize;
    while search_from < output.len() {
        let lowercase = output[search_from..].to_ascii_lowercase();
        let Some(position) = lowercase.find(&key_lowercase) else {
            break;
        };
        let start = search_from + position + key.len();
        let rest = &output[start..];
        let whitespace_len = rest.len() - rest.trim_start().len();
        let value_start = start + whitespace_len;
        let value = &output[value_start..];
        let end_relative = if key.ends_with(':') {
            value.find(|character: char| matches!(character, '\n' | '\r' | ',' | '"' | '\'' | '}'))
        } else {
            value.find(|character: char| matches!(character, '&' | ',' | ' ' | '\n' | '\r' | '\t' | '"' | '\'' | '}'))
        }
        .unwrap_or(value.len());
        let end = value_start + end_relative;
        output.replace_range(start..end, "***");
        search_from = (start + 3).min(output.len());
    }
    output
}

/// 脱敏数据库连接 URL 中的密码片段。
fn redact_url_password(msg: &str, scheme: &str) -> String {
    let mut out = msg.to_string();
    let mut search_from = 0usize;
    while let Some(pos) = out[search_from..].find(scheme) {
        let start = search_from + pos + scheme.len();
        let rest = &out[start..];
        let Some(at_rel) = rest.find('@') else {
            break;
        };
        let creds_end = start + at_rel;
        let creds = &out[start..creds_end];
        if let Some(colon_rel) = creds.rfind(':') {
            let colon = start + colon_rel;
            out.replace_range(colon + 1..creds_end, "***");
            search_from = creds_end.min(out.len());
        } else {
            search_from = creds_end.min(out.len());
        }
    }
    out
}

/// 脱敏错误文本中可能夹带的 data URL 或超长 base64 片段。
fn redact_base64_from_text(text: &str) -> String {
    let mut out = redact_data_url_base64(text);
    for key in ["\"data\"", "\"b64_json\"", "data", "b64_json"] {
        out = redact_json_like_base64_value(&out, key);
    }
    out
}

/// 脱敏 `data:*;base64,...` 图片或文件片段，避免日志写入大块二进制文本。
fn redact_data_url_base64(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(pos) = rest.find("data:") {
        out.push_str(&rest[..pos]);
        let after_prefix = &rest[pos..];
        let Some(base64_marker) = after_prefix.find(";base64,") else {
            out.push_str("data:");
            rest = &after_prefix["data:".len()..];
            continue;
        };
        let value_start = base64_marker + ";base64,".len();
        let value = &after_prefix[value_start..];
        let value_len = value
            .find(|c: char| c == '"' || c == '\'' || c.is_whitespace())
            .unwrap_or(value.len());
        let approx_bytes = value_len.saturating_mul(3) / 4;

        out.push_str(&after_prefix[..value_start]);
        out.push_str(&format!("[BASE64 ≈{} bytes]", approx_bytes));
        rest = &value[value_len..];
    }

    out.push_str(rest);
    out
}

/// 脱敏 JSON 风格 `data` / `b64_json` 字段里的超长裸 base64 字符串。
fn redact_json_like_base64_value(text: &str, key: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(key_pos) = rest.find(key) {
        out.push_str(&rest[..key_pos]);
        let after_key = &rest[key_pos..];
        let Some(colon_rel) = after_key.find(':') else {
            out.push_str(after_key);
            return out;
        };
        let value_area_start = colon_rel + 1;
        let value_area = &after_key[value_area_start..];
        let leading_ws_len = value_area.len() - value_area.trim_start().len();
        let trimmed = &value_area[leading_ws_len..];
        let Some(quote) = trimmed.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            out.push_str(&after_key[..value_area_start + leading_ws_len]);
            rest = trimmed;
            continue;
        };
        let quoted = &trimmed[quote.len_utf8()..];
        let Some(end_rel) = quoted.find(quote) else {
            out.push_str(after_key);
            return out;
        };
        let value = &quoted[..end_rel];

        out.push_str(&after_key[..value_area_start + leading_ws_len + quote.len_utf8()]);
        if value.len() > 128 && looks_like_base64(value) {
            let approx_bytes = value.len().saturating_mul(3) / 4;
            out.push_str(&format!("[BASE64 ≈{} bytes]", approx_bytes));
        } else {
            out.push_str(value);
        }
        out.push(quote);
        rest = &quoted[end_rel + quote.len_utf8()..];
    }

    out.push_str(rest);
    out
}

/// 粗略判断一段文本是否像 base64 内容。
fn looks_like_base64(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{debug_log_json, sanitize_debug_log_text};

    #[test]
    fn debug_json_keeps_diagnostic_content_but_redacts_credentials() {
        let payload = json!({
            "prompt": "请更新销售图表",
            "apiKey": "sk-secret-value",
            "nested": {"access_token": "token-value"},
            "tokenUsage": 42,
            "image": "data:image/png;base64,QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo="
        });

        let log = debug_log_json(&payload);

        assert!(log.contains("请更新销售图表"));
        assert!(log.contains("\"tokenUsage\":42"));
        assert!(log.contains("[REDACTED]"));
        assert!(log.contains("[BASE64"));
        assert!(!log.contains("sk-secret-value"));
        assert!(!log.contains("token-value"));
        assert!(!log.contains("QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo"));
    }

    #[test]
    fn debug_text_redacts_key_value_credentials() {
        let log = sanitize_debug_log_text("api_key=secret-value prompt=保留内容\nauthorization: bearer-value");

        assert!(log.contains("api_key=***"));
        assert!(log.contains("authorization:***"));
        assert!(log.contains("prompt=保留内容"));
        assert!(!log.contains("secret-value"));
        assert!(!log.contains("bearer-value"));
    }
}
