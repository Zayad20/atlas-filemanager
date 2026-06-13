//! Details view (table): `render_pane` (dispatch + Details body) + row/header cells.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use crate::app::App;
use crate::input::action::Action;
use crate::state::file_entry::{FileEntry, FileKind, SortColumn, SortDir};
use crate::state::pane::TabId;
use crate::state::settings::ViewMode;
use crate::state::tab::LoadState;
use crate::ui::glass::GLASS_RADIUS;
use crate::ui::icons::file_type_icon;
use crate::util::display::{format_date, format_size};
use gpui::prelude::*;
use gpui::{
    div, img, px, uniform_list, AnyElement, Context, Div,
    Entity, Hsla, MouseButton, SharedString, Stateful, Window,
};
use gpui_component::button::ButtonVariants as _;
use gpui_component::input::InputState;
use gpui_component::{h_flex, v_flex, ActiveTheme, ElementExt};
use crate::ui::root::{ctrl_scroll, rect_recorder, zone_recorder, DraggedFiles};

impl App {
    /// Render the file view for a pane (Details/Grid/List/Miller). Public to the
    /// crate so `ui::pane_panel::FilePanel` (a Dock `Panel`) can delegate here.
    pub(crate) fn render_pane(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view_mode = self
            .tab_data_for(pane_id)
            .map(|t| t.view_mode.clone())
            .unwrap_or_default();
        match view_mode {
            ViewMode::Grid => return self.render_grid(pane_id, window, cx),
            ViewMode::List => return self.render_list_compact(pane_id, window, cx),
            ViewMode::Miller => return self.render_miller(pane_id, window, cx),
            ViewMode::Details => {}
        }
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
        let (sort_col, sort_dir) = self
            .tab_data_for(pane_id)
            .map(|t| (t.sort_col.clone(), t.sort_dir.clone()))
            .unwrap_or_default();
        let count = self
            .state
            .tab_states
            .get_mut(&pane_id)
            .map_or(0, |ts| ts.visible_indices().len());

        let header = h_flex()
            .w_full()
            .px_2()
            .py_1()
            .gap_2()
            .bg(header_bg)
            .text_xs()
            .text_color(muted)
            .child(div().w(px(20.)))
            .child(
                sort_cell(
                    &view,
                    pane_id,
                    SortColumn::Name,
                    "Nombre",
                    &sort_col,
                    &sort_dir,
                )
                .flex_1(),
            )
            .child(
                sort_cell(
                    &view,
                    pane_id,
                    SortColumn::Size,
                    "Tamaño",
                    &sort_col,
                    &sort_dir,
                )
                .w(px(90.)),
            )
            .child(
                sort_cell(
                    &view,
                    pane_id,
                    SortColumn::Modified,
                    "Modificado",
                    &sort_col,
                    &sort_dir,
                )
                .w(px(150.)),
            )
            // Trailing empty column: a deliberate "dead zone" to start a rubber-band
            // drag-select without clicking on a data cell.
            .child(div().w(px(40.)));

        // Empty / loading / error placeholder, or the virtualized list.
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
                SharedString::from(format!("list-{}", pane_id.0)),
                count,
                move |range, _window, cx| {
                    let app = list_view.read(cx);
                    let Some(ts) = app.state.tab_states.get(&pane_id) else {
                        return Vec::new();
                    };
                    let record_rects = app.rubber.as_ref().is_some_and(|r| r.pane == pane_id);
                    range
                        .filter_map(|i| {
                            let entry_idx = *ts.visible.get(i)?;
                            let entry = ts.entries.get(entry_idx)?;
                            let selected = ts.selection.selected.contains(&entry_idx);
                            let rename_input = app
                                .rename_edit
                                .as_ref()
                                .filter(|e| e.pane == pane_id && e.idx == entry_idx)
                                .map(|e| e.input.clone());
                            let icon_tex =
                                app.icon_tex_for_entry(&entry.kind, entry.extension.as_deref());
                            Some(render_row(
                                entry,
                                entry_idx,
                                selected,
                                pane_id,
                                &list_view,
                                accent,
                                accent_fg,
                                hover,
                                muted,
                                rename_input,
                                record_rects,
                                icon_tex,
                            ))
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
        let zone_view = view.clone();
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
            // FASE F: register this pane's window-space rect each frame for OLE drop routing.
            .when(cfg!(windows), |el| {
                el.on_prepaint(zone_recorder(pane_id, &zone_view))
            })
            .child(crumb_strip)
            .child(header)
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

/// A clickable column header that toggles the sort column/direction.
fn sort_cell(
    view: &Entity<App>,
    pane_id: TabId,
    col: SortColumn,
    label: &str,
    cur_col: &SortColumn,
    cur_dir: &SortDir,
) -> Stateful<Div> {
    let arrow = if col == *cur_col {
        if *cur_dir == SortDir::Asc {
            " ▲"
        } else {
            " ▼"
        }
    } else {
        ""
    };
    let next_dir = if col == *cur_col && *cur_dir == SortDir::Asc {
        SortDir::Desc
    } else {
        SortDir::Asc
    };
    let v = view.clone();
    div()
        .id(SharedString::from(format!("hdr-{}-{:?}", pane_id.0, col)))
        .cursor_pointer()
        .child(format!("{label}{arrow}"))
        .on_click(move |_ev, _window, cx| {
            let col = col.clone();
            let dir = next_dir.clone();
            v.update(cx, |this, cx| {
                this.process_actions(vec![Action::SetSort { col, dir }]);
                cx.notify();
            });
        })
}

/// One file row in the Details list. Selection/activation route through `Action`s,
/// reading click modifiers (shift = range, ctrl/cmd = toggle) and count (≥2 = open).
#[allow(clippy::too_many_arguments)]
fn render_row(
    entry: &FileEntry,
    entry_idx: usize,
    selected: bool,
    pane_id: TabId,
    view: &Entity<App>,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    muted: Hsla,
    rename_input: Option<Entity<InputState>>,
    record_rects: bool,
    icon_tex: Option<std::sync::Arc<gpui::RenderImage>>,
) -> AnyElement {
    let is_dir = matches!(entry.kind, FileKind::Dir);
    let name = entry.name.clone();
    let size = if is_dir {
        String::new()
    } else {
        entry.size.map(format_size).unwrap_or_default()
    };
    let date = entry.modified.map(format_date).unwrap_or_default();
    let v = view.clone();
    h_flex()
        .id(SharedString::from(format!(
            "row-{}-{}",
            pane_id.0, entry_idx
        )))
        .when(record_rects, |s| {
            s.on_prepaint(rect_recorder(pane_id, entry_idx, view))
        })
        .w_full()
        .px_2()
        .py(px(3.))
        .gap_2()
        .items_center()
        .rounded_sm()
        .cursor_pointer()
        .when(selected, |s| s.bg(accent).text_color(accent_fg))
        .when(!selected, |s| s.hover(|h| h.bg(hover)))
        .child(
            div().w(px(20.)).child(match icon_tex {
                Some(tex) => img(tex)
                    .w(px(16.))
                    .h(px(16.))
                    .flex_none()
                    .into_any_element(),
                None => file_type_icon(&entry.kind, entry.extension.as_deref(), 16.0),
            }),
        )
        .child(match &rename_input {
            Some(input) => div()
                .flex_1()
                .min_w(px(0.))
                .child(crate::ui::text_input::bordered(input))
                .into_any_element(),
            None => div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .child(name)
                .into_any_element(),
        })
        .child(
            div()
                .w(px(90.))
                .text_right()
                .when(!selected, |s| s.text_color(muted))
                .child(size),
        )
        .child(
            div()
                .w(px(150.))
                .when(!selected, |s| s.text_color(muted))
                .child(date),
        )
        // Trailing "dead zone": starts a rubber-band drag from within a row (the row
        // swallows left mouse-down for selection, so this column re-initiates it).
        .child({
            let rv = view.clone();
            div()
                .w(px(40.))
                .h_full()
                .on_mouse_down(MouseButton::Left, move |ev, _window, cx| {
                    let (pos, additive) =
                        (ev.position, ev.modifiers.control || ev.modifiers.platform);
                    rv.update(cx, |this, cx| {
                        this.begin_rubber(pane_id, pos, additive);
                        cx.notify();
                    });
                    cx.stop_propagation();
                })
        })
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
        // While renaming this row, Enter commits and Esc cancels (capture phase so the
        // focused input doesn't swallow them first).
        .when_some(rename_input, |row, _input| {
            let cv = view.clone();
            row.capture_key_down(move |ev, _window, app| match ev.keystroke.key.as_str() {
                "enter" => {
                    cv.update(app, App::commit_rename);
                    app.stop_propagation();
                }
                "escape" => {
                    cv.update(app, |this, _cx| this.cancel_rename());
                    app.stop_propagation();
                }
                _ => {}
            })
        })
        .into_any_element()
}

