//! Window chrome: tab pills, breadcrumb strip, floating search pill, rubber marquee.
#![allow(clippy::too_many_lines)] // reason: cohesive UI block kept whole

use crate::app::App;
use crate::input::action::Action;
use crate::ui::drag_hover::{
    drag_hover_recorder, drag_hover_recorder_with_drop, DragHoverCommand, DragHoverDropCommand,
    DragHoverId, DragHoverKind,
};
use crate::ui::glass::{glass_fill, glass_shadows_enabled, layer_color, GlassExt};
use crate::ui::icons::{is_windows_theme, theme_icon, AtlasIcon};
use crate::ui::search_shell::search_icon;
use crate::ui::tooltips::shortcut_tooltip;
use atlas_model::location::Location;
use atlas_model::pane::TabId;
use atlas_model::settings::ViewMode;
use gpui::prelude::*;
use gpui::{
    div, px, AnimationExt, AnyElement, Context, Entity, Hsla, MouseButton, SharedString, Window,
};
use gpui_component::button::ButtonVariants as _;
use gpui_component::menu::{ContextMenuExt, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_component::{h_flex, v_flex, ActiveTheme, ElementExt, Icon, Sizable as _};
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
            .bottom(crate::ui::rem_px(18.))
            .flex()
            .justify_center()
            // The wrapper is non-interactive so only the pill captures clicks.
            .child(
                h_flex()
                    .id("search-pill")
                    .h(crate::ui::rem_px(36.))
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

    /// Phase C: the visible selection rectangle for the active drag, in window coords
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
        bg: Hsla,
        border: Hsla,
        hover: Hsla,
        accent: Hsla,
        font: gpui::Font,
        font_size: gpui::Pixels,
    ) -> AnyElement {
        let pane_location = self.tab_data_for(pane_id).map(|t| t.location.clone());
        let (can_back, can_forward, can_up, bookmarked) = self
            .tab_data_for(pane_id)
            .map(|t| {
                (
                    !t.history.is_empty(),
                    !t.history_forward.is_empty(),
                    t.location.ascend().is_some(),
                    self.bookmark_target_for_pane(pane_id).is_some(),
                )
            })
            .unwrap_or_default();
        let windows_theme = is_windows_theme(&self.state.profiles.active().theme);

        let segments = if let Some(Location::Directory(p)) = pane_location {
            crate::ui::views::crumb_strip_element::path_segments(&p)
        } else {
            // For virtual locations, just a single segment with the label.
            let label = pane_location
                .as_ref()
                .map(|loc| self.resolved_location_label(loc).to_string())
                .unwrap_or_default();
            vec![crate::ui::views::crumb_strip_element::CrumbSeg {
                label,
                location: pane_location.unwrap_or(Location::RecycleBin),
            }]
        };

        let history = self.tab_data_for(pane_id).map(|t| t.history.clone()).unwrap_or_default();
        let history_fwd = self.tab_data_for(pane_id).map(|t| t.history_forward.clone()).unwrap_or_default();
        let recent_back: Vec<_> = history.into_iter().rev().map(|loc| (self.resolved_location_label(&loc), loc)).collect();
        let recent_fwd: Vec<_> = history_fwd.into_iter().rev().map(|loc| (self.resolved_location_label(&loc), loc)).collect();

        let nav_buttons = h_flex()
            .gap_1()
            .items_center()
            .child(nav_button(NavButtonProps {
                id: "nav-back",
                pane_id,
                glyph: AtlasIcon::ArrowLeft,
                tooltip_title: rust_i18n::t!("navigate_back_title").into_owned(),
                tooltip_desc: rust_i18n::t!("navigate_back_desc").into_owned(),
                shortcuts: &["Alt", "Left"],
                enabled: can_back,
                action: Action::NavigateBack(pane_id),
                view: view.clone(),
                recent: recent_back,
                icon_color: fg,
                hover_bg: hover,
                windows_theme,
            }))
            .child(nav_button(NavButtonProps {
                id: "nav-fwd",
                pane_id,
                glyph: AtlasIcon::ArrowRight,
                tooltip_title: rust_i18n::t!("navigate_forward_title").into_owned(),
                tooltip_desc: rust_i18n::t!("navigate_forward_desc").into_owned(),
                shortcuts: &["Alt", "Right"],
                enabled: can_forward,
                action: Action::NavigateForward(pane_id),
                view: view.clone(),
                recent: recent_fwd,
                icon_color: fg,
                hover_bg: hover,
                windows_theme,
            }))
            .child(nav_button(NavButtonProps {
                id: "nav-up",
                pane_id,
                glyph: AtlasIcon::ArrowUp,
                tooltip_title: rust_i18n::t!("navigate_up_title").into_owned(),
                tooltip_desc: rust_i18n::t!("navigate_up_desc").into_owned(),
                shortcuts: &["Alt", "Up"],
                enabled: can_up,
                action: Action::NavigateUp(pane_id),
                view: view.clone(),
                recent: vec![],
                icon_color: fg,
                hover_bg: hover,
                windows_theme,
            }))
            .child(bookmark_button(
                pane_id,
                view.clone(),
                self.bookmark_target_for_pane(pane_id),
                accent,
                hover,
                windows_theme,
            ))
            .child(
                div()
                    .w(gpui::px(1.0))
                    .h(crate::ui::rem_px(if windows_theme { 18.0 } else { 12.0 }))
                    .bg(border.opacity(0.35))
                    .mx(crate::ui::rem_px(4.0)),
            );

        let app_drop_crumb = view.clone();
        let app_drop_crumb_ext = view.clone();
        h_flex()
            .w_full()
            .items_center()
            .child(nav_buttons)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(
                        crate::ui::views::crumb_strip_element::CrumbStripElement::new(
                            pane_id,
                            view.downgrade(),
                            segments,
                            font,
                            font_size,
                            windows_theme,
                            fg,
                            muted,
                            bg,
                            border,
                            hover,
                            accent,
                        )
                    )
                    .on_drop::<crate::ui::root::DraggedFiles>(move |d, window, cx| {
                        let pos = window.mouse_position();
                        let paths = d.paths.clone();
                        app_drop_crumb.update(cx, |this, cx| {
                            if let Some(target) = this.pending_drag_hover_targets.iter().rev().find(|t| crate::ui::drag_hover::contains(t.bounds, pos)) {
                                if let Some(cmd) = &target.drop_command {
                                    this.execute_drag_hover_drop_command(cmd.clone(), paths, true);
                                    cx.notify();
                                }
                            }
                        });
                    })
                    .on_drop::<gpui::ExternalPaths>(move |paths, window, cx| {
                        let pos = window.mouse_position();
                        let paths = paths.paths().to_vec();
                        app_drop_crumb_ext.update(cx, |this, cx| {
                            if let Some(target) = this.pending_drag_hover_targets.iter().rev().find(|t| crate::ui::drag_hover::contains(t.bounds, pos)) {
                                if let Some(cmd) = &target.drop_command {
                                    this.execute_drag_hover_drop_command(cmd.clone(), paths, false);
                                    cx.notify();
                                }
                            }
                        });
                    })
            )
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
        for path in self.recent_folders().iter().rev() {
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
        let index = self
            .state
            .profiles
            .active()
            .bookmarks
            .iter()
            .position(|bookmark| bookmark.path == path);
        Some(BookmarkTarget {
            name,
            path,
            exists: index.is_some(),
            index,
        })
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

        let (active, recursive, status, count, error) = self
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
                )
            })
            .unwrap_or_default();
        let selected_count = self
            .state
            .tab_states
            .get(&pane_id)
            .map(|ts| ts.selection.selected.len())
            .unwrap_or_default();
        let filtering = active && !value.trim().is_empty();
        let figma_glass = self.state.profiles.active().theme == "Atlas Figma Glass";
        let (fg, accent, accent_fg, muted, border, secondary, hover) = {
            let t = cx.theme();
            (
                t.foreground,
                t.accent,
                t.accent_foreground,
                t.muted_foreground,
                t.border,
                t.secondary,
                t.list_active,
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
                    .py(crate::ui::rem_px(3.))
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
                            this.activate_tab(pane_id);
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
                                this.activate_tab(pane_id);
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
                        .w(crate::ui::rem_px(180.))
                        .child(crate::ui::text_input::flat(&input).xsmall()),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("filter-rec-{}", pane_id.0)))
                        .p(crate::ui::rem_px(3.))
                        .rounded_md()
                        .cursor_pointer()
                        .when(recursive, |s| s.bg(accent))
                        .hover(move |s| s.bg(hover))
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
                        .p(crate::ui::rem_px(3.))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
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
                .p(crate::ui::rem_px(6.))
                .rounded_full()
                .border_1()
                .border_color(border.opacity(0.35))
                .bg(layer_color(secondary, 0.04, 0.82))
                .raised_surface()
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(theme_icon(AtlasIcon::Search, 14.0, muted, windows_theme))
                .tooltip(|window, cx| {
                    gpui_component::tooltip::Tooltip::new(
                        rust_i18n::t!("tooltip_filter_files").into_owned(),
                    )
                    .build(window, cx)
                })
                .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                    cx.stop_propagation();
                    open_view.update(cx, |this, cx| {
                        this.activate_tab(pane_id);
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
                .p(crate::ui::rem_px(4.))
                .rounded_md()
                .cursor_pointer()
                .when(!on, |s| s.opacity(0.45))
                .hover(move |s| s.bg(hover))
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
                        this.flush_dirty_panes(cx);
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
        let view_label = match view_mode {
            ViewMode::Details => rust_i18n::t!("view_details").into_owned(),
            ViewMode::Grid => rust_i18n::t!("view_grid").into_owned(),
            ViewMode::List => rust_i18n::t!("view_list").into_owned(),
            ViewMode::Miller => rust_i18n::t!("view_columns").into_owned(),
        };
        let view_slider_value = self.pane_view_slider_value(pane_id);
        let zoom_slider = if let Some(slider) = self.pane_zoom_sliders.get(&pane_id) {
            slider.clone()
        } else {
            let slider = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(22.0)
                    .step(1.0)
                    .default_value(view_slider_value)
            });
            let sub = cx.subscribe_in(
                &slider,
                window,
                move |this, _, ev: &SliderEvent, _window, cx| {
                    let value = match ev {
                        SliderEvent::Change(value) | SliderEvent::Release(value) => value.end(),
                    }
                    .round()
                    .clamp(0.0, 22.0);
                    if this.set_pane_view_from_slider_value(pane_id, value) {
                        this.notify_pane(pane_id, cx);
                    }
                    cx.notify();
                },
            );
            self.pane_zoom_sliders.insert(pane_id, slider.clone());
            self.pane_zoom_slider_subs.insert(pane_id, sub);
            slider
        };
        let slider_value = zoom_slider.read(cx).value().end();
        if (slider_value - view_slider_value).abs() > f32::EPSILON {
            zoom_slider.update(cx, |slider, cx| {
                slider.set_value(SliderValue::Single(view_slider_value), window, cx);
            });
        }
        let zoom_slider_for_menu = zoom_slider.clone();
        let menu_view = view.clone();
        let view_btn = gpui_component::button::Button::new(SharedString::from(format!(
            "view-select-{}",
            pane_id.0
        )))
        .ghost()
        .xsmall()
        .child(theme_icon(view_icon, 14.0, accent, windows_theme))
        .tooltip(format!("{} - Ctrl+scroll", view_label))
        // Anchored to the bottom so the menu opens upward, above the icon.
        .dropdown_menu_with_anchor(
            gpui::Anchor::BottomRight,
            move |menu, _window, _cx| {
                let slider = zoom_slider_for_menu.clone();
                let view = menu_view.clone();
                menu.item(
                    PopupMenuItem::element(move |_: &mut Window, cx| {
                        let active_mode = view
                            .read(cx)
                            .tab_data_for(pane_id)
                            .map(|pane| pane.view_mode.clone())
                            .unwrap_or_default();
                        let row = |glyph: AtlasIcon,
                                   label: String,
                                   mode: ViewMode,
                                   active: bool,
                                   view: Entity<App>,
                                   slider: Entity<SliderState>| {
                            h_flex()
                                .gap_1()
                                .items_center()
                                .rounded_md()
                                .px_2()
                                .h(px(24.0))
                                .when(active, |this| this.bg(accent.opacity(0.14)))
                                .child(theme_icon(
                                    glyph,
                                    14.0,
                                    if active { accent } else { muted },
                                    windows_theme,
                                ))
                                .child(
                                    div()
                                        .w(px(82.0))
                                        .text_xs()
                                        .text_color(if active {
                                            cx.theme().foreground
                                        } else {
                                            cx.theme().muted_foreground
                                        })
                                        .child(label),
                                )
                                .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                                    cx.stop_propagation();
                                    let mut slider_value = None;
                                    view.update(cx, |this, cx| {
                                        this.activate_tab(pane_id);
                                        if this.set_pane_view_mode(pane_id, mode.clone()) {
                                            this.notify_pane(pane_id, cx);
                                        }
                                        slider_value = Some(this.pane_view_slider_value(pane_id));
                                        cx.notify();
                                    });
                                    if let Some(slider_value) = slider_value {
                                        slider.update(cx, |state, cx| {
                                            state.set_value(
                                                SliderValue::Single(slider_value),
                                                window,
                                                cx,
                                            );
                                        });
                                    }
                                })
                        };
                        h_flex()
                            .gap_2()
                            .items_center()
                            .p_1()
                            .child(
                                v_flex()
                                    .gap_0p5()
                                    .child(row(
                                        AtlasIcon::Columns,
                                        rust_i18n::t!("view_columns").into_owned(),
                                        ViewMode::Miller,
                                        active_mode == ViewMode::Miller,
                                        view.clone(),
                                        slider.clone(),
                                    ))
                                    .child(row(
                                        AtlasIcon::Details,
                                        rust_i18n::t!("view_details").into_owned(),
                                        ViewMode::Details,
                                        active_mode == ViewMode::Details,
                                        view.clone(),
                                        slider.clone(),
                                    ))
                                    .child(row(
                                        AtlasIcon::List,
                                        rust_i18n::t!("view_list").into_owned(),
                                        ViewMode::List,
                                        active_mode == ViewMode::List,
                                        view.clone(),
                                        slider.clone(),
                                    ))
                                    .child(row(
                                        AtlasIcon::Grid,
                                        rust_i18n::t!("view_grid").into_owned(),
                                        ViewMode::Grid,
                                        active_mode == ViewMode::Grid,
                                        view.clone(),
                                        slider.clone(),
                                    )),
                            )
                            .child(
                                div().w(px(22.0)).h(px(96.0)).items_center().child(
                                    Slider::new(&slider)
                                        .vertical()
                                        .h(px(88.0))
                                        .bg(accent.opacity(0.55))
                                        .text_color(accent),
                                ),
                            )
                    })
                    .disabled(true),
                )
            },
        );

        let focus_view = view.clone();
        let animate_selection_badge = !self.state.settings.disable_animations;
        let controls_pill = h_flex()
            .id(SharedString::from(format!("controls-pill-{}", pane_id.0)))
            .absolute()
            .bottom_2()
            .right_2()
            .px_1()
            .py(crate::ui::rem_px(3.))
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
                    this.activate_tab(pane_id);
                });
            })
            .children((selected_count > 1).then(|| {
                let badge = h_flex()
                    .id(SharedString::from(format!("selection-count-{}", pane_id.0)))
                    .gap_1()
                    .items_center()
                    .px_1()
                    .child(theme_icon(AtlasIcon::Grid, 14.0, accent, windows_theme))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child(selected_count.to_string()),
                    );
                if animate_selection_badge {
                    badge
                        .with_animation(
                            SharedString::from(format!("selection-count-appear-{}", pane_id.0)),
                            crate::ui::motion::animation(crate::ui::motion::MICRO_MS),
                            |this, t| this.opacity(t).ml(px(6.0 * (1.0 - t))),
                        )
                        .into_any_element()
                } else {
                    badge.into_any_element()
                }
            }))
            .children((selected_count > 1).then(|| {
                div()
                    .w(px(1.))
                    .h(crate::ui::rem_px(14.))
                    .bg(border.opacity(0.35))
            }))
            .child(tbtn(
                format!("bb-folders-{}", pane_id.0),
                AtlasIcon::Folder,
                show_folders,
                fg,
                rust_i18n::t!("tooltip_toggle_folders").into_owned(),
                Action::ToggleShowFolders(pane_id),
                view.clone(),
            ))
            .child(tbtn(
                format!("bb-files-{}", pane_id.0),
                AtlasIcon::File,
                show_files,
                fg,
                rust_i18n::t!("tooltip_toggle_files").into_owned(),
                Action::ToggleShowFiles(pane_id),
                view.clone(),
            ))
            .child(tbtn(
                format!("bb-ffirst-{}", pane_id.0),
                AtlasIcon::SortAscending,
                folders_first,
                accent,
                rust_i18n::t!("tooltip_folders_first").into_owned(),
                Action::ToggleFoldersFirst(pane_id),
                view.clone(),
            ))
            .child(
                div()
                    .w(px(1.))
                    .h(crate::ui::rem_px(14.))
                    .bg(border.opacity(0.35)),
            )
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
    index: Option<usize>,
}

struct NavButtonProps {
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
    icon_color: Hsla,
    hover_bg: Hsla,
    windows_theme: bool,
}

fn nav_button(props: NavButtonProps) -> impl IntoElement {
    let menu_view = props.view.clone();
    let menu_recent = props.recent.clone();
    let drop_dest = props.recent.first().and_then(|(_, loc)| {
        if let Location::Directory(path) = loc {
            Some(path.clone())
        } else {
            None
        }
    });

    let hover_id = DragHoverId::Chrome(props.id, props.pane_id);
    let hover_command = match props.action.clone() {
        Action::NavigateBack(tile) => DragHoverCommand::NavigateBack(tile),
        Action::NavigateForward(tile) => DragHoverCommand::NavigateForward(tile),
        Action::NavigateUp(tile) => DragHoverCommand::NavigateUp(tile),
        other => DragHoverCommand::Process(other),
    };

    let mut el = div()
        .id(SharedString::from(format!(
            "{}-{}",
            props.id, props.pane_id.0
        )))
        .flex_none()
        .size(crate::ui::rem_px(if props.windows_theme {
            34.0
        } else {
            26.0
        }))
        .rounded(px(if props.windows_theme { 8.0 } else { 7.0 }))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(!props.enabled, |s| s.opacity(0.35))
        .hover(move |s| s.bg(props.hover_bg))
        .child(theme_icon(
            props.glyph,
            if props.windows_theme { 18.0 } else { 15.0 },
            props.icon_color.opacity(0.88),
            props.windows_theme,
        ))
        .tooltip(move |window, cx| {
            shortcut_tooltip(
                props.tooltip_title.clone(),
                props.tooltip_desc.clone(),
                props.shortcuts,
                window,
                cx,
            )
        })
        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
            cx.stop_propagation();
            if props.enabled {
                props.view.update(cx, |this, cx| {
                    this.process_actions(vec![props.action.clone()]);
                    this.flush_dirty_panes(cx);
                });
            }
        })
        .on_mouse_up(MouseButton::Right, {
            let view = menu_view.clone();
            move |_ev, _window, cx| {
                view.update(cx, |this, cx| {
                    this.suppress_next_shell_ctx_menu = true;
                    this.close_ctx_menu(cx);
                });
            }
        });

    if let Some(dest) = drop_dest {
        let drop_v = props.view.clone();
        let drop_v_external = props.view.clone();
        let drop_target = dest.clone();
        let drop_target_external = dest.clone();
        el = el
            .when(props.enabled, |s| {
                s.on_prepaint(drag_hover_recorder_with_drop(
                    hover_id,
                    DragHoverKind::Folder,
                    hover_command,
                    DragHoverDropCommand::TransferTo(dest.clone()),
                    &props.view,
                ))
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
            });
    } else {
        el = el.when(props.enabled, |s| {
            s.on_prepaint(drag_hover_recorder(
                hover_id,
                DragHoverKind::Folder,
                hover_command,
                &props.view,
            ))
        });
    }

    el.context_menu(move |menu, _window, _cx| {
        nav_context_menu(menu, props.pane_id, menu_view.clone(), menu_recent.clone())
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
        .size(crate::ui::rem_px(if windows_theme { 34.0 } else { 26.0 }))
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
            view.update(cx, |this, cx| {
                if let Some(index) = bookmark.index {
                    this.process_actions(vec![Action::RemoveBookmark(index)]);
                } else {
                    this.process_actions(vec![Action::AddBookmark {
                        name: bookmark.name,
                        path: bookmark.path,
                    }]);
                }
                cx.notify();
            });
        })
}

fn nav_context_menu(
    mut menu: PopupMenu,
    pane_id: TabId,
    view: Entity<App>,
    recent: Vec<(SharedString, Location)>,
) -> PopupMenu {
    if recent.is_empty() {
        menu = menu.label(rust_i18n::t!("no_recent_history"));
    } else {
        menu = menu.item(
            PopupMenuItem::new(rust_i18n::t!("recent_places"))
                .icon(Icon::empty().path(AtlasIcon::ClockCounterClockwise.path()))
                .disabled(true),
        );
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
    menu
}

fn location_icon(location: &Location) -> AtlasIcon {
    match location {
        Location::Directory(_) => AtlasIcon::Folder,
        Location::Archive { .. } => AtlasIcon::File,
        Location::Tag(_) => AtlasIcon::Tag,
        Location::RecycleBin => AtlasIcon::Trash,
        #[cfg(feature = "provider-host")]
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
        #[cfg(feature = "provider-host")]
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
    hover_bg: Hsla,
    windows_theme: bool,
) -> impl IntoElement {
    let mut row = h_flex()
        .flex_1()
        .min_w(px(0.))
        .gap(crate::ui::rem_px(if windows_theme { 7.0 } else { 5.0 }))
        .items_center()
        // Plain clip (not overflow_x_scrollbar): the Scrollable wrapper adds its own
        // relative+size_full box that anchored each crumb's on_prepaint canvas to the
        // wrong rect, so drag-hover targets captured a region offset below the crumb.
        // Crumbs already truncate via max_w + text_ellipsis.
        .overflow_hidden();
    let mut cumulative = std::path::PathBuf::new();
    for (i, comp) in path.components().enumerate() {
        let parent = cumulative.clone();
        cumulative.push(comp.as_os_str());
        let raw = comp.as_os_str().to_string_lossy().into_owned();
        let label = if raw.is_empty() { "/".to_string() } else { raw };
        if i > 0 {
            row = row.child(breadcrumb_chevron(
                view,
                tile,
                parent,
                muted,
                hover_bg,
                windows_theme,
            ));
        }
        let target = cumulative.clone();
        row = row.child(breadcrumb_segment(BreadcrumbSegmentProps {
            view: view.clone(),
            tile,
            id: SharedString::from(format!("crumb-{}-{}", tile.0, i)),
            label,
            location: Location::Directory(target.clone()),
            drop_dest: Some(target),
            fg,
            hover_bg,
            windows_theme,
        }));
    }
    row
}

struct BreadcrumbSegmentProps {
    view: Entity<App>,
    tile: TabId,
    id: SharedString,
    label: String,
    location: Location,
    drop_dest: Option<PathBuf>,
    fg: Hsla,
    hover_bg: Hsla,
    windows_theme: bool,
}

fn breadcrumb_segment(props: BreadcrumbSegmentProps) -> gpui::Stateful<gpui::Div> {
    let down_v = props.view.clone();
    let down_location = props.location.clone();
    let hover_id = match &props.location {
        Location::Directory(path) => DragHoverId::Folder(props.tile, path.clone()),
        Location::Archive { archive, inner } => {
            DragHoverId::Folder(props.tile, archive.join(inner))
        }
        _ => DragHoverId::Chrome("breadcrumb", props.tile),
    };
    let command = DragHoverCommand::Process(Action::NavigateLocation {
        tab: props.tile,
        location: props.location.clone(),
    });
    let mut el = div()
        .id(props.id.clone())
        .flex_none()
        .relative()
        .h(crate::ui::rem_px(if props.windows_theme {
            32.0
        } else {
            26.0
        }))
        .max_w(crate::ui::rem_px(if props.windows_theme {
            260.0
        } else {
            220.0
        }))
        .px(crate::ui::rem_px(if props.windows_theme {
            10.0
        } else {
            8.0
        }))
        .rounded(px(if props.windows_theme { 7.0 } else { 6.0 }))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_color(props.fg)
        .overflow_hidden()
        .text_ellipsis()
        .whitespace_nowrap()
        .child(SharedString::from(props.label))
        .hover(move |s| s.bg(props.hover_bg))
        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
            let location = down_location.clone();
            down_v.update(cx, |this, cx| {
                this.state.focused_tile = props.tile;
                this.process_actions(vec![Action::NavigateLocation {
                    tab: props.tile,
                    location,
                }]);
                cx.notify();
            });
            cx.stop_propagation();
        });

    if let Some(dest) = props.drop_dest {
        let drop_v = props.view.clone();
        let drop_v_external = props.view.clone();
        let drop_target = dest.clone();
        let drop_target_external = dest.clone();
        el = el
            .on_prepaint(drag_hover_recorder_with_drop(
                hover_id,
                DragHoverKind::Folder,
                command,
                DragHoverDropCommand::TransferTo(dest),
                &props.view,
            ))
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
            });
    } else {
        el = el.on_prepaint(drag_hover_recorder(
            hover_id,
            DragHoverKind::Folder,
            command,
            &props.view,
        ));
    }

    el
}

fn breadcrumb_chevron(
    view: &Entity<App>,
    tile: TabId,
    dir: PathBuf,
    muted: Hsla,
    hover_bg: Hsla,
    windows_theme: bool,
) -> impl IntoElement {
    let v = view.clone();
    div()
        .flex_none()
        .size(crate::ui::rem_px(if windows_theme { 22.0 } else { 18.0 }))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(if windows_theme { 6.0 } else { 5.0 }))
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .child(theme_icon(
            AtlasIcon::CaretRight,
            11.0,
            muted,
            windows_theme,
        ))
        .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
            let dir = dir.clone();
            let anchor = ev.position;
            v.update(cx, |this, cx| {
                this.state.focused_tile = tile;
                this.open_breadcrumb_menu(tile, dir, anchor, window, cx);
            });
            cx.stop_propagation();
        })
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
    hover_bg: Hsla,
    windows_theme: bool,
) -> impl IntoElement {
    let mut row = h_flex()
        .flex_1()
        .min_w(px(0.))
        .gap(crate::ui::rem_px(if windows_theme { 7.0 } else { 5.0 }))
        .items_center()
        .overflow_hidden();
    let mut sep = false;

    // Real filesystem ancestors of the archive file.
    let mut cumulative = std::path::PathBuf::new();
    for (i, comp) in archive
        .parent()
        .unwrap_or_else(|| std::path::Path::new(""))
        .components()
        .enumerate()
    {
        cumulative.push(comp.as_os_str());
        let raw = comp.as_os_str().to_string_lossy().into_owned();
        let label = if raw.is_empty() { "/".to_string() } else { raw };
        if sep {
            row = row.child(div().text_color(muted).child("›"));
        }
        sep = true;
        let target = cumulative.clone();
        row = row.child(breadcrumb_segment(BreadcrumbSegmentProps {
            view: view.clone(),
            tile,
            id: SharedString::from(format!("crumb-r{}-{}", tile.0, i)),
            label,
            location: Location::Directory(target.clone()),
            drop_dest: Some(target),
            fg,
            hover_bg,
            windows_theme,
        }));
    }

    if sep {
        row = row.child(div().text_color(muted).child("›"));
    }
    let archive_target = archive.to_path_buf();
    row = row.child(breadcrumb_segment(BreadcrumbSegmentProps {
        view: view.clone(),
        tile,
        id: SharedString::from(format!("crumb-archive-{}", tile.0)),
        label: archive
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| archive.display().to_string()),
        location: Location::Archive {
            archive: archive_target,
            inner: PathBuf::new(),
        },
        drop_dest: None,
        fg,
        hover_bg,
        windows_theme,
    }));

    // Inner path components within the archive.
    let mut inner_so_far = std::path::PathBuf::new();
    for (j, comp) in inner.components().enumerate() {
        inner_so_far.push(comp.as_os_str());
        let raw = comp.as_os_str().to_string_lossy().into_owned();
        row = row.child(div().text_color(muted).child("›"));
        let archive_path = archive.to_path_buf();
        let inner_target = inner_so_far.clone();
        row = row.child(breadcrumb_segment(BreadcrumbSegmentProps {
            view: view.clone(),
            tile,
            id: SharedString::from(format!("crumb-a{}-{}", tile.0, j)),
            label: raw,
            location: Location::Archive {
                archive: archive_path.clone(),
                inner: inner_target,
            },
            drop_dest: None,
            fg,
            hover_bg,
            windows_theme,
        }));
    }

    row
}
