/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! LCP 项目包格式和压缩标记的兼容识别。
//! TODO(legacy-compat): 停止支持 LCP 项目包后，移除此 crate 及其调用和相关测试。

const LEGACY_PROJECT_FORMAT: &str = "light-chaser-project";
const LEGACY_COMPRESSION_PREFIX: &str = "LCPZ1:gzip:";
pub fn is_legacy_project_format(format: &str) -> bool {
    format == LEGACY_PROJECT_FORMAT
}

pub fn legacy_compressed_gzip_payload(input: &str) -> Option<&str> {
    input.strip_prefix(LEGACY_COMPRESSION_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_current_and_legacy_formats() {
        assert!(is_legacy_project_format(LEGACY_PROJECT_FORMAT));
        assert!(!is_legacy_project_format("unknown"));
        assert_eq!(legacy_compressed_gzip_payload("LCPZ1:gzip:abc"), Some("abc"));
        assert_eq!(legacy_compressed_gzip_payload("DVZ1:gzip:abc"), None);
    }
}
