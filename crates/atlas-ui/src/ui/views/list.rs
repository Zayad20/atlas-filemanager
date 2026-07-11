//! List (compact) view: `render_list_compact` + compact row cell.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use crate::app::App;
use crate::ui::drag_hover::DragHoverId;
use crate::ui::file_icons::FileIcon;
use crate::ui::glass::{backdrop_tint, glass_fill, glass_header_fill};
use crate::ui::root::{ctrl_scroll, pane_bounds_recorder, rect_recorder, zone_recorder};
use atlas_model::file_entry::{FileEntry, FileKind};
use atlas_model::pane::TabId;
use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, AnyElement, Context, Entity, Hsla, MouseButton, SharedString, Window,
};
use gpui_component::{h_flex, v_flex, ActiveTheme, ElementExt};

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
        let (fg, accent, accent_fg, hover, muted, header_bg) = {
            let t = cx.theme();
            (
                t.foreground,
                t.accent,
                t.accent_foreground,
                t.list_active,
                t.muted_foreground,
                if figma_glass {
                    glass_header_fill()
                } else {
                    backdrop_tint(t.secondary, 0.18)
                },
            )
        };
        let crumb_strip = self.render_crumb_strip(pane_id, &view, fg, muted, header_bg);
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
        // px_2 = 8px padding each side on the row; subtract before fitting cells.
        let avail = (pane_w - 16.0 * zoom_scale).max(cell_stride);
        let cols = ((avail / cell_stride).floor() as usize).max(1);
        let row_count = count.div_ceil(cols);
        let _perf = crate::perf::span_detail("list.render", || {
            format!(
                "pane={} count={count} rows={row_count} cols={cols} pane_w={pane_w:.1}",
                pane_id.0
            )
        });

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
            let scroll = self.scroll_handle_for(pane_id);
            let list_view = view.clone();
            let scroll_view = view.clone();
            div()
                .size_full()
                .on_mouse_down(
                    MouseButton::Left,
                    crate::ui::views::on_pane_rubber_mouse_down(&view, pane_id),
                )
                .child(
                    uniform_list(
                        SharedString::from(format!("listc-{}", pane_id.0)),
                        row_count,
                        move |range, window, cx| {
                            let start = range.start;
                            let end = range.end;
                            crate::perf::count_by(
                                "list.rows.requested",
                                end.saturating_sub(start) as u64,
                            );
                            let _perf = crate::perf::span_detail("list.rows.build", || {
                                format!("pane={} range={start}..{end} cols={cols}", pane_id.0)
                            });
                            list_view.update(cx, |app, _cx| {
                                let record_rects = app.dock_split_animations.is_empty()
                                    && app.pane_layout_animations.is_empty();
                                let rows_data = {
                                    let Some(ts) = app.state.tab_states.get(&pane_id) else {
                                        return Vec::new();
                                    };
                                    range
                                        .map(|row| {
                                            let mut cells = Vec::with_capacity(cols);
                                            for c in 0..cols {
                                                let vi = row * cols + c;
                                                let Some(&entry_idx) = ts.visible.get(vi) else {
                                                    break;
                                                };
                                                let Some(entry) = ts.entries.get(entry_idx) else {
                                                    continue;
                                                };
                                                let selected =
                                                    ts.selection.selected.contains(&entry_idx);
                                                cells.push((entry_idx, entry.clone(), selected));
                                            }
                                            cells
                                        })
                                        .collect::<Vec<_>>()
                                };
                                rows_data
                                    .into_iter()
                                    .map(|row_entries| {
                                        let mut cells: Vec<AnyElement> = Vec::with_capacity(cols);
                                        for (entry_idx, entry, selected) in row_entries {
                                            let file_ref = crate::ui::file_icons::FileIconRef {
                                                path: entry.path.clone(),
                                                kind: entry.kind.clone(),
                                                extension: entry.extension.clone(),
                                            };
                                            app.maybe_request_file_icon(&file_ref);
                                            let file_icon = app.file_icon(&file_ref);
                                            let flashing = matches!(entry.kind, FileKind::Dir)
                                                && app.drag_hover.blink_id.as_ref()
                                                    == Some(&DragHoverId::Folder(
                                                        pane_id,
                                                        entry.path.clone(),
                                                    ));
                                            let rename_input =
                                                crate::ui::views::rename::active_input(
                                                    app, pane_id, entry_idx,
                                                );
                                            let selected = selected
                                                && !crate::ui::views::rename::suppress_row_chrome(
                                                    app, pane_id, entry_idx,
                                                );
                                            cells.push(render_cell_compact(ListCellProps {
                                                entry: &entry,
                                                entry_idx,
                                                selected,
                                                pane_id,
                                                view: &list_view,
                                                accent,
                                                accent_fg,
                                                hover,
                                                flashing,
                                                record_rects,
                                                file_icon,
                                                rename_input,
                                                dimmed: crate::ui::views::rename::dim_other_entry(
                                                    app, pane_id, entry_idx,
                                                ),
                                                window,
                                            }));
                                        }
                                        let row_el = h_flex()
                                            .w_full()
                                            .gap(crate::ui::rem_px(6.))
                                            .px_2()
                                            .children(cells);
                                        row_el.into_any_element()
                                    })
                                    .collect::<Vec<_>>()
                            })
                        },
                    )
                    .track_scroll(&scroll)
                    .on_scroll_wheel(move |ev, window, cx| {
                        ctrl_scroll(ev, pane_id, &scroll_view, window, cx)
                    })
                    .h_full(),
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
                v.update(cx, |this, cx| {
                    if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                        if (ts.last_pane_width - new_w).abs() > 0.5 {
                            let old_w = ts.last_pane_width;
                            crate::perf::event(|| {
                                format!(
                                    "list.width pane={} old={old_w:.1} new={new_w:.1}",
                                    pane_id.0
                                )
                            });
                            ts.last_pane_width = new_w;
                            this.notify_pane(pane_id, cx);
                        }
                    }
                });
            }
        })
        // Phase F: register this pane's window-space rect each frame for OLE drop routing.
        .when(cfg!(windows), |el| {
            el.on_prepaint(zone_recorder(pane_id, &zone_view))
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
                .child(controls_pill),
        )
        .into_any_element()
    }
}

/// One compact List cell: 16px icon + truncated name in a fixed 220px row. Click
/// routing mirrors the Details rows (shift = range, ctrl = toggle, dbl = open).
struct ListCellProps<'a> {
    entry: &'a FileEntry,
    entry_idx: usize,
    selected: bool,
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
        .child(
            div()
                .w(crate::ui::rem_px(16.))
                .flex_none()
                .child(file_icon.element(14.0, accent)),
        )
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
