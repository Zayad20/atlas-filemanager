//! Details view (table): `render_pane` (dispatch + Details body) + row/header cells.
#![allow(clippy::too_many_lines)] // reason: cohesive view renderer kept whole

use crate::app::{App, ColResizeState};
use crate::input::action::Action;
use crate::state::file_entry::{FileEntry, FileKind, SortColumn, SortDir};
use crate::state::pane::TabId;
use crate::state::settings::ViewMode;
use crate::ui::icons::{file_type_icon, icon, AtlasIcon};
use crate::ui::root::{ctrl_scroll, rect_recorder, zone_recorder, DraggedFiles};
use crate::util::display::{format_date, format_size};
use gpui::prelude::*;
use gpui::{
    div, img, px, uniform_list, AnyElement, Context, Div, Entity, Hsla, MouseButton, SharedString,
    Stateful, Window,
};
use gpui_component::input::InputState;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex, ActiveTheme, ElementExt};
use std::sync::Arc;

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

        let col_widths = self.col_widths;
        let type_filter_set = self.type_filters.get(&pane_id).cloned().unwrap_or_default();
        let has_type_filter = !type_filter_set.is_empty();

        // Unique type labels for the filter overlay (computed from all entries).
        let type_labels: Vec<String> = {
            let mut set = std::collections::BTreeSet::new();
            if let Some(ts) = self.state.tab_states.get(&pane_id) {
                for entry in &ts.entries {
                    set.insert(entry_type_label(entry));
                }
            }
            set.into_iter().collect()
        };

        // Filtered visible list: applies type_filters on top of the fuzzy filter.
        let filtered_visible: Arc<Vec<usize>> =
            if let Some(ts) = self.state.tab_states.get_mut(&pane_id) {
                let base: Vec<usize> = ts.visible_indices().to_vec();
                if type_filter_set.is_empty() {
                    Arc::new(base)
                } else {
                    let entries = &ts.entries;
                    Arc::new(
                        base.into_iter()
                            .filter(|&idx| {
                                entries
                                    .get(idx)
                                    .is_some_and(|e| type_filter_set.contains(&entry_type_label(e)))
                            })
                            .collect(),
                    )
                }
            } else {
                Arc::new(vec![])
            };
        let count = filtered_visible.len();

        let header = h_flex()
            .w_full()
            .px_2()
            .py_1()
            .gap_2()
            .bg(header_bg)
            .text_xs()
            .text_color(muted)
            // Icon placeholder.
            .child(div().w(px(20.)))
            // Name: flex_1, sortable.
            .child(
                sort_cell(
                    &view,
                    pane_id,
                    SortColumn::Name,
                    "Nombre",
                    &sort_col,
                    &sort_dir,
                    muted,
                    accent,
                )
                .flex_1(),
            )
            // Size: fixed width, sortable, resizable.
            .child(col_wrapper(
                col_widths.size,
                sort_cell(
                    &view,
                    pane_id,
                    SortColumn::Size,
                    "Tamaño",
                    &sort_col,
                    &sort_dir,
                    muted,
                    accent,
                )
                .w_full()
                .into_any_element(),
                "size",
                pane_id,
                col_widths.size,
                &view,
                muted,
            ))
            // Modified: fixed width, sortable, resizable.
            .child(col_wrapper(
                col_widths.modified,
                sort_cell(
                    &view,
                    pane_id,
                    SortColumn::Modified,
                    "Modificado",
                    &sort_col,
                    &sort_dir,
                    muted,
                    accent,
                )
                .w_full()
                .into_any_element(),
                "modified",
                pane_id,
                col_widths.modified,
                &view,
                muted,
            ))
            // Type: fixed width, sort by extension, right-click for filter, resizable.
            .child(col_wrapper(
                col_widths.type_w,
                type_header_cell(
                    &view,
                    pane_id,
                    &sort_col,
                    &sort_dir,
                    has_type_filter,
                    type_labels,
                    muted,
                    accent,
                )
                .w_full()
                .into_any_element(),
                "type",
                pane_id,
                col_widths.type_w,
                &view,
                muted,
            ))
            // Trailing dead zone: rubber-band drag starts here.
            .child(div().w(px(40.)));

        // Empty / loading / error placeholder, or the virtualized list.
        let inner = if count == 0 {
            let msg = self.empty_pane_message(pane_id, "Carpeta vacía");
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(muted)
                .children(msg)
                .into_any_element()
        } else {
            let scroll = self.scroll_handle_for(pane_id);
            let base_scroll = scroll.0.borrow().base_handle.clone();
            let list_view = view.clone();
            let fv = filtered_visible.clone();
            div()
                .size_full()
                .relative()
                .child(
                    uniform_list(
                        SharedString::from(format!("list-{}", pane_id.0)),
                        count,
                        move |range, _window, cx| {
                            let app = list_view.read(cx);
                            let Some(ts) = app.state.tab_states.get(&pane_id) else {
                                return Vec::new();
                            };
                            let record_rects =
                                app.rubber.as_ref().is_some_and(|r| r.pane == pane_id);
                            let cw = app.col_widths;
                            range
                                .filter_map(|i| {
                                    let entry_idx = *fv.get(i)?;
                                    let entry = ts.entries.get(entry_idx)?;
                                    let selected = ts.selection.selected.contains(&entry_idx);
                                    let rename_input = app
                                        .rename_edit
                                        .as_ref()
                                        .filter(|e| e.pane == pane_id && e.idx == entry_idx)
                                        .map(|e| e.input.clone());
                                    let icon_tex = app.icon_tex_for_entry(
                                        &entry.kind,
                                        entry.extension.as_deref(),
                                    );
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
                                        cw.size,
                                        cw.modified,
                                        cw.type_w,
                                    ))
                                })
                                .collect::<Vec<_>>()
                        },
                    )
                    .track_scroll(&scroll)
                    .pr(px(16.))
                    .h_full(),
                )
                .vertical_scrollbar(&base_scroll)
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
            // rounded glass card, with the tab selector inside it.
            .overflow_hidden()
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
                        if let Some(dest) = this
                            .tab_data_for(pane_id)
                            .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
                        {
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
                        if let Some(dest) = this
                            .tab_data_for(pane_id)
                            .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
                        {
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

/// Type label string for the Type column cell and filter overlay.
fn entry_type_label(entry: &FileEntry) -> String {
    match &entry.kind {
        FileKind::Dir => "Carpeta".to_string(),
        FileKind::Symlink => "Enlace".to_string(),
        FileKind::File => entry
            .extension
            .as_deref()
            .map(|e| format!(".{e}"))
            .unwrap_or_else(|| "—".to_string()),
    }
}

/// Relative wrapper that gives a column a fixed width and embeds a resize grip
/// at its left edge (= boundary with the previous column).
fn col_wrapper(
    width: f32,
    content: AnyElement,
    col: &'static str,
    pane_id: TabId,
    cur_width: f32,
    view: &Entity<App>,
    muted: Hsla,
) -> Div {
    div()
        .relative()
        .w(px(width))
        .child(content)
        .child(col_resize_handle(col, cur_width, pane_id, view, muted))
}

/// Thin absolute-positioned grab handle at the left edge of a column wrapper.
/// Mouse-down captures the drag origin; `rubber_layer` updates widths on move.
fn col_resize_handle(
    col: &'static str,
    cur_width: f32,
    pane_id: TabId,
    view: &Entity<App>,
    muted: Hsla,
) -> Stateful<Div> {
    let v = view.clone();
    div()
        .id(SharedString::from(format!("rh-{col}-{}", pane_id.0)))
        .absolute()
        .left(px(0.))
        .top_0()
        .bottom_0()
        .w(px(5.))
        .cursor_col_resize()
        .hover(|s| s.bg(muted.opacity(0.35)))
        .on_mouse_down(MouseButton::Left, move |ev, _window, cx| {
            let start_x = f32::from(ev.position.x);
            v.update(cx, |this, _cx| {
                this.col_resize = Some(ColResizeState {
                    col,
                    start_x,
                    start_width: cur_width,
                });
            });
            cx.stop_propagation();
        })
}

/// Clickable column header that toggles sort column/direction.
/// Phosphor ArrowUp/Down icons indicate the active sort direction.
fn sort_cell(
    view: &Entity<App>,
    pane_id: TabId,
    col: SortColumn,
    label: &str,
    cur_col: &SortColumn,
    cur_dir: &SortDir,
    muted: Hsla,
    accent: Hsla,
) -> Stateful<Div> {
    let is_active = col == *cur_col;
    let next_dir = if is_active && *cur_dir == SortDir::Asc {
        SortDir::Desc
    } else {
        SortDir::Asc
    };
    let label_color = if is_active { accent } else { muted };
    let sort_icon = if is_active {
        let kind = if *cur_dir == SortDir::Asc {
            AtlasIcon::ArrowUp
        } else {
            AtlasIcon::ArrowDown
        };
        Some(icon(kind, 9.0, accent))
    } else {
        None
    };
    let v = view.clone();
    div()
        .id(SharedString::from(format!("hdr-{}-{:?}", pane_id.0, col)))
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_1()
        .text_color(label_color)
        .child(label.to_string())
        .when_some(sort_icon, |el, ico| el.child(ico))
        .on_click(move |_ev, _window, cx| {
            let col = col.clone();
            let dir = next_dir.clone();
            v.update(cx, |this, cx| {
                this.process_actions(vec![Action::SetSort { col, dir }]);
                cx.notify();
            });
        })
}

/// Type column header: left-click sorts by extension, right-click opens the
/// searchable multi-select filter overlay.
#[allow(clippy::too_many_arguments)]
fn type_header_cell(
    view: &Entity<App>,
    pane_id: TabId,
    cur_col: &SortColumn,
    cur_dir: &SortDir,
    has_filter: bool,
    type_labels: Vec<String>,
    muted: Hsla,
    accent: Hsla,
) -> Stateful<Div> {
    let col = SortColumn::Extension;
    let is_active = col == *cur_col;
    let next_dir = if is_active && *cur_dir == SortDir::Asc {
        SortDir::Desc
    } else {
        SortDir::Asc
    };
    let label_color = if has_filter || is_active { accent } else { muted };
    let sort_icon = if is_active {
        let kind = if *cur_dir == SortDir::Asc {
            AtlasIcon::ArrowUp
        } else {
            AtlasIcon::ArrowDown
        };
        Some(icon(kind, 9.0, accent))
    } else {
        None
    };
    let v = view.clone();
    let vr = view.clone();
    div()
        .id(SharedString::from(format!("hdr-type-{}", pane_id.0)))
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_1()
        .text_color(label_color)
        .child("Tipo")
        // Active filter indicator: small dot next to the label.
        .when(has_filter, |el| {
            el.child(div().w(px(5.)).h(px(5.)).rounded_full().bg(accent))
        })
        .when_some(sort_icon, |el, ico| el.child(ico))
        .on_click(move |_ev, _window, cx| {
            let col = col.clone();
            let dir = next_dir.clone();
            v.update(cx, |this, cx| {
                this.process_actions(vec![Action::SetSort { col, dir }]);
                cx.notify();
            });
        })
        .on_mouse_down(MouseButton::Right, move |ev, window, cx| {
            let pos = ev.position;
            let labels = type_labels.clone();
            vr.update(cx, |this, cx| {
                this.open_type_filter_menu(pane_id, pos, labels, window, cx);
            });
            cx.stop_propagation();
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
    size_w: f32,
    modified_w: f32,
    type_w: f32,
) -> AnyElement {
    let is_dir = matches!(entry.kind, FileKind::Dir);
    let name = entry.name.clone();
    let size = if is_dir {
        String::new()
    } else {
        entry.size.map(format_size).unwrap_or_default()
    };
    let date = entry.modified.map(format_date).unwrap_or_default();
    let type_label = entry_type_label(entry);
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
        // Icon
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
        // Name
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
        // Size
        .child(
            div()
                .w(px(size_w))
                .text_right()
                .when(!selected, |s| s.text_color(muted))
                .child(size),
        )
        // Modified
        .child(
            div()
                .w(px(modified_w))
                .when(!selected, |s| s.text_color(muted))
                .child(date),
        )
        // Type
        .child(
            div()
                .w(px(type_w))
                .when(!selected, |s| s.text_color(muted))
                .child(type_label),
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
