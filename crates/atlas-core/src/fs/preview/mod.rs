#![allow(dead_code)]
// reason: retained API surface not exercised by every GPUI flow yet
// reason: preview generation + temp-file cleanup run on background workers, never
// the gpui render thread; the sync-fs ban targets UI-foreground blocking.
#![allow(clippy::disallowed_methods)]

mod archive;
mod audio;
mod dir;
mod image;
mod markdown;
mod pdf;
mod text;

pub use archive::load_archive_preview;
pub use pdf::{preview_pdf_public, preview_pdf_with_pdftoppm};

use std::path::PathBuf;

use atlas_model::app_state::BgResult;
use atlas_model::pane::TabId;
use atlas_model::preview_cache::{is_audio_ext, is_image_ext, MultiPreviewKey, PreviewResult};

pub(crate) const MAX_TEXT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_IMAGE_PREVIEW_EDGE: u32 = 1600;
pub(crate) const MAX_MULTI_THUMBNAIL_EDGE: u32 = 384;
pub(crate) const MAX_PDF_PREVIEW_EDGE: &str = "1024";

/// Per-position tilt (degrees) for the multi-selection "butterfly" fan.
const MULTI_BUTTERFLY_ANGLES: [f32; 4] = [-8.0, -3.0, 3.0, 8.0];

pub async fn load_preview(tab: TabId, path: PathBuf, tx: flume::Sender<BgResult>) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let result = if path.is_dir() {
        dir::preview_dir(&path).await
    } else if is_image_ext(&ext) {
        let p = path.clone();
        tokio::task::spawn_blocking(move || image::preview_image(&p))
            .await
            .unwrap_or_else(|e| PreviewResult::Error(e.to_string()))
    } else if ext == "pdf" {
        let p = path.clone();
        tokio::task::spawn_blocking(move || pdf::preview_pdf_public(&p))
            .await
            .unwrap_or_else(|e| PreviewResult::Error(e.to_string()))
    } else if is_audio_ext(&ext) {
        let p = path.clone();
        tokio::task::spawn_blocking(move || audio::preview_audio(&p))
            .await
            .unwrap_or_else(|e| PreviewResult::Error(e.to_string()))
    } else if markdown::is_markdown_ext(&ext) {
        markdown::preview_markdown(&path).await
    } else {
        text::preview_text(&path, &ext).await
    };

    let _ = tx.send(BgResult::PreviewReady { tab, path, result });
}

pub async fn load_multi_thumbnail(
    tab: TabId,
    key: MultiPreviewKey,
    index: usize,
    path: PathBuf,
    tx: flume::Sender<BgResult>,
) {
    let thumbnail_path = path.clone();
    let thumbnail = tokio::task::spawn_blocking(move || {
        let (rgba, width, height) = image::preview_thumbnail(&thumbnail_path)?;
        let angle = MULTI_BUTTERFLY_ANGLES.get(index).copied().unwrap_or(0.0);
        Ok::<_, ::image::ImageError>(if angle == 0.0 {
            (rgba, width, height)
        } else {
            image::rotate_rgba(&rgba, width, height, angle)
        })
    })
    .await
    .ok()
    .and_then(Result::ok);
    let Some((rgba, width, height)) = thumbnail else {
        return;
    };
    let _ = tx.send(BgResult::MultiPreviewThumbnailReady {
        tab,
        key,
        path,
        rgba,
        width,
        height,
    });
}
