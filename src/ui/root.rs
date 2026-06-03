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
use crate::util::display::{display_name, format_date, format_size};
use gpui::prelude::*;
use gpui::{
    div, hsla, px, uniform_list, AnyElement, Context, Div, Entity, Hsla, SharedString, Stateful,
    Window,
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

        let body = match self.state.pane_tree.root {
            Some(id) => self.render_tile(id, window, cx),
            None => div().into_any_element(),
        };

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .text_sm()
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .bg(title_bar)
                    .child("Atlas")
                    .child(div().text_color(muted).child(path)),
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
                    .child("listo"),
            )
    }
}

impl App {
    /// Recursively render a tile (pane / tabs / split) into an element.
    fn render_tile(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.state.pane_tree.tiles.get(id) {
            Some(Tile::Pane(_)) => self.render_pane(id, cx),
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
    fn render_pane(&mut self, pane_id: TabId, cx: &mut Context<Self>) -> AnyElement {
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

        let list_view = view.clone();
        let list = uniform_list(
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
                            entry, entry_idx, selected, pane_id, &list_view, accent, accent_fg,
                            hover, muted,
                        ))
                    })
                    .collect::<Vec<_>>()
            },
        );

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
            .child(div().flex_1().min_h(px(0.)).child(list.h_full()))
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
