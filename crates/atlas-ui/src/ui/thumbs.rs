//! FASE4 (optimización) — Downscaled grid thumbnails.
//!
//! The Grid view used to paint image cells with `img(path)`, which makes gpui decode
//! every file at full resolution and keep a full-size GPU texture — brutal for a
//! folder of hundreds of large photos. Instead, the visible cells request a small
//! thumbnail that is decoded + resized off-thread (`request_thumbnail`), uploaded to
//! a cached `Arc<RenderImage>` (UI-side only — `state` never holds a renderer handle),
//! and reused on every later frame. Requests are virtualized: only the cells the grid
//! actually builds ask for a thumbnail.

use crate::app::App;
use crate::state::app_state::BgResult;
use crate::ui::inspector::rgba_to_render_image;
use gpui::RenderImage;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// UI-side cache entry for one path's grid thumbnail.
pub enum ThumbState {
    /// A decode task is in flight; render the placeholder glyph meanwhile.
    Loading,
    /// Ready to paint.
    Ready(Arc<RenderImage>),
    /// Decode failed (or produced nothing); never re-requested.
    Missing,
}

/// Longest edge (px) of the decoded thumbnail. Generous vs. the largest zoom step so
/// it still looks crisp, but a tiny fraction of a full-res photo's pixels.
const THUMB_MAX_EDGE: u32 = 128;

/// Max concurrent *extension* thumbnailer processes (ffmpeg/magick/…). A large folder
/// of videos must not spawn hundreds of processes at once; over-cap requests are simply
/// dropped and re-issued on the next frame (the cell isn't cached yet).
const MAX_EXT_THUMB_INFLIGHT: usize = 8;

/// Max concurrent native image decodes. Over-cap requests are dropped and re-issued
/// next frame (no Loading marker inserted), keeping the blocking pool free for
/// directory listing while preventing a burst of hundreds of simultaneous decodes.
const MAX_NATIVE_THUMB_INFLIGHT: usize = 16;

impl App {
    /// Ensure a thumbnail decode is requested for `path`. If the native-decode cap is
    /// reached the request is silently dropped (the cell will re-request next frame
    /// because no Loading marker is inserted). The task checks `thumb_wanted` before
    /// the expensive decode; if the path left the viewport it sends `GridThumbnailSkipped`
    /// so the Loading marker is cleared and the cell can re-request when visible again.
    pub(crate) fn request_thumbnail(&mut self, path: &std::path::Path) {
        if self.thumb_textures.contains_key(path) {
            return;
        }
        if self.native_thumb_inflight >= MAX_NATIVE_THUMB_INFLIGHT {
            return;
        }
        let path = path.to_path_buf();
        self.thumb_textures
            .insert(path.clone(), ThumbState::Loading);
        self.native_thumb_inflight += 1;
        let tx = self.tx.clone();
        let wanted = self.thumb_wanted.clone();
        self.rt.spawn(async move {
            if let Ok(bg) = tokio::task::spawn_blocking(move || {
                if !wanted.lock().unwrap().contains(&path) {
                    return BgResult::GridThumbnailSkipped { path };
                }
                decode_thumbnail(&path, THUMB_MAX_EDGE)
            })
            .await
            {
                let _ = tx.send(bg);
            }
        });
    }

    /// Remove `path` from the thumbnail cache (called when a decode was skipped because
    /// the path left the viewport), so the cell can re-request it when visible again.
    pub(crate) fn apply_thumb_skipped(&mut self, path: PathBuf) {
        self.native_thumb_inflight = self.native_thumb_inflight.saturating_sub(1);
        self.thumb_textures.remove(&path);
    }

    /// Request a thumbnail produced by an extension thumbnailer (`run_tmpl` is the
    /// command template). Idempotent and throttled: skips if already known or if the
    /// external-process cap is reached (re-issued next frame). On completion it reports
    /// via the same `BgResult::GridThumbnailReady` path as native decodes.
    pub(crate) fn request_ext_thumbnail(
        &mut self,
        path: &std::path::Path,
        run_tmpl: String,
        tools: HashMap<String, PathBuf>,
    ) {
        if self.thumb_textures.contains_key(path)
            || self.ext_thumb_inflight.len() >= MAX_EXT_THUMB_INFLIGHT
        {
            return;
        }
        let path = path.to_path_buf();
        self.thumb_textures
            .insert(path.clone(), ThumbState::Loading);
        self.ext_thumb_inflight.insert(path.clone());
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let report = path.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::ext::runner::run_thumbnail(&path, &run_tmpl, &tools, THUMB_MAX_EDGE)
            })
            .await
            .ok()
            .flatten();
            let bg = match result {
                Some((rgba, width, height)) => BgResult::GridThumbnailReady {
                    path: report,
                    rgba,
                    width,
                    height,
                },
                None => BgResult::GridThumbnailReady {
                    path: report,
                    rgba: Vec::new(),
                    width: 0,
                    height: 0,
                },
            };
            let _ = tx.send(bg);
        });
    }

    /// Request a thumbnail produced by the Windows Shell thumbnail cache
    /// (`IShellItemImageFactory`).  Covers videos, RAW photos, HEIC, PDF, etc. — any
    /// format that has a registered Windows thumbnail provider.  Shares the extension
    /// inflight cap so we never exceed `MAX_EXT_THUMB_INFLIGHT` concurrent COM calls.
    #[cfg(target_os = "windows")]
    pub(crate) fn request_shell_thumbnail(&mut self, path: &std::path::Path) {
        self.request_shell_image(path, false);
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn request_shell_thumbnail_or_icon(&mut self, path: &std::path::Path) {
        self.request_shell_image(path, true);
    }

    #[cfg(target_os = "windows")]
    fn request_shell_image(&mut self, path: &std::path::Path, allow_icon: bool) {
        if self.thumb_textures.contains_key(path)
            || self.ext_thumb_inflight.len() >= MAX_EXT_THUMB_INFLIGHT
        {
            return;
        }
        let path = path.to_path_buf();
        self.thumb_textures
            .insert(path.clone(), ThumbState::Loading);
        self.ext_thumb_inflight.insert(path.clone());
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let report = path.clone();
            let result = tokio::task::spawn_blocking(move || {
                if allow_icon {
                    crate::shell_thumb::shell_thumbnail_or_icon(&path, THUMB_MAX_EDGE)
                } else {
                    crate::shell_thumb::shell_thumbnail(&path, THUMB_MAX_EDGE)
                }
            })
            .await
            .ok()
            .flatten();
            let bg = match result {
                Some((rgba, width, height)) => BgResult::GridThumbnailReady {
                    path: report,
                    rgba,
                    width,
                    height,
                },
                None => BgResult::GridThumbnailReady {
                    path: report,
                    rgba: Vec::new(),
                    width: 0,
                    height: 0,
                },
            };
            let _ = tx.send(bg);
        });
    }

    /// Look up an already-decoded thumbnail texture for `path`, if ready.
    pub(crate) fn grid_thumbnail(&self, path: &std::path::Path) -> Option<Arc<RenderImage>> {
        match self.thumb_textures.get(path) {
            Some(ThumbState::Ready(tex)) => Some(tex.clone()),
            _ => None,
        }
    }

    /// Apply a finished thumbnail decode (called from the bg-result drain).
    pub(crate) fn apply_grid_thumbnail(
        &mut self,
        path: PathBuf,
        rgba: Vec<u8>,
        width: usize,
        height: usize,
    ) {
        // Free an extension-thumbnailer slot if this path used one; otherwise it was
        // a native decode — decrement the native inflight counter.
        let was_ext = self.ext_thumb_inflight.remove(&path);
        if !was_ext {
            self.native_thumb_inflight = self.native_thumb_inflight.saturating_sub(1);
        }
        let state = if rgba.is_empty() {
            ThumbState::Missing
        } else {
            match rgba_to_render_image(&rgba, width, height) {
                Some(tex) => ThumbState::Ready(tex),
                None => ThumbState::Missing,
            }
        };
        self.thumb_textures.insert(path, state);
        if self.thumb_textures.len() > 1000 {
            self.thumb_textures.clear();
        }
    }
}

/// Decode `path` and box-resize so its longest edge is `max_edge`, returning packed
/// RGBA8. Returns an empty vec on any failure (cached as a permanent miss).
fn decode_thumbnail(path: &std::path::Path, max_edge: u32) -> BgResult {
    let result = image::open(path).ok().map(|img| {
        let thumb = img.thumbnail(max_edge, max_edge).to_rgba8();
        let (w, h) = (thumb.width() as usize, thumb.height() as usize);
        (thumb.into_raw(), w, h)
    });
    match result {
        Some((rgba, width, height)) => BgResult::GridThumbnailReady {
            path: path.to_path_buf(),
            rgba,
            width,
            height,
        },
        None => BgResult::GridThumbnailReady {
            path: path.to_path_buf(),
            rgba: Vec::new(),
            width: 0,
            height: 0,
        },
    }
}
