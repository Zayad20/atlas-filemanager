//! Window chrome: tab pills, breadcrumb strip, floating search pill, rubber marquee.
#![allow(clippy::too_many_lines)] // reason: cohesive UI block kept whole

use crate::app::App;
use crate::input::action::Action;
use crate::state::location::Location;
use crate::state::pane::TabId;
use crate::state::settings::ViewMode;
use crate::ui::drag_hover::{drag_hover_recorder, DragHoverCommand, DragHoverId, DragHoverKind};
use crate::ui::glass::{glass_border, glass_fill, glass_shadows_enabled};
use crate::ui::icons::{icon, AtlasIcon};
use crate::ui::search_shell::search_icon;
use crate::ui::tooltips::shortcut_tooltip;
use gpui::prelude::*;
use gpui::{
    deferred, div, px, AnyElement, Context, Entity, Hsla, MouseButton, SharedString, Window,
};
use gpui_component::button::ButtonVariants as _;
use gpui_component::menu::{ContextMenuExt, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, ActiveTheme, ElementExt, Icon, Sizable as _};
use std::collections::HashSet;
use std::path::PathBuf;

impl App {
    pub(crate) fn render_floating_search_pill(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(18.))
            .flex()
            .justify_center()
            // The wrapper is non-interactive so only the pill captures clicks.
            .child(
                h_flex()
                    .id("search-pill")
                    .h(px(36.))
                    .px_4()
                    .gap_2()
                    .items_center()
                    .rounded_full()
                    .bg(glass_fill())
                    .border_1()
                    .border_color(glass_border())
                    .when(glass_shadows_enabled(), |s| s.shadow_lg())
                    .cursor_pointer()
                    .text_color(muted)
                    .hover(|s| s.bg(glass_fill().opacity(0.68)))
                    .active(|s| s.bg(glass_fill().opacity(0.78)))
                    .child(search_icon(muted))
                    .child("Buscar")
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        this.process_actions(vec![Action::OpenGotoBar]);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// FASE C: the visible selection rectangle for the active drag, in window coords
    /// (same space as `row_rects`), painted as a deferred layer over the panes. Hidden
    /// until the drag actually moves, so a plain click doesn't flash a box.
    pub(crate) fn render_rubber_marquee(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let r = self.rubber.as_ref()?;
        let (ox, oy) = (f32::from(r.origin.x), f32::from(r.origin.y));
        let (cxp, cyp) = (f32::from(r.current.x), f32::from(r.current.y));
        let left = ox.min(cxp);
        let top = oy.min(cyp);
        let w = (ox.max(cxp) - left).max(0.0);
        let h = (oy.max(cyp) - top).max(0.0);
        if w < 2.0 && h < 2.0 {
            return None;
        }
        let accent = cx.theme().accent;
        let mut fill = accent;
        fill.a = 0.15;
        Some(
            deferred(
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(w))
                    .h(px(h))
                    .bg(fill)
                    .border_1()
                    .border_color(accent)
                    .rounded_sm(),
            )
            .into_any_element(),
        )
    }

    /// Compact breadcrumb strip rendered at the top of each pane, showing the
    /// pane's current path. Clicking any segment navigates that specific pane.
    pub(crate) fn render_crumb_strip(
        &self,
        pane_id: TabId,
        view: &Entity<App>,
        fg: Hsla,
        muted: Hsla,
        border: Hsla,
    ) -> AnyElement {
        let pane_archive_inner = self.tab_data_for(pane_id).and_then(|t| {
            t.location
                .as_archive()
                .map(|(archive, inner)| (archive.to_path_buf(), inner.to_path_buf()))
        });
        let pane_path = self
            .tab_data_for(pane_id)
            .map(|t| t.location.display_path());
        let (can_back, can_forward, up_location, recent_locations, bookmark_target) = self
            .tab_data_for(pane_id)
            .map(|t| {
                (
                    !t.history.is_empty(),
                    !t.history_forward.is_empty(),
                    t.location.ascend(),
                    self.recent_locations_for_pane(pane_id),
                    self.bookmark_target_for_pane(pane_id),
                )
            })
            .unwrap_or_default();
        let crumbs: Option<AnyElement> = if let Some((archive, inner)) = pane_archive_inner.as_ref()
        {
            Some(archive_breadcrumb(view, pane_id, archive, inner, fg, muted).into_any_element())
        } else {
            pane_path
                .as_ref()
                .map(|p| breadcrumb(view, pane_id, p, fg, muted).into_any_element())
        };
        let menu_view = view.clone();
        let menu_recent = recent_locations.clone();
        let menu_up = up_location.clone();
        let menu_bookmark = bookmark_target.clone();
        h_flex()
            .w_full()
            .h(px(36.))
            .px_3()
            .min_w(px(0.))
            .gap(px(6.))
            .items_center()
            .border_b_1()
            .border_color(border)
            .text_sm()
            .text_color(muted)
            .child(nav_button(
                "pane-back",
                pane_id,
                AtlasIcon::ArrowLeft,
                rust_i18n::t!("tooltip_nav_back_title").to_string(),
                rust_i18n::t!("tooltip_nav_back_desc").to_string(),
                &["alt-left"],
                can_back,
                Action::NavigateBack(pane_id),
                view.clone(),
                recent_locations.clone(),
                up_location.clone(),
                bookmark_target.clone(),
            ))
            .child(nav_button(
                "pane-forward",
                pane_id,
                AtlasIcon::ArrowRight,
                rust_i18n::t!("tooltip_nav_forward_title").to_string(),
                rust_i18n::t!("tooltip_nav_forward_desc").to_string(),
                &["alt-right"],
                can_forward,
                Action::NavigateForward(pane_id),
                view.clone(),
                recent_locations.clone(),
                up_location.clone(),
                bookmark_target.clone(),
            ))
            .child(nav_button(
                "pane-up",
                pane_id,
                AtlasIcon::ArrowUp,
                rust_i18n::t!("tooltip_nav_up_title").to_string(),
                rust_i18n::t!("tooltip_nav_up_desc").to_string(),
                &["alt-up"],
                up_location.is_some(),
                Action::NavigateUp(pane_id),
                view.clone(),
                recent_locations.clone(),
                up_location.clone(),
                bookmark_target.clone(),
            ))
            .child(bookmark_button(
                pane_id,
                view.clone(),
                bookmark_target.clone(),
            ))
            .child(
                div()
                    .flex_none()
                    .w(px(1.))
                    .h(px(18.))
                    .bg(border.opacity(0.65)),
            )
            .children(crumbs)
            .context_menu(move |menu, _window, _cx| {
                nav_context_menu(
                    menu,
                    pane_id,
                    menu_view.clone(),
                    menu_recent.clone(),
                    menu_up.clone(),
                    menu_bookmark.clone(),
                )
            })
            .into_any_element()
    }

    fn recent_locations_for_pane(&self, pane_id: TabId) -> Vec<Location> {
        let Some(tab) = self.tab_data_for(pane_id) else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        let mut locations = Vec::new();
        seen.insert(tab.location.clone());
        for loc in tab
            .history
            .iter()
            .rev()
            .chain(tab.history_forward.iter().rev())
        {
            if seen.insert(loc.clone()) {
                locations.push(loc.clone());
                if locations.len() == 6 {
                    return locations;
                }
            }
        }
        for path in self.state.settings.recent_folders.iter().rev() {
            let loc = Location::Directory(path.clone());
            if seen.insert(loc.clone()) {
                locations.push(loc);
                if locations.len() == 6 {
                    break;
                }
            }
        }
        locations
    }

    fn bookmark_target_for_pane(&self, pane_id: TabId) -> Option<BookmarkTarget> {
        let path = self
            .tab_data_for(pane_id)
            .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| path.display().to_string());
        let exists = self
            .state
            .profiles
            .active()
            .bookmarks
            .iter()
            .any(|bookmark| bookmark.path == path);
        Some(BookmarkTarget { name, path, exists })
    }

    /// Floating per-pane overlays, anchored to the bottom corners of the file
    /// list: a compact fuzzy-filter pill on the left (a lone magnifier until it
    /// activates) and a controls pill on the right (visibility toggles,
    /// folders-first, view selector). Both sit on translucent glass so the
    /// listing shows through.
    pub(crate) fn render_pane_pills(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, AnyElement) {
        // Mirror the per-tab toggles into the TabState so its cached filtering
        // (and the counts below) pick them up.
        let (folders_first, show_files, show_folders) = self
            .tab_data_for(pane_id)
            .map(|td| (td.folders_first, td.show_files, td.show_folders))
            .unwrap_or((true, true, true));
        if let Some(ts) = self.state.tab_states.get_mut(&pane_id) {
            ts.set_visibility(show_files, show_folders);
        }

        let initial = self
            .state
            .tab_states
            .get(&pane_id)
            .map(|ts| ts.search.query.clone())
            .unwrap_or_default();
        let input = self
            .search_inputs
            .entry(pane_id)
            .or_insert_with(|| crate::ui::text_input::single_line(window, cx, "Filtrar…", initial))
            .clone();

        let value = input.read(cx).value().to_string();
        let mut needs_refresh = false;
        if let Some(ts) = self.state.tab_states.get_mut(&pane_id) {
            // Focus is owned by `sync_overlay_focus`: while `search.active` the
            // field keeps the keyboard, so no per-render focus juggling here.
            ts.search.focus = false;
            if value != ts.search.query {
                ts.search.query.clone_from(&value);
                // Typing directly into the always-visible input activates the filter.
                if !value.is_empty() {
                    ts.search.active = true;
                }
                needs_refresh = true;
            }
        }
        if needs_refresh {
            self.refresh_search_in_place(pane_id);
        }

        let (active, recursive, status, count, error, counts) = self
            .state
            .tab_states
            .get_mut(&pane_id)
            .map(|ts| {
                (
                    ts.search.active,
                    ts.search.recursive,
                    ts.search.status,
                    ts.visible_indices().len(),
                    ts.search.error.clone(),
                    ts.counts(),
                )
            })
            .unwrap_or_default();
        let filtering = active && !value.trim().is_empty();
        let (border, muted, fg, accent, accent_fg, secondary) = {
            let t = cx.theme();
            (
                t.border,
                t.muted_foreground,
                t.foreground,
                t.accent,
                t.accent_foreground,
                t.secondary,
            )
        };
        let view = cx.entity();
        // Concise status inside the pill: result count, "…" while searching,
        // "!" (with the error in its own right) on failure.
        let status_text = if error.is_some() {
            Some("!".to_string())
        } else if status == crate::state::tab::SearchStatus::Searching {
            Some("…".to_string())
        } else if filtering {
            Some(format!("{count}"))
        } else {
            None
        };

        // Only the active pane shows the floating chrome. An inactive pane that
        // still has a filter applied keeps a tiny badge so it's obvious why
        // entries are missing; clicking it focuses the pane and the field.
        let focused = self.state.focused_tile == pane_id;
        if !focused {
            let badge: AnyElement = if active {
                let open_view = view.clone();
                h_flex()
                    .id(SharedString::from(format!("filter-badge-{}", pane_id.0)))
                    .absolute()
                    .bottom_2()
                    .left_2()
                    .px_2()
                    .py(px(3.))
                    .gap_1()
                    .items_center()
                    .rounded_full()
                    .border_1()
                    .border_color(accent.opacity(0.6))
                    .bg(secondary.opacity(0.85))
                    .shadow_md()
                    .text_xs()
                    .cursor_pointer()
                    .child(icon(AtlasIcon::Search, 12.0, accent))
                    .children(
                        status_text.map(|t| div().text_color(muted).whitespace_nowrap().child(t)),
                    )
                    .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                        cx.stop_propagation();
                        open_view.update(cx, |this, cx| {
                            this.state.focused_tile = pane_id;
                            if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                                ts.search.focus = true;
                            }
                            cx.notify();
                        });
                    })
                    .into_any_element()
            } else {
                div().into_any_element()
            };
            return (badge, div().into_any_element());
        }

        // ── Left pill: fuzzy filter (lone magnifier until active) ──
        let filter_pill: AnyElement = if active {
            let toggle_view = view.clone();
            let close_view = view.clone();
            h_flex()
                .id(SharedString::from(format!("filter-pill-{}", pane_id.0)))
                .absolute()
                .bottom_2()
                .left_2()
                .px_2()
                .py_1()
                .gap_1()
                .items_center()
                .rounded_lg()
                .border_1()
                .border_color(if filtering {
                    accent.opacity(0.8)
                } else {
                    border.opacity(0.5)
                })
                .bg(secondary.opacity(0.92))
                .shadow_md()
                .text_xs()
                // Keep pill clicks away from the pane underneath (rubber band,
                // pane-focus click handlers).
                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                    cx.stop_propagation();
                })
                .capture_key_down(
                    cx.listener(move |this, ev: &gpui::KeyDownEvent, _window, cx| {
                        let handled = match ev.keystroke.key.as_str() {
                            "escape" => {
                                this.state.focused_tile = pane_id;
                                this.process_actions(vec![Action::CloseSearchInPlace]);
                                true
                            }
                            "enter" => {
                                this.open_cursor();
                                true
                            }
                            "down" => {
                                this.nav_down(false);
                                true
                            }
                            "up" => {
                                this.nav_up(false);
                                true
                            }
                            _ => false,
                        };
                        if handled {
                            cx.stop_propagation();
                            cx.notify();
                        }
                    }),
                )
                .child(icon(
                    AtlasIcon::Search,
                    14.0,
                    if filtering { accent } else { muted },
                ))
                .child(
                    div()
                        .w(px(180.))
                        .child(crate::ui::text_input::flat(&input).xsmall()),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("filter-rec-{}", pane_id.0)))
                        .p(px(3.))
                        .rounded_md()
                        .cursor_pointer()
                        .when(recursive, |s| s.bg(accent))
                        .hover(|s| s.bg(border.opacity(0.4)))
                        .child(icon(
                            AtlasIcon::TreeStructure,
                            14.0,
                            if recursive { accent_fg } else { muted },
                        ))
                        .tooltip(|window, cx| {
                            gpui_component::tooltip::Tooltip::new("Buscar en subcarpetas")
                                .build(window, cx)
                        })
                        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                            toggle_view.update(cx, |this, cx| {
                                this.process_actions(vec![Action::ToggleSearchInSubfolders]);
                                cx.notify();
                            });
                        }),
                )
                .children(status_text.map(|t| {
                    let tip = error.clone();
                    div()
                        .id(SharedString::from(format!("filter-status-{}", pane_id.0)))
                        .text_color(if tip.is_some() { gpui::red() } else { muted })
                        .whitespace_nowrap()
                        .when_some(tip, |s, tip| {
                            s.tooltip(move |window, cx| {
                                gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                            })
                        })
                        .child(t)
                }))
                .child(
                    div()
                        .id(SharedString::from(format!("filter-close-{}", pane_id.0)))
                        .p(px(3.))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|s| s.bg(border.opacity(0.4)))
                        .child(icon(AtlasIcon::X, 12.0, muted))
                        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                            close_view.update(cx, |this, cx| {
                                this.process_actions(vec![Action::CloseSearchInPlace]);
                                cx.notify();
                            });
                        }),
                )
                .into_any_element()
        } else {
            let open_view = view.clone();
            div()
                .id(SharedString::from(format!("filter-pill-{}", pane_id.0)))
                .absolute()
                .bottom_2()
                .left_2()
                .p(px(6.))
                .rounded_full()
                .border_1()
                .border_color(border.opacity(0.5))
                .bg(secondary.opacity(0.75))
                .shadow_md()
                .cursor_pointer()
                .hover(|s| s.bg(secondary))
                .child(icon(AtlasIcon::Search, 14.0, muted))
                .tooltip(|window, cx| {
                    gpui_component::tooltip::Tooltip::new("Filtrar — solo empieza a escribir")
                        .build(window, cx)
                })
                .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                    cx.stop_propagation();
                    open_view.update(cx, |this, cx| {
                        this.state.focused_tile = pane_id;
                        this.process_actions(vec![Action::OpenSearchInPlace]);
                        cx.notify();
                    });
                })
                .into_any_element()
        };

        // ── Right pill: visibility toggles + folders-first + view selector ──
        let tbtn = |id: String,
                    glyph: AtlasIcon,
                    on: bool,
                    on_color: Hsla,
                    tip: String,
                    action: Action,
                    view: Entity<App>| {
            div()
                .id(SharedString::from(id))
                .p(px(4.))
                .rounded_md()
                .cursor_pointer()
                .when(!on, |s| s.opacity(0.45))
                .hover(move |s| s.bg(border.opacity(0.4)))
                .child(icon(glyph, 14.0, if on { on_color } else { muted }))
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                })
                .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                    view.update(cx, |this, cx| {
                        this.process_actions(vec![action.clone()]);
                        cx.notify();
                    });
                })
        };

        let view_mode = self
            .tab_data_for(pane_id)
            .map(|t| t.view_mode.clone())
            .unwrap_or_default();
        let view_icon = match view_mode {
            ViewMode::Details => AtlasIcon::Details,
            ViewMode::Grid => AtlasIcon::Grid,
            ViewMode::List => AtlasIcon::List,
            ViewMode::Miller => AtlasIcon::Columns,
        };
        let vm = view_mode.clone();
        let view_btn = gpui_component::button::Button::new(SharedString::from(format!(
            "view-select-{}",
            pane_id.0
        )))
        .ghost()
        .xsmall()
        .icon(gpui_component::Icon::empty().path(view_icon.path()))
        // Anchored to the bottom so the menu opens upward, above the icon.
        .dropdown_menu_with_anchor(
            gpui::Anchor::BottomRight,
            move |menu, _window, _cx| {
                let row = |glyph: AtlasIcon, label: &'static str| {
                    move |_: &mut Window, _: &mut gpui::App| {
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(gpui_component::Icon::empty().path(glyph.path()).small())
                            .child(label)
                    }
                };
                menu.menu_element_with_check(
                    vm == ViewMode::Details,
                    Box::new(crate::ViewModeDetails),
                    row(AtlasIcon::Details, "Detalles"),
                )
                .menu_element_with_check(
                    vm == ViewMode::Grid,
                    Box::new(crate::ViewModeGrid),
                    row(AtlasIcon::Grid, "Cuadrícula"),
                )
                .menu_element_with_check(
                    vm == ViewMode::List,
                    Box::new(crate::ViewModeList),
                    row(AtlasIcon::List, "Lista"),
                )
                .menu_element_with_check(
                    vm == ViewMode::Miller,
                    Box::new(crate::ViewModeMiller),
                    row(AtlasIcon::Columns, "Columnas"),
                )
            },
        );

        let focus_view = view.clone();
        let controls_pill = h_flex()
            .id(SharedString::from(format!("controls-pill-{}", pane_id.0)))
            .absolute()
            .bottom_2()
            .right_2()
            .px_1()
            .py(px(3.))
            .gap_1()
            .items_center()
            .rounded_lg()
            .border_1()
            .border_color(border.opacity(0.5))
            .bg(secondary.opacity(0.85))
            .shadow_md()
            // The view-selector actions act on the focused pane; make sure that's
            // us — and keep the click away from the pane underneath.
            .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                cx.stop_propagation();
                focus_view.update(cx, |this, _| {
                    this.state.focused_tile = pane_id;
                });
            })
            .child(tbtn(
                format!("bb-folders-{}", pane_id.0),
                AtlasIcon::Folder,
                show_folders,
                fg,
                format!("Carpetas {}/{}", counts.folders_shown, counts.folders_total),
                Action::ToggleShowFolders(pane_id),
                view.clone(),
            ))
            .child(tbtn(
                format!("bb-files-{}", pane_id.0),
                AtlasIcon::File,
                show_files,
                fg,
                format!("Archivos {}/{}", counts.files_shown, counts.files_total),
                Action::ToggleShowFiles(pane_id),
                view.clone(),
            ))
            .child(tbtn(
                format!("bb-ffirst-{}", pane_id.0),
                AtlasIcon::SortAscending,
                folders_first,
                accent,
                "Carpetas primero".to_string(),
                Action::ToggleFoldersFirst(pane_id),
                view.clone(),
            ))
            .child(div().w(px(1.)).h(px(14.)).bg(border.opacity(0.6)))
            .child(view_btn)
            .into_any_element();

        (filter_pill, controls_pill)
    }
}

#[derive(Clone)]
struct BookmarkTarget {
    name: String,
    path: PathBuf,
    exists: bool,
}

#[allow(clippy::too_many_arguments)]
fn nav_button(
    id: &'static str,
    pane_id: TabId,
    glyph: AtlasIcon,
    tooltip_title: String,
    tooltip_desc: String,
    shortcuts: &'static [&'static str],
    enabled: bool,
    action: Action,
    view: Entity<App>,
    recent: Vec<Location>,
    up_location: Option<Location>,
    bookmark: Option<BookmarkTarget>,
) -> impl IntoElement {
    let menu_view = view.clone();
    let menu_recent = recent.clone();
    let menu_up = up_location.clone();
    let menu_bookmark = bookmark.clone();
    let hover_id = DragHoverId::Chrome(id, pane_id);
    let hover_command = match action.clone() {
        Action::NavigateBack(tile) => DragHoverCommand::NavigateBack(tile),
        Action::NavigateForward(tile) => DragHoverCommand::NavigateForward(tile),
        Action::NavigateUp(tile) => DragHoverCommand::NavigateUp(tile),
        other => DragHoverCommand::Process(other),
    };
    div()
        .id(SharedString::from(format!("{id}-{}", pane_id.0)))
        .flex_none()
        .size(px(26.))
        .rounded(px(7.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(!enabled, |s| s.opacity(0.35))
        .hover(|s| s.bg(glass_fill().opacity(0.72)))
        .child(icon(glyph, 15.0, gpui::white()))
        .when(enabled, |s| {
            s.on_prepaint(drag_hover_recorder(
                hover_id,
                DragHoverKind::Nav,
                hover_command,
                &view,
            ))
        })
        .tooltip(move |window, cx| {
            shortcut_tooltip(
                tooltip_title.clone(),
                tooltip_desc.clone(),
                shortcuts,
                window,
                cx,
            )
        })
        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
            cx.stop_propagation();
            if enabled {
                view.update(cx, |this, cx| {
                    this.process_actions(vec![action.clone()]);
                    cx.notify();
                });
            }
        })
        .context_menu(move |menu, _window, _cx| {
            nav_context_menu(
                menu,
                pane_id,
                menu_view.clone(),
                menu_recent.clone(),
                menu_up.clone(),
                menu_bookmark.clone(),
            )
        })
}

fn bookmark_button(
    pane_id: TabId,
    view: Entity<App>,
    bookmark: Option<BookmarkTarget>,
) -> impl IntoElement {
    let enabled = bookmark.as_ref().is_some_and(|b| !b.exists);
    let active = bookmark.as_ref().is_some_and(|b| b.exists);
    let (tooltip_title, tooltip_desc) = if active {
        (
            rust_i18n::t!("tooltip_bookmark_active_title").to_string(),
            rust_i18n::t!("tooltip_bookmark_active_desc").to_string(),
        )
    } else {
        (
            rust_i18n::t!("tooltip_bookmark_add_title").to_string(),
            rust_i18n::t!("tooltip_bookmark_add_desc").to_string(),
        )
    };

    div()
        .id(SharedString::from(format!("pane-bookmark-{}", pane_id.0)))
        .flex_none()
        .size(px(26.))
        .rounded(px(7.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(!enabled && !active, |s| s.opacity(0.35))
        .hover(|s| s.bg(glass_fill().opacity(0.72)))
        .child(icon(
            AtlasIcon::BookmarkSimple,
            15.0,
            if active {
                gpui::yellow()
            } else {
                gpui::white()
            },
        ))
        .tooltip(move |window, cx| {
            shortcut_tooltip(tooltip_title.clone(), tooltip_desc.clone(), &[], window, cx)
        })
        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
            cx.stop_propagation();
            let Some(bookmark) = bookmark.clone() else {
                return;
            };
            if bookmark.exists {
                return;
            }
            view.update(cx, |this, cx| {
                this.process_actions(vec![Action::AddBookmark {
                    name: bookmark.name,
                    path: bookmark.path,
                }]);
                cx.notify();
            });
        })
}

fn nav_context_menu(
    mut menu: PopupMenu,
    pane_id: TabId,
    view: Entity<App>,
    recent: Vec<Location>,
    up_location: Option<Location>,
    bookmark: Option<BookmarkTarget>,
) -> PopupMenu {
    if recent.is_empty() {
        menu = menu.label("Sin historial reciente");
    } else {
        menu = menu.label("Últimos lugares");
        for location in recent.into_iter().take(6) {
            let label = location_label(&location);
            let icon_name = location_icon(&location);
            let item_view = view.clone();
            menu = menu.item(
                PopupMenuItem::new(label)
                    .icon(Icon::empty().path(icon_name.path()))
                    .on_click(move |_ev, _window, cx| {
                        let location = location.clone();
                        item_view.update(cx, |this, cx| {
                            this.process_actions(vec![Action::NavigateLocation {
                                tab: pane_id,
                                location,
                            }]);
                            cx.notify();
                        });
                    }),
            );
        }
    }

    let up_view = view.clone();
    let up_disabled = up_location.is_none();
    menu = menu.separator().item(
        PopupMenuItem::new("Subir un nivel")
            .icon(Icon::empty().path(AtlasIcon::ArrowUp.path()))
            .disabled(up_disabled)
            .on_click(move |_ev, _window, cx| {
                if let Some(location) = up_location.clone() {
                    up_view.update(cx, |this, cx| {
                        this.process_actions(vec![Action::NavigateLocation {
                            tab: pane_id,
                            location,
                        }]);
                        cx.notify();
                    });
                }
            }),
    );

    let bookmark_disabled = bookmark.as_ref().map_or(true, |b| b.exists);
    let bookmark_label = if bookmark.as_ref().is_some_and(|b| b.exists) {
        "Ya está en marcadores"
    } else {
        "Agregar a marcadores"
    };
    menu.separator().item(
        PopupMenuItem::new(bookmark_label)
            .icon(Icon::empty().path(AtlasIcon::BookmarkSimple.path()))
            .disabled(bookmark_disabled)
            .on_click(move |_ev, _window, cx| {
                if let Some(bookmark) = bookmark.clone() {
                    view.update(cx, |this, cx| {
                        this.process_actions(vec![Action::AddBookmark {
                            name: bookmark.name,
                            path: bookmark.path,
                        }]);
                        cx.notify();
                    });
                }
            }),
    )
}

fn location_icon(location: &Location) -> AtlasIcon {
    match location {
        Location::Directory(_) => AtlasIcon::Folder,
        Location::Archive { .. } => AtlasIcon::File,
        Location::Tag(_) => AtlasIcon::Tag,
        Location::RecycleBin => AtlasIcon::Trash,
        #[cfg(feature = "cloud")]
        Location::Remote { .. } => AtlasIcon::Cloud,
    }
}

fn location_label(location: &Location) -> SharedString {
    let display = location.display_path();
    match location {
        Location::Directory(path) => path.display().to_string().into(),
        Location::Archive { archive, inner } => {
            if inner.as_os_str().is_empty() {
                archive.display().to_string().into()
            } else {
                format!("{} › {}", archive.display(), inner.display()).into()
            }
        }
        Location::Tag(_) | Location::RecycleBin => display.display().to_string().into(),
        #[cfg(feature = "cloud")]
        Location::Remote { .. } => display.display().to_string().into(),
    }
}

/// A clickable path breadcrumb: each segment navigates the focused pane to that
/// ancestor directory. Replaces the old plain truncated path label.
fn breadcrumb(
    view: &Entity<App>,
    tile: TabId,
    path: &std::path::Path,
    fg: Hsla,
    muted: Hsla,
) -> impl IntoElement {
    let mut row = h_flex()
        .flex_1()
        .min_w(px(0.))
        .gap(px(5.))
        .items_center()
        .overflow_x_scrollbar();
    let mut cumulative = std::path::PathBuf::new();
    for (i, comp) in path.components().enumerate() {
        cumulative.push(comp.as_os_str());
        let raw = comp.as_os_str().to_string_lossy().into_owned();
        let label = if raw.is_empty() { "/".to_string() } else { raw };
        if i > 0 {
            row = row.child(div().text_color(muted).child("›"));
        }
        let target = cumulative.clone();
        let v = view.clone();
        row = row.child(
            div()
                .id(SharedString::from(format!("crumb-{}-{}", tile.0, i)))
                .flex_none()
                .max_w(px(220.))
                .px_2()
                .py(px(3.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_color(fg)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(SharedString::from(label))
                .hover(|s| s.bg(glass_fill().opacity(0.62)))
                .on_click(move |_ev, _window, cx| {
                    let target = target.clone();
                    v.update(cx, |this, cx| {
                        this.navigate(tile, target);
                        cx.notify();
                    });
                }),
        );
    }
    row
}

/// Breadcrumb for a tab currently browsing inside an archive.
/// Segments: real parent dirs → archive file → inner dirs.
fn archive_breadcrumb(
    view: &Entity<App>,
    tile: TabId,
    archive: &std::path::Path,
    inner: &std::path::Path,
    fg: Hsla,
    muted: Hsla,
) -> impl IntoElement {
    let mut row = h_flex()
        .flex_1()
        .min_w(px(0.))
        .gap(px(5.))
        .items_center()
        .overflow_x_scrollbar();
    let mut sep = false;

    // Real filesystem ancestors of the archive file.
    let mut cumulative = std::path::PathBuf::new();
    for (i, comp) in archive.components().enumerate() {
        cumulative.push(comp.as_os_str());
        let raw = comp.as_os_str().to_string_lossy().into_owned();
        let label = if raw.is_empty() { "/".to_string() } else { raw };
        if sep {
            row = row.child(div().text_color(muted).child("›"));
        }
        sep = true;
        let target = cumulative.clone();
        let v = view.clone();
        row = row.child(
            div()
                .id(SharedString::from(format!("crumb-r{}-{}", tile.0, i)))
                .flex_none()
                .max_w(px(220.))
                .px_2()
                .py(px(3.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_color(fg)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(SharedString::from(label))
                .hover(|s| s.bg(glass_fill().opacity(0.62)))
                .on_click(move |_ev, _window, cx| {
                    let target = target.clone();
                    v.update(cx, |this, cx| {
                        this.navigate(tile, target);
                        cx.notify();
                    });
                }),
        );
    }

    // Inner path components within the archive.
    let archive_clone = archive.to_path_buf();
    let mut inner_so_far = std::path::PathBuf::new();
    for (j, comp) in inner.components().enumerate() {
        inner_so_far.push(comp.as_os_str());
        let raw = comp.as_os_str().to_string_lossy().into_owned();
        row = row.child(div().text_color(muted).child("›"));
        let target_archive = archive_clone.clone();
        let target_inner = inner_so_far.clone();
        let v = view.clone();
        row = row.child(
            div()
                .id(SharedString::from(format!("crumb-a{}-{}", tile.0, j)))
                .flex_none()
                .max_w(px(220.))
                .px_2()
                .py(px(3.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_color(fg)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(SharedString::from(raw))
                .hover(|s| s.bg(glass_fill().opacity(0.62)))
                .on_click(move |_ev, _window, cx| {
                    let arc = target_archive.clone();
                    let inn = target_inner.clone();
                    v.update(cx, |this, cx| {
                        this.navigate_into_archive(tile, arc, inn);
                        cx.notify();
                    });
                }),
        );
    }

    row
}
