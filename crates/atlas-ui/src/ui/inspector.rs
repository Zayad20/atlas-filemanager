//! FASE5 — Inspector / preview panel (gpui port of the egui `src/ui/inspector.rs`).
//!
//! Renders the right-hand preview panel for the focused tab's selection:
//! single-file metadata + content preview (text/markdown/image/pdf/dir/binary),
//! or a multi-selection summary with a thumbnail stack.
//!
//! Framework split (per the migration rules):
//! - `state::preview` (neutral) holds the decoded RGBA (`PreviewResult` / `ImageData`).
//! - The GPU textures (`Arc<RenderImage>`) are cached HERE, on the UI side, in new
//!   `App` fields (`preview_textures` / `multi_textures`). `state` never holds a
//!   renderer handle.
//!
//! The egui version drove preview loading through `sync_active_preview(ctx)` which
//! needed an `egui::Context` for `request_repaint`. Under gpui the ~60 Hz drain loop
//! already repaints when background results land, so the gpui variants drop `ctx`
//! entirely (`sync_active_preview_gpui` / `load_preview_gpui` / `load_multi_preview_gpui`).

use crate::app::App;
use crate::state::file_entry::FileKind;
use crate::state::layout_tree::TileId;
use crate::state::preview_cache::{
    MultiPreviewState, PreviewRequest, PreviewResult, DIR_PREVIEW_LIMIT,
};
use crate::state::settings::ViewMode;
use crate::state::tab::TabState;
use crate::ui::icons::{file_type_icon, icon, AtlasIcon};
use crate::util::display::{format_date, format_size};
use gpui::prelude::*;
use gpui::{
    div, img, px, AnyElement, ClickEvent, Context, Hsla, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, RenderImage, ScrollDelta, ScrollWheelEvent, Window,
};
use gpui_component::{
    h_flex, v_flex, ActiveTheme,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;



/// Scale `w`×`h` to fit inside `box_w`×`box_h` preserving aspect ratio. Returns the
/// fitted display size. A zero/empty source falls back to the box size.
fn fit_into(w: f32, h: f32, box_w: f32, box_h: f32) -> (f32, f32) {
    if w <= 0.0 || h <= 0.0 {
        return (box_w, box_h);
    }
    let scale = (box_w / w).min(box_h / h);
    (w * scale, h * scale)
}

fn preview_language_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => "rust",
        "js" | "jsx" => "javascript",
        "ts" | "tsx" => "typescript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" => "cpp",
        "cs" => "c_sharp",
        "rb" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "sh" | "bash" => "bash",
        "ps1" => "powershell",
        "html" | "htm" => "html",
        "css" => "css",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "xml" => "xml",
        "md" | "markdown" => "markdown",
        _ => "text",
    }
}

/// Upload tightly-packed RGBA8 (`width*height*4`) to a gpui `RenderImage`. gpui's
/// renderer expects BGRA, so we swap R/B per pixel (same as gpui's own decoder).
pub fn rgba_to_render_image(rgba: &[u8], width: usize, height: usize) -> Option<Arc<RenderImage>> {
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return None;
    }
    let mut buf = rgba[..width * height * 4].to_vec();
    for pixel in buf.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let image = image::RgbaImage::from_raw(width as u32, height as u32, buf)?;
    let frame = image::Frame::new(image);
    Some(Arc::new(RenderImage::new(vec![frame])))
}

impl App {
    // ── Preview loading (gpui variants, no egui ctx) ─────────────────────────

    /// gpui replacement for `sync_active_preview(ctx)`: decides what (if anything)
    /// should be previewed for the focused tab and kicks off a debounced load. The
    /// drain loop handles repaints, so no `ctx.request_repaint` is needed.
    pub(crate) fn sync_active_preview_gpui(&mut self) {
        let tab = self.state.focused_tile;
        if !self.preview_visible_for_focused() {
            self.pending_preview = None;
            return;
        }
        let request = self.state.active_preview_request();
        let Some(request) = request else {
            self.pending_preview = None;
            self.state.clear_preview(tab);
            return;
        };

        let already_loaded = match &request {
            PreviewRequest::Single(path) => self.state.preview.paths.get(&tab) == Some(path),
            PreviewRequest::ArchiveSingle { inner, .. } => {
                self.state.preview.paths.get(&tab).map(PathBuf::as_path) == Some(inner.as_path())
            }
            PreviewRequest::Multiple { key, .. } => self
                .state
                .preview
                .multi
                .get(&tab)
                .is_some_and(|multi| &multi.key == key),
        };
        if already_loaded {
            return;
        }

        // The egui path debounced via `ctx.request_repaint_after`; under gpui a render
        // only happens when state changed (selection/nav already called `cx.notify()`),
        // so we kick the load immediately. Superseded loads are discarded on arrival
        // (`BgResult::PreviewReady` checks `preview.paths`), so fast arrow-key scrolling
        // is still safe. (`pending_preview` / `PREVIEW_DEBOUNCE` remain used by the
        // legacy egui `sync_active_preview` path.)
        match request {
            PreviewRequest::Single(path) => self.load_preview_gpui(tab, path),
            PreviewRequest::ArchiveSingle { archive, inner } => {
                self.load_archive_preview_gpui(tab, archive, inner);
            }
            PreviewRequest::Multiple { key, paths } => {
                self.load_multi_preview_gpui(tab, key, paths);
            }
        }
    }

    /// Is the preview panel visible for the focused tab? True when the global
    /// `show_inspector` is on OR the per-tab `preview_enabled` toggle is set.
    pub(crate) fn preview_visible_for_focused(&self) -> bool {
        let global = self.state.settings.show_inspector;
        let per_tab = self
            .state
            .active_tab_data()
            .is_some_and(|d| d.preview_enabled);
        global || per_tab
    }

    fn load_preview_gpui(&mut self, tab: TileId, path: PathBuf) {
        if self.state.preview.paths.get(&tab).map(PathBuf::as_path) == Some(path.as_path()) {
            return;
        }
        // New selection: reset zoom/pan (keep `expanded` so browsing in the big view
        // stays open).
        if let Some(v) = self.preview_view.get_mut(&tab) {
            v.zoom = 1.0;
            v.pan_x = 0.0;
            v.pan_y = 0.0;
        }
        self.preview_drag = None;
        self.state.preview.paths.insert(tab, path.clone());
        self.state.preview.results.remove(&tab);
        self.state.preview.multi.remove(&tab);
        self.preview_textures.remove(&tab);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::preview::load_preview(tab, path, tx).await;
        });
    }

    fn load_archive_preview_gpui(&mut self, tab: TileId, archive: PathBuf, inner: PathBuf) {
        if self.state.preview.paths.get(&tab).map(PathBuf::as_path) == Some(inner.as_path()) {
            return;
        }
        if let Some(v) = self.preview_view.get_mut(&tab) {
            v.zoom = 1.0;
            v.pan_x = 0.0;
            v.pan_y = 0.0;
        }
        self.preview_drag = None;
        self.state.preview.paths.insert(tab, inner.clone());
        self.state.preview.results.remove(&tab);
        self.state.preview.multi.remove(&tab);
        self.preview_textures.remove(&tab);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::preview::load_archive_preview(tab, archive, inner, tx).await;
        });
    }

    fn load_multi_preview_gpui(
        &mut self,
        tab: TileId,
        key: crate::state::preview_cache::MultiPreviewKey,
        paths: Vec<PathBuf>,
    ) {
        if self
            .state
            .preview
            .multi
            .get(&tab)
            .is_some_and(|multi| multi.key == key)
        {
            return;
        }
        self.state.clear_preview(tab);
        self.preview_textures.remove(&tab);
        self.multi_textures.remove(&tab);
        self.state.preview.multi.insert(
            tab,
            MultiPreviewState {
                key: key.clone(),
                paths: paths.clone(),
                images: Vec::with_capacity(paths.len()),
            },
        );
        for (index, path) in paths.into_iter().enumerate() {
            let tx = self.tx.clone();
            let key = key.clone();
            self.rt.spawn(async move {
                crate::fs::preview::load_multi_thumbnail(tab, key, index, path, tx).await;
            });
        }
    }

    /// Get-or-build the cached single-preview texture for the focused tab from the
    /// decoded RGBA in `state.preview.results` (Image / Pdf variants). Keyed by path
    /// so a new selection invalidates the old texture.
    fn focused_preview_texture(&mut self) -> Option<Arc<RenderImage>> {
        let tab = self.state.focused_tile;
        let path = self.state.preview.paths.get(&tab).cloned()?;
        if let Some((cached_path, tex)) = self.preview_textures.get(&tab) {
            if *cached_path == path {
                return Some(tex.clone());
            }
        }
        let (rgba, w, h) = match self.state.preview.results.get(&tab)? {
            PreviewResult::Image {
                rgba,
                width,
                height,
                ..
            }
            | PreviewResult::Pdf {
                rgba,
                width,
                height,
                ..
            } => (rgba, *width, *height),
            PreviewResult::Audio {
                cover: Some(cover), ..
            } => (&cover.rgba, cover.width, cover.height),
            _ => return None,
        };
        let tex = rgba_to_render_image(rgba, w, h)?;
        self.preview_textures.insert(tab, (path, tex.clone()));
        Some(tex)
    }

    /// Get-or-build the cached multi-selection thumbnail textures for the focused tab.
    fn focused_multi_textures(&mut self) -> Vec<(PathBuf, Arc<RenderImage>)> {
        let tab = self.state.focused_tile;
        let Some(multi) = self.state.preview.multi.get(&tab) else {
            return Vec::new();
        };
        // Rebuild only when the set of decoded images grew.
        let need = multi.images.len();
        let have = self.multi_textures.get(&tab).map_or(0, std::vec::Vec::len);
        if have != need {
            let built: Vec<(PathBuf, Arc<RenderImage>)> = multi
                .images
                .iter()
                .filter_map(|(p, img)| {
                    rgba_to_render_image(&img.rgba, img.width, img.height).map(|t| (p.clone(), t))
                })
                .collect();
            self.multi_textures.insert(tab, built);
        }
        self.multi_textures.get(&tab).cloned().unwrap_or_default()
    }

    /// Toggle the expanded (near-full-window) preview for the focused tab. Bound to
    /// Space. Resets zoom/pan so the new size starts fitted.
    pub(crate) fn toggle_preview_expanded(&mut self) {
        let tab = self.state.focused_tile;
        let v = self.preview_view.entry(tab).or_default();
        v.expanded = !v.expanded;
        v.zoom = 1.0;
        v.pan_x = 0.0;
        v.pan_y = 0.0;
        self.preview_drag = None;
    }

    /// Collapse the expanded preview for the focused tab (Esc / `CloseOverlays`).
    pub(crate) fn collapse_preview_expanded(&mut self) -> bool {
        let tab = self.state.focused_tile;
        match self.preview_view.get_mut(&tab) {
            Some(v) if v.expanded => {
                v.expanded = false;
                v.zoom = 1.0;
                v.pan_x = 0.0;
                v.pan_y = 0.0;
                true
            }
            _ => false,
        }
    }

    fn preview_view_for_focused(&self) -> crate::app::PreviewView {
        self.preview_view
            .get(&self.state.focused_tile)
            .copied()
            .unwrap_or_default()
    }

    // ── Rendering ────────────────────────────────────────────────────────────

    /// Render the inspector panel (right side of the central row). The caller in
    /// `root.rs` only invokes this when `preview_visible_for_focused()` is true.
    pub(crate) fn render_inspector(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Trigger / advance the debounced preview load for the current selection.
        self.sync_active_preview_gpui();
        self.sync_selection_tags();

        let (bg, fg, muted, border, danger) = {
            let t = cx.theme();
            (
                t.sidebar,
                t.foreground,
                t.muted_foreground,
                t.border,
                t.danger,
            )
        };
        // FASE6: when `expanded` is set (Space), widen the inspector to most of the
        // window so the zoomable image has room. The image-preview box dimensions are
        // derived here and threaded down to `render_image_preview`.
        let expanded = self
            .preview_view
            .get(&self.state.focused_tile)
            .is_some_and(|v| v.expanded);
        let viewport = window.viewport_size();
        let width = if expanded {
            (f32::from(viewport.width) * 0.9).max(360.0)
        } else {
            self.state.settings.inspector_width.max(180.0)
        };
        let box_w = (width - 24.0).max(80.0);
        let box_h = if expanded {
            (f32::from(viewport.height) - 160.0).max(300.0)
        } else {
            420.0
        };

        let summary = self
            .state
            .active_tab_state()
            .zip(self.state.active_tab_data())
            .map(|(tab, data)| selection_summary(tab, &data.view_mode))
            .unwrap_or_default();

        let mut col = v_flex()
            .w(px(width))
            .h_full()
            .bg(bg)
            .text_color(fg)
            .border_l_1()
            .border_color(border)
            .p_2()
            .gap_2();

        if summary.count == 0 {
            return col
                .items_center()
                .justify_center()
                .child(div().text_color(muted).child(rust_i18n::t!("inspector_no_selection").to_string()))
                .into_any_element();
        }

        if summary.count > 1 {
            col = col.child(self.render_multi(&summary, muted, cx));
            return col.into_any_element();
        }

        // Single selection: metadata header + content preview.
        if let Some(entry) = &summary.single {
            col = col.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        match self.icon_tex_for_entry(&entry.kind, entry.extension.as_deref()) {
                            Some(tex) => gpui::img(tex)
                                .w(gpui::px(22.))
                                .h(gpui::px(22.))
                                .flex_none()
                                .into_any_element(),
                            None => file_type_icon(&entry.kind, entry.extension.as_deref(), 22.0),
                        },
                    )
                    .child(
                        v_flex()
                            .min_w(px(0.))
                            .flex_1()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(entry.name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .truncate()
                                    .child(entry.path.display().to_string()),
                            ),
                    ),
            );
            let mut meta = v_flex().text_xs().text_color(muted).gap_1();
            if let Some(size) = entry.size {
                meta = meta.child(rust_i18n::t!("inspector_size", size = format_size(size)).to_string());
            }
            if let Some(modified) = entry.modified {
                meta = meta.child(rust_i18n::t!("inspector_modified", date = format_date(modified)).to_string());
            }
            if let Some(ext) = &entry.extension {
                meta = meta.child(rust_i18n::t!("inspector_type", ext = ext).to_string());
            }
            col = col.child(meta);
        }

        col = col.child(div().h(px(1.)).w_full().bg(border));
        col = col.child(self.render_preview_body(muted, danger, box_w, box_h, window, cx));
        col.into_any_element()
    }

    /// The content-preview portion for a single selection.
    fn render_preview_body(
        &mut self,
        muted: Hsla,
        danger: Hsla,
        box_w: f32,
        box_h: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab = self.state.focused_tile;
        // Borrow the result immutably only to branch; texture build needs &mut self,
        // so resolve the texture first where needed.
        let kind = match self.state.preview.results.get(&tab) {
            None => PreviewKind::Loading,
            Some(PreviewResult::Text { .. }) => PreviewKind::Text,
            Some(PreviewResult::Markdown { .. }) => PreviewKind::Markdown,
            Some(PreviewResult::Image { .. }) => PreviewKind::Image,
            Some(PreviewResult::Pdf { .. }) => PreviewKind::Pdf,
            Some(PreviewResult::Audio { .. }) => PreviewKind::Audio,
            Some(PreviewResult::Dir { .. }) => PreviewKind::Dir,
            Some(PreviewResult::Binary { .. }) => PreviewKind::Binary,
            Some(PreviewResult::Error(_)) => PreviewKind::Error,
        };

        match kind {
            PreviewKind::Loading => div()
                .flex_1()
                .text_color(muted)
                .child(rust_i18n::t!("inspector_loading").to_string())
                .into_any_element(),
            PreviewKind::Text => self.render_text_preview(muted, window, cx),
            PreviewKind::Markdown => self.render_markdown_preview(muted, cx),
            PreviewKind::Image | PreviewKind::Pdf => {
                self.render_image_preview(muted, box_w, box_h, cx)
            }
            PreviewKind::Audio => self.render_audio_preview(muted),
            PreviewKind::Dir => self.render_dir_preview(muted),
            PreviewKind::Binary => {
                let size = match self.state.preview.results.get(&tab) {
                    Some(PreviewResult::Binary { size }) => *size,
                    _ => 0,
                };
                div()
                    .flex_1()
                    .text_color(muted)
                    .items_center()
                    .justify_center()
                    .child(rust_i18n::t!("inspector_binary", size = format_size(size)).to_string())
                    .into_any_element()
            }
            PreviewKind::Error => {
                let msg = match self.state.preview.results.get(&tab) {
                    Some(PreviewResult::Error(e)) => e.clone(),
                    _ => String::new(),
                };
                div()
                    .flex_1()
                    .text_color(danger)
                    .child(msg)
                    .into_any_element()
            }
        }
    }

    /// Selectable text/code preview. The `InputState` is kept read-only by rendering it
    /// disabled, but still gives users native selection and `Ctrl+C` behavior.
    fn render_text_preview(
        &mut self,
        muted: Hsla,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab = self.state.focused_tile;
        let Some(PreviewResult::Text { text, .. }) = self.state.preview.results.get(&tab) else {
            return div().into_any_element();
        };
        let raw = text.clone();
        let path = self
            .state
            .preview
            .paths
            .get(&tab)
            .cloned()
            .unwrap_or_else(|| PathBuf::from("<preview>"));
        let language = preview_language_for_path(&path);
        let markdown = format!("```{}\n{}\n```", language, raw);

        div()
            .id("inspector-text")
            .relative()
            .flex_1()
            .min_h(px(0.))
            .child(
                gpui_component::text::TextView::markdown(
                    gpui::SharedString::from(format!("preview-{:?}", tab)),
                    markdown,
                )
                .selectable(true)
                .scrollable(true)
                .h_full()
                .p_0()
            )
            .child(Self::copy_button(raw, muted, cx))
            .into_any_element()
    }



    /// A floating icon button that writes `text` to the system clipboard.
    fn copy_button(text: String, _muted: Hsla, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .absolute()
            .top(px(8.))
            .right(px(8.))
            .child(
                gpui_component::clipboard::Clipboard::new("preview-copy")
                    .value(text),
            )
            .into_any_element()
    }

    fn render_markdown_preview(&self, muted: Hsla, cx: &mut Context<Self>) -> AnyElement {
        let tab = self.state.focused_tile;
        let source: String = match self.state.preview.results.get(&tab) {
            Some(PreviewResult::Markdown { source, .. }) => source.clone(),
            _ => String::new(),
        };
        v_flex()
            .flex_1()
            .min_h(px(0.))
            .child(
                div()
                    .relative()
                    .id("inspector-md")
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        gpui_component::text::TextView::markdown(
                            gpui::SharedString::from(format!("preview-md-{:?}", tab)),
                            source.clone(),
                        )
                        .selectable(true)
                        .scrollable(true)
                        .h_full()
                        .p_0()
                    )
                    .child(Self::copy_button(source, muted, cx)),
            )
            .into_any_element()
    }

    /// Image / PDF: show the decoded texture fitted into a `box_w`×`box_h` viewport
    /// with scroll-to-zoom and drag-to-pan. Double-click toggles fit ↔ 2×. EXIF
    /// metadata (if present) follows as a key/value list.
    fn render_image_preview(
        &mut self,
        muted: Hsla,
        box_w: f32,
        box_h: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab = self.state.focused_tile;
        let (ow, oh, dw, dh, exif, is_pdf) = match self.state.preview.results.get(&tab) {
            Some(PreviewResult::Image {
                original_width,
                original_height,
                width,
                height,
                exif,
                ..
            }) => (
                *original_width,
                *original_height,
                *width,
                *height,
                exif.clone(),
                false,
            ),
            Some(PreviewResult::Pdf { width, height, .. }) => {
                (*width, *height, *width, *height, Vec::new(), true)
            }
            _ => return div().into_any_element(),
        };
        let view = self.preview_view_for_focused();
        let texture = self.focused_preview_texture();

        let mut col = v_flex().flex_1().min_h(px(0.)).gap_2();
        let label = if is_pdf {
            rust_i18n::t!("inspector_pdf_instructions").to_string()
        } else {
            rust_i18n::t!("inspector_image_instructions", width = ow, height = oh).to_string()
        };
        col = col.child(div().text_xs().text_color(muted).child(label));

        if let Some(tex) = texture {
            let (fit_w, fit_h) = fit_into(dw as f32, dh as f32, box_w, box_h);
            let img_w = fit_w * view.zoom;
            let img_h = fit_h * view.zoom;
            let left = (box_w - img_w) / 2.0 + view.pan_x;
            let top = (box_h - img_h) / 2.0 + view.pan_y;
            let image = img(tex)
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(img_w))
                .h(px(img_h))
                .rounded_sm();
            col = col.child(
                div()
                    .id("preview-image")
                    .relative()
                    .overflow_hidden()
                    .w(px(box_w))
                    .h(px(box_h))
                    .child(image)
                    .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _w, cx| {
                        let dy = match ev.delta {
                            ScrollDelta::Pixels(p) => f32::from(p.y),
                            ScrollDelta::Lines(l) => l.y,
                        };
                        if dy != 0.0 {
                            let tab = this.state.focused_tile;
                            let v = this.preview_view.entry(tab).or_default();
                            let factor = if dy > 0.0 { 1.1 } else { 1.0 / 1.1 };
                            v.zoom = (v.zoom * factor).clamp(0.1, 12.0);
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _w, _cx| {
                            this.preview_drag = Some(ev.position);
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _w, cx| {
                        if ev.pressed_button == Some(MouseButton::Left) {
                            if let Some(prev) = this.preview_drag {
                                let dx = f32::from(ev.position.x - prev.x);
                                let dy = f32::from(ev.position.y - prev.y);
                                let tab = this.state.focused_tile;
                                let v = this.preview_view.entry(tab).or_default();
                                v.pan_x += dx;
                                v.pan_y += dy;
                                this.preview_drag = Some(ev.position);
                                cx.notify();
                            }
                        } else {
                            this.preview_drag = None;
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _ev: &MouseUpEvent, _w, _cx| {
                            this.preview_drag = None;
                        }),
                    )
                    .on_click(cx.listener(|this, ev: &ClickEvent, _w, cx| {
                        if ev.click_count() >= 2 {
                            let tab = this.state.focused_tile;
                            let v = this.preview_view.entry(tab).or_default();
                            v.zoom = if (v.zoom - 1.0).abs() < 0.01 {
                                2.0
                            } else {
                                1.0
                            };
                            v.pan_x = 0.0;
                            v.pan_y = 0.0;
                            cx.notify();
                        }
                    })),
            );
        } else {
            col = col.child(div().text_color(muted).child("…"));
        }
        if !exif.is_empty() {
            let mut grid = v_flex().text_xs().text_color(muted).gap_1().mt_2();
            for (k, v) in exif.iter().take(30) {
                grid = grid.child(
                    h_flex()
                        .gap_2()
                        .child(div().w(px(120.)).truncate().child(k.clone()))
                        .child(div().flex_1().truncate().child(v.clone())),
                );
            }
            col = col.child(div().h(px(1.)).w_full().bg(muted).opacity(0.3));
            col = col.child(
                div()
                    .id("inspector-exif")
                    .max_h(px(200.))
                    .overflow_y_scroll()
                    .child(grid),
            );
        }
        col.into_any_element()
    }

    /// Audio: cover art (or a placeholder) plus title/artist/album/duration/bitrate.
    fn render_audio_preview(&mut self, muted: Hsla) -> AnyElement {
        let tab = self.state.focused_tile;
        let (title, artist, album, duration_secs, bitrate, has_cover) =
            match self.state.preview.results.get(&tab) {
                Some(PreviewResult::Audio {
                    title,
                    artist,
                    album,
                    duration_secs,
                    bitrate,
                    cover,
                }) => (
                    title.clone(),
                    artist.clone(),
                    album.clone(),
                    *duration_secs,
                    *bitrate,
                    cover.is_some(),
                ),
                _ => return div().into_any_element(),
            };
        let texture = if has_cover {
            self.focused_preview_texture()
        } else {
            None
        };

        let mut col = v_flex().flex_1().min_h(px(0.)).gap_2();
        if let Some(tex) = texture {
            col = col.child(img(tex).max_w(px(220.)).max_h(px(220.)).rounded_md());
        } else {
            col = col.child(
                h_flex()
                    .w(px(220.))
                    .h(px(220.))
                    .rounded_md()
                    .bg(muted)
                    .opacity(0.15)
                    .items_center()
                    .justify_center()
                    .child(icon(AtlasIcon::Music, 44.0, muted)),
            );
        }

        let mut meta = v_flex().gap_1();
        meta = meta.child(
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title.unwrap_or_else(|| "(sin título)".into())),
        );
        if let Some(artist) = artist {
            meta = meta.child(div().text_sm().text_color(muted).child(artist));
        }
        if let Some(album) = album {
            meta = meta.child(div().text_sm().text_color(muted).child(album));
        }
        let (mins, secs) = (duration_secs / 60, duration_secs % 60);
        let line = match bitrate {
            Some(br) => format!("{mins}:{secs:02} · {br} kbps"),
            None => format!("{mins}:{secs:02}"),
        };
        meta = meta.child(div().text_xs().text_color(muted).child(line));
        col.child(meta).into_any_element()
    }

    fn render_dir_preview(&self, muted: Hsla) -> AnyElement {
        let tab = self.state.focused_tile;
        let Some(PreviewResult::Dir {
            entries,
            total_count,
            total_size,
        }) = self.state.preview.results.get(&tab)
        else {
            return div().into_any_element();
        };
        let mut list = v_flex().gap_1().text_xs();
        for e in entries {
            let row_kind = if e.is_dir {
                FileKind::Dir
            } else {
                FileKind::File
            };
            let size = e.size.map(format_size).unwrap_or_default();
            list = list.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .w(px(16.))
                            .child(match self.icon_tex_for_entry(&row_kind, None) {
                                Some(tex) => gpui::img(tex)
                                    .w(px(14.))
                                    .h(px(14.))
                                    .flex_none()
                                    .into_any_element(),
                                None => file_type_icon(&row_kind, None, 14.0),
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .child(e.name.clone()),
                    )
                    .child(div().text_color(muted).child(size)),
            );
        }
        if *total_count > DIR_PREVIEW_LIMIT {
            list = list.child(
                div()
                    .text_color(muted)
                    .child(rust_i18n::t!("inspector_and_more", count = total_count - DIR_PREVIEW_LIMIT).to_string()),
            );
        }
        v_flex()
            .flex_1()
            .min_h(px(0.))
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .text_color(muted)
                    .child(rust_i18n::t!("inspector_elements_count", count = total_count).to_string())
                    .child("·")
                    .child(format_size(*total_size)),
            )
            .child(
                div()
                    .id("inspector-dir")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .child(list),
            )
            .into_any_element()
    }

    /// Multi-selection: counts + an overlapping thumbnail stack. gpui does not expose
    /// a cheap per-element rotation path here, so the fan is expressed with offset
    /// cards rather than rotated meshes.
    fn render_multi(
        &mut self,
        summary: &SelectionSummary,
        muted: Hsla,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let textures = self.focused_multi_textures();
        let mut col = v_flex().flex_1().min_h(px(0.)).gap_2();
        col = col.child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(rust_i18n::t!("inspector_elements_selected", count = summary.count).to_string()),
        );
        let mut meta = h_flex().gap_2().text_xs().text_color(muted);
        if summary.file_count > 0 {
            meta = meta.child(rust_i18n::t!("inspector_files_count", count = summary.file_count).to_string());
        }
        if summary.dir_count > 0 {
            meta = meta.child(rust_i18n::t!("inspector_folders_count", count = summary.dir_count).to_string());
        }
        if summary.known_size_count > 0 {
            meta = meta.child(format_size(summary.total_size));
        }
        col = col.child(meta);

        if summary.image_count > 0 {
            if textures.is_empty() {
                col = col.child(div().text_color(muted).child(rust_i18n::t!("inspector_loading_thumbnails").to_string()));
            } else {
                // The first four thumbnails arrive with a tilt baked into the bitmap
                // (gpui can't rotate rasters on screen), so we just overlap them into a
                // fan. Corners are transparent, so no clipping/border here.
                let thumbs: Vec<AnyElement> = textures
                    .into_iter()
                    .take(4)
                    .enumerate()
                    .map(|(idx, (_, tex))| {
                        div()
                            .when(idx > 0, |d| d.ml(px(-30.)))
                            .child(img(tex).max_w(px(104.)).max_h(px(104.)))
                            .into_any_element()
                    })
                    .collect();
                col = col.child(h_flex().pl_2().items_center().children(thumbs));
                if summary.count > summary.image_count {
                    col = col.child(div().text_xs().text_color(muted).child(rust_i18n::t!(
                        "inspector_not_images",
                        count = summary.count - summary.image_count
                    ).to_string()));
                }
            }
        } else {
            col = col.child(
                v_flex()
                    .items_center()
                    .gap_1()
                    .text_color(muted)
                    .child(icon(AtlasIcon::Grid, 28.0, muted))
                    .child(rust_i18n::t!("inspector_mixed_selection").to_string()),
            );
        }
        col.into_any_element()
    }
}

enum PreviewKind {
    Loading,
    Text,
    Markdown,
    Image,
    Pdf,
    Audio,
    Dir,
    Binary,
    Error,
}

// ── Selection summary (neutral; mirrors the egui inspector's SelectionSummary) ──

#[derive(Default)]
pub struct SelectionSummary {
    pub count: usize,
    pub file_count: usize,
    pub dir_count: usize,
    pub image_count: usize,
    pub total_size: u64,
    pub known_size_count: usize,
    pub single: Option<SelectedEntry>,
}

pub struct SelectedEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: FileKind,
    pub size: Option<u64>,
    pub modified: Option<std::time::SystemTime>,
    pub extension: Option<String>,
}

fn selection_summary(tab_state: &TabState, view_mode: &ViewMode) -> SelectionSummary {
    use crate::state::file_entry::FileEntry;

    let entries: Vec<&FileEntry> = if view_mode == &ViewMode::Miller {
        tab_state
            .miller
            .columns
            .get(tab_state.miller.focused)
            .and_then(|c| c.entries.get(c.primary_selected()?))
            .into_iter()
            .collect()
    } else {
        let mut selected: Vec<usize> = tab_state.selection.selected.iter().copied().collect();
        selected.sort_unstable();
        selected
            .into_iter()
            .filter_map(|idx| tab_state.entries.get(idx))
            .collect()
    };

    let mut s = SelectionSummary::default();
    for entry in &entries {
        s.count += 1;
        match entry.kind {
            FileKind::Dir => s.dir_count += 1,
            FileKind::File | FileKind::Symlink => s.file_count += 1,
        }
        if let Some(size) = entry.size {
            s.total_size += size;
            s.known_size_count += 1;
        }
        if entry
            .extension
            .as_deref()
            .is_some_and(crate::state::preview_cache::is_image_ext)
        {
            s.image_count += 1;
        }
    }
    if let [entry] = entries.as_slice() {
        s.single = Some(SelectedEntry {
            name: entry.name.clone(),
            path: entry.path.clone(),
            kind: entry.kind.clone(),
            size: entry.size,
            modified: entry.modified,
            extension: entry.extension.clone(),
        });
    }
    s
}
