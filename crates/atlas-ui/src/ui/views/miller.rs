//! Miller-columns view: `render_miller` + column row.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use super::should_suppress_loading;
use crate::app::App;
use crate::input::action::Action;
use crate::state::file_entry::{FileEntry, FileKind};
use crate::state::pane::TabId;
use crate::state::tab::LoadState;
use crate::ui::drag_hover::{
    drag_hover_recorder_with_drop, DragHoverCommand, DragHoverDropCommand, DragHoverId,
    DragHoverKind,
};
use crate::ui::file_icons::FileIcon;
use crate::ui::glass::{backdrop_tint, glass_fill, glass_header_fill};
use crate::ui::icons::{icon, AtlasIcon};
use crate::ui::root::{ctrl_scroll, zone_recorder, DraggedFiles};
use crate::util::display::display_name;
use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, AnyElement, Context, Entity, Hsla, MouseButton, SharedString, Window,
};
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
        // Lazily seed the first column from the pane path on entry.
        let need_init = self
            .state
            .tab_states
            .get(&pane_id)
            .is_none_or(|ts| ts.miller.columns.is_empty());
        if need_init {
            if let Some(loc) = self.tab_data_for(pane_id).map(|t| t.location.clone()) {
                self.process_actions(vec![Action::MillerInit {
                    tile: pane_id,
                    location: loc,
                }]);
            }
        }
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

        // Snapshot each column's rows up-front; Miller dirs are browsed one level at a
        // time, so a column is rarely huge — a scrollable v_flex per column is enough.
        let mut columns: Vec<AnyElement> = Vec::new();
        if let Some(ts) = self.state.tab_states.get(&pane_id) {
            for (ci, col) in ts.miller.columns.iter().enumerate() {
                let title = display_name(&col.location.display_path());
                let body: AnyElement = if col.entries.is_empty() {
                    let msg = match col.load_state {
                        LoadState::Loading if should_suppress_loading(&col.location) => None,
                        LoadState::Loading => Some(rust_i18n::t!("miller_loading")),
                        LoadState::Error(_) => Some(rust_i18n::t!("miller_error")),
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
                    uniform_list(
                        SharedString::from(format!("mcol-{}-{}", pane_id.0, ci)),
                        count,
                        move |range, _window, cx| {
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
                                        render_miller_row(
                                            &entry, ci, i, selected, pane_id, &list_view, accent,
                                            accent_fg, hover, flashing, file_icon,
                                        )
                                        .into_any_element()
                                    })
                                    .collect::<Vec<_>>()
                            })
                        },
                    )
                    .flex_1()
                    .h_full()
                    .min_h(px(0.))
                    .into_any_element()
                };

                let col_loc = col.location.fs_dir().map(|p| p.to_path_buf());
                let mut col_container = v_flex()
                    .w(px(240.))
                    .h_full()
                    .border_r_1()
                    .border_color(header_bg)
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
                    let dest_internal = col_path.clone();
                    let dest_external = col_path.clone();
                    let hover_id = crate::ui::drag_hover::DragHoverId::Folder(pane_id, col_path.clone());
                    col_container = col_container
                        .drag_over::<crate::ui::root::DraggedFiles>(|s, _, _, _| s.bg(crate::ui::glass::glass_fill().opacity(0.1)))
                        .on_prepaint(crate::ui::drag_hover::drag_hover_recorder_with_drop(
                            hover_id,
                            crate::ui::drag_hover::DragHoverKind::Folder,
                            crate::ui::drag_hover::DragHoverCommand::NavigatePathIn(pane_id, col_path.clone()),
                            crate::ui::drag_hover::DragHoverDropCommand::TransferTo(col_path.clone()),
                            &view,
                        ))
                        .on_drop::<crate::ui::root::DraggedFiles>(move |d, _window, cx| {
                            let paths = d.paths.clone();
                            mv.update(cx, |this, cx| {
                                this.execute_drag_hover_drop_command(
                                    crate::ui::drag_hover::DragHoverDropCommand::TransferTo(dest_internal.clone()),
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
                                    crate::ui::drag_hover::DragHoverDropCommand::TransferTo(dest_external.clone()),
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
        let focus_view = view.clone();
        let rubber_view = view.clone();
        let zone_view = view.clone();
        let scroll_view = view;
        v_flex()
            .id(SharedString::from(format!("pane-{}", pane_id.0)))
            .size_full()
            .on_mouse_down(MouseButton::Left, {
                let v = rubber_view.clone();
                move |ev, _window, cx| {
                    let (pos, additive) =
                        (ev.position, ev.modifiers.control || ev.modifiers.platform);
                    v.update(cx, |this, _| this.begin_rubber(pane_id, pos, additive));
                }
            })
            .on_mouse_down(MouseButton::Right, {
                let v = rubber_view.clone();
                move |ev, window, cx| {
                    let pos = ev.position;
                    v.update(cx, |this, cx| {
                        if let Some(path) = this
                            .tab_data_for(pane_id)
                            .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
                        {
                            this.open_background_ctx_menu(Some(path), pos, window, cx);
                            cx.notify();
                        }
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
#[allow(clippy::too_many_arguments)]
fn render_miller_row(
    entry: &FileEntry,
    column: usize,
    idx: usize,
    selected: bool,
    pane_id: TabId,
    view: &Entity<App>,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    flashing: bool,
    file_icon: FileIcon,
) -> AnyElement {
    let is_dir = matches!(entry.kind, FileKind::Dir);
    let trailing = if is_dir {
        icon(
            AtlasIcon::CaretRight,
            12.0,
            if selected { accent_fg } else { accent },
        )
    } else {
        div().w(px(12.)).flex_none().into_any_element()
    };
    let name = entry.name.clone();
    let dest = entry.path.clone();
    let v = view.clone();
    let mut row = h_flex()
        .id(SharedString::from(format!(
            "mrow-{}-{}-{}",
            pane_id.0, column, idx
        )))
        .w_full()
        .px_2()
        .py(px(3.))
        .gap_2()
        .items_center()
        .cursor_pointer()
        .when(selected, |s| s.bg(accent).text_color(accent_fg))
        .when(flashing && !selected, |s| s.bg(accent.opacity(0.38)))
        .when(!selected, |s| s.hover(|h| h.bg(hover)))
        .child(
            div()
                .w(px(16.))
                .flex_none()
                .child(file_icon.element(14.0, accent)),
        )
        .child(div().flex_1().min_w(px(0.)).truncate().child(name))
        .child(trailing)
        .on_drag(
            crate::ui::drag::file_drag_value(vec![dest.clone()]),
            {
                let v = view.clone();
                let _drag_dest = dest.clone();
                move |_d, _pos, _window, cx| {
                    v.update(cx, |this, cx| {
                        this.process_actions(vec![Action::MillerSelect {
                            tile: pane_id,
                            column,
                            idx,
                        }]);
                        this.begin_drag_hover();
                        cx.notify();
                    });
                    #[cfg(windows)]
                    {
                        cx.new(|_| crate::ui::root::DragNoop)
                    }
                    #[cfg(not(windows))]
                    {
                        cx.new(|_| crate::ui::root::DraggedFiles { paths: vec![drag_dest.clone()] })
                    }
                }
            },
        )
        .on_click(move |ev, _window, cx| {
            let activate = ev.click_count() >= 2;
            v.update(cx, |this, cx| {
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
                    }]);
                }
                cx.notify();
            });
        });
    if is_dir {
        let hover_id = DragHoverId::Folder(pane_id, dest.clone());
        let dest_internal = dest.clone();
        let dest_external = dest.clone();
        let mv = view.clone();
        let cpv = view.clone();
        row = row
            .drag_over::<DraggedFiles>(|s, _, _, _| s.opacity(0.6))
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
            });
    }
    row.into_any_element()
}
