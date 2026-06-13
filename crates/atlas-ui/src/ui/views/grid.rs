//! Grid (icons) view: `render_grid` + grid cell + thumbnail predicate helpers.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use crate::app::App;
use crate::input::action::Action;
use crate::state::file_entry::{FileEntry, FileKind};
use crate::state::pane::TabId;
use crate::state::tab::LoadState;
use crate::ui::glass::GLASS_RADIUS;
use crate::ui::icons::file_type_icon;
use gpui::prelude::*;
use gpui::{
    div, img, px, uniform_list, AnyElement, Context,
    Entity, Hsla, MouseButton,
    RenderImage, SharedString, Window,
};
use gpui_component::button::ButtonVariants as _;
use gpui_component::{h_flex, v_flex, ActiveTheme, ElementExt};
use std::path::PathBuf;
use std::sync::Arc;
use crate::ui::root::{ctrl_scroll, rect_recorder, DraggedFiles};

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
        let view = cx.entity();
        let focused = self.state.focused_tile == pane_id;
        let (fg, accent, accent_fg, hover, muted, header_bg) = {
            let t = cx.theme();
            (
                t.foreground,
                t.accent,
                t.accent_foreground,
                t.list_active,
                t.muted_foreground,
                t.secondary,
            )
        };
        let crumb_strip = self.render_crumb_strip(pane_id, &view, fg, muted, header_bg);
        let thumb = self
            .tab_data_for(pane_id)
            .map_or(128.0, |t| t.thumbnail_size);
        let count = self
            .state
            .tab_states
            .get_mut(&pane_id)
            .map_or(0, |ts| ts.visible_indices().len());
        // Grid layout constants matching the row's gap_2 / px_2 styling.
        const GRID_GAP: f32 = 8.0; // gap_2 between cells
        const GRID_PAD: f32 = 8.0; // px_2 on each side of the row
                                   // Use the width captured by on_prepaint from the previous frame — this
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
        let inner_avail = (pane_w - 2.0 * GRID_PAD).max(thumb);
        // n*(thumb+GAP) - GAP = inner_avail  →  n = ⌊(inner_avail + GAP) / (thumb + GAP)⌋
        let cols = (((inner_avail + GRID_GAP) / (thumb + GRID_GAP)).floor() as usize).max(1);
        // Distribute remaining space so every cell fills exactly its column width.
        let fill_w = ((inner_avail - (cols - 1) as f32 * GRID_GAP) / cols as f32).max(thumb);
        let row_count = count.div_ceil(cols);

        let inner = if count == 0 {
            let msg = match self.state.tab_states.get(&pane_id).map(|t| &t.load_state) {
                Some(LoadState::Loading) => "Cargando…",
                Some(LoadState::Error(_)) => "No se pudo leer la carpeta",
                _ => "Carpeta vacía",
            };
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child(msg)
                .into_any_element()
        } else {
            let scroll = self.scroll_handle_for(pane_id);
            let list_view = view.clone();
            uniform_list(
                SharedString::from(format!("grid-{}", pane_id.0)),
                row_count,
                move |range, _window, cx| {
                    // Phase 1: collect visible paths still missing a thumbnail. Native
                    // images decode in-process; other types (video/3D/psd) go through an
                    // extension thumbnailer if one accepts the extension.
                    let (to_request, to_request_ext): (Vec<PathBuf>, Vec<(PathBuf, String)>) = {
                        let app = list_view.read(cx);
                        let Some(ts) = app.state.tab_states.get(&pane_id) else {
                            return Vec::new();
                        };
                        let mut imgs = Vec::new();
                        let mut exts: Vec<(PathBuf, String)> = Vec::new();
                        for row in range.clone() {
                            for c in 0..cols {
                                let vi = row * cols + c;
                                let Some(&ei) = ts.visible.get(vi) else { break };
                                let Some(entry) = ts.entries.get(ei) else {
                                    continue;
                                };
                                if app.thumb_textures.contains_key(&entry.path) {
                                    continue;
                                }
                                if is_thumbnailable(entry) {
                                    imgs.push(entry.path.clone());
                                } else if entry.kind == FileKind::File {
                                    if let Some(ext) = entry.extension.as_deref() {
                                        if let Some(cmd) = app.ext_registry.thumbnailer_for(ext) {
                                            exts.push((
                                                entry.path.clone(),
                                                cmd.command.run.clone(),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        (imgs, exts)
                    };
                    // Phase 2: enqueue off-thread decodes / thumbnailer runs.
                    if !to_request.is_empty() || !to_request_ext.is_empty() {
                        list_view.update(cx, |this, _| {
                            for p in to_request {
                                this.request_thumbnail(p);
                            }
                            for (p, run) in to_request_ext {
                                this.request_ext_thumbnail(p, run);
                            }
                        });
                    }
                    // Phase 3: build the rows, painting ready thumbnails.
                    let app = list_view.read(cx);
                    let Some(ts) = app.state.tab_states.get(&pane_id) else {
                        return Vec::new();
                    };
                    let record_rects = app.rubber.as_ref().is_some_and(|r| r.pane == pane_id);
                    range
                        .map(|row| {
                            let mut cells: Vec<AnyElement> = Vec::with_capacity(cols);
                            for c in 0..cols {
                                let vi = row * cols + c;
                                let Some(&entry_idx) = ts.visible.get(vi) else {
                                    break;
                                };
                                let Some(entry) = ts.entries.get(entry_idx) else {
                                    continue;
                                };
                                let selected = ts.selection.selected.contains(&entry_idx);
                                let thumb_tex = app.grid_thumbnail(&entry.path);
                                let icon_tex =
                                    app.icon_tex_for_entry(&entry.kind, entry.extension.as_deref());
                                cells.push(render_cell(
                                    entry,
                                    entry_idx,
                                    selected,
                                    pane_id,
                                    &list_view,
                                    thumb,
                                    fill_w,
                                    thumb_tex,
                                    icon_tex,
                                    accent,
                                    accent_fg,
                                    hover,
                                    record_rects,
                                ));
                            }
                            h_flex().w_full().gap_2().px_2().py_1().children(cells)
                        })
                        .collect::<Vec<_>>()
                },
            )
            .track_scroll(&scroll)
            .h_full()
            .into_any_element()
        };

        let (filter_pill, controls_pill) = self.render_pane_pills(pane_id, window, cx);
        let focus_view = view.clone();
        let rubber_view = view.clone();
        let prepaint_view = view.clone();
        let scroll_view = view;
        v_flex()
            .id(SharedString::from(format!("pane-{}", pane_id.0)))
            .size_full()
            // The pane content is transparent: the hosting TabPanel (fork) is the
            // rounded glass card, with the tab selector inside it. Clip the list to the
            // card radius; show an accent ring only on the focused pane.
            .overflow_hidden()
            .when(focused, |s| {
                s.rounded(px(GLASS_RADIUS)).border_2().border_color(accent)
            })
            .on_mouse_down(MouseButton::Left, {
                let v = rubber_view.clone();
                move |ev, _window, cx| {
                    let (pos, additive) =
                        (ev.position, ev.modifiers.control || ev.modifiers.platform);
                    v.update(cx, |this, cx| {
                        this.begin_rubber(pane_id, pos, additive);
                        cx.notify();
                    });
                }
            })
            .on_drop::<DraggedFiles>({
                let v = rubber_view.clone();
                move |d, _window, cx| {
                    let paths = d.paths.clone();
                    v.update(cx, |this, cx| {
                        if let Some(dest) = this.tab_data_for(pane_id).and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf())) {
                            this.process_actions(vec![Action::Move { paths, dest }]);
                            cx.notify();
                        }
                    });
                }
            })
            .on_drop::<gpui::ExternalPaths>({
                let v = rubber_view;
                move |paths, _window, cx| {
                    let paths = paths.paths().to_vec();
                    v.update(cx, |this, cx| {
                        if let Some(dest) = this.tab_data_for(pane_id).and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf())) {
                            this.process_actions(vec![Action::CopyTo { paths, dest }]);
                            cx.notify();
                        }
                    });
                }
            })
            .on_click(move |_ev, _window, cx| {
                focus_view.update(cx, |this, cx| {
                    this.state.focused_tile = pane_id;
                    cx.notify();
                });
            })
            .on_scroll_wheel(move |ev, _window, cx| ctrl_scroll(ev, pane_id, &scroll_view, cx))
            // Capture the real pane pixel width every frame so the next frame's
            // column calculation uses the actual container size (handles preview
            // panel open/close and splitter drags, not just window resize).
            .on_prepaint(move |bounds: gpui::Bounds<gpui::Pixels>, _window, cx| {
                let new_w = f32::from(bounds.size.width);
                prepaint_view.update(cx, |this, cx| {
                    if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                        if (ts.last_pane_width - new_w).abs() > 0.5 {
                            ts.last_pane_width = new_w;
                            cx.notify();
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
                    .child(inner)
                    .child(filter_pill)
                    .child(controls_pill),
            )
            .into_any_element()
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
    thumb_tex: Option<Arc<RenderImage>>,
    icon_tex: Option<Arc<RenderImage>>,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    record_rects: bool,
) -> AnyElement {
    let name = entry.name.clone();
    let v = view.clone();
    v_flex()
        .id(SharedString::from(format!(
            "cell-{}-{}",
            pane_id.0, entry_idx
        )))
        .when(record_rects, |s| {
            s.on_prepaint(rect_recorder(pane_id, entry_idx, view))
        })
        .w(px(cell_w))
        .p_1()
        .gap_1()
        .items_center()
        .rounded_md()
        .cursor_pointer()
        .when(selected, |s| s.bg(accent).text_color(accent_fg))
        .when(!selected, |s| s.hover(|h| h.bg(hover)))
        .child({
            let area = div()
                .h(px(thumb * 0.62))
                .flex()
                .items_center()
                .justify_center();
            match thumb_tex {
                Some(tex) => area.child(
                    img(tex)
                        .max_h(px(thumb * 0.6))
                        .max_w(px(thumb))
                        .rounded_sm(),
                ),
                None => match icon_tex {
                    Some(tex) => area.child(
                        img(tex)
                            .max_h(px(thumb * 0.56))
                            .max_w(px(thumb * 0.56))
                            .flex_none(),
                    ),
                    None => area.child(file_type_icon(
                        &entry.kind,
                        entry.extension.as_deref(),
                        thumb * 0.42,
                    )),
                },
            }
        })
        .child(
            div()
                .w_full()
                .text_center()
                .text_xs()
                .truncate()
                .child(name),
        )
        .on_click(move |ev, _window, cx| {
            let m = ev.modifiers();
            let shift = m.shift;
            let ctrl = m.control || m.platform;
            let activate = ev.click_count() >= 2;
            v.update(cx, |this, cx| {
                if activate {
                    this.process_actions(vec![Action::Activate {
                        tile: pane_id,
                        idx: entry_idx,
                    }]);
                } else {
                    this.process_actions(vec![Action::Select {
                        tile: pane_id,
                        idx: entry_idx,
                        shift,
                        ctrl,
                    }]);
                }
                cx.notify();
            });
        })
        .on_mouse_down(MouseButton::Right, {
            let rv = view.clone();
            move |ev, window, cx| {
                let pos = ev.position;
                rv.update(cx, |this, cx| {
                    this.process_actions(vec![Action::Select {
                        tile: pane_id,
                        idx: entry_idx,
                        shift: false,
                        ctrl: false,
                    }]);
                    // Right-click opens the Atlas–style menu on the (now
                    // selected) item; native shell items stream in afterwards.
                    let paths = this.selected_paths();
                    this.open_ctx_menu(paths, pos, window, cx);
                    cx.notify();
                });
            }
        })
        .when(!selected, |s| {
            let rv = view.clone();
            s.on_mouse_down(MouseButton::Left, move |ev, _window, cx| {
                let (pos, additive) = (ev.position, ev.modifiers.control || ev.modifiers.platform);
                rv.update(cx, |this, cx| {
                    this.begin_rubber(pane_id, pos, additive);
                    cx.notify();
                });
                cx.stop_propagation();
            })
        })
        .when(selected, |s| {
            s.on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                cx.stop_propagation();
            })
            .on_drag(
                DraggedFiles {
                    paths: vec![entry.path.clone()],
                },
                crate::ui::drag::file_drag_handler(view.clone(), entry.path.clone()),
            )
        })
        .when(matches!(entry.kind, FileKind::Dir), |s| {
            let dest_move = entry.path.clone();
            let dest_copy = entry.path.clone();
            let mv = view.clone();
            let cpv = view.clone();
            s.drag_over::<DraggedFiles>(|s, _, _, _| s.opacity(0.6))
                .on_drop::<DraggedFiles>(move |d, _window, cx| {
                    let (paths, dest) = (d.paths.clone(), dest_move.clone());
                    mv.update(cx, |this, cx| {
                        this.process_actions(vec![Action::Move { paths, dest }]);
                        cx.notify();
                    });
                })
                .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
                    let (paths, dest) = (paths.paths().to_vec(), dest_copy.clone());
                    cpv.update(cx, |this, cx| {
                        this.process_actions(vec![Action::CopyTo { paths, dest }]);
                        cx.notify();
                    });
                })
        })
        .into_any_element()
}

