use std::path::PathBuf;

use atlas_model::app_state::BgResult;
use atlas_model::pane::TabId;
use atlas_model::preview_cache::{is_audio_ext, is_image_ext, PreviewResult};

use super::markdown::{is_markdown_ext, parse_markdown};
use super::text::highlight_text;
use super::MAX_TEXT_BYTES;

pub async fn load_archive_preview(
    tab: TabId,
    archive: PathBuf,
    inner: PathBuf,
    tx: flume::Sender<BgResult>,
) {
    let ext = inner
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let result = {
        let inner2 = inner.clone();
        tokio::task::spawn_blocking(move || -> PreviewResult {
            let provider = match crate::fs::archive::open_with(&archive, None) {
                Some(p) => p,
                None => return PreviewResult::Error("Formato de archivo no soportado".into()),
            };
            let mut reader = match provider.read_bytes(&inner2) {
                Ok(r) => r,
                Err(e) => return PreviewResult::Error(e.to_string()),
            };
            let cap = if is_image_ext(&ext) {
                16 * 1024 * 1024
            } else {
                MAX_TEXT_BYTES
            };
            let mut buf = Vec::with_capacity(cap.min(1024 * 1024));
            let mut tmp = [0u8; 65536];
            loop {
                use std::io::Read as _;
                match reader.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if buf.len() >= cap {
                            break;
                        }
                    }
                    Err(e) => return PreviewResult::Error(e.to_string()),
                }
            }
            bytes_to_preview(buf, &ext)
        })
        .await
        .unwrap_or_else(|e| PreviewResult::Error(e.to_string()))
    };

    let _ = tx.send(BgResult::PreviewReady {
        tab,
        path: inner,
        result,
    });
}

fn bytes_to_preview(bytes: Vec<u8>, ext: &str) -> PreviewResult {
    if is_image_ext(ext) {
        match image::load_from_memory(&bytes) {
            Ok(img) => {
                let original_width = img.width() as usize;
                let original_height = img.height() as usize;
                let img =
                    img.thumbnail(super::MAX_IMAGE_PREVIEW_EDGE, super::MAX_IMAGE_PREVIEW_EDGE);
                let rgba = img.to_rgba8();
                let (width, height) = rgba.dimensions();
                PreviewResult::Image {
                    rgba: rgba.into_raw(),
                    width: width as usize,
                    height: height as usize,
                    original_width,
                    original_height,
                    exif: Vec::new(),
                }
            }
            Err(e) => PreviewResult::Error(e.to_string()),
        }
    } else if is_markdown_ext(ext) {
        if bytes.len() > MAX_TEXT_BYTES {
            return PreviewResult::Binary {
                size: bytes.len() as u64,
            };
        }
        match String::from_utf8(bytes) {
            Ok(source) => {
                let blocks = parse_markdown(&source);
                PreviewResult::Markdown {
                    blocks,
                    source,
                    base_uri: String::new(),
                }
            }
            Err(e) => PreviewResult::Binary {
                size: e.into_bytes().len() as u64,
            },
        }
    } else if ext == "pdf" || is_audio_ext(ext) {
        PreviewResult::Binary {
            size: bytes.len() as u64,
        }
    } else {
        if bytes.len() > MAX_TEXT_BYTES {
            return PreviewResult::Binary {
                size: bytes.len() as u64,
            };
        }
        match String::from_utf8(bytes) {
            Ok(text) => {
                let lines = highlight_text(&text, ext);
                PreviewResult::Text { lines, text }
            }
            Err(e) => PreviewResult::Binary {
                size: e.into_bytes().len() as u64,
            },
        }
    }
}
