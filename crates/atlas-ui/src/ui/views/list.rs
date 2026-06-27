//! List (compact) view: `render_list_compact` + compact row cell.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use crate::app::App;
use crate::input::action::Action;
use crate::state::file_entry::{FileEntry, FileKind};
use crate::state::pane::TabId;
use crate::ui::drag_hover::{
    drag_hover_recorder_with_drop, DragHoverCommand, DragHoverDropCommand, DragHoverId,
    DragHoverKind,
};
use crate::ui::file_icons::FileIcon;
use crate::ui::glass::{backdrop_tint, glass_fill, glass_header_fill};
use crate::ui::root::{
    ctrl_scroll, pane_bounds_recorder, rect_recorder, zone_recorder, DraggedFiles,
};
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
        let cell_stride = CELL_W + 6.0;
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
        let avail = (pane_w - 16.0).max(cell_stride);
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
                .children(msg)
                .into_any_element()
        } else {
            let scroll = self.scroll_handle_for(pane_id);
            let list_view = view.clone();
            uniform_list(
                SharedString::from(format!("listc-{}", pane_id.0)),
                row_count,
                move |range, window, cx| {
                    let start = range.start;
                    let end = range.end;
                    crate::perf::count_by("list.rows.requested", end.saturating_sub(start) as u64);
                    let _perf = crate::perf::span_detail("list.rows.build", || {
                        format!("pane={} range={start}..{end} cols={cols}", pane_id.0)
                    });
                    list_view.update(cx, |app, _cx| {
                        let record_rects = app.dock_split_animations.is_empty() && app.pane_layout_animations.is_empty();
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
                                        let selected = ts.selection.selected.contains(&entry_idx);
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
                                    let rename_input = crate::ui::views::rename::active_input(
                                        app, pane_id, entry_idx,
                                    );
                                    let selected = selected
                                        && !crate::ui::views::rename::suppress_row_chrome(
                                            app, pane_id, entry_idx,
                                        );
                                    cells.push(render_cell_compact(
                                        &entry,
                                        entry_idx,
                                        selected,
                                        pane_id,
                                        &list_view,
                                        accent,
                                        accent_fg,
                                        hover,
                                        flashing,
                                        record_rects,
                                        file_icon,
                                        rename_input,
                                        window,
                                    ));
                                }
                                let row_el = h_flex().w_full().gap(px(6.)).px_2().children(cells);
                                row_el.into_any_element()
                            })
                            .collect::<Vec<_>>()
                    })
                },
            )
            .track_scroll(&scroll)
            .h_full()
            .into_any_element()
        };

        let (filter_pill, controls_pill) = self.render_pane_pills(pane_id, window, cx);
        let focus_view = view.clone();
        let rubber_view = view.clone();
        let zone_view = view.clone();
        let prepaint_view = view.clone();
        let bounds_view = view.clone();
        let scroll_view = view;
        v_flex()
            .id(SharedString::from(format!("pane-{}", pane_id.0)))
            .size_full()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, {
                let v = rubber_view.clone();
                move |ev, _window, cx| {
                    let (pos, additive) =
                        (ev.position, ev.modifiers.control || ev.modifiers.platform);
                    v.update(cx, |this, cx| {
                        if this.rename_edit.is_some() {
                            this.commit_rename(cx);
                        }
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
                        this.drop_files_on_pane(pane_id, paths, false);
                        cx.notify();
                    });
                }
            })
            .on_drop::<gpui::ExternalPaths>({
                let v = rubber_view;
                move |paths, _window, cx| {
                    let paths = paths.paths().to_vec();
                    v.update(cx, |this, cx| {
                        this.drop_files_on_pane(pane_id, paths, true);
                        cx.notify();
                    });
                }
            })
            .on_click(move |_ev, _window, cx| {
                focus_view.update(cx, |this, cx| {
                    this.push_focus_history();
                    this.state.focused_tile = pane_id;
                    cx.notify();
                });
            })
            .on_scroll_wheel(move |ev, _window, cx| ctrl_scroll(ev, pane_id, &scroll_view, cx))
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
            // FASE F: register this pane's window-space rect each frame for OLE drop routing.
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
#[allow(clippy::too_many_arguments)]
fn render_cell_compact(
    entry: &FileEntry,
    entry_idx: usize,
    selected: bool,
    pane_id: TabId,
    view: &Entity<App>,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    flashing: bool,
    record_rects: bool,
    file_icon: FileIcon,
    rename_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    window: &mut Window,
) -> AnyElement {
    let name = entry.name.clone();
    let v = view.clone();
    let font_size = window.text_style().font_size.to_pixels(window.rem_size());
    h_flex()
        .id(SharedString::from(format!(
            "lc-{}-{}",
            pane_id.0, entry_idx
        )))
        .when(record_rects, |s| {
            s.on_prepaint(rect_recorder(pane_id, entry_idx, view))
        })
        .w(px(220.))
        .px_1()
        .py(px(2.))
        .gap_1()
        .items_center()
        .rounded_sm()
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
                .w(px(16.))
                .flex_none()
                .child(file_icon.element(14.0, accent)),
        )
        .child(match &rename_input {
            Some(input) => div()
                .w(px(crate::ui::views::rename::input_width(
                    &name, 188.0, font_size, window,
                )))
                .h(px(crate::ui::views::rename::INLINE_INPUT_H))
                .flex_none()
                .child(crate::ui::views::rename::input_element(
                    input,
                    view,
                    crate::ui::views::rename::INLINE_INPUT_H,
                    font_size,
                ))
                .into_any_element(),
            None => div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .child(name)
                .into_any_element(),
        })
        .on_click(move |ev, _window, cx| {
            let m = ev.modifiers();
            let shift = m.shift;
            let ctrl = m.control || m.platform;
            let activate = ev.click_count() >= 2;
            v.update(cx, |this, cx| {
                if this.rename_edit.is_some() {
                    this.commit_rename(cx);
                }
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
                    let paths = this.select_context_menu_paths(pane_id, entry_idx);
                    this.open_ctx_menu(paths, pos, window, cx);
                    cx.notify();
                });
            }
        })
        .when(!selected && rename_input.is_none(), |s| {
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
        .when(matches!(entry.kind, FileKind::Dir), |s| {
            let dest = entry.path.clone();
            let hover_id = DragHoverId::Folder(pane_id, dest.clone());
            let dest_internal = dest.clone();
            let dest_external = dest.clone();
            let mv = view.clone();
            let cpv = view.clone();
            s.drag_over::<DraggedFiles>(|s, _, _, _| s.opacity(0.6))
                .on_prepaint(drag_hover_recorder_with_drop(
                    hover_id,
                    DragHoverKind::Folder,
                    DragHoverCommand::NavigatePathIn(pane_id, dest.clone()),
                    DragHoverDropCommand::TransferTo(dest.clone()),
                    view,
                ))
                .on_drop::<DraggedFiles>(move |d, _window, cx| {
                    let paths = d.paths.clone();
                    mv.update(cx, |this, cx| {
                        this.execute_drag_hover_drop_command(
                            DragHoverDropCommand::TransferTo(dest_internal.clone()),
                            paths,
                            true,
                        );
                        cx.notify();
                    });
                })
                .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
                    let paths = paths.paths().to_vec();
                    cpv.update(cx, |this, cx| {
                        this.execute_drag_hover_drop_command(
                            DragHoverDropCommand::TransferTo(dest_external.clone()),
                            paths,
                            false,
                        );
                        cx.notify();
                    });
                })
        })
        .into_any_element()
}
