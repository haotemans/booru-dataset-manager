use super::AiError;
use base64::{engine::general_purpose::STANDARD, Engine};
use bdm_config::{AiRequestSettings, AI_IMAGE_MAX_BYTES};
use image::{ImageDecoder, ImageReader};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

const MIN_JPEG_QUALITY: u8 = 40;

pub(super) struct PreparedImage {
    pub id: i64,
    pub width: u32,
    pub height: u32,
    pub data_url: String,
}

/// Decode one managed image at a time; no temporary images or metadata are uploaded.
pub(super) fn prepare(
    id: i64,
    path: &Path,
    settings: &AiRequestSettings,
    cancel: &AtomicBool,
) -> Result<PreparedImage, AiError> {
    settings.validate().map_err(|_| AiError::Configuration)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(AiError::Cancelled);
    }
    let mut reader = ImageReader::open(path)
        .map_err(|_| AiError::Image)?
        .with_guessed_format()
        .map_err(|_| AiError::Image)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| AiError::Image)?;
    let orientation = decoder.orientation().map_err(|_| AiError::Image)?;
    let mut decoded = image::DynamicImage::from_decoder(decoder).map_err(|_| AiError::Image)?;
    decoded.apply_orientation(orientation);
    if cancel.load(Ordering::Relaxed) {
        return Err(AiError::Cancelled);
    }
    let edge = settings.image_max_edge;
    if decoded.width() > edge || decoded.height() > edge {
        decoded = decoded.resize(edge, edge, image::imageops::FilterType::Lanczos3);
    }
    let rgba = decoded.to_rgba8();
    drop(decoded);
    // Composite transparency onto white before lossy JPEG encoding.
    let rgb = image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y).0;
        let alpha = u32::from(p[3]);
        image::Rgb(
            [0, 1, 2].map(|c| ((u32::from(p[c]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8),
        )
    });
    let (rgb, bytes) = fit_jpeg(rgb, settings.jpeg_quality, cancel)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(AiError::Cancelled);
    }
    Ok(PreparedImage {
        id,
        width: rgb.width(),
        height: rgb.height(),
        data_url: format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes)),
    })
}

fn fit_jpeg(
    mut rgb: image::RgbImage,
    initial_quality: u8,
    cancel: &AtomicBool,
) -> Result<(image::RgbImage, Vec<u8>), AiError> {
    // JPEG size depends on image content, so quality alone cannot guarantee the
    // provider limit. Lower quality first, then shrink while preserving the ratio.
    let mut quality = initial_quality;
    let mut bytes = encode_jpeg(&rgb, quality)?;
    while bytes.len() > AI_IMAGE_MAX_BYTES {
        if cancel.load(Ordering::Relaxed) {
            return Err(AiError::Cancelled);
        }
        if quality > MIN_JPEG_QUALITY {
            quality = quality.saturating_sub(5).max(MIN_JPEG_QUALITY);
            bytes = encode_jpeg(&rgb, quality)?;
            continue;
        }
        let scale = ((AI_IMAGE_MAX_BYTES as f64 / bytes.len() as f64).sqrt() * 0.9).min(0.9);
        let width = ((rgb.width() as f64 * scale).floor() as u32).max(1);
        let height = ((rgb.height() as f64 * scale).floor() as u32).max(1);
        if width >= rgb.width() && height >= rgb.height() {
            return Err(AiError::Image);
        }
        rgb = image::imageops::resize(&rgb, width, height, image::imageops::FilterType::Lanczos3);
        bytes = encode_jpeg(&rgb, quality)?;
    }
    Ok((rgb, bytes))
}

fn encode_jpeg(rgb: &image::RgbImage, quality: u8) -> Result<Vec<u8>, AiError> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|_| AiError::Image)?;
    Ok(bytes)
}
