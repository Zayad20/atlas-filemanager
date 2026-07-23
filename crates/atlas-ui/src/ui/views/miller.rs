//! Miller-columns view: `render_miller` + column row.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use super::should_suppress_loading;
use crate::app::App;
use crate::input::action::Action;
use crate::ui::drag_hover::DragHoverId;
use crate::ui::file_icons::FileIcon;
use crate::ui::glass::{backdrop_tint, glass_fill, glass_header_fill};
use crate::ui::icons::{icon, AtlasIcon};
use crate::ui::root::{miller_column_bounds_recorder, miller_rect_recorder, zone_recorder};
use atlas_core::util::display::display_name;
use atlas_model::file_entry::{FileEntry, FileKind};
use atlas_model::pane::TabId;
use atlas_model::tab::LoadState;
use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, AnyElement, Context, Entity, Hsla, MouseButton, Pixels, SharedString,
    Window,
};
use gpui_component::input::InputState;
use gpui_component::{h_flex, v_flex, ActiveTheme, ElementExt};

impl App {
    /// Miller (columns) view: a horizontal row of directory columns. Selecting a
    /// folder in a column opens its contents in the next column to the right. The
    /// cascade lives in `TabState.miller` (loaded via `BgResult::MillerListed`); this
    /// only paints it and routes clicks through the `Miller*` actions.
    pub(crate) fn render_miller(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        crate::perf::count("render.view.miller");
        let _perf = crate::perf::span_detail("miller.render", || format!("pane={}", pane_id.0));
        // Seed (or re-seed) the cascade from the pane location. Comparing the
        // first column against the live location catches external navigation
        // (sidebar/breadcrumb/back) — otherwise the columns would show the old
        // path forever, since nothing else ever resets `ts.miller`.
        let pane_loc = self.tab_data_for(pane_id).map(|t| t.location.clone());
        let need_init = match (&pane_loc, self.state.tab_states.get(&pane_id)) {
            (Some(loc), Some(ts)) => ts.miller.columns.first().map(|c| &c.location) != Some(loc),
            (Some(_), None) => true,
            _ => false,
        };
        if need_init {
            if let Some(loc) = pane_loc {
                self.process_actions(vec![Action::MillerInit {
                    tile: pane_id,
                    location: loc,
                }]);
            }
        }
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
        let font_size = window.text_style().font_size.to_pixels(window.rem_size());
        let row_font = window.text_style().font();
        let crumb_strip = self.render_crumb_strip(
            pane_id, &view, fg, muted, border,
        );

        // Snapshot each column's rows up-front; Miller dirs are browsed one level at a
        // time, so a column is rarely huge — a scrollable v_flex per column is enough.
        let mut columns: Vec<AnyElement> = Vec::new();
        let column_count = self
            .state
            .tab_states
            .get(&pane_id)
            .map(|ts| ts.miller.columns.len())
            .unwrap_or_default();
        for ci in 0..column_count {
            if let Some(col) = self
                .state
                .tab_states
                .get(&pane_id)
                .and_then(|ts| ts.miller.columns.get(ci))
            {
                let title = display_name(&col.location.display_path());
                let body: AnyElement = if col.entries.is_empty() {
                    let msg = match &col.load_state {
                        LoadState::Loading if should_suppress_loading(&col.location) => None,
                        LoadState::Loading => Some(rust_i18n::t!("miller_loading")),
                        LoadState::Error(error) if error.is_empty() => {
                            Some(rust_i18n::t!("miller_error"))
                        }
                        LoadState::Error(error) => Some(error.clone().into()),
                        _ => Some(rust_i18n::t!("miller_empty")),
                    };
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_color(muted)
                        .children(msg)
                        .into_any_element()
                } else {
                    let count = col.entries.len();
                    let list_view = view.clone();
                    let scroll = self
                        .miller_scroll_handles
                        .entry((pane_id, ci))
                        .or_default()
                        .clone();
                    uniform_list(
                        SharedString::from(format!("mcol-{}-{}", pane_id.0, ci)),
                        count,
                        move |range, window, cx| {
                            let start = range.start;
                            let end = range.end;
                            crate::perf::count_by(
                                "miller.rows.requested",
                                end.saturating_sub(start) as u64,
                            );
                            let _perf = crate::perf::span_detail("miller.rows.build", || {
                                format!("pane={} col={ci} range={start}..{end}", pane_id.0)
                            });
                            list_view.update(cx, |app, _cx| {
                                let rows_data = {
                                    let Some(col) = app
                                        .state
                                        .tab_states
                                        .get(&pane_id)
                                        .and_then(|ts| ts.miller.columns.get(ci))
                                    else {
                                        return Vec::new();
                                    };
                                    range
                                        .filter_map(|i| {
                                            let entry = col.entries.get(i)?;
                                            let selected = col.selection.selected.contains(&i);
                                            Some((i, entry.clone(), selected))
                                        })
                                        .collect::<Vec<_>>()
                                };
                                rows_data
                                    .into_iter()
                                    .map(|(i, entry, selected)| {
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
                                            crate::ui::views::rename::active_input_for_path(
                                                app,
                                                pane_id,
                                                &entry.path,
                                            );
                                        render_miller_row(MillerRowProps {
                                            entry: &entry,
                                            column: ci,
                                            idx: i,
                                            selected,
                                            pane_id,
                                            view: &list_view,
                                            accent,
                                            accent_fg,
                                            hover,
                                            flashing,
                                            file_icon,
                                            rename_input,
                                            dimmed: crate::ui::views::rename::dim_other_path(
                                                app,
                                                pane_id,
                                                &entry.path,
                                            ),
                                            window,
                                        })
                                        .into_any_element()
                                    })
                                    .collect::<Vec<_>>()
                            })
                        },
                    )
                    .track_scroll(&scroll)
                    .flex_1()
                    .h_full()
                    .min_h(px(0.))
                    .into_any_element()
                };

                let col_loc = col.location.fs_dir().map(|p| p.to_path_buf());
                let rubber_view = view.clone();
                let bounds_view = view.clone();
                let column_flashing = col_loc.as_ref().is_some_and(|path| {
                    self.drag_hover.blink_id.as_ref()
                        == Some(&DragHoverId::Folder(pane_id, path.clone()))
                });
                let mut col_container = v_flex()
                    .w(crate::ui::rem_px(240.))
                    .h_full()
                    .border_r_1()
                    .border_color(header_bg)
                    .when(column_flashing, |s| s.border_color(accent))
                    .on_prepaint(miller_column_bounds_recorder(pane_id, ci, &bounds_view))
                    .on_mouse_down(MouseButton::Left, move |ev, _window, cx| {
                        let (pos, additive) = (
                            ev.position,
                            ev.modifiers.control || ev.modifiers.platform || ev.modifiers.shift,
                        );
                        rubber_view.update(cx, |this, cx| {
                            if this.rename_edit.is_some() {
                                this.commit_rename(cx);
                            }
                            this.begin_miller_rubber(pane_id, ci, pos, additive);
                            cx.notify();
                        });
                        cx.stop_propagation();
                    })
                    .child(
                        div()
                            .w_full()
                            .px_2()
                            .py_1()
                            .bg(header_bg)
                            .text_xs()
                            .text_color(muted)
                            .truncate()
                            .child(title),
                    )
                    .child(body);

                if let Some(col_path) = col_loc {
                    let mv = view.clone();
                    let cpv = view.clone();
                    let bounds_v = view.clone();
                    let dest_internal = col_path.clone();
                    let dest_external = col_path.clone();
                    let hover_id =
                        crate::ui::drag_hover::DragHoverId::Folder(pane_id, col_path.clone());
                    let hover_record = crate::ui::drag_hover::drag_hover_recorder_with_drop(
                        hover_id,
                        crate::ui::drag_hover::DragHoverKind::Folder,
                        crate::ui::drag_hover::DragHoverCommand::NavigatePathIn(
                            pane_id,
                            col_path.clone(),
                        ),
                        crate::ui::drag_hover::DragHoverDropCommand::TransferTo(col_path.clone()),
                        &view,
                    );
                    col_container = col_container
                        .drag_over::<crate::ui::root::DraggedFiles>(|s, _, _, _| {
                            s.bg(crate::ui::glass::glass_fill().opacity(0.1))
                        })
                        .on_prepaint(move |bounds, window, cx| {
                            bounds_v.update(cx, |this, _| {
                                this.miller_column_bounds.insert((pane_id, ci), bounds);
                            });
                            hover_record(bounds, window, cx);
                        })
                        .on_drop::<crate::ui::root::DraggedFiles>(move |d, _window, cx| {
                            let paths = d.paths.clone();
                            mv.update(cx, |this, cx| {
                                this.execute_drag_hover_drop_command(
                                    crate::ui::drag_hover::DragHoverDropCommand::TransferTo(
                                        dest_internal.clone(),
                                    ),
                                    paths,
                                    true,
                                );
                                cx.notify();
                            });
                            cx.stop_propagation();
                        })
                        .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
                            let paths = paths.paths().to_vec();
                            cpv.update(cx, |this, cx| {
                                this.execute_drag_hover_drop_command(
                                    crate::ui::drag_hover::DragHoverDropCommand::TransferTo(
                                        dest_external.clone(),
                                    ),
                                    paths,
                                    false,
                                );
                                cx.notify();
                            });
                            cx.stop_propagation();
                        });
                }

                columns.push(col_container.into_any_element());
            }
        }

        let (filter_pill, controls_pill) = self.render_pane_pills(pane_id, window, cx);
        let zone_view = view.clone();
        let pane_view = view;
        crate::ui::views::pane_background_interactions(
            v_flex()
                .id(SharedString::from(format!("pane-{}", pane_id.0)))
                .size_full(),
            &pane_view,
            pane_id,
        )
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
                .child(
                    h_flex()
                        .id(SharedString::from(format!("miller-{}", pane_id.0)))
                        .size_full()
                        .overflow_x_scroll()
                        .children(columns),
                )
                .child(filter_pill)
                .child(controls_pill),
        )
        .into_any_element()
    }
}

/// One row in a Miller column. Single click selects (cascading the next column);
/// double click on a file opens it. Routed through the `Miller*` actions.
struct MillerRowProps<'a> {
    entry: &'a FileEntry,
    column: usize,
    idx: usize,
    selected: bool,
    pane_id: TabId,
    view: &'a Entity<App>,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    flashing: bool,
    file_icon: FileIcon,
    rename_input: Option<Entity<InputState>>,
    dimmed: bool,
    window: &'a mut Window,
}

/// One row in a Miller column. Single click selects (cascading the next column);
/// double click on a file opens it. Routed through the `Miller*` actions.
fn render_miller_row(props: MillerRowProps) -> AnyElement {
    let MillerRowProps {
        entry,
        column,
        idx,
        selected,
        pane_id,
        view,
        accent,
        accent_fg,
        hover,
        flashing,
        file_icon,
        rename_input,
        dimmed,
        window,
    } = props;
    let is_dir = matches!(entry.kind, FileKind::Dir);
    let trailing = if is_dir {
        icon(
            AtlasIcon::CaretRight,
            12.0,
            if selected { accent_fg } else { accent },
        )
    } else {
        div()
            .w(crate::ui::rem_px(12.))
            .flex_none()
            .into_any_element()
    };
    let name = entry.name.clone();
    let dest = entry.path.clone();
    let tip_name = entry.name.clone();
    let tip_path = entry.path.clone();
    let v = view.clone();
    let font_size: Pixels = window.text_style().font_size.to_pixels(window.rem_size());
    let mut row = h_flex()
        .id(("mrow", idx))
        .on_prepaint(miller_rect_recorder(pane_id, column, idx, view))
        .w_full()
        .px_2()
        .py(crate::ui::rem_px(3.))
        .gap_2()
        .items_center()
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
                h_flex()
                    .w(px(crate::ui::views::rename::input_width(
                        &name,
                        190.0 * f32::from(window.rem_size()) / crate::ui::BASE_PX,
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
        .child(trailing)
        .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Right, {
            let v = view.clone();
            let ctx_path = dest.clone();
            move |ev, window, cx| {
                let pos = ev.position;
                v.update(cx, |this, cx| {
                    let already_selected = this
                        .state
                        .tab_states
                        .get(&pane_id)
                        .and_then(|ts| ts.miller.columns.get(column))
                        .is_some_and(|col| col.selection.selected.contains(&idx));
                    if !already_selected {
                        this.process_actions(vec![Action::MillerSelect {
                            tile: pane_id,
                            column,
                            idx,
                            shift: false,
                            ctrl: false,
                        }]);
                    } else {
                        this.activate_tab(pane_id);
                        if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                            ts.miller.focused = column;
                        }
                    }
                    let paths = {
                        let p = this.miller_focused_selection_paths().unwrap_or_default();
                        if p.is_empty() {
                            vec![ctx_path.clone()]
                        } else {
                            p
                        }
                    };
                    this.open_ctx_menu(paths, pos, window, cx);
                    cx.notify();
                });
                cx.stop_propagation();
            }
        })
        .on_click(move |ev, _window, cx| {
            let activate = ev.click_count() >= 2;
            let m = ev.modifiers();
            let (shift, ctrl) = (m.shift, m.control || m.platform);
            v.update(cx, |this, cx| {
                if this.rename_edit.is_some() {
                    this.commit_rename(cx);
                }
                if activate {
                    this.process_actions(vec![Action::MillerActivate {
                        tile: pane_id,
                        column,
                        idx,
                    }]);
                } else {
                    this.process_actions(vec![Action::MillerSelect {
                        tile: pane_id,
                        column,
                        idx,
                        shift,
                        ctrl,
                    }]);
                }
                cx.notify();
            });
        });
    if rename_input.is_none() {
        let drag_view = view.clone();
        row = row.on_drag(crate::ui::drag::file_drag_value(vec![dest.clone()]), {
            let anchor = dest.clone();
            let in_archive = entry.archive_root.is_some();
            move |_d, _pos, _window, cx| {
                drag_view.update(cx, |this, cx| {
                    let selected = this
                        .state
                        .tab_states
                        .get(&pane_id)
                        .and_then(|ts| ts.miller.columns.get(column))
                        .is_some_and(|col| col.selection.selected.contains(&idx));
                    if !selected {
                        this.process_actions(vec![Action::MillerSelect {
                            tile: pane_id,
                            column,
                            idx,
                            shift: false,
                            ctrl: false,
                        }]);
                    } else {
                        this.activate_tab(pane_id);
                        if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                            ts.miller.focused = column;
                        }
                    }
                    this.begin_drag_hover();
                    cx.notify();
                });
                let paths = {
                    let app = drag_view.read(&*cx);
                    let p = app.miller_focused_selection_paths().unwrap_or_default();
                    if p.is_empty() {
                        vec![anchor.clone()]
                    } else {
                        p
                    }
                };
                #[cfg(windows)]
                {
                    // Archive rows carry virtual paths — no OS drag for
                    // them; extract-on-drag when someone needs it.
                    if !in_archive {
                        drag_view.read(&*cx).trigger_os_drag(paths, None);
                    }
                    cx.new(|_| crate::ui::root::DragNoop)
                }
                #[cfg(not(windows))]
                {
                    let _ = in_archive;
                    let urls: Vec<String> = paths
                        .iter()
                        .map(|p| format!("file://{}", p.display()))
                        .collect();
                    _window.start_urilist_drag(urls);
                    cx.new(|_| crate::ui::root::DraggedFiles { paths })
                }
            }
        });
    }
    let entry_size = entry.size;
    let entry_modified = entry.modified;
    row = row.tooltip(move |window, cx| {
        crate::ui::tooltips::entry_hover_tooltip(
            tip_name.clone(),
            tip_path.clone(),
            is_dir,
            entry_size,
            entry_modified,
            window,
            cx,
        )
    });
    if is_dir {
        row = crate::ui::views::folder_drop_target(row, view, pane_id, dest);
    }
    row.into_any_element()
}
