//! List (compact) view: `render_list_compact` + compact row cell.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use crate::app::App;
use crate::ui::drag_hover::DragHoverId;
use crate::ui::file_icons::FileIcon;
use crate::ui::glass::{backdrop_tint, glass_fill, glass_header_fill};
use crate::ui::layout_region::{
    layout_region, timed_layout_region, PANE_CRUMBS_TIMINGS, PANE_PILLS_TIMINGS,
    PANE_VIEWPORT_TIMINGS,
};
use crate::ui::root::{ctrl_scroll, pane_bounds_recorder, rect_recorder, zone_recorder};
use crate::ui::selection_checkbox::SelectionCheckbox;
use crate::ui::views::grid_surface_element::{GridSurfaceColors, GridSurfaceElement};
use atlas_model::file_entry::{FileEntry, FileKind};
use atlas_model::pane::TabId;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, Entity, Hsla, MouseButton, SharedString, Window};
use gpui_component::{h_flex, v_flex, ActiveTheme, ElementExt};

const USE_LIST_SURFACE_ELEMENT: bool = true;

impl App {
    /// Compact List view: small `icon + name` cells flowing row-major into as many
    /// columns as fit the width — the classic Windows "List" layout. Virtualized by
    /// row (`uniform_list`) like the grid, so huge dirs stay smooth.
    pub(crate) fn render_list_compact(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        crate::perf::count("render.view.list");
        let view = cx.entity();
        let figma_glass = self.state.profiles.active().theme == "Atlas Figma Glass";
        let (fg, accent, accent_fg, hover, muted, border, _header_bg) = {
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
        let font_size = window.text_style().font_size.to_pixels(window.rem_size());
        let row_font = window.text_style().font();
        let crumb_strip = self.render_crumb_strip(pane_id, &view, fg, muted, border);
        let count = self
            .state
            .tab_states
            .get_mut(&pane_id)
            .map_or(0, |ts| ts.visible_indices().len());
        #[allow(clippy::items_after_statements)] // local constant kept next to its use
        const CELL_W: f32 = 220.0;
        // `cols` is fitted against real measured pixels (pane_w), so it needs the
        // zoom-scaled cell width — the cell itself renders via `rem_px(220.)` in
        // `render_cell_compact`, which resolves to the same pixel value.
        let zoom_scale = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        let cell_stride = CELL_W * zoom_scale + 6.0 * zoom_scale;
        // Use the real pane width captured by on_prepaint (set below), not a viewport
        // estimate — otherwise an open preview pane / split makes `cols` too large and
        // the rightmost column overflows and clips. Fall back to viewport on frame 1.
        let pane_w = self
            .state
            .tab_states
            .get(&pane_id)
            .map(|ts| ts.last_pane_width)
            .filter(|&w| w > 1.0)
            .unwrap_or_else(|| (f32::from(window.viewport_size().width) - 217.0).max(0.0));
        let cols = list_column_count(pane_w, cell_stride, zoom_scale);
        let row_count = count.div_ceil(cols);
        let _perf = crate::perf::span_detail("list.render", || {
            format!(
                "pane={} count={count} rows={row_count} cols={cols} pane_w={pane_w:.1}",
                pane_id.0
            )
        });

        let inner = if count == 0 {
            self.list_hit.remove(&pane_id);
            self.row_rects.borrow_mut().remove(&pane_id);
            if self
                .list_hover
                .is_some_and(|(hover_pane, _)| hover_pane == pane_id)
            {
                self.list_hover = None;
            }
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
            let scroll = self.scroll_handle_for(pane_id);
            let base_scroll = scroll.0.borrow().base_handle.clone();
            let mut scroll_y = (-f32::from(base_scroll.offset().y)).max(0.0);
            
            // Grid Snapping Smooth Scroll Interpolation
            if let Some(&target) = self.scroll_targets.get(&pane_id) {
                let current = f32::from(base_scroll.offset().y);
                if (target - current).abs() > 0.5 {
                    let (ease, _) = crate::app::scroll_smooth_params(self.state.settings.scroll_smoothness);
                    let mut next = current + (target - current) * ease;
                    if (target - next).abs() < 1.0 {
                        next = target;
                        self.scroll_targets.remove(&pane_id);
                    }
                    base_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(next)));
                    window.request_animation_frame();
                } else {
                    base_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(target)));
                    self.scroll_targets.remove(&pane_id);
                }
                scroll_y = (-f32::from(base_scroll.offset().y)).max(0.0);
            }
            let app_handle = cx.entity().clone();
            let viewport_h = {
                let tracked = f32::from(base_scroll.bounds().size.height);
                if tracked > 1.0 {
                    tracked
                } else {
                    f32::from(window.viewport_size().height)
                }
            };
            let cell_h_px =
                f32::from(window.line_height()).max(16.0 * zoom_scale) + 4.0 * zoom_scale;
            let pitch = cell_h_px;
            let rows = count.div_ceil(cols);
            let content_h = if rows == 0 { 0.0 } else { rows as f32 * pitch };
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

            let missing_icons = {
                let mut m = Vec::new();
                if let Some(ts) = self.state.tab_states.get(&pane_id) {
                    for i in start..end {
                        if let Some(&idx) = ts.visible.get(i) {
                            if let Some(e) = ts.entries.get(idx) {
                                let fref = crate::ui::file_icons::FileIconRef {
                                    path: e.path.clone(),
                                    kind: e.kind.clone(),
                                    extension: e.extension.clone(),
                                };
                                m.push(fref);
                            }
                        }
                    }
                }
                m
            };
            for fref in missing_icons {
                self.maybe_request_file_icon(&fref);
            }

            let list_view = view.clone();
            let scroll_view = view.clone();

            let animate_list_reflow = !self.state.settings.disable_animations
                && !self.suppress_window_resize_animations()
                && self.dock_split_animations.is_empty()
                && self.pane_layout_animations.is_empty()
                && self.file_panel_cache.len() <= if cfg!(debug_assertions) { 4 } else { 6 };

            let surface_colors = GridSurfaceColors {
                fg,
                accent,
                accent_fg,
                hover,
                muted,
            };
            let surface = if USE_LIST_SURFACE_ELEMENT {
                GridSurfaceElement::new(
                    pane_id,
                    view.downgrade(),
                    self.state
                        .tab_states
                        .get(&pane_id)
                        .map(|ts| ts.visible.clone())
                        .unwrap_or_default(),
                    start,
                    end,
                    cols,
                    count,
                    CELL_W * zoom_scale,
                    cell_h_px,
                    cell_stride,
                    pitch,
                    14.0 * zoom_scale,
                    scroll_y,
                    viewport_h,
                    surface_colors,
                    row_font.clone(),
                    font_size,
                )
                .compact_list()
                .absolute()
                .inset_0()
                .into_any_element()
            } else {
                crate::ui::views::virtual_grid::AnimatedVirtualGrid::new(
                    SharedString::from(format!("anim-list-{}", pane_id.0)),
                    count,
                    cols,
                    gpui::Size {
                        width: gpui::px(CELL_W * zoom_scale),
                        height: gpui::px(cell_h_px),
                    },
                    gpui::Size {
                        width: gpui::px(6.0 * zoom_scale),
                        height: gpui::px(0.0),
                    },
                    scroll_y,
                    viewport_h,
                    animate_list_reflow,
                    move |idx, _is_animating, window, cx| {
                        let app = list_view.read(cx);
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
                        let flashing = matches!(entry.kind, FileKind::Dir)
                            && app.drag_hover.blink_id.as_ref()
                                == Some(&DragHoverId::Folder(pane_id, entry.path.clone()));
                        let rename_input =
                            crate::ui::views::rename::active_input(app, pane_id, entry_idx);
                        let selected = ts.selection.selected.contains(&entry_idx)
                            && !crate::ui::views::rename::suppress_row_chrome(
                                app, pane_id, entry_idx,
                            );

                        render_cell_compact(ListCellProps {
                            entry,
                            entry_idx,
                            selected,
                            show_checkbox: ts.selection.selected.len() > 1,
                            pane_id,
                            view: &list_view,
                            accent,
                            accent_fg,
                            hover,
                            flashing,
                            record_rects: true,
                            file_icon,
                            rename_input,
                            dimmed: crate::ui::views::rename::dim_other_entry(
                                app, pane_id, entry_idx,
                            ),
                            window,
                        })
                    },
                )
                .into_any_element()
            };

            let mut rename_overlay = None;
            if USE_LIST_SURFACE_ELEMENT {
                if let Some(ts) = self.state.tab_states.get(&pane_id) {
                    for visible_pos in start..end {
                        let Some(&entry_idx) = ts.visible.get(visible_pos) else {
                            continue;
                        };
                        let Some(entry) = ts.entries.get(entry_idx) else {
                            continue;
                        };
                        let Some(rename_input) =
                            crate::ui::views::rename::active_input(self, pane_id, entry_idx)
                        else {
                            continue;
                        };
                        let selected = ts.selection.selected.contains(&entry_idx)
                            && !crate::ui::views::rename::suppress_row_chrome(
                                self, pane_id, entry_idx,
                            );
                        let file_icon = self.file_icon_for_entry(
                            &entry.path,
                            &entry.kind,
                            entry.extension.as_deref(),
                        );
                        let cell = render_cell_compact(ListCellProps {
                            entry,
                            entry_idx,
                            selected,
                            show_checkbox: ts.selection.selected.len() > 1,
                            pane_id,
                            view: &view,
                            accent,
                            accent_fg,
                            hover,
                            flashing: false,
                            record_rects: false,
                            file_icon,
                            rename_input: Some(rename_input),
                            dimmed: false,
                            window,
                        });
                        let col = visible_pos % cols;
                        let row = visible_pos / cols;
                        rename_overlay = Some(
                            div()
                                .absolute()
                                .left(px(col as f32 * cell_stride))
                                .top(px(row as f32 * pitch))
                                .w(px(CELL_W * zoom_scale))
                                .h(px(cell_h_px))
                                .child(cell)
                                .into_any_element(),
                        );
                        break;
                    }
                }
            }
            let drag_layer = USE_LIST_SURFACE_ELEMENT.then(|| {
                div()
                    .id(("list-drag-layer", pane_id.0))
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(content_h))
                    .opacity(0.0)
                    .on_drag(
                        crate::ui::drag::file_drag_value(Vec::new()),
                        crate::ui::drag::file_drag_handler_for_list_content(view.clone(), pane_id),
                    )
                    .into_any_element()
            });
            let tooltip_data = self
                .list_hover
                .filter(|(hover_pane, _)| *hover_pane == pane_id)
                .and_then(|(_, entry_idx)| {
                    self.state
                        .tab_states
                        .get(&pane_id)
                        .and_then(|ts| ts.entries.get(entry_idx))
                        .map(|entry| {
                            (
                                entry.name.clone(),
                                entry.path.clone(),
                                matches!(entry.kind, FileKind::Dir),
                                entry.size,
                                entry.modified,
                            )
                        })
                });

            div()
                .id(("anim-list-scroll", pane_id.0))
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&base_scroll)
                .on_mouse_down(
                    MouseButton::Left,
                    crate::ui::views::on_pane_rubber_mouse_down(&view, pane_id),
                )
                .on_scroll_wheel({
                    let base_scroll = base_scroll.clone();
                    let app_handle = app_handle.clone();
                    move |ev, window, cx| {
                        if ev.modifiers.control || ev.modifiers.platform {
                            ctrl_scroll(ev, pane_id, &scroll_view, window, cx);
                        } else if cfg!(windows) {
                            if let gpui::ScrollDelta::Lines(l) = ev.delta {
                                window.prevent_default();
                                cx.stop_propagation();
                                let dy = l.y * pitch * 3.0;
                                
                                app_handle.update(cx, |app, cx| {
                                    let current_target = *app.scroll_targets.entry(pane_id).or_insert_with(|| f32::from(base_scroll.offset().y));
                                    let mut new_y = current_target + dy;
                                    let max_scroll = (content_h - viewport_h).max(0.0);
                                    new_y = (new_y / pitch).round() * pitch;
                                    new_y = new_y.clamp(-max_scroll, 0.0);
                                    
                                    app.scroll_targets.insert(pane_id, new_y);
                                    cx.notify();
                                });
                            }
                        }
                    }
                })
                .when_some(tooltip_data, |el, (name, path, is_dir, size, modified)| {
                    el.tooltip(move |window, cx| {
                        crate::ui::tooltips::entry_hover_tooltip(
                            name.clone(),
                            path.clone(),
                            is_dir,
                            size,
                            modified,
                            window,
                            cx,
                        )
                    })
                })
                .child(
                    div()
                        .w_full()
                        .h(px(content_h))
                        .relative()
                        .child(surface)
                        .when_some(rename_overlay, |el, overlay| el.child(overlay))
                        .when_some(drag_layer, |el, layer| el.child(layer)),
                )
                .into_any_element()
        };

        let (filter_pill, controls_pill) = self.render_pane_pills(pane_id, window, cx);
        let zone_view = view.clone();
        let prepaint_view = view.clone();
        let bounds_view = view.clone();
        let pane_view = view;
        crate::ui::views::pane_background_interactions(
            v_flex()
                .id(("pane", pane_id.0))
                .size_full()
                .overflow_hidden(),
            &pane_view,
            pane_id,
        )
        .on_prepaint({
            let v = prepaint_view.clone();
            move |bounds: gpui::Bounds<gpui::Pixels>, _window, cx| {
                crate::perf::count("prepaint.list_pane");
                let new_w = f32::from(bounds.size.width);
                let should_update = v
                    .read(cx)
                    .state
                    .tab_states
                    .get(&pane_id)
                    .map(|ts| (ts.last_pane_width - new_w).abs() > 0.5)
                    .unwrap_or(false);

                if should_update {
                    v.update(cx, |this, cx| {
                        let mut columns_changed = false;
                        if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                            let old_w = ts.last_pane_width;
                            let new_cols = list_column_count(new_w, cell_stride, zoom_scale);
                            columns_changed = new_cols != cols;
                            crate::perf::event(|| {
                                format!(
                                    "list.width pane={} old={old_w:.1} new={new_w:.1} cols={cols}->{new_cols}",
                                    pane_id.0,
                                )
                            });
                            ts.last_pane_width = new_w;
                        }
                        if columns_changed {
                            crate::perf::count("list.width.columns_changed");
                            this.notify_pane(pane_id, cx);
                        } else {
                            crate::perf::count("list.width.same_columns");
                        }
                    });
                }
            }
        })
        // Phase F: register this pane's window-space rect each frame for OLE drop routing.
        .when(cfg!(windows), |el| {
            el.on_prepaint(zone_recorder(pane_id, &zone_view))
        })
        .child(timed_layout_region(
            "pane.crumbs",
            PANE_CRUMBS_TIMINGS,
            crumb_strip,
        ))
        .child(timed_layout_region(
            "pane.viewport",
            PANE_VIEWPORT_TIMINGS,
            div()
                .flex_1()
                .min_h(px(0.))
                .relative()
                .when(figma_glass, |s| s.bg(glass_fill()))
                .rounded_bl(px(12.0))
                .rounded_br(px(12.0))
                .on_prepaint(pane_bounds_recorder(pane_id, &bounds_view))
                .child(layout_region("pane.viewport_body", inner))
                .child(timed_layout_region(
                    "pane.pills",
                    PANE_PILLS_TIMINGS,
                    filter_pill,
                ))
                .child(timed_layout_region(
                    "pane.pills",
                    PANE_PILLS_TIMINGS,
                    controls_pill,
                )),
        ))
        .into_any_element()
    }
}

fn list_column_count(pane_w: f32, cell_stride: f32, zoom: f32) -> usize {
    // The compact cell owns 8px horizontal padding per side; reserve it before
    // fitting fixed-pitch columns so the right edge never clips.
    let avail = (pane_w - 16.0 * zoom).max(cell_stride);
    ((avail / cell_stride).floor() as usize).max(1)
}

#[cfg(test)]
mod tests {
    use super::list_column_count;

    #[test]
    fn list_columns_change_only_at_fitted_stride_boundary() {
        let zoom = 1.0;
        let stride = 226.0;

        assert_eq!(list_column_count(467.99, stride, zoom), 1);
        assert_eq!(list_column_count(468.0, stride, zoom), 2);
        assert_eq!(list_column_count(693.99, stride, zoom), 2);
        assert_eq!(list_column_count(694.0, stride, zoom), 3);
    }

    #[test]
    fn list_columns_scale_with_zoom_and_never_drop_below_one() {
        assert_eq!(list_column_count(0.0, 452.0, 2.0), 1);
        assert_eq!(list_column_count(936.0, 452.0, 2.0), 2);
    }
}

/// One compact List cell: 16px icon + truncated name in a fixed 220px row. Click
/// routing mirrors the Details rows (shift = range, ctrl = toggle, dbl = open).
struct ListCellProps<'a> {
    entry: &'a FileEntry,
    entry_idx: usize,
    selected: bool,
    show_checkbox: bool,
    pane_id: TabId,
    view: &'a Entity<App>,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    flashing: bool,
    record_rects: bool,
    file_icon: FileIcon,
    rename_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    dimmed: bool,
    window: &'a mut Window,
}

/// One compact List cell: 16px icon + truncated name in a fixed 220px row. Click
/// routing mirrors the Details rows (shift = range, ctrl = toggle, dbl = open).
fn render_cell_compact(props: ListCellProps) -> AnyElement {
    let ListCellProps {
        entry,
        entry_idx,
        selected,
        show_checkbox,
        pane_id,
        view,
        accent,
        accent_fg,
        hover,
        flashing,
        record_rects,
        file_icon,
        rename_input,
        dimmed,
        window,
    } = props;
    let name = entry.name.clone();
    let font_size = window.text_style().font_size.to_pixels(window.rem_size());
    h_flex()
        .id(("lc", entry_idx))
        .when(record_rects, |s| {
            s.on_prepaint(rect_recorder(pane_id, entry_idx, view))
        })
        .w(crate::ui::rem_px(220.))
        .px_1()
        .py(crate::ui::rem_px(2.))
        .gap_1()
        .items_center()
        .rounded_sm()
        .when(dimmed, |s| s.opacity(0.45))
        .cursor_pointer()
        .when(selected && rename_input.is_none(), |s| {
            s.bg(accent).text_color(accent_fg)
        })
        .when(flashing && !selected, |s| s.bg(accent.opacity(0.38)))
        .when(!selected && rename_input.is_none(), |s| {
            s.hover(|h| h.bg(hover))
        })
        .child(if selected && show_checkbox {
            SelectionCheckbox::new(
                SharedString::from(format!("list-selection-{pane_id:?}-{entry_idx}")),
                15.0 * f32::from(window.rem_size()) / crate::ui::BASE_PX,
            )
            .into_any_element()
        } else {
            div()
                .w(crate::ui::rem_px(16.))
                .flex_none()
                .child(file_icon.element(14.0, accent))
                .into_any_element()
        })
        .child(match &rename_input {
            Some(input) => {
                let line_h = window.line_height();
                // `Input` paints its single line pinned to the top of its own box
                // rather than centering it like plain text does, so this wrapper
                // must do real flex-centering (not just a matching height) to keep
                // the glyphs lined up with the row when swapping in/out of rename.
                h_flex()
                    .w(px(crate::ui::views::rename::input_width(
                        &name,
                        188.0 * f32::from(window.rem_size()) / crate::ui::BASE_PX,
                        font_size,
                        window,
                    )))
                    .h(line_h)
                    .items_center()
                    .flex_none()
                    .child(crate::ui::views::rename::input_element(
                        input, view, line_h, font_size, window,
                    ))
                    .into_any_element()
            }
            None => div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .child(name)
                .into_any_element(),
        })
        .on_click(crate::ui::views::on_entry_click(view, pane_id, entry_idx))
        .on_mouse_down(
            MouseButton::Right,
            crate::ui::views::on_entry_ctx_menu(view, pane_id, entry_idx),
        )
        .when(!selected && rename_input.is_none(), |s| {
            let rv = view.clone();
            s.on_mouse_down(MouseButton::Left, move |ev, _window, cx| {
                let (pos, additive) = (
                    ev.position,
                    ev.modifiers.control || ev.modifiers.platform || ev.modifiers.shift,
                );
                rv.update(cx, |this, cx| {
                    this.begin_rubber(pane_id, pos, additive);
                    cx.notify();
                });
                cx.stop_propagation();
            })
        })
        .when(rename_input.is_some(), |s| {
            s.on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                cx.stop_propagation();
            })
        })
        .when(selected && rename_input.is_none(), |s| {
            s.on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                cx.stop_propagation();
            })
            .on_drag(
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
        .when(matches!(entry.kind, FileKind::Dir), |s| {
            crate::ui::views::folder_drop_target(s, view, pane_id, entry.path.clone())
        })
        .into_any_element()
}
