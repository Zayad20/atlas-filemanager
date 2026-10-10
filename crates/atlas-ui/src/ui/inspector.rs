//! Phase5 — Inspector / preview panel (gpui port of the egui `src/ui/inspector.rs`).
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
use crate::input::action::Action;
use crate::ui::glass::{backdrop_tint, layer_color, GlassExt};
use crate::ui::icons::{file_type_icon, icon, AtlasIcon};
use crate::ui::root::DraggedFiles;
use atlas_core::util::display::{format_date, format_size};
use atlas_model::file_entry::FileKind;
use atlas_model::layout_tree::TileId;
use atlas_model::preview_cache::{
    MultiPreviewState, PreviewRequest, PreviewResult, DIR_PREVIEW_LIMIT,
};
use atlas_model::settings::ViewMode;
use atlas_model::tab::TabState;
use gpui::prelude::*;
use gpui::ColorExt as _;
use gpui::SharedString;
use gpui::{
    div, img, px, AnyElement, ClickEvent, Context, Entity, Hsla, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, RenderImage, ScrollDelta, ScrollWheelEvent, WeakEntity, Window,
};
use gpui_component::input::InputEvent;
use gpui_component::scroll::ScrollableElement;
use gpui_component::ElementExt;
use gpui_component::{h_flex, v_flex, ActiveTheme, Sizable as _};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Retained host for the preview body. The root still owns the panel shell and
/// resize/close animation, while this entity only re-renders when the focused
/// preview's visual fingerprint changes.
pub(crate) struct InspectorHost {
    app: WeakEntity<App>,
    fingerprint: u64,
    is_rendering: bool,
}

impl InspectorHost {
    pub(crate) fn new(app: Entity<App>, cx: &mut Context<Self>) -> Self {
        cx.observe(&app, |this: &mut Self, app, cx| {
            if this.is_rendering {
                return;
            }
            let next = preview_fingerprint(app.read(cx));
            if this.fingerprint != next {
                this.fingerprint = next;
                crate::perf::count("inspector.invalidate.changed");
                cx.notify();
            } else {
                crate::perf::count("inspector.invalidate.skipped");
            }
        })
        .detach();
        Self {
            app: app.downgrade(),
            fingerprint: 0,
            is_rendering: false,
        }
    }
}

impl Render for InspectorHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.is_rendering = true;
        let el = match self.app.upgrade() {
            Some(app) => app.update(cx, |app, cx| {
                app.pending_drag_hover_targets.retain(|target| !matches!(target.id, crate::ui::drag_hover::DragHoverId::Inspector(_)));
                crate::perf::count("render.inspector_body");
                app.render_inspector(window, cx)
            }),
            None => div().into_any_element(),
        };
        self.is_rendering = false;
        el
    }
}

/// The inspector only depends on the focused tab's selection/preview result,
/// display knobs, and its image/text interaction state. Hashing metadata and
/// lengths instead of RGBA/text contents keeps the observer cheap while still
/// invalidating on each meaningful result transition.
fn preview_fingerprint(app: &App) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    app.drag_hover_maybe_active().hash(&mut hasher);
    let tab = app.state.focused_tile;
    tab.hash(&mut hasher);
    app.state.settings.show_inspector.hash(&mut hasher);
    app.state.settings.show_titlebar.hash(&mut hasher);
    app.state.profiles.active().theme.hash(&mut hasher);
    app.state.settings.glass_opacity.to_bits().hash(&mut hasher);
    app.state
        .settings
        .inspector_width
        .to_bits()
        .hash(&mut hasher);
    app.preview_visible_for_focused().hash(&mut hasher);

    if let Some(tab_data) = app.state.active_tab_data() {
        tab_data.preview_enabled.hash(&mut hasher);
    }
    if let Some(tab_state) = app.state.tab_states.get(&tab) {
        tab_state.selection.selected.len().hash(&mut hasher);
        tab_state.selection.keyboard_cursor.hash(&mut hasher);
    }

    if let Some(path) = app.state.preview.paths.get(&tab) {
        1u8.hash(&mut hasher);
        path.hash(&mut hasher);
    } else {
        0u8.hash(&mut hasher);
    }

    hash_preview_result(app.state.preview.results.get(&tab), &mut hasher);

    if let Some(multi) = app.state.preview.multi.get(&tab) {
        1u8.hash(&mut hasher);
        multi.key.fingerprint.hash(&mut hasher);
        multi.key.selected_count.hash(&mut hasher);
        multi.images.len().hash(&mut hasher);
        for (path, image) in &multi.images {
            path.hash(&mut hasher);
            image.width.hash(&mut hasher);
            image.height.hash(&mut hasher);
            image.rgba.len().hash(&mut hasher);
        }
    } else {
        0u8.hash(&mut hasher);
    }

    if let Some(view) = app.preview_view.get(&tab) {
        view.zoom.to_bits().hash(&mut hasher);
        view.pan_x.to_bits().hash(&mut hasher);
        view.pan_y.to_bits().hash(&mut hasher);
        view.expanded.hash(&mut hasher);
    }

    hasher.finish()
}

fn hash_preview_result(
    result: Option<&PreviewResult>,
    hasher: &mut std::collections::hash_map::DefaultHasher,
) {
    match result {
        None => 0u8.hash(hasher),
        Some(PreviewResult::Text { lines, text }) => {
            1u8.hash(hasher);
            lines.len().hash(hasher);
            text.len().hash(hasher);
        }
        Some(PreviewResult::Markdown {
            blocks,
            source,
            base_uri,
        }) => {
            2u8.hash(hasher);
            blocks.len().hash(hasher);
            source.len().hash(hasher);
            base_uri.hash(hasher);
        }
        Some(PreviewResult::Image {
            rgba,
            width,
            height,
            original_width,
            original_height,
            exif,
        }) => {
            3u8.hash(hasher);
            rgba.len().hash(hasher);
            width.hash(hasher);
            height.hash(hasher);
            original_width.hash(hasher);
            original_height.hash(hasher);
            exif.len().hash(hasher);
        }
        Some(PreviewResult::Pdf {
            rgba,
            width,
            height,
        }) => {
            4u8.hash(hasher);
            rgba.len().hash(hasher);
            width.hash(hasher);
            height.hash(hasher);
        }
        Some(PreviewResult::Table {
            headers,
            rows,
            truncated,
        }) => {
            5u8.hash(hasher);
            headers.len().hash(hasher);
            rows.len().hash(hasher);
            truncated.hash(hasher);
        }
        Some(PreviewResult::Document {
            title,
            blocks,
            truncated,
        }) => {
            6u8.hash(hasher);
            title.hash(hasher);
            blocks.len().hash(hasher);
            truncated.hash(hasher);
        }
        Some(PreviewResult::Presentation { slides, truncated }) => {
            7u8.hash(hasher);
            slides.len().hash(hasher);
            slides
                .iter()
                .map(|slide| slide.body.len())
                .sum::<usize>()
                .hash(hasher);
            truncated.hash(hasher);
        }
        Some(PreviewResult::Audio {
            title,
            artist,
            album,
            duration_secs,
            bitrate,
            cover,
        }) => {
            8u8.hash(hasher);
            title.hash(hasher);
            artist.hash(hasher);
            album.hash(hasher);
            duration_secs.hash(hasher);
            bitrate.hash(hasher);
            cover
                .as_ref()
                .map(|image| (image.width, image.height, image.rgba.len()))
                .hash(hasher);
        }
        Some(PreviewResult::Dir {
            entries,
            total_count,
            total_size,
        }) => {
            9u8.hash(hasher);
            entries.len().hash(hasher);
            total_count.hash(hasher);
            total_size.hash(hasher);
        }
        Some(PreviewResult::Binary { size }) => {
            10u8.hash(hasher);
            size.hash(hasher);
        }
        Some(PreviewResult::Error(message)) => {
            11u8.hash(hasher);
            message.hash(hasher);
        }
    }
}

impl App {
    /// Get-or-create the retained preview body host. The surrounding panel
    /// remains in `root.rs` so width and exit animations continue to work.
    pub(crate) fn ensure_inspector_host(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Entity<InspectorHost> {
        if let Some(host) = &self.inspector_host {
            return host.clone();
        }
        let app = cx.entity();
        let host = cx.new(|cx| InspectorHost::new(app, cx));
        self.inspector_host = Some(host.clone());
        host
    }
}

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
    // ── Code editor lifecycle ────────────────────────────────────────────────

    /// Ensure `self.preview_editor` exists and matches `path`/`lang`. If the path or
    /// language changed (new file selected), the old editor is dropped and a fresh
    /// `InputState` is created. Returns `true` when the editor is ready.
    fn ensure_preview_editor(
        &mut self,
        path: PathBuf,
        text: &str,
        lang: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let same = self.preview_editor.as_ref().is_some_and(|e| e.path == path);
        if same {
            return true;
        }
        // New file — build a fresh code editor.
        let state_entity = crate::ui::text_input::code_editor(
            window,
            cx,
            gpui::SharedString::from(lang.to_string()),
            text,
            false,
        );
        let text_str = text.to_string();
        let sub = cx.subscribe_in(
            &state_entity,
            window,
            |this, st, ev: &InputEvent, _w, cx| {
                if matches!(ev, InputEvent::Change) {
                    let new_text = st.read(cx).value().to_string();
                    if let Some(editor) = &mut this.preview_editor {
                        editor.cached_text = new_text;
                        editor.dirty = true;
                    }
                    cx.notify();
                }
            },
        );
        self.preview_editor = Some(crate::app::PreviewEditor {
            state: state_entity,
            path,
            cached_text: text_str,
            dirty: false,
            editing: true,
            line_numbers: false,
            ln_needs_sync: false,
            _sub: sub,
        });
        true
    }

    // ── Preview loading (gpui variants, no egui ctx) ─────────────────────────

    /// gpui replacement for `sync_active_preview(ctx)`: decides what (if anything)
    /// should be previewed for the focused tab and kicks off a debounced load. The
    /// drain loop handles repaints, so no `ctx.request_repaint` is needed.
    pub(crate) fn sync_active_preview_gpui(&mut self) {
        let tab = self.state.focused_tile;
        if !self.preview_visible_for_focused() {
            return;
        }
        let request = self.state.active_preview_request();
        let Some(request) = request else {
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

        // A render only happens when state changed (selection/nav already called
        // `cx.notify()`), so we kick the load immediately. Superseded loads are
        // discarded on arrival (`BgResult::PreviewReady` checks `preview.paths`),
        // so fast arrow-key scrolling is still safe.
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

    pub(crate) fn load_preview_gpui(&mut self, tab: TileId, path: PathBuf) {
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
        let previewer = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|ext| self.ext_registry.previewer_for(ext));
        if let Some(cmd) = previewer {
            atlas_core::ext::runner::run_preview(&self.rt, tx, cmd, tab, path);
        } else if let Some((extension, missing)) = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|ext| self.ext_registry.missing_preview_tools_for(ext))
        {
            let installing = if let Some(request) = path
                .extension()
                .and_then(|e| e.to_str())
                .and_then(|ext| self.ext_registry.missing_preview_tool_install_for(ext))
            {
                let tx2 = tx.clone();
                let retry_path = path.clone();
                let tool = request.tool.name.clone();
                self.rt.spawn(async move {
                    let result = atlas_core::ext::marketplace::install_tool(
                        request.provider_dir,
                        request.tool,
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                    let _ = tx2.send(atlas_model::app_state::BgResult::ExtToolInstalled {
                        tool,
                        tab,
                        path: retry_path,
                        result,
                    });
                });
                true
            } else {
                false
            };
            let tools = missing
                .iter()
                .map(|name| tool_label(name))
                .collect::<Vec<_>>()
                .join(", ");
            let result = PreviewResult::Error(if installing {
                format!("Installing {tools} for {extension}...")
            } else {
                format!("{extension} needs {tools}. Update Atlas Runtime Tools.")
            });
            let _ = tx.send(atlas_model::app_state::BgResult::PreviewReady { tab, path, result });
        } else {
            self.rt.spawn(async move {
                atlas_core::fs::preview::load_preview(tab, path, tx).await;
            });
        }
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
            atlas_core::fs::preview::load_archive_preview(tab, archive, inner, tx).await;
        });
    }

    fn load_multi_preview_gpui(
        &mut self,
        tab: TileId,
        key: atlas_model::preview_cache::MultiPreviewKey,
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
                atlas_core::fs::preview::load_multi_thumbnail(tab, key, index, path, tx).await;
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

        let (fg, muted, border, danger) = {
            let t = cx.theme();
            (t.foreground, t.muted_foreground, t.border, t.danger)
        };
        // Phase6: expanded preview is rendered as an overlay by root.rs, so widening
        // it here no longer pushes the main dock/sidebar during window resize.
        let expanded = self
            .preview_view
            .get(&self.state.focused_tile)
            .is_some_and(|v| v.expanded);
        let viewport = window.viewport_size();
        let width = if expanded {
            let max_width = (f32::from(viewport.width) - 32.0).max(260.0);
            self.state
                .settings
                .inspector_width
                .max(180.0)
                .min(max_width)
        } else {
            self.state
                .settings
                .inspector_width
                .max(180.0)
                .min((f32::from(viewport.width) - 96.0).max(180.0))
        };
        let box_w = (width - 24.0).max(80.0);
        let available_h = (f32::from(viewport.height) - 190.0).max(360.0);
        let box_h = if expanded {
            (f32::from(viewport.height) - 120.0).max(520.0)
        } else {
            available_h
        };

        let summary = self
            .state
            .active_tab_state()
            .zip(self.state.active_tab_data())
            .map(|(tab, data)| selection_summary(tab, &data.view_mode))
            .unwrap_or_default();

        let active_theme = self.state.profiles.active().theme.as_str();
        let is_glass = active_theme.ends_with(" Glass");
        let (panel_bg, panel_border) = {
            let t = cx.theme();
            (
                if is_glass {
                    t.sidebar
                } else {
                    backdrop_tint(t.secondary, 0.10)
                },
                t.border,
            )
        };
        let mut col = v_flex()
            .w(px(width))
            .h_full()
            .text_color(fg)
            .bg(panel_bg)
            .rounded(px(10.))
            .when(!is_glass && !crate::ui::icons::is_windows_theme(active_theme), |s| {
                s.border_1().border_color(panel_border.opacity(0.28))
            })
            .when(is_glass, |s| s.glass_surface(panel_bg))
            .p_3()
            .when(self.state.settings.show_titlebar, |s| {
                s.pt(crate::ui::rem_px(
                    if crate::ui::icons::is_windows_theme(active_theme) { 46.0 } else { 34.0 },
                ))
            })
            .gap_3();

        if summary.count == 0 {
            return col
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_color(muted)
                        .child(rust_i18n::t!("inspector_no_selection").to_string()),
                )
                .into_any_element();
        }

        if summary.count > 1 {
            col = col.child(self.render_multi(&summary, muted, window, cx));
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
                                .w(gpui::px(18.))
                                .h(gpui::px(18.))
                                .flex_none()
                                .into_any_element(),
                            None => file_type_icon(&entry.kind, entry.extension.as_deref(), 18.0),
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
                meta = meta
                    .child(rust_i18n::t!("inspector_size", size = format_size(size)).to_string());
            }
            if let Some(modified) = entry.modified {
                meta = meta.child(
                    rust_i18n::t!("inspector_modified", date = format_date(modified)).to_string(),
                );
            }
            if let Some(ext) = &entry.extension {
                meta = meta.child(rust_i18n::t!("inspector_type", ext = ext).to_string());
            }
            col = col.child(meta);
        }

        // Capture the single-selection path before the mutable borrows below.
        let single_path: Option<PathBuf> = summary.single.as_ref().map(|e| e.path.clone());

        col = col.child(div().h(px(1.)).w_full().bg(border.opacity(0.35)));

        // ── Preview body ─────────────────────────────────────────────────────
        col = col.child(self.render_preview_body(muted, danger, box_w, box_h, window, cx));

        // Tags + add-tag action (all file types, single selection).
        if let Some(ref path) = single_path {
            col = col.child(self.render_inspector_tags(path, muted, cx));
            col = col.child(self.render_tag_section(vec![path.clone()], muted, cx));
        }

        col.into_any_element()
    }

    fn render_floating_preview_controls(
        &mut self,
        kind: PreviewKind,
        copy_text: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_image = matches!(kind, PreviewKind::Image | PreviewKind::Pdf);
        if !is_image && copy_text.is_none() {
            return div().into_any_element();
        }

        let (surface, border, fg, muted) = {
            let t = cx.theme();
            (t.secondary, t.border, t.foreground, t.muted_foreground)
        };
        let view = cx.entity();

        let mut bar = h_flex()
            .absolute()
            .top(px(10.))
            .right(px(10.))
            .gap_1()
            .items_center()
            .p(px(3.))
            .rounded(px(999.))
            .bg(layer_color(surface, 0.06, 0.72))
            .border_1()
            .border_color(border.opacity(0.24))
            .raised_surface()
            .invisible()
            .group_hover("preview-surface", gpui::Styled::visible);

        if is_image {
            let v_zo = view.clone();
            bar = bar.child(
                floating_icon_button(
                    "float-zout",
                    gpui_component::IconName::Minus,
                    rust_i18n::t!("inspector_preview_zoom_out").to_string(),
                    false,
                    fg,
                    muted,
                )
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    v_zo.update(cx, |this, _cx| {
                        this.process_actions(vec![Action::PreviewZoomOut]);
                    });
                }),
            );

            let v_fit = view.clone();
            bar = bar.child(
                floating_icon_button(
                    "float-fit",
                    gpui_component::IconName::Maximize,
                    rust_i18n::t!("inspector_preview_zoom_fit").to_string(),
                    false,
                    fg,
                    muted,
                )
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    v_fit.update(cx, |this, _cx| {
                        this.process_actions(vec![Action::PreviewZoomFit]);
                    });
                }),
            );

            let v_zi = view.clone();
            bar = bar.child(
                floating_icon_button(
                    "float-zin",
                    gpui_component::IconName::Plus,
                    rust_i18n::t!("inspector_preview_zoom_in").to_string(),
                    false,
                    fg,
                    muted,
                )
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    v_zi.update(cx, |this, _cx| {
                        this.process_actions(vec![Action::PreviewZoomIn]);
                    });
                }),
            );
        }

        if let Some(text) = copy_text {
            bar = bar.child(
                gpui_component::clipboard::Clipboard::new("preview-copy")
                    .value(text)
                    .tooltip(rust_i18n::t!("inspector_preview_copy").to_string()),
            );
        }

        bar.into_any_element()
    }

    /// Tags shared by ALL paths in a multiselect (intersection). Empty when none.
    fn render_multi_tags(
        &mut self,
        paths: &[PathBuf],
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if paths.is_empty() {
            return div().into_any_element();
        }
        // Intersection: tags present in every path's tag list.
        let first_ids = self.state.tag_cache.tags_of(&paths[0]).to_vec();
        let shared: Vec<atlas_model::tag_cache::TagId> = first_ids
            .into_iter()
            .filter(|id| {
                paths[1..]
                    .iter()
                    .all(|p| self.state.tag_cache.tags_of(p).contains(id))
            })
            .collect();
        if shared.is_empty() {
            return div().into_any_element();
        }
        let profile_name = self.state.profiles.active().name.clone();
        let chips: Vec<AnyElement> = shared
            .iter()
            .filter_map(|&id| {
                self.state
                    .tag_cache
                    .visible_defs(&profile_name)
                    .find(|d| d.id == id)
                    .map(|d| {
                        let [r, g, b] = d.color;
                        let swatch = gpui::rgb(((r as u32) << 16) | ((g as u32) << 8) | (b as u32));
                        h_flex()
                            .gap_1()
                            .px(px(6.))
                            .py(px(2.))
                            .rounded_full()
                            .border_1()
                            .border_color(swatch)
                            .text_xs()
                            .text_color(muted)
                            .child(
                                div()
                                    .w(px(8.))
                                    .h(px(8.))
                                    .rounded_full()
                                    .flex_none()
                                    .bg(swatch),
                            )
                            .child(SharedString::from(d.name.clone()))
                            .into_any_element()
                    })
            })
            .collect();
        let _ = cx;
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_1()
            .children(chips)
            .into_any_element()
    }

    /// Renders the row of tag chips assigned to `path`. Empty when no tags are known.
    fn render_inspector_tags(
        &mut self,
        path: &std::path::Path,
        muted: Hsla,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let tag_ids = self.state.tag_cache.tags_of(path).to_vec();
        if tag_ids.is_empty() {
            return div().into_any_element();
        }

        let profile_name = self.state.profiles.active().name.clone();
        let chips: Vec<AnyElement> = tag_ids
            .iter()
            .filter_map(|&id| {
                self.state
                    .tag_cache
                    .visible_defs(&profile_name)
                    .find(|d| d.id == id)
                    .map(|d| {
                        let [r, g, b] = d.color;
                        let swatch_color =
                            gpui::rgb(((r as u32) << 16) | ((g as u32) << 8) | (b as u32));
                        let name = SharedString::from(d.name.clone());
                        h_flex()
                            .gap_1()
                            .px(px(6.))
                            .py(px(2.))
                            .rounded_full()
                            .border_1()
                            .border_color(swatch_color)
                            .text_xs()
                            .text_color(muted)
                            .child(
                                div()
                                    .w(px(8.))
                                    .h(px(8.))
                                    .rounded_full()
                                    .flex_none()
                                    .bg(swatch_color),
                            )
                            .child(name)
                            .into_any_element()
                    })
            })
            .collect();

        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_1()
            .children(chips)
            .into_any_element()
    }

    /// Renders the add-tag button for any file type / multiselect.
    fn render_tag_section(
        &mut self,
        paths: Vec<PathBuf>,
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if paths.is_empty() {
            return div().into_any_element();
        }
        div()
            .id("tag-add-btn")
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .rounded(px(crate::ui::RADIUS))
            .text_sm()
            .text_color(muted)
            .cursor_pointer()
            .hover(|s| s.opacity(0.8))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    this.open_tag_picker(ev.position, paths.clone(), window, cx);
                }),
            )
            .child(icon(AtlasIcon::Tag, 14.0, muted))
            .child(rust_i18n::t!("inspector_add_tag").to_string())
            .into_any_element()
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
            Some(PreviewResult::Table { .. }) => PreviewKind::Table,
            Some(PreviewResult::Document { .. }) => PreviewKind::Document,
            Some(PreviewResult::Presentation { .. }) => PreviewKind::Presentation,
            Some(PreviewResult::Audio { .. }) => PreviewKind::Audio,
            Some(PreviewResult::Dir { .. }) => PreviewKind::Dir,
            Some(PreviewResult::Binary { .. }) => PreviewKind::Binary,
            Some(PreviewResult::Error(_)) => PreviewKind::Error,
        };

        match kind {
            PreviewKind::Loading => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child(rust_i18n::t!("inspector_loading").to_string())
                .into_any_element(),
            PreviewKind::Text => self.render_text_preview(muted, window, cx),
            PreviewKind::Markdown => self.render_markdown_preview(muted, window, cx),
            PreviewKind::Image | PreviewKind::Pdf => {
                self.render_image_preview(muted, box_w, box_h, cx)
            }
            PreviewKind::Table => self.render_table_preview(muted, cx),
            PreviewKind::Document => self.render_document_preview(muted),
            PreviewKind::Presentation => self.render_presentation_preview(muted),
            PreviewKind::Audio => self.render_audio_preview(muted),
            PreviewKind::Dir => self.render_dir_preview(muted, cx),
            PreviewKind::Binary => {
                let size = match self.state.preview.results.get(&tab) {
                    Some(PreviewResult::Binary { size }) => *size,
                    _ => 0,
                };
                div()
                    .flex_1()
                    .flex()
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
                let lines = wrap_preview_error(&msg);
                v_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .p_2()
                    .gap_1()
                    .text_color(danger)
                    .items_center()
                    .justify_center()
                    .children(lines.into_iter().map(|line| {
                        div()
                            .w_full()
                            .min_w(px(0.))
                            .text_center()
                            .child(line)
                            .into_any_element()
                    }))
                    .into_any_element()
            }
        }
    }

    /// Code/text preview with real syntax highlighting via gpui-component's code editor.
    fn render_text_preview(
        &mut self,
        _muted: Hsla,
        window: &mut Window,
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
        let lang = preview_language_for_path(&path);

        // Ensure the editor entity exists (creates lazily or on file change).
        self.ensure_preview_editor(path, &raw, lang, window, cx);

        let Some(editor) = self.preview_editor.as_ref() else {
            return div().into_any_element();
        };

        let state_ref = editor.state.clone();
        let raw_for_copy = raw.clone();

        let (surface, border) = {
            let t = cx.theme();
            (t.secondary, t.border)
        };

        v_flex()
            .id("inspector-text")
            .group("preview-surface")
            .relative()
            .flex_1()
            .min_h(px(0.))
            .gap_0()
            .bg(layer_color(surface, -0.02, 0.30))
            .border_1()
            .border_color(border.opacity(0.18))
            .rounded(px(10.))
            .overflow_hidden()
            .child(
                gpui_component::input::Input::new(&state_ref)
                    .h_full()
                    .appearance(false),
            )
            .child(self.render_floating_preview_controls(PreviewKind::Text, Some(raw_for_copy), cx))
            .into_any_element()
    }

    fn render_markdown_preview(
        &mut self,
        _muted: Hsla,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab = self.state.focused_tile;
        let source = match self.state.preview.results.get(&tab) {
            Some(PreviewResult::Markdown { source, .. }) => source.clone(),
            _ => return div().into_any_element(),
        };
        let (smooth_ease, smooth_boost) =
            crate::app::scroll_smooth_params(self.state.settings.scroll_smoothness);

        v_flex()
            .flex_1()
            .min_h(px(0.))
            .child(
                div()
                    .id("inspector-md")
                    .group("preview-surface")
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        gpui_component::text::TextView::markdown(
                            gpui::SharedString::from(format!("preview-md-{:?}", tab)),
                            source.clone(),
                        )
                        .selectable(true)
                        .scrollable(true)
                        .scroll_smooth(smooth_ease, smooth_boost)
                        .h_full()
                        .p_0(),
                    )
                    .child(self.render_floating_preview_controls(
                        PreviewKind::Markdown,
                        Some(source),
                        cx,
                    )),
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
        let (_ow, _oh, dw, dh, exif, is_pdf) = match self.state.preview.results.get(&tab) {
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
        let (well, border) = {
            let t = cx.theme();
            (backdrop_tint(t.secondary, 0.16), t.border)
        };
        let kind = if is_pdf {
            PreviewKind::Pdf
        } else {
            PreviewKind::Image
        };

        let mut col = v_flex().flex_1().min_h(px(0.)).gap_2();

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
                    .group("preview-surface")
                    .relative()
                    .overflow_hidden()
                    .w(px(box_w))
                    .h(px(box_h))
                    .bg(well)
                    .rounded(px(10.))
                    .border_1()
                    .border_color(border.opacity(0.18))
                    .child(image)
                    .child(self.render_floating_preview_controls(kind, None, cx))
                    .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _w, cx| {
                        let dy = match ev.delta {
                            ScrollDelta::Pixels(p) => f32::from(p.y),
                            ScrollDelta::Lines(l) => l.y,
                        };
                        if dy != 0.0 {
                            let tab = this.state.focused_tile;
                            let v = this.preview_view.entry(tab).or_default();
                            let factor = if dy > 0.0 { 1.1 } else { 1.0 / 1.1 };
                            v.zoom = (v.zoom * factor).clamp(0.1, 48.0);
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

    fn render_table_preview(&mut self, muted: Hsla, cx: &mut Context<Self>) -> AnyElement {
        let tab = self.state.focused_tile;
        let Some(PreviewResult::Table {
            headers,
            rows,
            truncated,
        }) = self.state.preview.results.get(&tab)
        else {
            return div().into_any_element();
        };
        let is_glass = self.state.profiles.active().theme.ends_with(" Glass");
        let (border, stripe, header_bg, fg) = {
            let t = cx.theme();
            (
                t.border,
                if is_glass {
                    t.list_even
                } else {
                    backdrop_tint(t.list_even, 0.08)
                },
                if is_glass {
                    t.list_head
                } else {
                    layer_color(t.secondary, 0.04, 0.88)
                },
                t.foreground,
            )
        };
        let cols = headers
            .len()
            .max(rows.iter().map(Vec::len).max().unwrap_or(0))
            .min(50);
        let mut table = v_flex().min_w(px((cols as f32).max(1.0) * 120.0)).text_xs();
        if !headers.is_empty() {
            let mut row = h_flex().h(px(28.)).bg(header_bg).text_color(muted);
            for cell in headers.iter().take(cols) {
                row = row.child(div().w(px(120.)).px_2().truncate().child(cell.clone()));
            }
            table = table.child(row);
        }
        for (i, cells) in rows.iter().take(200).enumerate() {
            let mut row = h_flex()
                .h(px(26.))
                .text_color(fg)
                .border_t_1()
                .border_color(border.opacity(0.35))
                .when(i % 2 == 1, |s| s.bg(stripe));
            for col in 0..cols {
                row = row.child(
                    div()
                        .w(px(120.))
                        .px_2()
                        .truncate()
                        .child(cells.get(col).cloned().unwrap_or_default()),
                );
            }
            table = table.child(row);
        }
        let mut wrap = v_flex().flex_1().min_h(px(0.)).gap_2();
        if *truncated {
            wrap = wrap.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("preview_truncated")),
            );
        }
        wrap.child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_x_scrollbar()
                .child(div().h_full().overflow_y_scrollbar().child(table)),
        )
        .into_any_element()
    }

    fn render_document_preview(&mut self, muted: Hsla) -> AnyElement {
        let tab = self.state.focused_tile;
        let Some(PreviewResult::Document {
            title,
            blocks,
            truncated,
        }) = self.state.preview.results.get(&tab)
        else {
            return div().into_any_element();
        };
        let mut page = v_flex()
            .gap_3()
            .p_4()
            .w_full()
            .max_w(px(560.))
            .min_w(px(0.))
            .min_h(px(720.))
            .bg(gpui::white())
            .text_color(gpui::black())
            .rounded(px(3.))
            .border_1()
            .border_color(muted.opacity(0.25))
            .raised_surface()
            .text_sm();
        if !title.is_empty() {
            page = page.child(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title.clone()),
            );
        }
        for (i, block) in blocks.iter().take(200).enumerate() {
            let is_heading =
                i == 0 && title.is_empty() || block.len() < 80 && !block.ends_with('.');
            page = page.child(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .when(is_heading, |s| {
                        s.font_weight(gpui::FontWeight::SEMIBOLD).text_lg()
                    })
                    .child(block.clone()),
            );
        }
        if *truncated {
            page = page.child(div().text_xs().text_color(muted).child("Preview truncado"));
        }
        div()
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scrollbar()
            .child(v_flex().w_full().items_center().p_3().child(page))
            .into_any_element()
    }

    fn render_presentation_preview(&mut self, muted: Hsla) -> AnyElement {
        let tab = self.state.focused_tile;
        let Some(PreviewResult::Presentation { slides, truncated }) =
            self.state.preview.results.get(&tab)
        else {
            return div().into_any_element();
        };
        let mut col = v_flex()
            .flex_1()
            .min_h(px(0.))
            .gap_3()
            .overflow_y_scrollbar()
            .items_center()
            .p_2()
            .text_sm();
        for (i, slide) in slides.iter().enumerate() {
            let title = if slide.title.is_empty() {
                format!("Slide {}", i + 1)
            } else {
                slide.title.clone()
            };
            let mut card = v_flex()
                .gap_2()
                .w_full()
                .max_w(px(520.))
                .min_w(px(0.))
                .min_h(px(220.))
                .p_4()
                .bg(gpui::white())
                .text_color(gpui::black())
                .rounded(px(4.))
                .border_1()
                .border_color(muted.opacity(0.25))
                .raised_surface();
            card = card.child(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .text_color(muted)
                    .text_xs()
                    .child(format!("Slide {}", i + 1)),
            );
            card = card.child(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title),
            );
            for line in &slide.body {
                card = card.child(
                    h_flex()
                        .w_full()
                        .min_w(px(0.))
                        .gap_2()
                        .child("•")
                        .child(div().min_w(px(0.)).child(line.clone())),
                );
            }
            col = col.child(card);
        }
        if *truncated {
            col = col.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("preview_truncated")),
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
            col = col.child(
                img(tex)
                    .max_w(px(220.))
                    .max_h(px(220.))
                    .rounded_md()
                    .raised_surface(),
            );
        } else {
            col = col.child(
                h_flex()
                    .w(px(220.))
                    .h(px(220.))
                    .rounded_md()
                    .bg(muted)
                    .opacity(0.15)
                    .raised_surface()
                    .items_center()
                    .justify_center()
                    .child(icon(AtlasIcon::Music, 44.0, muted)),
            );
        }

        let mut meta = v_flex().gap_1();
        meta = meta.child(
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title.unwrap_or_else(|| rust_i18n::t!("untitled").into())),
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

    fn render_dir_preview(&mut self, muted: Hsla, cx: &mut Context<Self>) -> AnyElement {
        let tab = self.state.focused_tile;
        let view = cx.entity();
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
            let path = e.path.clone();
            let is_dir = e.is_dir;
            let size = e.size.map(format_size).unwrap_or_default();
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!(
                        "inspector-dir-row-{}",
                        path.display()
                    )))
                    .gap_2()
                    .items_center()
                    .rounded(px(6.))
                    .px_1()
                    .py(px(2.))
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.82))
                    .child(
                        div().w(px(16.)).child(
                            match self.icon_tex_for_entry(&row_kind, e.extension.as_deref()) {
                                Some(tex) => gpui::img(tex)
                                    .w(px(14.))
                                    .h(px(14.))
                                    .flex_none()
                                    .into_any_element(),
                                None => file_type_icon(&row_kind, e.extension.as_deref(), 14.0),
                            },
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .child(e.name.clone()),
                    )
                    .child(div().text_color(muted).child(size))
                    .on_click({
                        let view = view.clone();
                        let path = path.clone();
                        move |ev, _window, cx| {
                            if ev.click_count() < 2 {
                                return;
                            }
                            view.update(cx, |this, cx| {
                                let tile = this.state.focused_tile;
                                if is_dir {
                                    this.navigate(tile, path.clone());
                                } else {
                                    this.process_actions(vec![Action::Open(vec![path.clone()])]);
                                }
                                cx.notify();
                            });
                        }
                    })
                    .on_drag(
                        crate::ui::drag::file_drag_value(vec![path.clone()]),
                        crate::ui::drag::file_drag_handler(view.clone(), path.clone()),
                    )
                    .external_drag_payload(crate::ui::drag::file_external_drag_payload(view.clone()))
                    .when(is_dir, {
                        let dest_move = path.clone();
                        let dest_copy = path.clone();
                        let mv = view.clone();
                        let cpv = view.clone();
                        move |row| {
                            use crate::ui::drag_hover::{drag_hover_recorder_with_drop, DragHoverId, DragHoverKind, DragHoverCommand, DragHoverDropCommand};
                            row.on_prepaint_child(drag_hover_recorder_with_drop(
                                DragHoverId::Inspector(dest_move.clone()),
                                DragHoverKind::Folder,
                                DragHoverCommand::NavigatePathIn(tab, dest_move.clone()),
                                DragHoverDropCommand::TransferTo(dest_move.clone()),
                                &mv,
                            )).on_drop::<DraggedFiles>(move |d, _window, cx| {
                                let (paths, dest) = (d.paths.clone(), dest_move.clone());
                                mv.update(cx, |this, cx| {
                                    this.execute_drag_hover_drop_command(DragHoverDropCommand::TransferTo(dest), paths, true);
                                    cx.notify();
                                });
                            })
                            .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
                                let (paths, dest) = (paths.paths().to_vec(), dest_copy.clone());
                                cpv.update(cx, |this, cx| {
                                    this.execute_drag_hover_drop_command(DragHoverDropCommand::TransferTo(dest), paths, false);
                                    cx.notify();
                                });
                            })
                        }
                    }),
            );
        }
        if *total_count > DIR_PREVIEW_LIMIT {
            list = list.child(
                div().text_color(muted).child(
                    rust_i18n::t!(
                        "inspector_and_more",
                        count = total_count - DIR_PREVIEW_LIMIT
                    )
                    .to_string(),
                ),
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
                    .child(
                        rust_i18n::t!("inspector_elements_count", count = total_count).to_string(),
                    )
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let textures = self.focused_multi_textures();
        let mut col = v_flex().flex_1().min_h(px(0.)).gap_2();
        col = col.child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(
                    rust_i18n::t!("inspector_elements_selected", count = summary.count).to_string(),
                ),
        );
        let mut meta = h_flex().gap_2().text_xs().text_color(muted);
        if summary.file_count > 0 {
            meta = meta.child(
                rust_i18n::t!("inspector_files_count", count = summary.file_count).to_string(),
            );
        }
        if summary.dir_count > 0 {
            meta = meta.child(
                rust_i18n::t!("inspector_folders_count", count = summary.dir_count).to_string(),
            );
        }
        if summary.known_size_count > 0 {
            meta = meta.child(format_size(summary.total_size));
        }
        col = col.child(meta);

        if summary.image_count > 0 {
            if textures.is_empty() {
                col = col.child(
                    div()
                        .text_color(muted)
                        .child(rust_i18n::t!("inspector_loading_thumbnails").to_string()),
                );
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
                    col = col.child(
                        div().text_xs().text_color(muted).child(
                            rust_i18n::t!(
                                "inspector_not_images",
                                count = summary.count - summary.image_count
                            )
                            .to_string(),
                        ),
                    );
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

        // Tags shared by the selection + add-tag action.
        let paths = self.selected_paths();
        col = col.child(self.render_multi_tags(&paths, muted, cx));
        col = col.child(self.render_tag_section(paths, muted, cx));
        let _ = window; // kept for future use
        col.into_any_element()
    }
}

pub(crate) fn tool_label(name: &str) -> String {
    match name {
        "magick" => "ImageMagick".into(),
        "pdftoppm" => "PDF renderer".into(),
        "ffmpeg" => "FFmpeg".into(),
        "gs" => "Ghostscript".into(),
        other => other.to_string(),
    }
}

enum PreviewKind {
    Loading,
    Text,
    Markdown,
    Image,
    Pdf,
    Table,
    Document,
    Presentation,
    Audio,
    Dir,
    Binary,
    Error,
}

fn wrap_preview_error(msg: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in msg.split_whitespace() {
        if !line.is_empty() && line.len() + word.len() + 1 > 52 {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push("Preview unavailable".to_string());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::wrap_preview_error;

    #[test]
    fn preview_errors_wrap_to_short_lines() {
        let lines =
            wrap_preview_error("PSD preview unavailable. Install a PSD decoder preview extension.");
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| line.len() <= 52));
    }
}

fn floating_icon_button(
    id: &'static str,
    icon_name: gpui_component::IconName,
    tooltip: String,
    disabled: bool,
    active_color: Hsla,
    muted: Hsla,
) -> gpui::Stateful<gpui::Div> {
    let color = if disabled { muted } else { active_color };
    let el = div()
        .id(gpui::SharedString::from(id))
        .flex()
        .items_center()
        .justify_center()
        .w(px(24.))
        .h(px(24.))
        .rounded(px(6.))
        .text_color(color)
        .cursor_pointer();
    if !disabled {
        el.hover(|s| s.opacity(0.75))
    } else {
        el
    }
    .child(
        gpui_component::Icon::new(icon_name)
            .with_size(gpui_component::Size::XSmall)
            .text_color(color),
    )
    .tooltip(move |window, cx| {
        gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
    })
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
    use atlas_model::file_entry::FileEntry;

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
            .is_some_and(atlas_model::preview_cache::is_image_ext)
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
