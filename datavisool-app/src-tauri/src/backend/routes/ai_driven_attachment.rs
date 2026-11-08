/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI驱动图片附件接口。
//!
//! 负责接收前端上传的图片 data URL，校验图片类型后落盘，并写入临时 Agent 附件表。

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use rand::{distributions::Alphanumeric, Rng};
use serde::Deserialize;
use serde_json::Value;

use crate::app_state::{AppState, AI_DRIVEN_STORAGE_DIR_NAME};
use crate::backend::types::{err, err_with_detail, ok, parse_i64_param, HttpResponse};
use crate::backend::utils::{sanitize_error_message, sanitize_path_segment, sha256_hex, to_asset_url};
use crate::db::DbState;

const DEFAULT_ATTACHMENT_TTL_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// AI驱动图片附件上传请求体。
struct AiDrivenAttachmentUploadReq {
    /// 附件归属的项目 id。
    project_id: String,
    /// AI驱动会话 id，用于限制附件在当前会话内复用。
    session_id: String,
    /// 上传动作 id，用于排查一次上传产生的附件链路。
    upload_request_id: Option<String>,
    /// 原始文件名，用于记录审计信息和辅助识别图片后缀。
    file_name: String,
    /// 图片文件内容，必须是 base64 data URL。
    data_url: String,
    /// 前端识别到的 MIME 类型，data URL 未携带时作为回退值。
    mime: Option<String>,
    /// 可选过期时间戳，单位为毫秒；未传时默认 24 小时后过期。
    expire_at: Option<i64>,
}

#[derive(Debug)]
/// 已通过校验的图片附件文件信息。
struct ValidatedImageAttachment {
    /// 规范化后的 MIME 类型。
    mime: String,
    /// 规范化后的文件后缀。
    ext: String,
    /// 原始文件字节。
    bytes: Vec<u8>,
    /// 能识别时记录图片宽度。
    width: Option<i64>,
    /// 能识别时记录图片高度。
    height: Option<i64>,
}

/// 上传 AI驱动图片附件，并返回前端后续引用该附件所需的信息。
pub(crate) async fn upload(db: &DbState, app_state: &AppState, body: Value) -> Result<HttpResponse, String> {
    let req: AiDrivenAttachmentUploadReq = serde_json::from_value(body).map_err(|e| format!("ai_driven_attachment.upload: invalid body: {e}"))?;

    let project_id = match parse_i64_param(req.project_id.trim(), "projectId") {
        Ok(project_id) => project_id,
        Err(response) => return Ok(response),
    };
    let session_id = match normalize_required_text(&req.session_id, "sessionId") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let upload_request_id = req
        .upload_request_id
        .as_deref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(generate_upload_request_id);
    let original_name = match normalize_required_text(&req.file_name, "fileName") {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };

    let project_exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM project WHERE id = ?1 AND deleted = 0")
        .bind(project_id)
        .fetch_one(&db.pool)
        .await
        .map_err(|e| format!("ai_driven_attachment.upload: query project failed: {e}"))?;
    if project_exists <= 0 {
        return Ok(err(404, "项目不存在或已删除"));
    }

    if let Err(error) = crate::backend::ai_driven::cleanup_expired_agent_attachments(&db.pool, &app_state.resource_storage_root()).await {
        tracing::warn!(%error, "上传前清理过期 Agent 附件失败");
    }

    let image = match validate_image_attachment(&req.data_url, req.mime.as_deref(), &original_name) {
        Ok(image) => image,
        Err(detail) => return Ok(err_with_detail(400, "请求参数错误：图片附件不合法", detail)),
    };

    let now = now_ms();
    let expire_at = attachment_expire_at(req.expire_at, now);
    let attachment_id = generate_attachment_id(now);
    let hash = sha256_hex(&image.bytes);
    let byte_size = image.bytes.len() as i64;

    let storage_dir = attachment_storage_dir(project_id, &session_id);
    let relative_dir = format!("{AI_DRIVEN_STORAGE_DIR_NAME}/{storage_dir}");
    let file_name = format!("{}.{}", attachment_id, image.ext);
    let relative_path = format!("{relative_dir}/{file_name}");
    let absolute_dir = app_state.ai_driven_storage_dir().join(&storage_dir);
    let absolute_path = absolute_dir.join(&file_name);

    std::fs::create_dir_all(&absolute_dir).map_err(|e| format!("ai_driven_attachment.upload: create dir failed: {e}"))?;
    std::fs::write(&absolute_path, &image.bytes).map_err(|e| format!("ai_driven_attachment.upload: write file failed: {e}"))?;

    let insert_result = sqlx::query(
        "INSERT INTO ai_agent_attachment (\
            attachment_id, project_id, upload_session_id, upload_request_id, mime_type, original_name, \
            sha256, relative_path, byte_size, width, height, status, expire_at, \
            last_used_at, created_at, updated_at\
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'temporary', ?12, NULL, ?13, ?14)",
    )
    .bind(&attachment_id)
    .bind(project_id)
    .bind(&session_id)
    .bind(&upload_request_id)
    .bind(&image.mime)
    .bind(&original_name)
    .bind(&hash)
    .bind(&relative_path)
    .bind(byte_size)
    .bind(image.width)
    .bind(image.height)
    .bind(expire_at)
    .bind(now)
    .bind(now)
    .execute(&db.pool)
    .await;

    match insert_result {
        Ok(_) => Ok(ok(serde_json::json!({
            "attachmentId": attachment_id,
            "id": attachment_id,
            "projectId": project_id.to_string(),
            "sessionId": session_id,
            "uploadRequestId": upload_request_id,
            "mimeType": image.mime,
            "originalName": original_name,
            "sha256": hash,
            "relativePath": relative_path,
            "url": to_asset_url(&absolute_path),
            "byteSize": byte_size,
            "width": image.width,
            "height": image.height,
            "status": "temporary",
            "expireAt": expire_at,
            "createdAt": now,
            "updatedAt": now
        }))),
        Err(e) => {
            let _ = std::fs::remove_file(&absolute_path);
            Ok(err_with_detail(500, "保存图片附件失败，请稍后重试", sanitize_error_message(&e.to_string())))
        }
    }
}

/// 构造 AI 统一目录内的附件子目录。
fn attachment_storage_dir(project_id: i64, session_id: &str) -> String {
    format!("attachments/{}/{}", project_id, sanitize_path_segment(session_id, "default"))
}

/// 解析并校验上传的图片附件数据。
fn validate_image_attachment(data_url: &str, fallback_mime: Option<&str>, file_name: &str) -> Result<ValidatedImageAttachment, String> {
    let (raw_mime, bytes) = parse_data_url(data_url, fallback_mime)?;
    if bytes.is_empty() {
        return Err("图片内容不能为空".to_string());
    }

    let ext_from_name = file_extension(file_name);
    let detected = detect_image_format(&bytes);
    let mime_from_data = normalize_image_mime(&raw_mime);
    let ext_from_mime = mime_from_data.as_deref().and_then(image_mime_to_ext);
    let ext = detected
        .as_ref()
        .map(|format| format.ext.to_string())
        .or(ext_from_mime.map(str::to_string))
        .or(ext_from_name.clone())
        .ok_or_else(|| "无法识别图片类型".to_string())?;

    if !is_allowed_image_ext(&ext) {
        return Err("仅支持 bmp/jpg/jpeg/png/gif/svg/webp 图片".to_string());
    }
    if let Some(file_ext) = ext_from_name.as_deref() {
        if !is_allowed_image_ext(file_ext) {
            return Err("文件后缀不是受支持的图片类型".to_string());
        }
    }
    if let Some(format) = detected.as_ref() {
        if let Some(file_ext) = ext_from_name.as_deref() {
            if !image_ext_matches_format(file_ext, format.ext) {
                return Err("图片内容与文件后缀不匹配".to_string());
            }
        }
        if let Some(mime_ext) = ext_from_mime {
            if !image_ext_matches_format(mime_ext, format.ext) {
                return Err("图片内容与 MIME 类型不匹配".to_string());
            }
        }
        if !image_ext_matches_format(&ext, format.ext) {
            return Err("图片内容与文件类型不匹配".to_string());
        }
    } else if !looks_like_svg(&bytes) {
        return Err("图片内容无法通过格式校验".to_string());
    }

    let mime = detected
        .as_ref()
        .map(|format| format.mime.to_string())
        .or(mime_from_data)
        .unwrap_or_else(|| image_ext_to_mime(&ext).to_string());
    let dimensions = read_image_dimensions(&bytes, &ext);

    Ok(ValidatedImageAttachment {
        mime,
        ext: normalize_image_ext(&ext),
        bytes,
        width: dimensions.map(|item| item.0),
        height: dimensions.map(|item| item.1),
    })
}

/// 解析 base64 data URL，返回 MIME 和原始字节。
fn parse_data_url(data_url: &str, fallback_mime: Option<&str>) -> Result<(String, Vec<u8>), String> {
    let s = data_url.trim();
    if !s.starts_with("data:") {
        return Err("not data url".to_string());
    }
    let Some((meta, b64)) = s.split_once(',') else {
        return Err("data url missing comma".to_string());
    };
    if !meta.contains(";base64") {
        return Err("data url missing base64".to_string());
    }
    let mime = meta
        .strip_prefix("data:")
        .and_then(|item| item.split(';').next())
        .filter(|item| !item.trim().is_empty())
        .or(fallback_mime)
        .unwrap_or("application/octet-stream")
        .trim()
        .to_ascii_lowercase();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("base64 decode failed: {e}"))?;
    Ok((mime, bytes))
}

/// 从文件名提取小写扩展名。
fn file_extension(file_name: &str) -> Option<String> {
    Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

struct DetectedImageFormat {
    mime: &'static str,
    ext: &'static str,
}

/// 按文件魔数识别图片类型。
fn detect_image_format(bytes: &[u8]) -> Option<DetectedImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(DetectedImageFormat { mime: "image/png", ext: "png" });
    }
    if bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff {
        return Some(DetectedImageFormat { mime: "image/jpeg", ext: "jpg" });
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(DetectedImageFormat { mime: "image/gif", ext: "gif" });
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(DetectedImageFormat { mime: "image/webp", ext: "webp" });
    }
    if bytes.starts_with(b"BM") {
        return Some(DetectedImageFormat { mime: "image/bmp", ext: "bmp" });
    }
    if looks_like_svg(bytes) {
        return Some(DetectedImageFormat { mime: "image/svg+xml", ext: "svg" });
    }
    None
}

/// 判断字节内容是否看起来是 SVG 图片。
fn looks_like_svg(bytes: &[u8]) -> bool {
    let head_len = bytes.len().min(512);
    let head = String::from_utf8_lossy(&bytes[..head_len]).to_ascii_lowercase();
    let trimmed = head.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && trimmed.contains("<svg"))
}

/// 只接受明确的图片 MIME 类型。
fn normalize_image_mime(mime: &str) -> Option<String> {
    match mime.trim().to_ascii_lowercase().as_str() {
        "image/bmp" | "image/x-ms-bmp" => Some("image/bmp".to_string()),
        "image/jpeg" | "image/jpg" => Some("image/jpeg".to_string()),
        "image/png" => Some("image/png".to_string()),
        "image/gif" => Some("image/gif".to_string()),
        "image/svg+xml" => Some("image/svg+xml".to_string()),
        "image/webp" => Some("image/webp".to_string()),
        _ => None,
    }
}

/// 将图片 MIME 映射为标准文件后缀。
fn image_mime_to_ext(mime: &str) -> Option<&'static str> {
    match mime {
        "image/bmp" => Some("bmp"),
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/gif" => Some("gif"),
        "image/svg+xml" => Some("svg"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// 将图片后缀映射为标准 MIME。
fn image_ext_to_mime(ext: &str) -> &'static str {
    match normalize_image_ext(ext).as_str() {
        "bmp" => "image/bmp",
        "jpg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

/// 将图片后缀归一为保存文件使用的后缀。
fn normalize_image_ext(ext: &str) -> String {
    match ext.trim().to_ascii_lowercase().as_str() {
        "jpeg" => "jpg".to_string(),
        other => other.to_string(),
    }
}

/// 判断后缀是否属于允许的图片类型。
fn is_allowed_image_ext(ext: &str) -> bool {
    matches!(ext.trim().to_ascii_lowercase().as_str(), "bmp" | "jpg" | "jpeg" | "png" | "gif" | "svg" | "webp")
}

/// 判断文件后缀和魔数识别结果是否一致。
fn image_ext_matches_format(ext: &str, detected_ext: &str) -> bool {
    normalize_image_ext(ext) == normalize_image_ext(detected_ext)
}

/// 从常见图片格式中读取宽高，读取失败时返回空。
fn read_image_dimensions(bytes: &[u8], ext: &str) -> Option<(i64, i64)> {
    match normalize_image_ext(ext).as_str() {
        "png" => read_png_dimensions(bytes),
        "jpg" => read_jpeg_dimensions(bytes),
        "gif" => read_gif_dimensions(bytes),
        "bmp" => read_bmp_dimensions(bytes),
        "webp" => read_webp_dimensions(bytes),
        "svg" => read_svg_dimensions(bytes),
        _ => None,
    }
}

/// 解析 SVG 视口尺寸，供后续区域坐标和程序渲染使用。
fn read_svg_dimensions(bytes: &[u8]) -> Option<(i64, i64)> {
    let tree = resvg::usvg::Tree::from_data_nested(bytes, &resvg::usvg::Options::default()).ok()?;
    let width = tree.size().width().round() as i64;
    let height = tree.size().height().round() as i64;
    positive_dimensions(width, height)
}

/// 读取 PNG 图片宽高。
fn read_png_dimensions(bytes: &[u8]) -> Option<(i64, i64)> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?) as i64;
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?) as i64;
    positive_dimensions(width, height)
}

/// 读取 GIF 图片宽高。
fn read_gif_dimensions(bytes: &[u8]) -> Option<(i64, i64)> {
    if bytes.len() < 10 || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return None;
    }
    let width = u16::from_le_bytes(bytes[6..8].try_into().ok()?) as i64;
    let height = u16::from_le_bytes(bytes[8..10].try_into().ok()?) as i64;
    positive_dimensions(width, height)
}

/// 读取 BMP 图片宽高。
fn read_bmp_dimensions(bytes: &[u8]) -> Option<(i64, i64)> {
    if bytes.len() < 26 || !bytes.starts_with(b"BM") {
        return None;
    }
    let width = i32::from_le_bytes(bytes[18..22].try_into().ok()?) as i64;
    let height = i32::from_le_bytes(bytes[22..26].try_into().ok()?) as i64;
    positive_dimensions(width.abs(), height.abs())
}

/// 读取 JPEG 图片宽高。
fn read_jpeg_dimensions(bytes: &[u8]) -> Option<(i64, i64)> {
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return None;
    }
    let mut offset = 2usize;
    while offset + 9 < bytes.len() {
        while offset < bytes.len() && bytes[offset] == 0xff {
            offset += 1;
        }
        if offset >= bytes.len() {
            return None;
        }
        let marker = bytes[offset];
        offset += 1;
        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        if offset + 2 > bytes.len() {
            return None;
        }
        let segment_len = u16::from_be_bytes(bytes[offset..offset + 2].try_into().ok()?) as usize;
        if segment_len < 2 || offset + segment_len > bytes.len() {
            return None;
        }
        if is_jpeg_sof_marker(marker) && segment_len >= 7 {
            let height = u16::from_be_bytes(bytes[offset + 3..offset + 5].try_into().ok()?) as i64;
            let width = u16::from_be_bytes(bytes[offset + 5..offset + 7].try_into().ok()?) as i64;
            return positive_dimensions(width, height);
        }
        offset += segment_len;
    }
    None
}

/// 判断 JPEG marker 是否为带尺寸信息的 SOF 段。
fn is_jpeg_sof_marker(marker: u8) -> bool {
    matches!(marker, 0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf)
}

/// 读取 WebP 图片宽高。
fn read_webp_dimensions(bytes: &[u8]) -> Option<(i64, i64)> {
    if bytes.len() < 30 || !bytes.starts_with(b"RIFF") || &bytes[8..12] != b"WEBP" {
        return None;
    }
    match &bytes[12..16] {
        b"VP8X" if bytes.len() >= 30 => {
            let width = read_webp_24_bit_dimension(&bytes[24..27])? + 1;
            let height = read_webp_24_bit_dimension(&bytes[27..30])? + 1;
            positive_dimensions(width, height)
        }
        b"VP8 " if bytes.len() >= 30 && &bytes[23..26] == b"\x9d\x01\x2a" => {
            let width = (u16::from_le_bytes(bytes[26..28].try_into().ok()?) & 0x3fff) as i64;
            let height = (u16::from_le_bytes(bytes[28..30].try_into().ok()?) & 0x3fff) as i64;
            positive_dimensions(width, height)
        }
        b"VP8L" if bytes.len() >= 25 && bytes[20] == 0x2f => {
            let b0 = bytes[21] as i64;
            let b1 = bytes[22] as i64;
            let b2 = bytes[23] as i64;
            let b3 = bytes[24] as i64;
            let width = 1 + (((b1 & 0x3f) << 8) | b0);
            let height = 1 + (((b3 & 0x0f) << 10) | (b2 << 2) | ((b1 & 0xc0) >> 6));
            positive_dimensions(width, height)
        }
        _ => None,
    }
}

/// 读取 WebP VP8X 的 24-bit 小端尺寸字段。
fn read_webp_24_bit_dimension(bytes: &[u8]) -> Option<i64> {
    if bytes.len() < 3 {
        return None;
    }
    Some(((bytes[2] as i64) << 16) | ((bytes[1] as i64) << 8) | bytes[0] as i64)
}

/// 过滤掉非法宽高，避免把 0 或负数写入附件元数据。
fn positive_dimensions(width: i64, height: i64) -> Option<(i64, i64)> {
    if width > 0 && height > 0 {
        Some((width, height))
    } else {
        None
    }
}

/// 读取当前时间戳，单位为毫秒。
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as i64
}

/// 临时附件最多保留 24 小时；客户端只能主动缩短，不能延长清理期限。
fn attachment_expire_at(requested: Option<i64>, now: i64) -> i64 {
    let deadline = now.saturating_add(DEFAULT_ATTACHMENT_TTL_MS);
    requested
        .filter(|value| *value > now)
        .map_or(deadline, |value| value.min(deadline))
}

/// 生成附件主键。
fn generate_attachment_id(now: i64) -> String {
    format!("aifa_{}_{}", now, random_suffix(10))
}

/// 生成默认上传请求 id。
fn generate_upload_request_id() -> String {
    format!("upload_{}_{}", now_ms(), random_suffix(8))
}

/// 生成短随机后缀，降低同毫秒上传时的文件名冲突概率。
fn random_suffix(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

/// 校验必填字符串并返回去空白后的值。
fn normalize_required_text(value: &str, field: &str) -> Result<String, HttpResponse> {
    let text = value.trim().to_string();
    if text.is_empty() {
        Err(err(400, format!("请求参数错误：缺少 {field}")))
    } else {
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_path_uses_ai_driven_storage_root() {
        let storage_dir = attachment_storage_dir(7, "session-1");
        assert_eq!(format!("{AI_DRIVEN_STORAGE_DIR_NAME}/{storage_dir}"), "ai-driven/attachments/7/session-1");
    }

    #[test]
    fn reads_svg_dimensions_for_region_processing() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1920" height="1080"></svg>"#;
        assert_eq!(read_image_dimensions(svg, "svg"), Some((1920, 1080)));
    }

    #[test]
    fn temporary_attachment_expiration_cannot_exceed_twenty_four_hours() {
        let now = 1_000;
        assert_eq!(attachment_expire_at(None, now), now + DEFAULT_ATTACHMENT_TTL_MS);
        assert_eq!(attachment_expire_at(Some(now + 60_000), now), now + 60_000);
        assert_eq!(attachment_expire_at(Some(i64::MAX), now), now + DEFAULT_ATTACHMENT_TTL_MS);
    }
}
