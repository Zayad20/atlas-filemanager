//! Window chrome: tab pills, breadcrumb strip, floating search pill, rubber marquee.
#![allow(clippy::too_many_lines)] // reason: cohesive UI block kept whole

use crate::app::App;
use crate::input::action::Action;
use crate::state::pane::TabId;
use crate::state::settings::ViewMode;
use crate::ui::glass::{
    glass_border, glass_fill, glass_shadows_enabled,
};
use crate::ui::icons::{icon, AtlasIcon};
use crate::ui::search_shell::search_icon;
use gpui::prelude::*;
use gpui::{
    deferred, div, px, AnyElement, Context,
    Entity, Hsla, MouseButton, SharedString, Window,
};
use gpui_component::button::ButtonVariants as _;
use gpui_component::menu::DropdownMenu as _;
use gpui_component::{h_flex, ActiveTheme, Sizable as _};

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
            t.location.as_archive().map(|(archive, inner)| {
                (archive.to_path_buf(), inner.to_path_buf())
            })
        });
        let pane_path = self.tab_data_for(pane_id).map(|t| t.location.display_path());
        let crumbs: Option<AnyElement> =
            if let Some((archive, inner)) = pane_archive_inner.as_ref() {
                Some(
                    archive_breadcrumb(view, pane_id, archive, inner, fg, muted)
                        .into_any_element(),
                )
            } else {
                pane_path
                    .as_ref()
                    .map(|p| breadcrumb(view, pane_id, p, fg, muted).into_any_element())
            };
        h_flex()
            .w_full()
            .px_2()
            .py_1()
            .min_w(px(0.))
            .border_b_1()
            .border_color(border)
            .text_xs()
            .text_color(muted)
            .children(crumbs)
            .into_any_element()
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
            .or_insert_with(|| {
                crate::ui::text_input::single_line(window, cx, "Filtrar…", initial)
            })
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
                        status_text
                            .map(|t| div().text_color(muted).whitespace_nowrap().child(t)),
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
                                gpui_component::tooltip::Tooltip::new(tip.clone())
                                    .build(window, cx)
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
        .dropdown_menu_with_anchor(gpui::Anchor::BottomRight, move |menu, _window, _cx| {
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
        });

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
                format!(
                    "Carpetas {}/{}",
                    counts.folders_shown, counts.folders_total
                ),
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

/// A clickable path breadcrumb: each segment navigates the focused pane to that
/// ancestor directory. Replaces the old plain truncated path label.
fn breadcrumb(
    view: &Entity<App>,
    tile: TabId,
    path: &std::path::Path,
    fg: Hsla,
    muted: Hsla,
) -> impl IntoElement {
    let mut row = h_flex().flex_1().gap_1().items_center().overflow_hidden();
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
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .text_color(fg)
                .truncate()
                .child(SharedString::from(label))
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
    let mut row = h_flex().flex_1().gap_1().items_center().overflow_hidden();
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
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .text_color(fg)
                .truncate()
                .child(SharedString::from(label))
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
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .text_color(fg)
                .truncate()
                .child(SharedString::from(raw))
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

