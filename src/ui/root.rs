//! Root gpui view: the window shell and the recursive render of the pane/tab/split
//! tree (`state::layout_tree`), replacing egui_tiles.
//!
//! - `Container::Tabs`  → a `TabBar` + the active pane's content.
//! - `Container::Linear`→ a `Resizable` group (h/v) with one panel per child.
//! - `Tile::Pane`       → the file listing for that tab (focus ring + click-to-focus).
//!
//! Structural change emits an `Action`/calls an `App` method; the tree never mutates
//! itself during rendering.

use crate::app::App;
use crate::input::action::Action;
use crate::state::file_entry::{FileEntry, FileKind, SortColumn, SortDir};
use crate::state::layout_tree::{Container, LinearDir, Tile};
use crate::state::pane::TabId;
use crate::state::settings::ViewMode;
use crate::state::tab::LoadState;
use crate::util::display::{display_name, format_date, format_size};
use gpui::prelude::*;
use gpui::{
    div, hsla, img, px, uniform_list, AnyElement, Context, Div, Entity, Hsla, SharedString,
    Stateful, Window,
};
use gpui_component::{
    h_flex,
    resizable::{h_resizable, resizable_panel, v_resizable},
    tab::{Tab, TabBar},
    v_flex, ActiveTheme,
};

/// A small clickable glyph in the tab toolbar, bound to an `App` mutation.
fn toolbar_btn(
    id: String,
    glyph: &'static str,
    view: Entity<App>,
    action: impl Fn(&mut App, &mut Context<App>) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(id))
        .px_1()
        .cursor_pointer()
        .child(glyph)
        .on_click(move |_ev, _window, cx| {
            view.update(cx, |this, cx| action(this, cx));
        })
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (bg, fg, muted, title_bar, sidebar) = {
            let t = cx.theme();
            (
                t.background,
                t.foreground,
                t.muted_foreground,
                t.title_bar,
                t.sidebar,
            )
        };
        let path = self
            .state
            .active_tab_data()
            .map(|t| t.path.display().to_string())
            .unwrap_or_default();

        // Status-bar summary for the focused tab.
        let status_text = match self.state.active_tab_state() {
            Some(ts) => match &ts.load_state {
                LoadState::Loading => "Cargando…".to_string(),
                LoadState::Error(e) => format!("Error: {e}"),
                _ => {
                    let items = ts.entries.len();
                    let sel = ts.selection.selected.len();
                    if sel > 0 {
                        let bytes: u64 = ts
                            .selection
                            .selected
                            .iter()
                            .filter_map(|&i| ts.entries.get(i))
                            .filter_map(|e| e.size)
                            .sum();
                        format!("{items} elementos · {sel} seleccionados · {}", format_size(bytes))
                    } else {
                        format!("{items} elementos")
                    }
                }
            },
            None => String::new(),
        };

        let body = match self.state.pane_tree.root {
            Some(id) => self.render_tile(id, window, cx),
            None => div().into_any_element(),
        };

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .text_sm()
            .key_context("Atlas")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &crate::NavUp, _w, cx| {
                this.nav_up(false);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NavDown, _w, cx| {
                this.nav_down(false);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::ExtendUp, _w, cx| {
                this.nav_up(true);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::ExtendDown, _w, cx| {
                this.nav_down(true);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NavHome, _w, cx| {
                this.nav_home(false);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NavEnd, _w, cx| {
                this.nav_end(false);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NavPageUp, _w, cx| {
                this.nav_page(false, false);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NavPageDown, _w, cx| {
                this.nav_page(true, false);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::OpenSelected, _w, cx| {
                this.open_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::SelectAllItems, _w, cx| {
                this.process_actions(vec![Action::SelectAll]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NewTab, _w, cx| {
                this.process_actions(vec![Action::NewTabFocused]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::CloseTab, _w, cx| {
                this.process_actions(vec![Action::CloseFocusedTab]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NextTab, _w, cx| {
                this.focus_next_tab();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::PrevTab, _w, cx| {
                this.focus_prev_tab();
                cx.notify();
            }))
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .bg(title_bar)
                    .child("Atlas")
                    .child(div().flex_1().text_color(muted).truncate().child(path))
                    .child(toolbar_btn("vm-details".into(), "≣", cx.entity(), |this, cx| {
                        this.process_actions(vec![Action::SetViewMode(ViewMode::Details)]);
                        cx.notify();
                    }))
                    .child(toolbar_btn("vm-grid".into(), "▦", cx.entity(), |this, cx| {
                        this.process_actions(vec![Action::SetViewMode(ViewMode::Grid)]);
                        cx.notify();
                    })),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .child(div().w(px(220.)).h_full().bg(sidebar).p_2().child("Sidebar"))
                    .child(div().flex_1().h_full().min_w(px(0.)).child(body)),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1()
                    .bg(title_bar)
                    .text_xs()
                    .text_color(muted)
                    .child(status_text),
            )
    }
}

impl App {
    /// Recursively render a tile (pane / tabs / split) into an element.
    fn render_tile(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.state.pane_tree.tiles.get(id) {
            Some(Tile::Pane(_)) => self.render_pane(id, window, cx),
            Some(Tile::Container(Container::Tabs(tabs))) => {
                let children = tabs.children.clone();
                let active = tabs.active;
                self.render_tabs(id, children, active, window, cx)
            }
            Some(Tile::Container(Container::Linear(lin))) => {
                let children = lin.children.clone();
                let vertical = matches!(lin.dir, LinearDir::Vertical);
                self.render_split(id, children, vertical, window, cx)
            }
            None => div().into_any_element(),
        }
    }

    fn render_split(
        &mut self,
        id: TabId,
        children: Vec<TabId>,
        vertical: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group_id = SharedString::from(format!("split-{}", id.0));
        let mut group = if vertical {
            v_resizable(group_id)
        } else {
            h_resizable(group_id)
        };
        for child in children {
            let content = self.render_tile(child, window, cx);
            group = group.child(resizable_panel().child(content));
        }
        div().size_full().child(group).into_any_element()
    }

    fn render_tabs(
        &mut self,
        container_id: TabId,
        children: Vec<TabId>,
        active: Option<TabId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let active_idx = active
            .and_then(|a| children.iter().position(|&c| c == a))
            .unwrap_or(0);

        // One Tab per child pane, each with a × close affordance (which must not
        // also trigger tab-select, hence `stop_propagation`).
        let tabs: Vec<Tab> = children
            .iter()
            .map(|&tile| {
                let label = self
                    .tab_data_for(tile)
                    .map(|t| display_name(&t.path))
                    .unwrap_or_default();
                let close_view = view.clone();
                Tab::new().label(label).suffix(
                    div()
                        .id(SharedString::from(format!("close-{}", tile.0)))
                        .px_1()
                        .cursor_pointer()
                        .child("×")
                        .on_click(move |_ev, _window, cx| {
                            cx.stop_propagation();
                            close_view.update(cx, |this, cx| {
                                this.close_tab(tile);
                                cx.notify();
                            });
                        }),
                )
            })
            .collect();

        let on_select_children = children.clone();
        let on_select_view = view.clone();
        let tab_bar = TabBar::new(SharedString::from(format!("tabbar-{}", container_id.0)))
            .selected_index(active_idx)
            .children(tabs)
            .on_click(move |ix, _window, cx| {
                if let Some(&tile) = on_select_children.get(*ix) {
                    on_select_view.update(cx, |this, cx| {
                        this.activate_tab(tile);
                        cx.notify();
                    });
                }
            })
            .suffix(
                h_flex()
                    .gap_1()
                    .px_1()
                    .items_center()
                    .child(toolbar_btn(
                        format!("split-r-{}", container_id.0),
                        "⬌",
                        view.clone(),
                        move |this, cx| {
                            this.split_pane(container_id, true);
                            cx.notify();
                        },
                    ))
                    .child(toolbar_btn(
                        format!("split-d-{}", container_id.0),
                        "⬍",
                        view.clone(),
                        move |this, cx| {
                            this.split_pane(container_id, false);
                            cx.notify();
                        },
                    ))
                    .child(toolbar_btn(
                        format!("newtab-{}", container_id.0),
                        "+",
                        view.clone(),
                        move |this, cx| {
                            this.new_tab(container_id);
                            cx.notify();
                        },
                    )),
            );

        let content = match active {
            Some(a) => self.render_tile(a, window, cx),
            None => div().into_any_element(),
        };

        v_flex()
            .size_full()
            .child(tab_bar)
            .child(div().flex_1().min_h(px(0.)).child(content))
            .into_any_element()
    }

    /// Details view: a virtualized (`uniform_list`) table with sortable header.
    /// The list closure reads live state (`view.read`) and formats only the visible
    /// range — no per-frame allocation over the full directory, so 100k-entry dirs
    /// stay smooth.
    fn render_pane(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view_mode = self
            .tab_data_for(pane_id)
            .map(|t| t.view_mode.clone())
            .unwrap_or_default();
        if matches!(view_mode, ViewMode::Grid) {
            return self.render_grid(pane_id, window, cx);
        }
        let view = cx.entity();
        let focused = self.state.focused_tile == pane_id;
        let (accent, accent_fg, hover, muted, header_bg) = {
            let t = cx.theme();
            (
                t.accent,
                t.accent_foreground,
                t.list_active,
                t.muted_foreground,
                t.secondary,
            )
        };
        let (sort_col, sort_dir) = self
            .tab_data_for(pane_id)
            .map(|t| (t.sort_col.clone(), t.sort_dir.clone()))
            .unwrap_or_default();
        let count = self
            .state
            .tab_states
            .get_mut(&pane_id)
            .map(|ts| ts.visible_indices().len())
            .unwrap_or(0);

        let header = h_flex()
            .w_full()
            .px_2()
            .py_1()
            .gap_2()
            .bg(header_bg)
            .text_xs()
            .text_color(muted)
            .child(div().w(px(20.)))
            .child(sort_cell(&view, pane_id, SortColumn::Name, "Nombre", &sort_col, &sort_dir).flex_1())
            .child(sort_cell(&view, pane_id, SortColumn::Size, "Tamaño", &sort_col, &sort_dir).w(px(90.)))
            .child(
                sort_cell(&view, pane_id, SortColumn::Modified, "Modificado", &sort_col, &sort_dir)
                    .w(px(150.)),
            );

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
                    range
                        .filter_map(|i| {
                            let entry_idx = *ts.visible.get(i)?;
                            let entry = ts.entries.get(entry_idx)?;
                            let selected = ts.selection.selected.contains(&entry_idx);
                            Some(render_row(
                                entry, entry_idx, selected, pane_id, &list_view, accent,
                                accent_fg, hover, muted,
                            ))
                        })
                        .collect::<Vec<_>>()
                },
            )
            .track_scroll(&scroll)
            .h_full()
            .into_any_element()
        };

        let focus_view = view.clone();
        v_flex()
            .id(SharedString::from(format!("pane-{}", pane_id.0)))
            .size_full()
            .border_2()
            .border_color(if focused { accent } else { hsla(0., 0., 0., 0.) })
            .on_click(move |_ev, _window, cx| {
                focus_view.update(cx, |this, cx| {
                    this.state.focused_tile = pane_id;
                    cx.notify();
                });
            })
            .child(header)
            .child(div().flex_1().min_h(px(0.)).child(inner))
            .into_any_element()
    }

    /// Icons (Grid) view: a virtualized grid of fixed-size cells. Column count is
    /// derived from the viewport width and the per-tab thumbnail size, so it stays
    /// virtualized (only visible rows built) for huge directories.
    fn render_grid(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let focused = self.state.focused_tile == pane_id;
        let (accent, accent_fg, hover, muted) = {
            let t = cx.theme();
            (t.accent, t.accent_foreground, t.list_active, t.muted_foreground)
        };
        let thumb = self
            .tab_data_for(pane_id)
            .map(|t| t.thumbnail_size)
            .unwrap_or(128.0);
        let count = self
            .state
            .tab_states
            .get_mut(&pane_id)
            .map(|ts| ts.visible_indices().len())
            .unwrap_or(0);
        let cell_w = thumb + 16.0;
        let avail = (f32::from(window.viewport_size().width) - 232.0).max(cell_w);
        let cols = ((avail / cell_w).floor() as usize).max(1);
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
                    let app = list_view.read(cx);
                    let Some(ts) = app.state.tab_states.get(&pane_id) else {
                        return Vec::new();
                    };
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
                                cells.push(render_cell(
                                    entry, entry_idx, selected, pane_id, &list_view, thumb, accent,
                                    accent_fg, hover,
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

        let focus_view = view.clone();
        v_flex()
            .id(SharedString::from(format!("pane-{}", pane_id.0)))
            .size_full()
            .border_2()
            .border_color(if focused { accent } else { hsla(0., 0., 0., 0.) })
            .on_click(move |_ev, _window, cx| {
                focus_view.update(cx, |this, cx| {
                    this.state.focused_tile = pane_id;
                    cx.notify();
                });
            })
            .child(inner)
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
) -> AnyElement {
    let is_dir = matches!(entry.kind, FileKind::Dir);
    let icon = if is_dir { "📁" } else { "📄" };
    let name = entry.name.clone();
    let size = if is_dir {
        String::new()
    } else {
        entry.size.map(format_size).unwrap_or_default()
    };
    let date = entry.modified.map(format_date).unwrap_or_default();
    let v = view.clone();
    h_flex()
        .id(SharedString::from(format!("row-{}-{}", pane_id.0, entry_idx)))
        .w_full()
        .px_2()
        .py(px(3.))
        .gap_2()
        .items_center()
        .rounded_sm()
        .cursor_pointer()
        .when(selected, |s| s.bg(accent).text_color(accent_fg))
        .when(!selected, |s| s.hover(|h| h.bg(hover)))
        .child(div().w(px(20.)).child(icon))
        .child(div().flex_1().min_w(px(0.)).truncate().child(name))
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
        .into_any_element()
}

/// Extensions rendered as inline image thumbnails in the grid.
fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff"
    )
}

/// One grid cell: big icon + truncated name, fixed to the thumbnail size.
/// (Thumbnails for image files land in the next step; for now a scaled glyph.)
#[allow(clippy::too_many_arguments)]
fn render_cell(
    entry: &FileEntry,
    entry_idx: usize,
    selected: bool,
    pane_id: TabId,
    view: &Entity<App>,
    thumb: f32,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
) -> AnyElement {
    let icon = if matches!(entry.kind, FileKind::Dir) {
        "📁"
    } else {
        "📄"
    };
    let is_image = matches!(entry.kind, FileKind::File)
        && entry.extension.as_deref().map(is_image_ext).unwrap_or(false);
    let path = entry.path.clone();
    let name = entry.name.clone();
    let v = view.clone();
    v_flex()
        .id(SharedString::from(format!("cell-{}-{}", pane_id.0, entry_idx)))
        .w(px(thumb))
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
            // gpui decodes + caches the image by path; the cell constrains display
            // size. (Full-res decode for now — real downscaled thumbnails later.)
            if is_image {
                area.child(img(path).max_h(px(thumb * 0.6)).max_w(px(thumb)).rounded_sm())
            } else {
                area.text_size(px(thumb * 0.5)).child(icon)
            }
        })
        .child(div().w_full().text_center().text_xs().truncate().child(name))
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
        .into_any_element()
}
