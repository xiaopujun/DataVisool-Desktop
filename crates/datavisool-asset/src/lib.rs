/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use base64::Engine;

pub fn parse_data_url(data_url: &str) -> Option<(String, Vec<u8>)> {
    let s = data_url.trim();
    if !s.starts_with("data:") {
        return None;
    }
    let (meta, b64) = s.split_once(',')?;
    if !meta.contains(";base64") {
        return None;
    }
    let mime = meta.strip_prefix("data:")?.split(';').next()?.to_string();
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    Some((mime, bytes))
}

pub fn mime_to_ext(mime: &str, fallback: &str) -> String {
    let m = mime.to_ascii_lowercase();
    if m.contains("font/ttf") || m.contains("application/x-font-ttf") {
        return "ttf".to_string();
    }
    if m.contains("font/otf") || m.contains("application/x-font-opentype") {
        return "otf".to_string();
    }
    if m.contains("font/woff2") {
        return "woff2".to_string();
    }
    if m.contains("font/woff") {
        return "woff".to_string();
    }
    if m.contains("application/vnd.ms-fontobject") {
        return "eot".to_string();
    }
    if m.contains("image/png") {
        return "png".to_string();
    }
    if m.contains("image/jpeg") {
        return "jpg".to_string();
    }
    if m.contains("image/svg") {
        return "svg".to_string();
    }
    if m.contains("image/gif") {
        return "gif".to_string();
    }
    if m.contains("image/bmp") {
        return "bmp".to_string();
    }
    if m.contains("javascript") {
        return "js".to_string();
    }
    fallback.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_data_url_ok() {
        let (mime, bytes) = parse_data_url("data:text/plain;base64,SGVsbG8=").unwrap();
        assert_eq!(mime, "text/plain");
        assert_eq!(bytes, b"Hello");
    }

    #[test]
    fn parse_data_url_invalid_returns_none() {
        assert!(parse_data_url("http://example.com").is_none());
        assert!(parse_data_url("data:text/plain,SGVsbG8=").is_none());
        assert!(parse_data_url("data:text/plain;base64,%%%").is_none());
    }

    #[test]
    fn mime_to_ext_maps_known_and_fallback() {
        assert_eq!(mime_to_ext("image/png", "bin"), "png");
        assert_eq!(mime_to_ext("application/javascript", "bin"), "js");
        assert_eq!(mime_to_ext("application/octet-stream", "bin"), "bin");
    }
}
