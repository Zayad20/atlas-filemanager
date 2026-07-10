//! Grid (icons) view: `render_grid` + grid cell + thumbnail predicate helpers.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use crate::app::App;
use crate::ui::drag_hover::DragHoverId;
use crate::ui::file_icons::FileIcon;
use crate::ui::glass::{backdrop_tint, glass_fill, glass_header_fill};
use crate::ui::root::{ctrl_scroll, pane_bounds_recorder, rect_recorder};
use crate::ui::views::virtual_grid::AnimatedVirtualGrid;
use atlas_model::file_entry::{FileEntry, FileKind};
use atlas_model::pane::TabId;
use gpui::prelude::*;
use gpui::{
    div, img, px, AbsoluteLength, AnyElement, Context, Entity, Hsla, MouseButton, SharedString,
    Window,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex, ActiveTheme, ElementExt};
use std::ops::Range;

pub(crate) const GRID_GAP: f32 = 8.0;
pub(crate) const GRID_PAD: f32 = 0.0;
pub(crate) const GRID_ICON_AREA: f32 = 0.72;
pub(crate) const GRID_PREVIEW_MAX: f32 = 0.70;
pub(crate) const GRID_TEXTURE_ICON_MAX: f32 = 0.66;
pub(crate) const GRID_BUILTIN_ICON: f32 = 0.54;
pub(crate) const GRID_LABEL_H: f32 = 32.0;

impl App {
    /// Icons (Grid) view: a virtualized grid of fixed-size cells. Column count is
    /// derived from the viewport width and the per-tab thumbnail size, so it stays
    /// virtualized (only visible rows built) for huge directories.
    pub(crate) fn render_grid(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        crate::perf::count("render.view.grid");
        let view = cx.entity();
        let figma_glass = self.state.profiles.active().theme == "Atlas Figma Glass";
        let (fg, accent, accent_fg, hover, muted, border, header_bg) = {
            let t = cx.theme();
            (
                t.foreground,
                t.accent,
                t.accent_foreground,
                t.list_active,
                t.muted_foreground,
                t.border,
                if figma_glass {
                    glass_header_fill()
                } else {
                    backdrop_tint(t.secondary, 0.18)
                },
            )
        };
        let crumb_strip = self.render_crumb_strip(pane_id, &view, fg, muted, header_bg);
        // `thumb` stays in rem units (same meaning as before the zoom refactor) —
        // it flows into `render_cell`/`FileIcon::element`, which resolve it via
        // `rem_px()`. The virtualization math below runs against real measured
        // pixels (pane_w, scroll offsets), so it needs its own scaled copy.
        let thumb = self
            .tab_data_for(pane_id)
            .map_or(128.0, |t| t.thumbnail_size);
        let zoom_scale = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        let thumb_px = thumb * zoom_scale;
        let count = self
            .state
            .tab_states
            .get_mut(&pane_id)
            .map_or(0, |ts| ts.visible_indices().len());
        let grid_gap_px = GRID_GAP * zoom_scale;
        let grid_pad_px = GRID_PAD * zoom_scale;
        // Use the width captured by on_prepaint from the previous frame. This
        // reflects the real pane size and adapts to preview panels and splitter
        // drags. Fall back to a viewport estimate on the very first frame before
        // any paint has run.
        let pane_w = self
            .state
            .tab_states
            .get(&pane_id)
            .map(|ts| ts.last_pane_width)
            .filter(|&w| w > 1.0)
            .unwrap_or_else(|| (f32::from(window.viewport_size().width) - 217.0).max(0.0));
        let inner_avail = (pane_w - 2.0 * grid_pad_px).max(thumb_px);
        // n*(thumb+GAP) - GAP = inner_avail  →  n = ⌊(inner_avail + GAP) / (thumb + GAP)⌋
        let cols =
            (((inner_avail + grid_gap_px) / (thumb_px + grid_gap_px)).floor() as usize).max(1);
        let _perf = crate::perf::span_detail("grid.render", || {
            format!(
                "pane={} count={count} cols={cols} thumb={thumb:.1} pane_w={pane_w:.1}",
                pane_id.0
            )
        });

        // Hoist scroll handle so the outer relative wrapper can host the scrollbar
        // (vertical_scrollbar must NOT be inside overflow_y_scroll — it scrolls away).
        let scroll = self.scroll_handle_for(pane_id);
        let base_scroll = scroll.0.borrow().base_handle.clone();

        let animate_grid_reflow = !self.state.settings.disable_animations
            && !self.suppress_window_resize_animations()
            && self.dock_split_animations.is_empty()
            && self.pane_layout_animations.is_empty()
            && self.file_panel_cache.len() <= if cfg!(debug_assertions) { 4 } else { 6 };

        let inner = if count == 0 {
            let msg = self.empty_pane_message(pane_id, rust_i18n::t!("empty_folder").into_owned());
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(muted)
                .on_mouse_down(
                    MouseButton::Left,
                    crate::ui::views::on_pane_rubber_mouse_down(&view, pane_id),
                )
                .children(msg)
                .into_any_element()
        } else {
            let mut scroll_y = (-f32::from(base_scroll.offset().y)).max(0.0);
            let viewport_h = {
                let tracked = f32::from(base_scroll.bounds().size.height);
                if tracked > 1.0 {
                    tracked
                } else {
                    f32::from(window.viewport_size().height)
                }
            };
            let cell_h_px = thumb_px * GRID_ICON_AREA + GRID_LABEL_H * zoom_scale;
            let pitch = cell_h_px + grid_gap_px;
            let rows = count.div_ceil(cols);
            let content_h = if rows == 0 {
                0.0
            } else {
                rows as f32 * pitch - grid_gap_px
            };
            let max_scroll = (content_h - viewport_h).max(0.0);
            if scroll_y > max_scroll {
                let offset = base_scroll.offset();
                base_scroll.set_offset(gpui::point(offset.x, gpui::px(-max_scroll)));
                scroll_y = max_scroll;
            }
            let start_row = (scroll_y / pitch).floor().max(0.0) as usize;
            let end_row = ((scroll_y + viewport_h) / pitch).ceil() as usize + 1;
            let start = (start_row * cols).min(count);
            let end = ((end_row * cols).min(count)).max(start);
            self.prepare_grid_visible(pane_id, start..end);

            let list_view = view.clone();
            let inner_scroll_view = view.clone();
            div()
                .id(SharedString::from(format!(
                    "anim-grid-scroll-{}",
                    pane_id.0
                )))
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&base_scroll)
                .on_mouse_down(
                    MouseButton::Left,
                    crate::ui::views::on_pane_rubber_mouse_down(&view, pane_id),
                )
                .on_scroll_wheel(move |ev, window, cx| {
                    ctrl_scroll(ev, pane_id, &inner_scroll_view, window, cx);
                })
                .child(AnimatedVirtualGrid::new(
                    SharedString::from(format!("anim-grid-{}", pane_id.0)),
                    count,
                    cols,
                    gpui::Size {
                        width: gpui::px(thumb_px),
                        height: gpui::px(cell_h_px),
                    },
                    gpui::Size {
                        width: gpui::px(grid_gap_px),
                        height: gpui::px(grid_gap_px),
                    },
                    scroll_y,
                    viewport_h,
                    animate_grid_reflow,
                    move |idx, mut is_animating, window, cx| {
                        let app = list_view.read(cx);
                        if !app.dock_split_animations.is_empty()
                            || !app.pane_layout_animations.is_empty()
                        {
                            is_animating = true;
                        }
                        let Some(ts) = app.state.tab_states.get(&pane_id) else {
                            return div().into_any_element();
                        };
                        let Some(&entry_idx) = ts.visible.get(idx) else {
                            return div().into_any_element();
                        };
                        let Some(entry) = ts.entries.get(entry_idx) else {
                            return div().into_any_element();
                        };
                        let file_icon = app.file_icon_for_entry(
                            &entry.path,
                            &entry.kind,
                            entry.extension.as_deref(),
                        );
                        let record_rects = true;
                        let flashing = matches!(entry.kind, FileKind::Dir)
                            && app.drag_hover.blink_id.as_ref()
                                == Some(&DragHoverId::Folder(pane_id, entry.path.clone()));
                        let rename_input =
                            crate::ui::views::rename::active_input(app, pane_id, entry_idx);
                        let selected = ts.selection.selected.contains(&entry_idx)
                            && !crate::ui::views::rename::suppress_row_chrome(
                                app, pane_id, entry_idx,
                            );
                        let cell = render_cell(
                            entry,
                            entry_idx,
                            selected,
                            pane_id,
                            &list_view,
                            thumb,
                            thumb,
                            file_icon,
                            accent,
                            accent_fg,
                            hover,
                            border,
                            flashing,
                            record_rects,
                            rename_input,
                            is_animating,
                            crate::ui::views::rename::dim_other_entry(app, pane_id, entry_idx),
                            window,
                        );
                        cell
                    },
                ))
                .into_any_element()
        };

        let (filter_pill, controls_pill) = self.render_pane_pills(pane_id, window, cx);
        let prepaint_view = view.clone();
        let bounds_view = view.clone();
        let pane_view = view;
        crate::ui::views::pane_background_interactions(
            v_flex()
                .id(SharedString::from(format!("pane-{}", pane_id.0)))
                .size_full()
                .overflow_hidden()
                .min_w(gpui::px(0.))
                .min_h(gpui::px(0.)),
            &pane_view,
            pane_id,
        )
        // Capture the real pane pixel width every frame so the next frame's
        // column calculation uses the actual container size (handles preview
        // panel open/close and splitter drags, not just window resize).
        .on_prepaint(move |bounds: gpui::Bounds<gpui::Pixels>, _window, cx| {
            crate::perf::count("prepaint.grid_pane");
            let new_w = f32::from(bounds.size.width);
            prepaint_view.update(cx, |this, cx| {
                if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                    if (ts.last_pane_width - new_w).abs() > 0.5 {
                        let old_w = ts.last_pane_width;
                        crate::perf::event(|| {
                            format!(
                                "grid.width pane={} old={old_w:.1} new={new_w:.1}",
                                pane_id.0
                            )
                        });
                        ts.last_pane_width = new_w;
                        this.notify_pane(pane_id, cx);
                    }
                }
                // FASE F: register this pane's bounds for OLE drop routing.
                this.push_pane_zone(pane_id, bounds);
            });
        })
        .child(crumb_strip)
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .relative()
                .when(figma_glass, |s| s.bg(glass_fill()))
                .rounded_bl(px(12.0))
                .rounded_br(px(12.0))
                .on_prepaint(pane_bounds_recorder(pane_id, &bounds_view))
                .child(inner)
                .child(filter_pill)
                .child(controls_pill)
                .vertical_scrollbar(&base_scroll),
        )
        .into_any_element()
    }

    fn prepare_grid_visible(&mut self, pane_id: TabId, range: Range<usize>) {
        let windows_theme = crate::ui::icons::is_windows_theme(&self.state.profiles.active().theme);
        let (to_request, to_request_ext, to_request_shell, request_folder_icon, all_visible) = {
            let Some(ts) = self.state.tab_states.get(&pane_id) else {
                return;
            };
            let mut imgs = Vec::new();
            let mut exts: Vec<(
                std::path::PathBuf,
                String,
                std::collections::HashMap<String, std::path::PathBuf>,
            )> = Vec::new();
            // Windows: non-native-image files routed through IShellItemImageFactory.
            let mut shell_paths: Vec<(std::path::PathBuf, bool)> = Vec::new();
            let mut request_folder_icon = false;
            let mut all_visible = Vec::new();
            for vi in range {
                let Some(&entry_idx) = ts.visible.get(vi) else {
                    break;
                };
                let Some(entry) = ts.entries.get(entry_idx) else {
                    continue;
                };
                let thumbable = is_thumbnailable(entry);
                let shell_thumbable = windows_theme
                    && !thumbable
                    && entry.kind == FileKind::File
                    && entry
                        .extension
                        .as_deref()
                        .is_some_and(is_windows_shell_thumbnail_ext);
                request_folder_icon |= windows_theme && entry.kind == FileKind::Dir;

                // Ask Shell only for types likely to have real thumbnails. Generic file
                // and folder icons stay builtin/theme icons; per-path Shell calls are too
                // expensive in huge dirs like C:\Windows\WinSxS.
                #[cfg(target_os = "windows")]
                let use_shell = shell_thumbable;
                #[cfg(not(target_os = "windows"))]
                let use_shell = false;

                #[cfg(not(target_os = "windows"))]
                let ext_cmd = if !thumbable && entry.kind == FileKind::File {
                    entry.extension.as_deref().and_then(|ext| {
                        self.ext_registry
                            .thumbnailer_for(ext)
                            .map(|cmd| (cmd.command.run.clone(), cmd.tools))
                    })
                } else {
                    None
                };
                #[cfg(target_os = "windows")]
                let ext_cmd: Option<(
                    String,
                    std::collections::HashMap<String, std::path::PathBuf>,
                )> = None;

                if thumbable || use_shell || ext_cmd.is_some() {
                    all_visible.push(entry.path.clone());
                }
                if self.thumb_textures.contains_key(&entry.path) {
                    continue;
                }
                if thumbable {
                    imgs.push(entry.path.clone());
                } else if use_shell {
                    shell_paths.push((entry.path.clone(), false));
                } else if let Some((run, tools)) = ext_cmd {
                    exts.push((entry.path.clone(), run, tools));
                }
            }
            (imgs, exts, shell_paths, request_folder_icon, all_visible)
        };

        {
            let mut wanted = self.thumb_wanted.lock().unwrap();
            wanted.clear();
            wanted.extend(all_visible);
        }
        #[cfg(target_os = "windows")]
        if request_folder_icon {
            self.request_shell_folder_icon();
        }
        for path in to_request {
            self.request_thumbnail(&path);
        }
        for (path, run, tools) in to_request_ext {
            self.request_ext_thumbnail(&path, run, tools);
        }
        #[cfg(target_os = "windows")]
        for (path, allow_icon) in to_request_shell {
            if allow_icon {
                self.request_shell_thumbnail_or_icon(&path);
            } else {
                self.request_visible_shell_thumbnail(&path);
            }
        }
        // On non-Windows to_request_shell is always empty; drop it to avoid unused warning.
        #[cfg(not(target_os = "windows"))]
        drop(to_request_shell);
    }
}

/// Extensions worth decoding into a grid thumbnail.
fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff"
    )
}

/// Whether this entry should get a downscaled thumbnail (an image file).
fn is_thumbnailable(entry: &FileEntry) -> bool {
    matches!(entry.kind, FileKind::File) && entry.extension.as_deref().is_some_and(is_image_ext)
}

/// File types where asking Windows Shell for a real thumbnail is worth the COM call.
/// Generic type icons are cheaper as builtins/theme icons and must not be requested per path.
fn is_windows_shell_thumbnail_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "3fr"
            | "arw"
            | "avif"
            | "cr2"
            | "cr3"
            | "crw"
            | "dng"
            | "heic"
            | "heif"
            | "iiq"
            | "mov"
            | "mp4"
            | "m4v"
            | "mkv"
            | "nef"
            | "orf"
            | "pdf"
            | "pef"
            | "psd"
            | "raf"
            | "rw2"
            | "srw"
            | "tga"
            | "webm"
            | "wmv"
    )
}

/// One grid cell: thumbnail (or a scaled glyph fallback while it loads / for
/// non-images) + truncated name, fixed to the thumbnail size. The texture is a
/// cached, downscaled `RenderImage` (see `ui::thumbs`) — never a full-res decode.
#[allow(clippy::too_many_arguments)]
fn render_cell(
    entry: &FileEntry,
    entry_idx: usize,
    selected: bool,
    pane_id: TabId,
    view: &Entity<App>,
    thumb: f32,
    cell_w: f32,
    file_icon: FileIcon,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    border: Hsla,
    flashing: bool,
    record_rects: bool,
    rename_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    is_animating: bool,
    dimmed: bool,
    window: &mut Window,
) -> AnyElement {
    let name = entry.name.clone();
    let font_size = window.rem_size() * 0.75;
    v_flex()
        .id(SharedString::from(format!(
            "cell-{}-{}",
            pane_id.0, entry_idx
        )))
        .when(record_rects && !is_animating, |s| {
            s.on_prepaint(rect_recorder(pane_id, entry_idx, view))
        })
        .w(crate::ui::rem_px(cell_w))
        .p_1()
        .gap_1()
        .items_center()
        .rounded_md()
        .when(dimmed, |s| s.opacity(0.45))
        .cursor_pointer()
        .when(selected && rename_input.is_none(), |s| {
            s.bg(accent).text_color(accent_fg)
        })
        .when(flashing && !selected, |s| s.bg(accent.opacity(0.38)))
        .when(!selected && rename_input.is_none() && !is_animating, |s| {
            s.hover(|h| h.bg(hover))
        })
        .child({
            let area = div()
                .h(crate::ui::rem_px(thumb * GRID_ICON_AREA))
                .flex()
                .items_center()
                .justify_center();
            match file_icon {
                FileIcon::PreviewTexture(tex) => {
                    let natural = tex.size(0);
                    let natural_w = (i32::from(natural.width).max(1)) as f32;
                    let natural_h = (i32::from(natural.height).max(1)) as f32;
                    let scale = (thumb / natural_w)
                        .min((thumb * GRID_PREVIEW_MAX) / natural_h)
                        .min(1.0);
                    let render_w = natural_w * scale;
                    let render_h = natural_h * scale;
                    area.child(
                        div()
                            .w(crate::ui::rem_px(render_w))
                            .h(crate::ui::rem_px(render_h))
                            .rounded(px(crate::ui::RADIUS))
                            .border_1()
                            .border_color(border.opacity(0.55))
                            .overflow_hidden()
                            .child(img(tex).size_full()),
                    )
                }
                FileIcon::IconTexture(tex) => area.child(
                    img(tex)
                        .max_h(crate::ui::rem_px(thumb * GRID_TEXTURE_ICON_MAX))
                        .max_w(crate::ui::rem_px(thumb * GRID_TEXTURE_ICON_MAX))
                        .flex_none(),
                ),
                FileIcon::Builtin { kind, extension } => area.child(
                    FileIcon::Builtin { kind, extension }
                        .element(thumb * GRID_BUILTIN_ICON, accent),
                ),
            }
        })
        .child({
            // The plain (non-editing) name is `.text_xs()`-sized, i.e. it uses
            // `font_size` (already `rem_size * 0.75`), not the ambient default.
            // The rename `Input` widget paints its single line pinned to the top
            // of its own box rather than centering it (unlike plain text), so
            // giving it the "right" height alone isn't enough — wrapping both
            // branches in the same fixed-height, real-flex-centered row is what
            // actually keeps the glyphs lined up between editing/not-editing.
            let line_h = window
                .text_style()
                .line_height
                .to_pixels(AbsoluteLength::Pixels(font_size), window.rem_size());
            h_flex()
                .w_full()
                .h(line_h)
                .items_center()
                .justify_center()
                .child(match &rename_input {
                    Some(input) => div()
                        .w(px(crate::ui::views::rename::input_width(
                            &name,
                            cell_w * f32::from(window.rem_size()) / crate::ui::BASE_PX,
                            font_size,
                            window,
                        )))
                        .child(crate::ui::views::rename::input_element(
                            input, view, line_h, font_size, window,
                        ))
                        .into_any_element(),
                    None => div()
                        .w_full()
                        .text_center()
                        .text_xs()
                        .truncate()
                        .child(name)
                        .into_any_element(),
                })
        })
        .when(!is_animating, |s| {
            s.on_click(crate::ui::views::on_entry_click(view, pane_id, entry_idx))
                .on_mouse_down(
                    MouseButton::Right,
                    crate::ui::views::on_entry_ctx_menu(view, pane_id, entry_idx),
                )
                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                    cx.stop_propagation();
                })
        })
        .when(rename_input.is_none() && !is_animating, |s| {
            s.on_drag(
                crate::ui::drag::file_drag_value(vec![entry.path.clone()]),
                crate::ui::drag::file_drag_handler_for_item(
                    view.clone(),
                    pane_id,
                    entry_idx,
                    entry.path.clone(),
                ),
            )
        })
        .tooltip({
            let tip_name = entry.name.clone();
            let tip_path = entry.path.clone();
            let is_dir = matches!(entry.kind, FileKind::Dir);
            let size = entry.size;
            let modified = entry.modified;
            move |window, cx| {
                crate::ui::tooltips::entry_hover_tooltip(
                    tip_name.clone(),
                    tip_path.clone(),
                    is_dir,
                    size,
                    modified,
                    window,
                    cx,
                )
            }
        })
        .when(matches!(entry.kind, FileKind::Dir) && !is_animating, |s| {
            crate::ui::views::folder_drop_target(s, view, pane_id, entry.path.clone())
        })
        .into_any_element()
}
