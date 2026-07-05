//! Window chrome: tab pills, breadcrumb strip, floating search pill, rubber marquee.
#![allow(clippy::too_many_lines)] // reason: cohesive UI block kept whole

use crate::app::App;
use crate::input::action::Action;
use crate::ui::drag_hover::{
    drag_hover_recorder, drag_hover_recorder_with_drop, DragHoverCommand, DragHoverDropCommand,
    DragHoverId, DragHoverKind,
};
use crate::ui::glass::{
    glass_fill, glass_header_fill, glass_shadows_enabled, layer_color, GlassExt,
};
use crate::ui::icons::{is_windows_theme, theme_icon, AtlasIcon};
use crate::ui::search_shell::search_icon;
use crate::ui::tooltips::shortcut_tooltip;
use atlas_model::location::Location;
use atlas_model::pane::TabId;
use atlas_model::settings::ViewMode;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, Entity, Hsla, MouseButton, SharedString, Window};
use gpui_component::button::ButtonVariants as _;
use gpui_component::menu::{ContextMenuExt, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, ActiveTheme, ElementExt, Icon, Sizable as _};
use std::collections::HashSet;
use std::path::PathBuf;

impl App {
    #[allow(dead_code)]
    pub(crate) fn render_floating_search_pill(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let figma_glass = self.state.profiles.active().theme == "Atlas Figma Glass";
        let windows_theme = is_windows_theme(&self.state.profiles.active().theme);
        let bg = if figma_glass {
            glass_fill()
        } else {
            layer_color(cx.theme().secondary, 0.02, 0.92)
        };
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
                    .bg(bg)
                    .border_1()
                    .border_color(
                        cx.theme()
                            .border
                            .opacity(if figma_glass { 0.45 } else { 0.60 }),
                    )
                    .when(glass_shadows_enabled(), |s| s.shadow_lg())
                    .cursor_pointer()
                    .text_color(muted)
                    .hover(move |s| s.bg(bg.opacity(0.82)))
                    .active(move |s| s.bg(bg.opacity(0.92)))
                    .child(if windows_theme {
                        theme_icon(AtlasIcon::Search, 15.0, muted, true)
                    } else {
                        search_icon(muted)
                    })
                    .child(rust_i18n::t!("search"))
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
    pub(crate) fn render_rubber_marquee(&self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        // Drawing is now handled natively via canvas in `interactions.rs` (rubber_layer).
        // This avoids DOM layout issues with GPUI's deferred overlays.
        None
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
        let pane_location = self.tab_data_for(pane_id).map(|t| t.location.clone());
        let (can_back, can_forward, up_location, recent_locations, bookmark_target) = self
            .tab_data_for(pane_id)
            .map(|t| {
                (
                    !t.history.is_empty(),
                    !t.history_forward.is_empty(),
                    t.location.ascend(),
                    self.recent_locations_for_pane(pane_id)
                        .into_iter()
                        .map(|loc| (self.resolved_location_label(&loc), loc))
                        .collect::<Vec<_>>(),
                    self.bookmark_target_for_pane(pane_id),
                )
            })
            .unwrap_or_default();
        let windows_theme = is_windows_theme(&self.state.profiles.active().theme);
        let figma_glass = self.state.profiles.active().theme == "Atlas Figma Glass";
        let crumbs: Option<AnyElement> = if let Some((archive, inner)) = pane_archive_inner.as_ref()
        {
            Some(
                archive_breadcrumb(
                    view,
                    pane_id,
                    archive,
                    inner,
                    fg,
                    muted,
                    border,
                    windows_theme,
                )
                .into_any_element(),
            )
        } else {
            pane_location.as_ref().map(|location| match location {
                Location::Directory(p) => {
                    breadcrumb(view, pane_id, p, fg, muted, border, windows_theme)
                        .into_any_element()
                }
                // Virtual locations (tag results, recycle bin, remotes) have no
                // filesystem ancestors to climb — show a static labelled crumb
                // instead of a clickable path that would navigate nowhere.
                other => h_flex()
                    .min_w(px(0.))
                    .gap_1()
                    .items_center()
                    .text_color(fg)
                    .child(crate::ui::icons::icon(location_icon(other), 13.0, muted))
                    .child(div().truncate().child(self.resolved_location_label(other)))
                    .into_any_element(),
            })
        };
        let button_hover = layer_color(border, 0.04, 0.42);
        let strip_bg = if figma_glass {
            glass_header_fill()
        } else {
            border.opacity(0.03)
        };
        h_flex()
            .w_full()
            .h(px(if windows_theme { 44.0 } else { 36.0 }))
            .px_3()
            .min_w(px(0.))
            .gap(px(if windows_theme { 9.0 } else { 6.0 }))
            .items_center()
            .bg(strip_bg)
            .border_b_1()
            .border_color(border.opacity(0.45))
            .when(!windows_theme, |s| s.text_sm())
            .when(windows_theme, |s| s.text_base())
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
                fg,
                button_hover,
                windows_theme,
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
                fg,
                button_hover,
                windows_theme,
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
                fg,
                button_hover,
                windows_theme,
            ))
            .child(bookmark_button(
                pane_id,
                view.clone(),
                bookmark_target.clone(),
                fg,
                button_hover,
                windows_theme,
            ))
            .child(
                div()
                    .flex_none()
                    .w(px(1.))
                    .h(px(if windows_theme { 22.0 } else { 18.0 }))
                    .bg(border.opacity(0.35)),
            )
            .children(crumbs)
            .into_any_element()
    }

    /// User-facing label for a location: tag names resolved from the tag cache
    /// and the recycle bin localized (never the `tag:N` sentinel path).
    pub(crate) fn resolved_location_label(&self, location: &Location) -> SharedString {
        match location {
            Location::Tag(id) => self
                .state
                .tag_cache
                .def(*id)
                .map_or_else(|| location_label(location), |def| def.name.clone().into()),
            Location::RecycleBin => rust_i18n::t!("sidebar_recycle_bin").into_owned().into(),
            _ => location_label(location),
        }
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
            .or_insert_with(|| {
                crate::ui::text_input::single_line(
                    window,
                    cx,
                    rust_i18n::t!("filter_placeholder").into_owned(),
                    initial,
                )
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
        } else if status == atlas_model::tab::SearchStatus::Searching {
            Some("…".to_string())
        } else if filtering {
            Some(format!("{count}"))
        } else {
            None
        };
        let windows_theme = is_windows_theme(&self.state.profiles.active().theme);

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
                    .border_color(accent.opacity(0.45))
                    .bg(layer_color(secondary, 0.04, 0.86))
                    .raised_surface()
                    .text_xs()
                    .cursor_pointer()
                    .child(theme_icon(AtlasIcon::Search, 12.0, accent, windows_theme))
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
                    border.opacity(0.35)
                })
                .bg(layer_color(secondary, 0.04, 0.92))
                .raised_surface()
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
                .child(theme_icon(
                    AtlasIcon::Search,
                    14.0,
                    if filtering { accent } else { muted },
                    windows_theme,
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
                        .hover(|s| s.bg(layer_color(border, 0.04, 0.42)))
                        .child(theme_icon(
                            AtlasIcon::TreeStructure,
                            14.0,
                            if recursive { accent_fg } else { muted },
                            windows_theme,
                        ))
                        .tooltip(|window, cx| {
                            gpui_component::tooltip::Tooltip::new(
                                rust_i18n::t!("search_subfolders").into_owned(),
                            )
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
                        .hover(|s| s.bg(layer_color(border, 0.04, 0.42)))
                        .child(theme_icon(AtlasIcon::X, 12.0, muted, windows_theme))
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
                .border_color(border.opacity(0.35))
                .bg(layer_color(secondary, 0.04, 0.82))
                .raised_surface()
                .cursor_pointer()
                .hover(|s| s.bg(layer_color(secondary, 0.07, 0.92)))
                .child(theme_icon(AtlasIcon::Search, 14.0, muted, windows_theme))
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
                .hover(move |s| s.bg(layer_color(border, 0.04, 0.42)))
                .child(theme_icon(
                    glyph,
                    14.0,
                    if on { on_color } else { muted },
                    windows_theme,
                ))
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
        .child(theme_icon(view_icon, 14.0, muted, windows_theme))
        // Anchored to the bottom so the menu opens upward, above the icon.
        .dropdown_menu_with_anchor(
            gpui::Anchor::BottomRight,
            move |menu, _window, _cx| {
                let row = |glyph: AtlasIcon, label: String| {
                    move |_: &mut Window, _: &mut gpui::App| {
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(theme_icon(glyph, 14.0, muted, windows_theme))
                            .child(label.clone())
                    }
                };
                menu.menu_element_with_check(
                    vm == ViewMode::Details,
                    Box::new(crate::ViewModeDetails),
                    row(
                        AtlasIcon::Details,
                        rust_i18n::t!("view_details").into_owned(),
                    ),
                )
                .menu_element_with_check(
                    vm == ViewMode::Grid,
                    Box::new(crate::ViewModeGrid),
                    row(AtlasIcon::Grid, rust_i18n::t!("view_grid").into_owned()),
                )
                .menu_element_with_check(
                    vm == ViewMode::List,
                    Box::new(crate::ViewModeList),
                    row(AtlasIcon::List, rust_i18n::t!("view_list").into_owned()),
                )
                .menu_element_with_check(
                    vm == ViewMode::Miller,
                    Box::new(crate::ViewModeMiller),
                    row(
                        AtlasIcon::Columns,
                        rust_i18n::t!("view_columns").into_owned(),
                    ),
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
            .border_color(border.opacity(0.35))
            .bg(layer_color(secondary, 0.04, 0.88))
            .raised_surface()
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
                rust_i18n::t!(
                    "folders_count",
                    shown = counts.folders_shown,
                    total = counts.folders_total
                )
                .into_owned(),
                Action::ToggleShowFolders(pane_id),
                view.clone(),
            ))
            .child(tbtn(
                format!("bb-files-{}", pane_id.0),
                AtlasIcon::File,
                show_files,
                fg,
                rust_i18n::t!(
                    "files_count",
                    shown = counts.files_shown,
                    total = counts.files_total
                )
                .into_owned(),
                Action::ToggleShowFiles(pane_id),
                view.clone(),
            ))
            .child(tbtn(
                format!("bb-ffirst-{}", pane_id.0),
                AtlasIcon::SortAscending,
                folders_first,
                accent,
                rust_i18n::t!("folders_first").into_owned(),
                Action::ToggleFoldersFirst(pane_id),
                view.clone(),
            ))
            .child(div().w(px(1.)).h(px(14.)).bg(border.opacity(0.35)))
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
    recent: Vec<(SharedString, Location)>,
    up_location: Option<Location>,
    bookmark: Option<BookmarkTarget>,
    icon_color: Hsla,
    hover_bg: Hsla,
    windows_theme: bool,
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
        .size(px(if windows_theme { 34.0 } else { 26.0 }))
        .rounded(px(if windows_theme { 8.0 } else { 7.0 }))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(!enabled, |s| s.opacity(0.35))
        .hover(move |s| s.bg(hover_bg))
        .child(theme_icon(
            glyph,
            if windows_theme { 18.0 } else { 15.0 },
            icon_color.opacity(0.88),
            windows_theme,
        ))
        .when(enabled, |s| {
            s.on_prepaint(drag_hover_recorder(
                hover_id,
                DragHoverKind::Folder,
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
    icon_color: Hsla,
    hover_bg: Hsla,
    windows_theme: bool,
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
        .size(px(if windows_theme { 34.0 } else { 26.0 }))
        .rounded(px(if windows_theme { 8.0 } else { 7.0 }))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(!enabled && !active, |s| s.opacity(0.35))
        .hover(move |s| s.bg(hover_bg))
        .child(theme_icon(
            AtlasIcon::BookmarkSimple,
            if windows_theme { 18.0 } else { 15.0 },
            if active {
                gpui::yellow()
            } else {
                icon_color.opacity(0.88)
            },
            windows_theme,
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
    recent: Vec<(SharedString, Location)>,
    up_location: Option<Location>,
    bookmark: Option<BookmarkTarget>,
) -> PopupMenu {
    if recent.is_empty() {
        menu = menu.label(rust_i18n::t!("no_recent_history"));
    } else {
        menu = menu.label(rust_i18n::t!("recent_places"));
        for (label, location) in recent.into_iter().take(6) {
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
        PopupMenuItem::new(rust_i18n::t!("go_up"))
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
        rust_i18n::t!("already_bookmarked")
    } else {
        rust_i18n::t!("add_bookmark")
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
    layer_base: Hsla,
    windows_theme: bool,
) -> impl IntoElement {
    let row_view = view.clone();
    let mut row = h_flex()
        .flex_1()
        .min_w(px(0.))
        .gap(px(if windows_theme { 7.0 } else { 5.0 }))
        .items_center()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
            row_view.update(cx, |this, cx| {
                this.state.focused_tile = tile;
                this.open_location_finder_or_editor();
                cx.notify();
            });
        })
        // Plain clip (not overflow_x_scrollbar): the Scrollable wrapper adds its own
        // relative+size_full box that anchored each crumb's on_prepaint canvas to the
        // wrong rect, so drag-hover targets captured a region offset below the crumb.
        // Crumbs already truncate via max_w + text_ellipsis.
        .overflow_hidden();
    let mut cumulative = std::path::PathBuf::new();
    for (i, comp) in path.components().enumerate() {
        cumulative.push(comp.as_os_str());
        let raw = comp.as_os_str().to_string_lossy().into_owned();
        let label = if raw.is_empty() { "/".to_string() } else { raw };
        if i > 0 {
            row = row.child(div().text_color(muted).child("›"));
        }
        let v = view.clone();
        let target = cumulative.clone();
        let drop_target = target.clone();
        let drop_target_external = target.clone();
        let down_v = view.clone();
        let drop_v = view.clone();
        let drop_v_external = view.clone();
        let hover_id = DragHoverId::Folder(tile, target.clone());
        let hover_bg = layer_color(layer_base, 0.04, 0.42);
        row = row.child(
            div()
                .id(SharedString::from(format!("crumb-{}-{}", tile.0, i)))
                .flex_none()
                // `relative` so the on_prepaint canvas anchors to this crumb. Without
                // it the canvas anchors to the `overflow_x_scrollbar` wrapper and the
                // drag-hover target captures a region offset below the crumb.
                .relative()
                .max_w(px(if windows_theme { 260.0 } else { 220.0 }))
                .px(px(if windows_theme { 10.0 } else { 8.0 }))
                .py(px(if windows_theme { 5.0 } else { 3.0 }))
                .rounded(px(if windows_theme { 7.0 } else { 6.0 }))
                .cursor_pointer()
                .text_color(fg)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(SharedString::from(label))
                .hover(move |s| s.bg(hover_bg))
                .on_prepaint(drag_hover_recorder_with_drop(
                    hover_id,
                    DragHoverKind::Folder,
                    DragHoverCommand::Process(Action::NavigateLocation {
                        tab: tile,
                        location: Location::Directory(target.clone()),
                    }),
                    DragHoverDropCommand::TransferTo(target.clone()),
                    view,
                ))
                .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                    down_v.update(cx, |this, _| {
                        this.state.focused_tile = tile;
                    });
                    cx.stop_propagation();
                })
                .on_drop::<crate::ui::root::DraggedFiles>(move |d, _window, cx| {
                    let paths = d.paths.clone();
                    drop_v.update(cx, |this, cx| {
                        this.execute_drag_hover_drop_command(
                            DragHoverDropCommand::TransferTo(drop_target.clone()),
                            paths,
                            true,
                        );
                        cx.notify();
                    });
                })
                .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
                    let paths = paths.paths().to_vec();
                    drop_v_external.update(cx, |this, cx| {
                        this.execute_drag_hover_drop_command(
                            DragHoverDropCommand::TransferTo(drop_target_external.clone()),
                            paths,
                            false,
                        );
                        cx.notify();
                    });
                })
                .on_click(move |_ev, _window, cx| {
                    let target = target.clone();
                    v.update(cx, |this, cx| {
                        this.state.focused_tile = tile;
                        this.process_actions(vec![Action::NavigateLocation {
                            tab: tile,
                            location: Location::Directory(target),
                        }]);
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
    layer_base: Hsla,
    windows_theme: bool,
) -> impl IntoElement {
    let row_view = view.clone();
    let mut row = h_flex()
        .flex_1()
        .min_w(px(0.))
        .gap(px(if windows_theme { 7.0 } else { 5.0 }))
        .items_center()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
            row_view.update(cx, |this, cx| {
                this.state.focused_tile = tile;
                this.open_location_finder_or_editor();
                cx.notify();
            });
        })
        .overflow_x_scrollbar();
    let mut sep = false;

    // Real filesystem ancestors of the archive file.
    let mut cumulative = std::path::PathBuf::new();
    for (i, comp) in archive.components().enumerate() {
        cumulative.push(comp.as_os_str());
        let raw = comp.as_os_str().to_string_lossy().into_owned();
        let label = if raw.is_empty() { "/".to_string() } else { raw };
        let hover_bg = layer_color(layer_base, 0.04, 0.42);
        if sep {
            row = row.child(div().text_color(muted).child("›"));
        }
        sep = true;
        let v = view.clone();
        row = row.child(
            div()
                .id(SharedString::from(format!("crumb-r{}-{}", tile.0, i)))
                .flex_none()
                .max_w(px(if windows_theme { 260.0 } else { 220.0 }))
                .px(px(if windows_theme { 10.0 } else { 8.0 }))
                .py(px(if windows_theme { 5.0 } else { 3.0 }))
                .rounded(px(if windows_theme { 7.0 } else { 6.0 }))
                .cursor_pointer()
                .text_color(fg)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(SharedString::from(label))
                .hover(move |s| s.bg(hover_bg))
                .on_click(move |_ev, _window, cx| {
                    v.update(cx, |this, cx| {
                        this.state.focused_tile = tile;
                        this.open_location_finder_or_editor();
                        cx.notify();
                    });
                }),
        );
    }

    // Inner path components within the archive.
    let mut inner_so_far = std::path::PathBuf::new();
    for (j, comp) in inner.components().enumerate() {
        inner_so_far.push(comp.as_os_str());
        let raw = comp.as_os_str().to_string_lossy().into_owned();
        let hover_bg = layer_color(layer_base, 0.04, 0.42);
        row = row.child(div().text_color(muted).child("›"));
        let v = view.clone();
        row = row.child(
            div()
                .id(SharedString::from(format!("crumb-a{}-{}", tile.0, j)))
                .flex_none()
                .max_w(px(if windows_theme { 260.0 } else { 220.0 }))
                .px(px(if windows_theme { 10.0 } else { 8.0 }))
                .py(px(if windows_theme { 5.0 } else { 3.0 }))
                .rounded(px(if windows_theme { 7.0 } else { 6.0 }))
                .cursor_pointer()
                .text_color(fg)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(SharedString::from(raw))
                .hover(move |s| s.bg(hover_bg))
                .on_click(move |_ev, _window, cx| {
                    v.update(cx, |this, cx| {
                        this.state.focused_tile = tile;
                        this.open_location_finder_or_editor();
                        cx.notify();
                    });
                }),
        );
    }

    row
}
