/* Licensed under the DataVisool License. See the LICENSE file distributed with this software. */
//! Only the exact legacy CLI bootstrap is recognized; arbitrary prefixes are rejected.
use regex::Regex;
use std::sync::OnceLock;

pub(super) fn split(source: &str) -> Result<Option<(String, &str)>, String> {
    if !source.starts_with("var __LCP_REMOTE_ENTRY_FILE__=") {
        return Ok(None);
    }
    let prefix_end = source
        .find("})();\n")
        .map(|i| i + "})();\n".len())
        .ok_or("不支持的组件资源加载格式")?;
    let prefix = &source[..prefix_end];
    static ENTRY: OnceLock<Regex> = OnceLock::new();
    static CSS: OnceLock<Regex> = OnceLock::new();
    let entry_re = ENTRY.get_or_init(|| Regex::new(r#"^var __LCP_REMOTE_ENTRY_FILE__=("(?:\\.|[^"\\])*");\n"#).unwrap());
    let css_re = CSS.get_or_init(|| Regex::new(r#"return;(\[(?:"(?:\\.|[^"\\])*"(?:,"(?:\\.|[^"\\])*")*)?\])\.forEach"#).unwrap());
    let entry = entry_re
        .captures(prefix)
        .and_then(|c| c.get(1))
        .ok_or("不支持的组件资源加载格式")?;
    let css = css_re
        .captures(prefix)
        .and_then(|c| c.get(1))
        .ok_or("不支持的组件资源加载格式")?;
    let _: String = serde_json::from_str(entry.as_str()).map_err(|_| "不支持的组件资源入口")?;
    let _: Vec<String> = serde_json::from_str(css.as_str()).map_err(|_| "不支持的组件样式列表")?;
    let expected = include_str!("legacy-cli-bootstrap.template.js")
        .replace("%%ENTRY%%", entry.as_str())
        .replace("%%CSS%%", css.as_str());
    if prefix != expected {
        return Err("不支持的组件资源加载格式".into());
    }
    let converted = prefix
        .replace("globalThis.__LCP_REMOTE_COMPONENT_BASE_URL_MAP__", "globalThis.__DATAVISOOL_REMOTE_COMPONENT_BASE_URL_MAP__")
        .replace("data-lcp-remote-style", "data-datavisool-remote-style");
    Ok(Some((converted, &source[prefix_end..])))
}
