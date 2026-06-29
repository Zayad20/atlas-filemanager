#![allow(clippy::disallowed_methods)]
// reason: image ops run on blocking workers, sync fs is intentional.

use std::path::Path;

use atlas_model::preview_cache::{ImageData, PreviewResult};

use super::{MAX_IMAGE_PREVIEW_EDGE, MAX_MULTI_THUMBNAIL_EDGE};

pub(super) fn preview_image(path: &Path) -> PreviewResult {
    let img = match image::open(path) {
        Ok(i) => i,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };
    let original_width = img.width() as usize;
    let original_height = img.height() as usize;
    let img = img.thumbnail(MAX_IMAGE_PREVIEW_EDGE, MAX_IMAGE_PREVIEW_EDGE);
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    let exif = read_exif(path);
    PreviewResult::Image {
        rgba: rgba.into_raw(),
        width: width as usize,
        height: height as usize,
        original_width,
        original_height,
        exif,
    }
}

pub(super) fn preview_thumbnail(path: &Path) -> Result<(Vec<u8>, usize, usize), image::ImageError> {
    let img = image::open(path)?.thumbnail(MAX_MULTI_THUMBNAIL_EDGE, MAX_MULTI_THUMBNAIL_EDGE);
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok((rgba.into_raw(), width as usize, height as usize))
}

pub(super) fn decode_cover(data: &[u8]) -> Option<ImageData> {
    let img = image::load_from_memory(data)
        .ok()?
        .thumbnail(MAX_MULTI_THUMBNAIL_EDGE, MAX_MULTI_THUMBNAIL_EDGE);
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(ImageData {
        rgba: rgba.into_raw(),
        width: width as usize,
        height: height as usize,
    })
}

fn read_exif(path: &Path) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let Ok(file) = std::fs::File::open(path) else {
        return result;
    };
    let mut reader = std::io::BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else {
        return result;
    };
    for field in exif.fields() {
        let tag = field.tag.to_string();
        let value = field.display_value().to_string();
        if !value.is_empty() && result.len() < 30 {
            result.push((tag, value));
        }
    }
    result
}

/// Rotate RGBA8 image by `angle_deg` onto transparent padded canvas (bilinear).
/// Returns `(rgba, new_w, new_h)`.
// reason: affine + bilinear math is clearer as plain arithmetic; runs once per thumbnail off render thread.
#[allow(clippy::suboptimal_flops)]
pub(super) fn rotate_rgba(rgba: &[u8], w: usize, h: usize, angle_deg: f32) -> (Vec<u8>, usize, usize) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return (rgba.to_vec(), w, h);
    }
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    let (wf, hf) = (w as f32, h as f32);
    let new_w = (wf * cos.abs() + hf * sin.abs()).ceil().max(1.0) as usize;
    let new_h = (wf * sin.abs() + hf * cos.abs()).ceil().max(1.0) as usize;
    let mut out = vec![0u8; new_w * new_h * 4];
    let (cx, cy) = (wf / 2.0, hf / 2.0);
    let (ncx, ncy) = (new_w as f32 / 2.0, new_h as f32 / 2.0);
    let idx = |x: usize, y: usize| (y * w + x) * 4;
    for oy in 0..new_h {
        for ox in 0..new_w {
            let dx = ox as f32 + 0.5 - ncx;
            let dy = oy as f32 + 0.5 - ncy;
            let sx = cos * dx + sin * dy + cx;
            let sy = -sin * dx + cos * dy + cy;
            if sx < 0.0 || sy < 0.0 || sx >= wf || sy >= hf {
                continue;
            }
            let x0 = sx.floor() as usize;
            let y0 = sy.floor() as usize;
            let x1 = (x0 + 1).min(w - 1);
            let y1 = (y0 + 1).min(h - 1);
            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;
            let o = (oy * new_w + ox) * 4;
            for c in 0..4 {
                let p00 = f32::from(rgba[idx(x0, y0) + c]);
                let p10 = f32::from(rgba[idx(x1, y0) + c]);
                let p01 = f32::from(rgba[idx(x0, y1) + c]);
                let p11 = f32::from(rgba[idx(x1, y1) + c]);
                let top = p00 + (p10 - p00) * fx;
                let bot = p01 + (p11 - p01) * fx;
                out[o + c] = (top + (bot - top) * fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    (out, new_w, new_h)
}
