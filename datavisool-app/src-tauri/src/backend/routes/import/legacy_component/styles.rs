/* Licensed under the DataVisool License. See the LICENSE file distributed with this software. */
//! Exact legacy UI token conversion. This parses stylesheet text without running it.
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

fn mappings() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        include_str!("legacy-ui-style-map.properties")
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.trim(), value.trim()))
            .collect()
    })
}

pub(super) fn map_token(name: &str) -> Option<&'static str> {
    mappings().get(name).copied()
}
pub(super) fn map_class_names(value: &str) -> String {
    static WORD: OnceLock<Regex> = OnceLock::new();
    let re = WORD.get_or_init(|| Regex::new(r"\S+").unwrap());
    let mut out = String::new();
    let mut last = 0;
    for token in re.find_iter(value) {
        out.push_str(&value[last..token.start()]);
        out.push_str(map_token(token.as_str()).unwrap_or(token.as_str()));
        last = token.end()
    }
    out.push_str(&value[last..]);
    out
}
pub(super) fn contains_known_class(value: &str) -> bool {
    static WORD: OnceLock<Regex> = OnceLock::new();
    WORD.get_or_init(|| Regex::new(r"[a-zA-Z_][a-zA-Z0-9_-]*").unwrap())
        .find_iter(value)
        .any(|m| map_token(m.as_str()).is_some())
}

fn is_name(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c >= 128
}
fn end_name(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len() && is_name(bytes[at]) {
        at += 1
    }
    at
}
fn is_hex(c: u8) -> bool { c.is_ascii_hexdigit() }
fn css_escape_end(bytes: &[u8], slash: usize) -> Result<usize, String> {
    let mut i = slash + 1;
    let Some(first) = bytes.get(i).copied() else { return Err(invalid("CSS 转义不完整")) };
    if matches!(first, b'\n' | b'\r' | b'\x0c') { return Err(invalid("CSS 转义中的换行续接")) }
    if is_hex(first) {
        let mut digits = 0;
        while i < bytes.len() && digits < 6 && is_hex(bytes[i]) { i += 1; digits += 1 }
        if i < bytes.len() && bytes[i].is_ascii_whitespace() {
            if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') { i += 1 }
            i += 1;
        }
        return Ok(i);
    }
    let ch = std::str::from_utf8(&bytes[i..]).map_err(|_| invalid("CSS 转义字符不是 UTF-8"))?.chars().next().unwrap();
    Ok(i + ch.len_utf8())
}
fn decode_css_escapes(text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            let ch = text[i..].chars().next().unwrap(); out.push(ch); i += ch.len_utf8(); continue;
        }
        let end = css_escape_end(bytes, i)?;
        if is_hex(bytes[i + 1]) {
            let mut digit_end = i + 1;
            while digit_end < end && is_hex(bytes[digit_end]) && digit_end - (i + 1) < 6 { digit_end += 1 }
            let value = u32::from_str_radix(&text[i + 1..digit_end], 16).map_err(|_| invalid("CSS 十六进制转义无效"))?;
            out.push(char::from_u32(value).filter(|c| *c != '\0' && !matches!(*c as u32, 0xD800..=0xDFFF)).unwrap_or('\u{fffd}'));
        } else {
            out.push_str(&text[i + 1..end]);
        }
        i = end;
    }
    Ok(out)
}
fn css_name_end(bytes: &[u8], mut at: usize) -> Result<usize, String> {
    while at < bytes.len() {
        if is_name(bytes[at]) { at += 1; continue }
        if bytes[at] == b'\\' { at = css_escape_end(bytes, at)?; continue }
        break;
    }
    Ok(at)
}
fn has_prefix(text: &str) -> bool {
    let v = text.to_ascii_lowercase();
    v.contains("lcp-") || v.contains("lc-dialog")
}
pub(super) fn contains_legacy_style_prefix(value: &str) -> Result<bool, String> {
    Ok(has_prefix(&decode_css_escapes(value)?))
}
/// Narrow CSS-token prefilter. Ordinary JavaScript escapes alone must not send
/// modern scripts through the legacy parser/converter path.
pub(super) fn contains_legacy_style_candidate(source: &str) -> bool {
    let normalized = normalize_search_escapes(source);
    let bytes = normalized.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let start = if bytes[i] == b'.' && bytes.get(i + 1).is_some_and(|byte| is_name(*byte) || *byte == b'\\') {
            Some(i + 1)
        } else if bytes[i..].starts_with(b"--") && bytes.get(i + 2).is_some_and(|byte| is_name(*byte) || *byte == b'\\') {
            Some(i)
        } else {
            None
        };
        if let Some(start) = start {
            if let Ok(end) = css_name_end(bytes, start) {
                let raw = &normalized[start..end];
                if let Ok(decoded) = decode_css_escapes(raw) {
                    let token = decoded.to_ascii_lowercase();
                    if token.starts_with("lcp-") || token.starts_with("lc-dialog") || token.starts_with("--lcp-")
                        || map_token(&decoded).is_some()
                    { return true; }
                }
                i = i.max(end.saturating_sub(1));
            }
        }
        i += 1;
    }
    false
}
fn normalize_search_escapes(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = String::with_capacity(source.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            let ch = source[i..].chars().next().unwrap();
            output.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i] == b'\\' { i += 1 }
        for _ in 0..(i - start).div_ceil(2) { output.push('\\') }
    }
    output
}
fn invalid(reason: &str) -> String {
    format!("旧 UI 样式无法安全转换：{reason}")
}
fn quoted_end(text: &[u8], start: usize) -> Result<usize, String> {
    let quote = text[start];
    let mut i = start + 1;
    while i < text.len() {
        if text[i] == b'\\' {
            i += 2;
            continue;
        }
        if text[i] == quote {
            return Ok(i + 1);
        }
        i += 1;
    }
    Err(invalid("字符串未结束"))
}
fn comment_end(text: &str, start: usize) -> Result<usize, String> {
    text[start + 2..]
        .find("*/")
        .map(|v| start + 2 + v + 2)
        .ok_or_else(|| invalid("注释未结束"))
}
fn attribute_end(text: &str, start: usize) -> Result<usize, String> {
    let bytes = text.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => {
                let end = quoted_end(bytes, i)?;
                i = end;
            }
            b'\\' => i = css_escape_end(bytes, i)?,
            b']' => return Ok(i + 1),
            _ => i += 1,
        }
    }
    Err(invalid("属性选择器未结束"))
}
fn exact_class_attribute(selector: &str) -> Option<String> {
    static CLASS: OnceLock<Regex> = OnceLock::new();
    let re = CLASS
        .get_or_init(|| Regex::new(r#"(?is)^\[\s*class\s*(?:=|~=)\s*(?:\"((?:\\.|[^\\])*)\"|'((?:\\.|[^\\])*)'|((?:\\[0-9a-fA-F]{1,6}[\t\n\r\x0c ]?|\\[^\r\n\x0c]|[^\s\]\"'])+))\s*[s]?\s*\]$"#).unwrap());
    let capture = re.captures(selector)?;
    let value = (1..=3).find_map(|i| capture.get(i))?;
    let mut mapped = String::new();
    let raw = value.as_str();
    let bytes = raw.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos].is_ascii_whitespace() { mapped.push(bytes[pos] as char); pos += 1; continue }
        let start = pos;
        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
            pos = if bytes[pos] == b'\\' { css_escape_end(bytes, pos).ok()? } else { pos + raw[pos..].chars().next()?.len_utf8() };
        }
        let token = &raw[start..pos];
        let decoded = decode_css_escapes(token).ok()?;
        let replacement = map_token(&decoded);
        if token.contains('\\') && replacement.is_none() && has_prefix(&decoded) { return None }
        mapped.push_str(replacement.unwrap_or(token));
    }
    Some(format!("{}{}{}", &selector[..value.start()], mapped, &selector[value.end()..]))
}
fn skip_forward(text: &str, mut i: usize) -> Option<usize> {
    let b = text.as_bytes();
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b[i..].starts_with(b"/*") {
            i = comment_end(text, i).ok()?;
            continue;
        }
        break;
    }
    Some(i)
}
fn skip_backward(text: &str, mut i: usize) -> usize {
    let b = text.as_bytes();
    loop {
        while i > 0 && b[i - 1].is_ascii_whitespace() {
            i -= 1
        }
        if i >= 2 && &b[i - 2..i] == b"*/" {
            if let Some(pos) = text[..i - 2].rfind("/*") {
                i = pos;
                continue;
            }
        }
        return i;
    }
}
fn follows_function(text: &str, index: usize, func: &str) -> bool {
    let b = text.as_bytes();
    let mut end = skip_backward(text, index);
    if end == 0 || b[end - 1] != b'(' {
        return false;
    }
    end = skip_backward(text, end - 1);
    if end < func.len() {
        return false;
    }
    let start = end - func.len();
    b[start..end].eq_ignore_ascii_case(func.as_bytes()) && (start == 0 || !is_name(b[start - 1]))
}
fn follows_property_rule(text: &str, index: usize) -> bool {
    let b = text.as_bytes();
    let end = skip_backward(text, index);
    let pat = b"@property";
    end >= pat.len() && b[end - pat.len()..end].eq_ignore_ascii_case(pat) && (end == pat.len() || !is_name(b[end - pat.len() - 1]))
}
fn follows_container_query(text: &str, index: usize) -> bool {
    if !follows_function(text, index, "style") {
        return false;
    }
    let start = text[..index].rfind(['{', '}', ';']).map_or(0, |i| i + 1);
    let Some(start) = skip_forward(text, start) else { return false };
    text.as_bytes()
        .get(start..start + 10)
        .is_some_and(|v| v.eq_ignore_ascii_case(b"@container"))
}
fn is_declaration(text: &str, start: usize) -> Result<bool, String> {
    let b = text.as_bytes();
    let (mut i, mut parens, mut brackets) = (start, 0usize, 0usize);
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = comment_end(text, i)?;
                continue;
            }
            b'\'' | b'"' => {
                i = quoted_end(b, i)?;
                continue;
            }
            b'(' => parens += 1,
            b')' => parens = parens.saturating_sub(1),
            b'[' => brackets += 1,
            b']' => brackets = brackets.saturating_sub(1),
            b'{' if parens == 0 && brackets == 0 => return Ok(false),
            b';' | b'}' if parens == 0 && brackets == 0 => return Ok(true),
            _ => {}
        }
        i += 1;
    }
    Ok(true)
}

pub(super) fn convert(css: &str) -> Result<String, String> {
    let b = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let (mut i, mut bracket_depth, mut value_parens) = (0usize, 0usize, 0usize);
    let (mut value, mut custom, mut statement) = (false, false, true);
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = comment_end(css, i)?;
            out.push_str(&css[i..end]);
            i = end;
            continue;
        }
        if c == b'\'' || c == b'"' {
            let end = quoted_end(b, i)?;
            let v = &css[i..end];
            if bracket_depth > 0 && has_prefix(v) {
                return Err(invalid("属性选择器中的旧 UI 名称"));
            }
            out.push_str(v);
            i = end;
            continue;
        }
        if b.get(i..i + 4).is_some_and(|v| v.eq_ignore_ascii_case(b"url(")) && (i == 0 || !is_name(b[i - 1])) {
            let start = i;
            i += 4;
            let mut closed = false;
            while i < b.len() {
                match b[i] {
                    b'\\' => i += 2,
                    b'\'' | b'"' => i = quoted_end(b, i)?,
                    b')' => {
                        i += 1;
                        closed = true;
                        break;
                    }
                    _ => i += 1,
                }
            }
            if !closed {
                return Err(invalid("URL 未结束"));
            }
            out.push_str(&css[start..i]);
            statement = false;
            continue;
        }
        if !value && c == b'[' && bracket_depth == 0 {
            let end = attribute_end(css, i)?;
            let selector = &css[i..end];
            if let Some(mapped) = exact_class_attribute(selector) {
                out.push_str(&mapped);
                i = end;
                statement = false;
                continue;
            }
            if has_prefix(&decode_css_escapes(selector)?) {
                return Err(invalid("属性选择器中的旧 UI 名称"));
            }
        }
        if custom && c == b'{' {
            return Err(invalid("自定义属性中的嵌套块"));
        }
        if value && c == b'(' {
            value_parens += 1
        }
        if value && c == b')' {
            value_parens = value_parens.saturating_sub(1)
        }
        if value && value_parens == 0 && (c == b';' || c == b'}') {
            value = false;
            custom = false
        }
        if !value && matches!(c, b'{' | b'}' | b';') {
            statement = true
        }
        if c == b'[' {
            bracket_depth += 1
        }
        if c == b']' {
            bracket_depth = bracket_depth.saturating_sub(1)
        }
        if c == b'.' && b.get(i + 1).is_some_and(|v| is_name(*v) || *v == b'\\') {
            let end = css_name_end(b, i + 1)?;
            let raw_name = &css[i + 1..end];
            let decoded = decode_css_escapes(raw_name)?;
            let mapped = map_token(&decoded);
            if raw_name.contains('\\') && mapped.is_none() && has_prefix(&decoded) {
                return Err(invalid("转义后的旧 UI 类名没有精确映射"));
            }
            if mapped.is_some() && bracket_depth > 0 {
                return Err(invalid("属性选择器中的旧 UI 名称"));
            }
            out.push('.');
            out.push_str(if value { raw_name } else { mapped.unwrap_or(raw_name) });
            i = end;
            statement = false;
            continue;
        }
        if c == b'-' && b.get(i + 1) == Some(&b'-') {
            let end = css_name_end(b, i)?;
            let raw_name = &css[i..end];
            let decoded = decode_css_escapes(raw_name)?;
            let mapped = map_token(&decoded);
            if raw_name.contains('\\') && mapped.is_none() && has_prefix(&decoded) {
                return Err(invalid("转义后的旧 UI 变量没有精确映射"));
            }
            let name = &decoded;
            if bracket_depth > 0 && has_prefix(name) {
                return Err(invalid("属性选择器中的旧 UI 名称"));
            }
            let declaration = !value && statement && skip_forward(css, end).is_some_and(|p| b.get(p) == Some(&b':'));
            let replace = follows_function(css, i, "var")
                || (!value && (declaration || follows_property_rule(css, i) || follows_container_query(css, i)));
            out.push_str(if replace { mapped.unwrap_or(raw_name) } else { raw_name });
            if declaration {
                value = true;
                custom = true;
                value_parens = 0
            }
            i = end;
            statement = false;
            continue;
        }
        if bracket_depth > 0 && is_name(c) {
            let end = end_name(b, i);
            let name = &css[i..end];
            if has_prefix(name) {
                return Err(invalid("属性选择器中的旧 UI 名称"));
            }
            out.push_str(name);
            i = end;
            continue;
        }
        if statement && is_name(c) {
            let end = end_name(b, i);
            if skip_forward(css, end).is_some_and(|p| b.get(p) == Some(&b':')) && is_declaration(css, end)? {
                value = true;
                value_parens = 0
            }
            out.push_str(&css[i..end]);
            i = end;
            statement = false;
            continue;
        }
        if c == b'\\' {
            let end = css_escape_end(b, i)?;
            out.push_str(&css[i..end]);
            i = end;
            continue;
        }
        let ch = css[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
        if !ch.is_whitespace() && !matches!(ch, '{' | '}' | ';') {
            statement = false
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::convert;
    #[test]
    fn converts_confirmed_selectors_and_variables_without_touching_content_or_urls() {
        let css = r#"/* .lcp-button */ .lcp-button, [class~="lcp-button"] { color: var(--lcp-grid-columns); background:url('/lcp-button.svg'); content:"lcp-button" }"#;
        let result = convert(css).unwrap();
        assert!(result.contains(".datavisool-button"));
        assert!(result.contains("[class~=\"datavisool-button\"]"));
        assert!(result.contains("var(--datavisool-grid-columns)"));
        assert!(result.contains("/* .lcp-button */"));
        assert!(result.contains("url('/lcp-button.svg')"));
        assert!(result.contains("content:\"lcp-button\""));
    }
    #[test]
    fn preserves_data_values_and_converts_nested_selectors() {
        let source = "button:hover .lcp-button {content: var(--missing, .lcp-button);font-family: .lcp-button; animation-name: --lcp-ui-primary:custom;}@media (min-width: 1px) {a:focus .lcp-button {color:var(--lcp-ui-primary)}}";
        let expected = "button:hover .datavisool-button {content: var(--missing, .lcp-button);font-family: .lcp-button; animation-name: --lcp-ui-primary:custom;}@media (min-width: 1px) {a:focus .datavisool-button {color:var(--datavisool-ui-primary)}}";
        assert_eq!(convert(source).unwrap(), expected);
    }
    #[test]
    fn converts_container_queries_and_preserves_custom_property_data() {
        let source = "/* note */ @container panel style(/* note */--lcp-ui-primary: red) {.lcp-button {color: var(--lcp-ui-primary); content: style(--lcp-ui-primary: red)}}";
        let expected = "/* note */ @container panel style(/* note */--datavisool-ui-primary: red) {.datavisool-button {color: var(--datavisool-ui-primary); content: style(--lcp-ui-primary: red)}}";
        assert_eq!(convert(source).unwrap(), expected);
        let source = "@property --lcp-ui-primary {syntax:'<color>';inherits:true;}.lcp-button {--user-data: .lcp-button --lcp-ui-primary var(--lcp-ui-primary);animation-name: --lcp-ui-primary;}";
        let expected = "@property --datavisool-ui-primary {syntax:'<color>';inherits:true;}.datavisool-button {--user-data: .lcp-button --lcp-ui-primary var(--datavisool-ui-primary);animation-name: --lcp-ui-primary;}";
        assert_eq!(convert(source).unwrap(), expected);
    }
    #[test]
    fn preserves_comments_and_custom_class_names() {
        let source = ".lcp-button { --lcp-ui-primary/**/: red; color: var(/* comment */--lcp-ui-primary); }";
        let expected = ".datavisool-button { --datavisool-ui-primary/**/: red; color: var(/* comment */--datavisool-ui-primary); }";
        assert_eq!(convert(source).unwrap(), expected);
        let source = r#"[class~='lcp-button'],[class~=lc-dialog],[class = "custom lcp-button  lcp-custom" s] {color:red}"#;
        let expected =
            r#"[class~='datavisool-button'],[class~=datavisool-dialog],[class = "custom datavisool-button  lcp-custom" s] {color:red}"#;
        assert_eq!(convert(source).unwrap(), expected);
    }
    #[test]
    fn converts_escaped_class_selectors_and_rejects_ambiguous_attribute_selectors() {
        for selector in [
            "[class*=lcp-button] { color:red }",
            "[class^=lcp-button] { color:red }",
            "[class$=lcp-button] { color:red }",
            "[class|=lcp-button] { color:red }",
        ] {
            assert!(convert(selector).is_err(), "accepted ambiguous selector: {selector}");
        }
        assert_eq!(convert(r".\6c cp-button {color:red}").unwrap(), ".datavisool-button {color:red}");
        assert_eq!(convert(r#"[class~="lcp-\62 utton custom"] {color:red}"#).unwrap(), r#"[class~="datavisool-button custom"] {color:red}"#);
        assert_eq!(convert(r".lc\2d dialog {content:\31 23}").unwrap(), r".datavisool-dialog {content:\31 23}");
        assert!(convert(r"[class*='\6c cp-button'] {color:red}").is_err());
        assert!(convert(r"[class*='lcp-\22 button'] {color:red}").is_err());
        assert!(convert(r#"[class~="lcp-button\20 custom"] {color:red}"#).is_err());
        assert!(convert(r"[class~=lcp-\75 nknown] {color:red}").is_err());
        assert_eq!(convert(r".custom\:hover {color:red}").unwrap(), r".custom\:hover {color:red}");
        assert!(convert(".lcp-\\\nbutton {color:red}").is_err());
    }

    #[test]
    fn converts_scrollbar_variable_and_code_mirror_theme_tokens() {
        assert_eq!(
            convert(".cm-s-lcp-dark { --lcp-scrollbar-thumb: red; color: var(--lcp-scrollbar-thumb) }").unwrap(),
            ".cm-s-datavisool-dark { --datavisool-scrollbar-thumb: red; color: var(--datavisool-scrollbar-thumb) }"
        );
    }
}
