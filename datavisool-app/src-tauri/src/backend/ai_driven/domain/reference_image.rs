/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 参考图确定性前置处理。

use std::io::Cursor;

use image::{imageops, DynamicImage, GenericImageView, ImageFormat, Pixel, Rgba, RgbaImage};

const RULER_MARGIN: u32 = 84;
const CONTRAST_AMOUNT: f32 = 18.0;
const RULER_LABEL_SCALE: u32 = 3;

#[derive(Clone, Debug)]
pub(crate) struct PreparedReferenceImage {
    pub(crate) attachment_id: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) source_png: Vec<u8>,
    pub(crate) coordinate_png: Vec<u8>,
    pub(crate) contrast_png: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReferenceImageProcessingError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl ReferenceImageProcessingError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl std::fmt::Display for ReferenceImageProcessingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

/// 解码真实附件、核对记录尺寸，并生成像素卡尺和对比度证据图。
pub(crate) fn prepare_reference_image(
    attachment_id: impl Into<String>,
    mime_type: &str,
    bytes: &[u8],
    stored_width: Option<u32>,
    stored_height: Option<u32>,
) -> Result<PreparedReferenceImage, ReferenceImageProcessingError> {
    let source = decode_source_image(bytes, source_is_svg(mime_type, bytes))?;
    let (width, height) = source.dimensions();
    if stored_width.is_some_and(|stored| stored != width) || stored_height.is_some_and(|stored| stored != height) {
        return Err(ReferenceImageProcessingError::new(
            "agent.reference_image.dimension_mismatch",
            format!("附件记录尺寸与图片内容不一致：记录 {stored_width:?}×{stored_height:?}，实际 {width}×{height}"),
        ));
    }
    Ok(PreparedReferenceImage {
        attachment_id: attachment_id.into(),
        width,
        height,
        source_png: encode_png(source.to_rgba8())?,
        coordinate_png: encode_png(create_coordinate_ruler_canvas(&source))?,
        contrast_png: encode_png(source.adjust_contrast(CONTRAST_AMOUNT).to_rgba8())?,
    })
}

fn source_is_svg(mime_type: &str, bytes: &[u8]) -> bool {
    if mime_type.eq_ignore_ascii_case("image/svg+xml") {
        return true;
    }
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    (text.starts_with("<svg") || text.starts_with("<?xml")) && text.contains("<svg")
}

fn decode_source_image(bytes: &[u8], is_svg: bool) -> Result<DynamicImage, ReferenceImageProcessingError> {
    if !is_svg {
        return image::load_from_memory(bytes).map_err(|error| image_error(format!("参考图解码失败：{error}")));
    }
    let tree = resvg::usvg::Tree::from_data_nested(bytes, &resvg::usvg::Options::default())
        .map_err(|error| image_error(format!("参考图 SVG 解析失败：{error}")))?;
    let width = tree.size().width().round().max(1.0) as u32;
    let height = tree.size().height().round().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or_else(|| image_error("参考图 SVG 尺寸过大，无法建立渲染画布"))?;
    let transform = resvg::tiny_skia::Transform::from_scale(width as f32 / tree.size().width(), height as f32 / tree.size().height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let png = pixmap
        .encode_png()
        .map_err(|error| image_error(format!("参考图 SVG 栅格化失败：{error}")))?;
    image::load_from_memory(&png).map_err(|error| image_error(format!("参考图 SVG 栅格图解码失败：{error}")))
}

fn create_coordinate_ruler_canvas(source: &DynamicImage) -> RgbaImage {
    let source = source.to_rgba8();
    let width = source.width();
    let height = source.height();
    let mut image = RgbaImage::from_pixel(width.saturating_add(RULER_MARGIN * 2), height.saturating_add(RULER_MARGIN * 2), Rgba([3, 10, 16, 255]));
    imageops::overlay(&mut image, &source, i64::from(RULER_MARGIN), i64::from(RULER_MARGIN));
    draw_external_ruler(&mut image, width, height);
    image
}

fn draw_external_ruler(image: &mut RgbaImage, source_width: u32, source_height: u32) {
    let x_color = Rgba([83, 255, 242, 255]);
    let y_color = Rgba([255, 205, 92, 255]);
    draw_rect_outline(
        image,
        RULER_MARGIN.saturating_sub(1),
        RULER_MARGIN.saturating_sub(1),
        RULER_MARGIN + source_width + 1,
        RULER_MARGIN + source_height + 1,
        2,
        Rgba([129, 159, 171, 255]),
    );
    for value in (0..=source_width).step_by(10) {
        let x = RULER_MARGIN + value.min(source_width);
        let tick = tick_length(value);
        draw_vertical_segment(image, x, RULER_MARGIN - tick, RULER_MARGIN - 1, tick_thickness(value), x_color);
        draw_vertical_segment(
            image,
            x,
            RULER_MARGIN + source_height + 1,
            RULER_MARGIN + source_height + tick,
            tick_thickness(value),
            x_color,
        );
        if value % 100 == 0 {
            draw_label(image, x, 8, &value.to_string(), x_color, true);
            draw_label(image, x, RULER_MARGIN + source_height + 26, &value.to_string(), x_color, true);
        }
    }
    for value in (0..=source_height).step_by(10) {
        let y = RULER_MARGIN + value.min(source_height);
        let tick = tick_length(value);
        draw_horizontal_segment(image, y, RULER_MARGIN - tick, RULER_MARGIN - 1, tick_thickness(value), y_color);
        draw_horizontal_segment(image, y, RULER_MARGIN + source_width + 1, RULER_MARGIN + source_width + tick, tick_thickness(value), y_color);
        if value % 100 == 0 {
            draw_label(image, 5, y, &value.to_string(), y_color, false);
            draw_label(image, RULER_MARGIN + source_width + 25, y, &value.to_string(), y_color, false);
        }
    }
}

fn tick_length(value: u32) -> u32 {
    if value % 100 == 0 {
        20
    } else if value % 50 == 0 {
        13
    } else {
        7
    }
}

fn tick_thickness(value: u32) -> u32 {
    if value % 100 == 0 {
        3
    } else {
        2
    }
}

fn draw_label(image: &mut RgbaImage, anchor_x: u32, anchor_y: u32, text: &str, color: Rgba<u8>, center_x: bool) {
    let width = text_pixel_width(text);
    let height = 5 * RULER_LABEL_SCALE;
    let x = if center_x {
        anchor_x.saturating_sub(width / 2).min(image.width().saturating_sub(width))
    } else {
        anchor_x.min(image.width().saturating_sub(width))
    };
    let y = anchor_y
        .saturating_sub(if center_x { 0 } else { height / 2 })
        .min(image.height().saturating_sub(height));
    fill_rect(image, x.saturating_sub(2), y.saturating_sub(2), x + width + 2, y + height + 2, Rgba([3, 10, 14, 225]));
    draw_text(image, x, y, text, color);
}

fn draw_vertical_segment(image: &mut RgbaImage, x: u32, top: u32, bottom: u32, thickness: u32, color: Rgba<u8>) {
    let start = x.saturating_sub(thickness / 2);
    for current in start..start.saturating_add(thickness) {
        for y in top.min(image.height().saturating_sub(1))..=bottom.min(image.height().saturating_sub(1)) {
            blend_pixel(image, current, y, color);
        }
    }
}

fn draw_horizontal_segment(image: &mut RgbaImage, y: u32, left: u32, right: u32, thickness: u32, color: Rgba<u8>) {
    let start = y.saturating_sub(thickness / 2);
    for current in start..start.saturating_add(thickness) {
        for x in left.min(image.width().saturating_sub(1))..=right.min(image.width().saturating_sub(1)) {
            blend_pixel(image, x, current, color);
        }
    }
}

fn draw_rect_outline(image: &mut RgbaImage, left: u32, top: u32, right: u32, bottom: u32, thickness: u32, color: Rgba<u8>) {
    for offset in 0..thickness {
        let x1 = (left + offset).min(image.width().saturating_sub(1));
        let y1 = (top + offset).min(image.height().saturating_sub(1));
        let x2 = right.saturating_sub(1 + offset).min(image.width().saturating_sub(1));
        let y2 = bottom.saturating_sub(1 + offset).min(image.height().saturating_sub(1));
        if x1 > x2 || y1 > y2 {
            break;
        }
        for x in x1..=x2 {
            blend_pixel(image, x, y1, color);
            blend_pixel(image, x, y2, color);
        }
        for y in y1..=y2 {
            blend_pixel(image, x1, y, color);
            blend_pixel(image, x2, y, color);
        }
    }
}

fn fill_rect(image: &mut RgbaImage, left: u32, top: u32, right: u32, bottom: u32, color: Rgba<u8>) {
    for y in top..bottom.min(image.height()) {
        for x in left..right.min(image.width()) {
            blend_pixel(image, x, y, color);
        }
    }
}

fn draw_text(image: &mut RgbaImage, x: u32, y: u32, text: &str, color: Rgba<u8>) {
    let mut cursor = x;
    for character in text.chars() {
        for (row_index, row) in glyph(character).iter().enumerate() {
            for column in 0..3 {
                if row & (1 << (2 - column)) != 0 {
                    fill_rect(
                        image,
                        cursor + column * RULER_LABEL_SCALE,
                        y + row_index as u32 * RULER_LABEL_SCALE,
                        cursor + (column + 1) * RULER_LABEL_SCALE,
                        y + (row_index as u32 + 1) * RULER_LABEL_SCALE,
                        color,
                    );
                }
            }
        }
        cursor = cursor.saturating_add(4 * RULER_LABEL_SCALE);
    }
}

fn text_pixel_width(text: &str) -> u32 {
    text.chars().count().saturating_mul(4).saturating_sub(1) as u32 * RULER_LABEL_SCALE
}

fn glyph(character: char) -> [u8; 5] {
    match character {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        _ => [0; 5],
    }
}

fn blend_pixel(image: &mut RgbaImage, x: u32, y: u32, color: Rgba<u8>) {
    if x < image.width() && y < image.height() {
        image.get_pixel_mut(x, y).blend(&color);
    }
}

fn encode_png(image: RgbaImage) -> Result<Vec<u8>, ReferenceImageProcessingError> {
    let mut cursor = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|error| image_error(format!("参考图辅助 PNG 编码失败：{error}")))?;
    Ok(cursor.into_inner())
}

fn image_error(message: impl Into<String>) -> ReferenceImageProcessingError {
    ReferenceImageProcessingError::new("agent.reference_image.preprocessing_failed", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_trusted_dimensions_and_deterministic_evidence_images() {
        let mut source = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(120, 80, Rgba([80, 110, 140, 255])))
            .write_to(&mut source, ImageFormat::Png)
            .unwrap();
        let prepared = prepare_reference_image("source-1", "image/png", source.get_ref(), Some(120), Some(80)).unwrap();
        assert_eq!((prepared.width, prepared.height), (120, 80));
        assert_eq!(image::load_from_memory(&prepared.coordinate_png).unwrap().dimensions(), (288, 248));
        assert_ne!(prepared.source_png, prepared.contrast_png);
    }

    #[test]
    fn rejects_stored_dimension_mismatch() {
        let mut source = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::new(20, 10))
            .write_to(&mut source, ImageFormat::Png)
            .unwrap();
        let error = prepare_reference_image("source-1", "image/png", source.get_ref(), Some(21), Some(10)).unwrap_err();
        assert_eq!(error.code, "agent.reference_image.dimension_mismatch");
    }
}
