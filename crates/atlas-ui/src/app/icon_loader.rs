//! App methods for the colored file-type icon cache.
//!
//! Rasterises SVGs with resvg and applies the premultiplied-alpha → `straight-BGRA`
//! conversion that GPUI's `img()` pipeline expects, then wraps into `Arc<RenderImage>`.
//! Results flow through a dedicated `icon_tx / icon_rx` channel so RenderImage
//! (a GPUI type) never touches atlas-model.

use std::path::Path;
use std::sync::Arc;

use gpui::RenderImage;

use crate::icon_theme::{icon_key, IconState};
use crate::state::file_entry::FileKind;

use super::App;

/// Target render edge (px). 64 px is a sweet spot: sharp at the default grid
/// icon area (~72 px) and only a 4× downscale for 16 px list rows, which the
/// alpha-dilation pass eliminates visually.
const TARGET_PX: u32 = 64;

/// Number of colour-dilation passes. Each pass spreads opaque-pixel colours
/// one pixel into adjacent transparent regions. 4 passes covers the ~4 px
/// border that matters for a 4× downscale (64 px → 16 px list icons).
const DILATE_PASSES: usize = 4;

impl App {
    /// Enqueue a background rasterisation for `key` if not already cached.
    pub(crate) fn request_icon(&mut self, key: String) {
        if self.icon_cache.contains_key(&key) {
            return;
        }
        let Some(ref theme) = self.icon_theme else {
            return;
        };
        let svg_path = if key == "/" {
            theme.default_folder.clone()
        } else if key.is_empty() {
            theme.default_file.clone()
        } else {
            theme.svg_for_ext(&key).map(std::path::Path::to_path_buf)
        };
        let Some(svg_path) = svg_path else {
            return;
        };

        self.icon_cache.insert(key.clone(), IconState::Loading);

        let icon_tx = self.icon_tx.clone();
        self.rt.spawn(async move {
            let result: Option<Arc<RenderImage>> =
                tokio::task::spawn_blocking(move || rasterize(&svg_path))
                    .await
                    .ok()
                    .flatten();
            icon_tx.send((key, result)).ok();
        });
    }

    /// Return the cached icon texture for `key`, or `None` if not yet ready.
    pub(crate) fn icon_texture(&self, key: &str) -> Option<Arc<RenderImage>> {
        match self.icon_cache.get(key) {
            Some(IconState::Ready(tex)) => Some(tex.clone()),
            _ => None,
        }
    }

    /// Resolve `(kind, ext)` → cache key → texture (read-only, no load trigger).
    pub(crate) fn icon_tex_for_entry(
        &self,
        kind: &FileKind,
        ext: Option<&str>,
    ) -> Option<Arc<RenderImage>> {
        self.icon_texture(&icon_key(kind, ext))
    }
}

/// Rasterise an SVG icon to [`TARGET_PX`]×[`TARGET_PX`] and wrap in a
/// `RenderImage` ready for `img()`.
///
/// Pixel pipeline:
///   resvg → `tiny_skia` Pixmap (premultiplied RGBA)
///   → colour-dilate: spread opaque colours into transparent border pixels
///     (prevents black fringing when the GPU downscales the texture)
///   → un-premultiply: `channel = channel_premul / (alpha/255)`
///   → swap R↔B → straight BGRA
///   → `RgbaImage` → `Frame` → `RenderImage`
fn rasterize(svg_path: &Path) -> Option<Arc<RenderImage>> {
    #[allow(clippy::disallowed_methods)] // reason: runs inside spawn_blocking
    let data = std::fs::read(svg_path).ok()?;
    let opts = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(&data, &opts).ok()?;
    let svg_w = tree.size().width();
    let svg_h = tree.size().height();
    let scale = TARGET_PX as f32 / svg_w.max(svg_h);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(TARGET_PX, TARGET_PX)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    let mut raw = pixmap.take();

    // Spread opaque-pixel colours into adjacent transparent pixels so that GPU
    // bilinear downscaling doesn't blend the icon edges with black (0,0,0,0).
    // We operate in premultiplied RGBA space: transparent pixels have colour=0,
    // so any non-zero colour neighbour provides a good fill value.
    dilate_colors(&mut raw, TARGET_PX as usize, DILATE_PASSES);

    // tiny_skia emits premultiplied RGBA. Convert to straight BGRA for GPUI.
    for px in raw.chunks_exact_mut(4) {
        let a = px[3];
        if a > 0 && a < 255 {
            let af = f32::from(a) / 255.0;
            px[0] = (f32::from(px[0]) / af).min(255.0) as u8;
            px[1] = (f32::from(px[1]) / af).min(255.0) as u8;
            px[2] = (f32::from(px[2]) / af).min(255.0) as u8;
        }
        px.swap(0, 2); // R↔B → BGRA
    }

    let buf = image::RgbaImage::from_raw(TARGET_PX, TARGET_PX, raw)?;
    let frame = image::Frame::new(buf);
    Some(Arc::new(RenderImage::new(vec![frame])))
}

/// One pass of 4-connectivity colour dilation in premultiplied RGBA space.
/// Transparent pixels (alpha == 0) inherit the average colour of their
/// non-transparent orthogonal neighbours, but keep alpha == 0.
fn dilate_pass(src: &[u8], dst: &mut [u8], side: usize) {
    dst.copy_from_slice(src);
    for y in 0..side {
        for x in 0..side {
            let i = (y * side + x) * 4;
            if src[i + 3] != 0 {
                continue; // opaque/semi — leave untouched
            }
            let (mut sums, mut n) = ([0u32; 3], 0u32);
            for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || nx >= side as i32 || ny < 0 || ny >= side as i32 {
                    continue;
                }
                let ni = (ny as usize * side + nx as usize) * 4;
                if src[ni + 3] > 0 {
                    sums[0] += u32::from(src[ni]);
                    sums[1] += u32::from(src[ni + 1]);
                    sums[2] += u32::from(src[ni + 2]);
                    n += 1;
                }
            }
            if let (Some(avg_r), Some(avg_g), Some(avg_b)) = (
                sums[0].checked_div(n),
                sums[1].checked_div(n),
                sums[2].checked_div(n),
            ) {
                dst[i] = avg_r as u8;
                dst[i + 1] = avg_g as u8;
                dst[i + 2] = avg_b as u8;
                // alpha stays 0 — pixel is still transparent
            }
        }
    }
}

fn dilate_colors(raw: &mut [u8], side: usize, passes: usize) {
    let mut tmp = raw.to_vec();
    for pass in 0..passes {
        if pass % 2 == 0 {
            dilate_pass(raw, &mut tmp, side);
        } else {
            dilate_pass(&tmp, raw, side);
        }
    }
    if passes % 2 == 1 {
        raw.copy_from_slice(&tmp);
    }
}
