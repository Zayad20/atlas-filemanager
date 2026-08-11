//! Root gpui view: the window shell and the recursive render of the pane/tab/split
//! tree (`state::layout_tree`), replacing `egui_tiles`.
//!
//! - `Container::Tabs`  → a `TabBar` + the active pane's content.
//! - `Container::Linear`→ a `Resizable` group (h/v) with one panel per child.
//! - `Tile::Pane`       → the file listing for that tab (click-to-focus).
//!
//! Structural change emits an `Action`/calls an `App` method; the tree never mutates
//! itself during rendering.

use crate::app::App;
use crate::input::action::Action;
use crate::ui::drag_hover::drag_hover_layer;
use crate::ui::glass::{backdrop_tint, glass_sidebar_fill, GlassExt};
use crate::ui::icons::{icon, is_windows_theme, AtlasIcon};
use crate::ui::interactions::{ctrl_scroll_layer, rubber_layer};
use atlas_model::pane::TabId;
use atlas_model::settings::ViewMode;
use gpui::prelude::*;
use gpui::{
    div, hsla, px, AnimationExt, Context, Entity, MouseButton, MouseDownEvent, NavigationDirection,
    SharedString, Window, WindowControlArea,
};
use gpui_component::{h_flex, v_flex, ActiveTheme};
use std::path::PathBuf;

/// A small clickable glyph in the tab toolbar, bound to an `App` mutation.
/// Phase F: payload for an internal file drag (paths dragged between panes / onto
/// folders). The platform/gpui renders the drag image from this view.
#[derive(Clone)]
pub struct DraggedFiles {
    pub(crate) paths: Vec<PathBuf>,
}

impl Render for DraggedFiles {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.paths.len();
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(cx.theme().accent)
            .text_color(cx.theme().accent_foreground)
            .child(SharedString::from(rust_i18n::t!(
                "root_elements",
                count = n
            )))
    }
}

/// Phase F (Windows): placeholder drag view used instead of `DraggedFiles` when the
/// OLE drag-and-drop path (`DoDragDrop`) takes over. Returns an invisible zero-size
/// element so gpui has no ghost to paint. The OLE drag loop provides its own OS
/// cursor feedback.  Using a *distinct type* ensures no `on_drop::<DraggedFiles>`
/// fires when the synthetic `WM_LBUTTONUP` is injected to clear `cx.active_drag`.
#[cfg(windows)]
#[derive(Clone)]
pub(crate) struct DragNoop;

#[cfg(windows)]
impl Render for DragNoop {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Phase C: record a visible item's window-space rect for rubber-band hit-testing.
/// Returns an `on_prepaint` closure to spread onto the item element. The per-pane
/// `row_rects` vec is cleared at the start of each view's render.
pub(crate) fn rect_recorder(
    pane: TabId,
    idx: usize,
    view: &Entity<App>,
) -> impl Fn(gpui::Bounds<gpui::Pixels>, &mut Window, &mut gpui::App) + 'static {
    let view = view.clone();
    move |bounds, _window, cx| {
        crate::perf::count("prepaint.row_rect");
        view.read(cx)
            .row_rects
            .borrow_mut()
            .entry(pane)
            .or_default()
            .push((idx, bounds));
    }
}

pub(crate) fn miller_rect_recorder(
    pane: TabId,
    column: usize,
    idx: usize,
    view: &Entity<App>,
) -> impl Fn(gpui::Bounds<gpui::Pixels>, &mut Window, &mut gpui::App) + 'static {
    let view = view.clone();
    move |bounds, _window, cx| {
        crate::perf::count("prepaint.miller_row_rect");
        view.read(cx)
            .miller_row_rects
            .borrow_mut()
            .entry((pane, column))
            .or_default()
            .push((idx, bounds));
    }
}

pub(crate) fn miller_column_bounds_recorder(
    pane: TabId,
    column: usize,
    view: &Entity<App>,
) -> impl Fn(gpui::Bounds<gpui::Pixels>, &mut Window, &mut gpui::App) + 'static {
    let view = view.clone();
    move |bounds, _window, cx| {
        let should_update = view
            .read(cx)
            .miller_column_bounds
            .get(&(pane, column))
            .copied()
            != Some(bounds);
        if should_update {
            view.update(cx, |this, _| {
                this.miller_column_bounds.insert((pane, column), bounds);
            });
        }
    }
}

#[allow(dead_code)]
pub(crate) fn pane_bounds_recorder(
    pane: TabId,
    view: &Entity<App>,
) -> impl Fn(gpui::Bounds<gpui::Pixels>, &mut Window, &mut gpui::App) + 'static {
    let view = view.clone();
    move |bounds, _window, cx| {
        crate::perf::count("prepaint.pane_bounds");
        let should_update = view.read(cx).pane_view_bounds.get(&pane).copied() != Some(bounds);
        if should_update {
            view.update(cx, |this, _| {
                this.pane_view_bounds.insert(pane, bounds);
            });
        }
    }
}

/// Phase F: record the pane's window-space rect every frame so the OLE drop target
/// can route external file drops to the correct pane directory.  The recorded zones
/// are committed to `dnd.zones` at the start of the next render frame (one-frame lag
/// is invisible; layout is stable between frames).
///
/// On non-Windows this is a no-op closure (`push_pane_zone` has a `#[cfg(windows)]`
/// body) so the `when(cfg!(windows), …)` gate in each chain is purely for clarity.
pub(crate) fn zone_recorder(
    pane_id: TabId,
    view: &Entity<App>,
) -> impl Fn(gpui::Bounds<gpui::Pixels>, &mut Window, &mut gpui::App) + 'static {
    let view = view.clone();
    move |bounds, _window, cx| {
        crate::perf::count("prepaint.zone");
        view.read(cx).push_pane_zone(pane_id, bounds);
    }
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let frame = crate::perf::next_frame();
        crate::perf::count("render.app");
        crate::perf::frame_summary(frame, || {
            let mut panes = 0;
            let mut tabs = 0;
            let mut linear = 0;
            let mut tab_children = 0;
            let mut linear_children = 0;
            for (_, tile) in self.state.pane_tree.tiles.iter() {
                match tile {
                    atlas_model::layout_tree::Tile::Pane(_) => panes += 1,
                    atlas_model::layout_tree::Tile::Container(
                        atlas_model::layout_tree::Container::Tabs(t),
                    ) => {
                        tabs += 1;
                        tab_children += t.children.len();
                    }
                    atlas_model::layout_tree::Tile::Container(
                        atlas_model::layout_tree::Container::Linear(l),
                    ) => {
                        linear += 1;
                        linear_children += l.children.len();
                    }
                }
            }
            // Track which interaction was active when the frame was drawn; slow
            // frames with cause=none mean hover or another untracked notification.
            let mut cause = String::new();
            if self.dock_resize.is_some() {
                cause.push_str("drag_split,");
            }
            if !self.dock_split_animations.is_empty() {
                cause.push_str("anim_split,");
            }
            if !self.pane_animations.is_empty() {
                cause.push_str("anim_pane,");
            }
            if !self.pane_closing.is_empty() {
                cause.push_str("closing,");
            }
            if cause.is_empty() {
                cause.push_str("none");
            } else {
                cause.pop();
            }
            format!(
                "cause={cause} panes={panes} tabs={tabs} linear={linear} tab_children={tab_children} linear_children={linear_children} panels={} signals={} row_rect_panes={} row_rects={} pane_bounds={} dock_bounds={} pending_zones={} dirty={} dirty_all={} closing={}",
                self.file_panel_cache.len(),
                self.pane_signals.len(),
                self.row_rects.borrow().len(),
                self.row_rects.borrow().values().map(Vec::len).sum::<usize>(),
                self.pane_view_bounds.len(),
                self.dock_linear_bounds.len(),
                self.pending_pane_zones.borrow().len(),
                self.dirty_panes.len(),
                self.dirty_all_panes,
                self.pane_closing.len()
            )
        });
        if crate::perf::ui_lab_active() {
            // Keep sampling through the quiet gaps between deterministic actions.
            window.request_animation_frame();
        }
        if let Some(action) = crate::perf::ui_lab_action() {
            crate::perf::event(|| format!("ui_lab action={action:?}"));
            match action {
                crate::perf::UiLabAction::SetDetails => {
                    self.process_actions(vec![crate::input::action::Action::SetViewMode(
                        atlas_model::settings::ViewMode::Details,
                    )]);
                }
                crate::perf::UiLabAction::SplitHorizontal => {
                    self.process_actions(vec![
                        crate::input::action::Action::SplitHorizontalFocused,
                    ]);
                }
                crate::perf::UiLabAction::ClosePane => {
                    self.process_actions(vec![crate::input::action::Action::CloseFocusedPane]);
                }
                crate::perf::UiLabAction::SetGrid => {
                    self.process_actions(vec![crate::input::action::Action::SetViewMode(
                        atlas_model::settings::ViewMode::Grid,
                    )]);
                }
                crate::perf::UiLabAction::Quit => cx.quit(),
            }
        }
        let _perf = crate::perf::span_detail("app.render", || {
            format!(
                "frame={frame} tiles={} panels={} dirty={} dirty_all={} closing={}",
                self.state.pane_tree.tiles.iter().count(),
                self.file_panel_cache.len(),
                self.dirty_panes.len(),
                self.dirty_all_panes,
                self.pane_closing.len()
            )
        });
        if !self.startup_first_render_logged {
            self.startup_first_render_logged = true;
            tracing::info!(
                total_ms = self.startup_started.elapsed().as_millis(),
                "startup: first render"
            );
        }
        let viewport_size = window.viewport_size();
        if self
            .last_viewport_size
            .is_some_and(|last| last != viewport_size)
        {
            self.window_resize_suppress_until =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(180));
            self.sidebar_width_animation = None;
            // The retained inspector body computes image/text geometry from
            // `Window::viewport_size()`, which is not part of AppState. A
            // window resize is therefore an explicit invalidation even when
            // the focused preview data itself is unchanged.
            if let Some(host) = &self.inspector_host {
                host.update(cx, |_, cx| cx.notify());
            }
        }
        self.last_viewport_size = Some(viewport_size);
        let suppress_resize_animations = self.suppress_window_resize_animations();
        // Drain any results that arrived between the 60Hz background loop and this
        // render frame.  The startup loop already calls cx.notify() when dirty; the
        // drain here handles the narrow window where results arrive mid-frame.
        // We do NOT call cx.notify() again — that would schedule a redundant render
        // cycle on top of the one that is already executing right now.
        {
            let _perf = crate::perf::span("app.render.drain_bg_state");
            self.drain_bg_results_state_only();
            self.tick_eco_mode(window);
        }
        // Phase C: reset per-frame item rects only when idle. During rubber-band drag
        // the pane content may be cached, so clearing here can leave hit-testing with
        // no geometry until the pane decides to repaint.
        if self.rubber.is_none() && !self.drag_hover_maybe_active() {
            self.row_rects.borrow_mut().clear();
            self.miller_row_rects.borrow_mut().clear();
            self.miller_column_bounds.clear();
            self.pane_view_bounds.clear();
            self.dropzone_rects.clear();
            self.dropzone_bounds = None;
        }
        self.dock_linear_bounds.clear();
        if !self.drag_hover_maybe_active() {
            self.pending_drag_hover_targets.clear();
        }
        self.prune_pane_animations();
        self.prune_pane_layout_animations();
        // Note: no App-level `request_animation_frame` here. Each animating `FilePanel`
        // self-drives its own frames via `with_animation`, so the whole App tree no
        // longer re-renders every animation frame (was the stutter source under many
        // panes). FLIP starts re-trigger a frame from the pane's `on_prepaint`.
        // Kick closing panes: the active panel view is `.cached()`, so a closing
        // `FilePanel` won't re-render (and thus won't start its exit `with_animation`)
        // on an App notify alone. Notify its entity directly; `with_animation` then
        // self-drives the rest. Cheap: only runs while a pane is mid-close.
        if !self.pane_closing.is_empty() {
            let _perf = crate::perf::span_detail("pane_close.notify", || {
                format!("closing={}", self.pane_closing.len())
            });
            crate::perf::count_by("pane_close.notify_count", self.pane_closing.len() as u64);
            let closing: Vec<_> = self.pane_closing.keys().copied().collect();
            for id in closing {
                if let Some(panel) = self.file_panel_cache.get(&id) {
                    panel.update(cx, |_, cx| cx.notify());
                }
            }
        }
        // Phase F (Windows): publish last frame's pane drop-zones to the OLE drop
        // target, then clear the vec so this frame re-populates it via on_prepaint.
        // One frame of lag is harmless — layout is stable between frames.
        #[cfg(windows)]
        if let Some(dnd) = &self.dnd {
            if let Ok(mut z) = dnd.zones.lock() {
                let zones = std::mem::take(&mut *self.pending_pane_zones.borrow_mut());
                z.update(zones);
            }
        }
        // Apply a pending theme switch (profile restore / switch / user selection)
        // before reading colors so this frame paints with the new palette.
        if self.theme_dirty {
            self.theme_dirty = false;
            self.apply_active_theme(window, cx);
        }
        if let Some((_, _, frames)) = &mut self.rename_visual_suppress {
            *frames = frames.saturating_sub(1);
            if *frames == 0 {
                self.rename_visual_suppress = None;
            }
        }
        if let Some((pane, path, select_all)) = self.pending_rename.clone() {
            let needs_scroll = self
                .state
                .tab_states
                .get(&pane)
                .is_some_and(|ts| ts.selection.scroll_to_selected);
            if needs_scroll && self.scroll_selected_into_view(pane, gpui::ScrollStrategy::Center) {
                if let Some(ts) = self.state.tab_states.get_mut(&pane) {
                    ts.selection.scroll_to_selected = false;
                }
            }
            let ready = self.state.tab_states.get(&pane).is_some_and(|ts| {
                if self
                    .tab_data_for(pane)
                    .is_some_and(|tab| tab.view_mode == atlas_model::settings::ViewMode::Miller)
                {
                    return ts
                        .miller
                        .columns
                        .get(ts.miller.focused)
                        .and_then(|col| {
                            col.selection
                                .keyboard_cursor
                                .or_else(|| col.selection.selected.iter().copied().next())
                                .and_then(|idx| col.entries.get(idx))
                        })
                        .is_some_and(|entry| entry.path == path);
                }
                !ts.selection.scroll_to_selected
                    && ts
                        .selection
                        .keyboard_cursor
                        .or_else(|| ts.selection.selected.iter().copied().next())
                        .and_then(|idx| ts.entries.get(idx))
                        .is_some_and(|entry| entry.path == path)
            });
            if ready {
                self.pending_rename = None;
                self.state.focused_tile = pane;
                self.begin_rename(window, cx);
                if select_all {
                    if let Some(edit) = self.rename_edit.as_mut() {
                        let len = edit.input.read(cx).value().chars().count() as u32;
                        edit.initial_select_chars = Some(len);
                    }
                }
            }
        }
        // (Rename-profile is now a normal overlay layer rendered by `render_overlays`
        // + focused via the overlay-focus arbiter; no render-time dialog open.)
        let sidebar_width_dirty = self.sidebar_width_dirty;
        self.sidebar_width_dirty = false;
        let (fg, _muted, _status_border, root_bg) = {
            let t = cx.theme();
            (t.foreground, t.muted_foreground, t.border, t.background)
        };
        let active_theme = self.state.profiles.active().theme.as_str();
        let windows_theme = is_windows_theme(active_theme);
        let windows_glass = active_theme == "Atlas Windows Glass";
        let figma_glass = active_theme == "Atlas Figma Glass";
        let glass_theme = active_theme.ends_with(" Glass") || active_theme == "Atlas Frost";
        let center_content = {
            let _perf = crate::perf::span("app.render.center_dock");
            // Re-assert pane focus every App frame (the dock now lives in its own
            // `DockHost` view, which doesn't render on every App frame).
            self.sync_pane_focus(window, cx);
            // Host the dock in its own view so split/resize animations
            // (`request_animation_frame`) re-render only the dock, not all of App.
            crate::ui::layout_region::layout_region(
                "dock",
                self.ensure_dock_host(cx).into_any_element(),
            )
            .into_any_element()
        };
        // ATLAS_PERF_SKIP_SIDEBAR=1: omit sidebar build/paint for isolation runs.
        let sidebar =
            (self.state.settings.show_sidebar && !crate::perf::skip_sidebar()).then(|| {
                let target_width = if self.state.settings.sidebar_collapsed {
                    if windows_theme {
                        64.0
                    } else {
                        56.0
                    }
                } else {
                    self.state.settings.sidebar_width
                };
                if sidebar_width_dirty {
                    match self.sidebar_render_width {
                        Some(from)
                            if !self.state.settings.disable_animations
                                && !suppress_resize_animations
                                && (from - target_width).abs() > f32::EPSILON =>
                        {
                            self.sidebar_width_animation =
                                Some(crate::app::SidebarWidthAnimation::new(from, target_width));
                        }
                        _ => {
                            self.sidebar_width_animation = None;
                        }
                    }
                }
                let width = match self.sidebar_width_animation {
                    Some(anim) if anim.is_alive() => {
                        window.request_animation_frame();
                        anim.width()
                    }
                    _ => {
                        self.sidebar_width_animation = None;
                        target_width
                    }
                };
                self.sidebar_render_width = Some(width);
                let panel = if let Some(panel) = &self.sidebar_panel {
                    panel.clone()
                } else {
                    let app = cx.entity();
                    let profile_names: Vec<String> = self
                        .state
                        .profiles
                        .profiles
                        .iter()
                        .map(|p| p.name.clone())
                        .collect();
                    let active_profile = self.state.profiles.active_idx;
                    let panel = cx.new(|cx| {
                        crate::ui::sidebar::SidebarPanel::new(
                            app,
                            profile_names,
                            active_profile,
                            window,
                            cx,
                        )
                    });
                    self.sidebar_panel = Some(panel.clone());
                    panel
                };
                let panel_bg = if figma_glass {
                    glass_sidebar_fill()
                } else if windows_glass {
                    cx.theme().sidebar
                } else if windows_theme {
                    hsla(0., 0., 1., 0.20)
                } else if glass_theme {
                    cx.theme().sidebar.opacity(0.42)
                } else {
                    backdrop_tint(cx.theme().secondary, 0.10)
                };
                let shell = div()
                    .w(px(width))
                    .h_full()
                    .flex_none()
                    .overflow_hidden()
                    .when(figma_glass, |s| s.glass_surface(panel_bg).shadow_lg())
                    .when(!figma_glass, |s| {
                        s.rounded(cx.theme().radius_lg).bg(panel_bg)
                    });
                // Keep the expensive sidebar tree, hitboxes, and paint commands
                // retained while its fingerprint and bounds are unchanged.
                let panel =
                    gpui::AnyView::from(panel).cached(gpui::StyleRefinement::default().size_full());
                crate::ui::layout_region::layout_region("sidebar", shell.child(panel))
                    .into_any_element()
            });
        let sidebar_resizer = (self.state.settings.show_sidebar
            && !self.state.settings.sidebar_collapsed
            && !crate::perf::skip_sidebar())
        .then(|| {
            div()
                .absolute()
                .left(-px(6.))
                .top_0()
                .w(px(12.))
                .h_full()
                .cursor_col_resize()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _window, cx| {
                        this.sidebar_resize = Some(crate::app::SidebarResizeState {
                            start_x: f32::from(ev.position.x),
                            start_width: this.state.settings.sidebar_width,
                        });
                        cx.stop_propagation();
                    }),
                )
                .into_any_element()
        });

        // Phase5: inspector panel (right of body) + overlay layers (above root).
        let preview_visible = self.preview_visible_for_focused();
        let preview_closing = self.preview_panel_closing.is_some_and(|started| {
            started.elapsed() < std::time::Duration::from_millis(crate::ui::motion::PANEL_MS)
        });
        if self.preview_panel_closing.is_some() && !preview_closing {
            self.preview_panel_closing = None;
        }
        if preview_closing {
            window.request_animation_frame();
        }
        let preview_expanded = self
            .preview_view
            .get(&self.state.focused_tile)
            .is_some_and(|v| v.expanded);
        let preview_should_render = preview_visible || preview_closing;
        let viewport = window.viewport_size();
        let inspector_width = if preview_expanded {
            self.state
                .settings
                .inspector_width
                .max(180.0)
                .min((f32::from(viewport.width) - 32.0).max(180.0))
        } else {
            self.state
                .settings
                .inspector_width
                .max(180.0)
                .min((f32::from(viewport.width) - 96.0).max(180.0))
        };
        let inspector_container =
            (preview_should_render && !crate::perf::skip_inspector()).then(|| {
                crate::perf::count("render.inspector");
                let resize_handle = (preview_visible && !preview_expanded && !preview_closing)
                    .then(|| {
                        div()
                            .absolute()
                            .left(-px(6.0))
                            .top_0()
                            .w(px(12.0))
                            .h_full()
                            .cursor_col_resize()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseDownEvent, _window, cx| {
                                    this.inspector_resize =
                                        Some(crate::app::InspectorResizeState {
                                            start_x: f32::from(ev.position.x),
                                            start_width: this.state.settings.inspector_width,
                                        });
                                    cx.stop_propagation();
                                }),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left(px(5.5))
                                    .top_0()
                                    .w(px(1.0))
                                    .h_full()
                                    .bg(cx.theme().border.opacity(0.7)),
                            )
                            .occlude()
                    });
                let panel = div()
                    .relative()
                    .h_full()
                    .w(px(inspector_width))
                    .flex_shrink_0()
                    .overflow_hidden()
                    // Keep preview content in a retained host. The App root
                    // still owns the panel shell/resize animation, but a pane
                    // split or unrelated App notification no longer rebuilds
                    // all preview rows and controls.
                    .child(self.ensure_inspector_host(cx).into_any_element())
                    .children(resize_handle);
                let panel: gpui::AnyElement = if self.state.settings.disable_animations {
                    panel.into_any_element()
                } else if suppress_resize_animations {
                    panel.into_any_element()
                } else {
                    let closing = preview_closing && !preview_visible;
                    panel
                        .with_animation(
                            SharedString::from(format!(
                                "preview-panel-{}-{}",
                                self.preview_panel_anim_seq, closing
                            )),
                            crate::ui::motion::animation(crate::ui::motion::PANEL_MS),
                            move |this, t| {
                                let openness = if closing { 1.0 - t } else { t };
                                this.w(px(inspector_width * openness))
                                    .opacity(openness)
                                    .paint_offset(gpui::point(px(10.0 * (1.0 - openness)), px(0.0)))
                            },
                        )
                        .into_any_element()
                };
                crate::ui::layout_region::layout_region("inspector", panel).into_any_element()
            });
        let overlays = if crate::perf::skip_overlays() {
            crate::perf::count("render.overlays.skipped");
            Vec::new()
        } else {
            crate::perf::count("render.overlays");
            let _perf = crate::perf::span("app.render.overlays");
            self.render_overlays(window, cx)
                .into_iter()
                .map(|el| {
                    crate::ui::layout_region::layout_region("overlays", el).into_any_element()
                })
                .collect()
        };
        let marquee = self.render_rubber_marquee(cx);
        // Floating search pill hidden for now (per request). Re-enable by
        // restoring `(!self.state.overlay.show_goto_bar).then(...)`.
        let search_pill: Option<gpui::AnyElement> = None;

        // While a modal owns the keyboard (an overlay or inline rename), drop the
        // "Atlas" key context so its nav/open shortcuts don't resolve and fire on the
        // file pane behind the overlay. The overlay's own `capture_key_down` (and the
        // focused input) drive the keys instead.
        // Single source of truth (includes the right-click menu + rename overlay,
        // which were previously missing — that let arrow keys reach the pane).
        let key_ctx = if self.modal_overlay_active() {
            "AtlasModal"
        } else {
            "Atlas"
        };

        let view = cx.entity();
        let mouse_back_view = view.clone();
        let mouse_forward_view = view.clone();

        v_flex()
            .size_full()
            // Keep theme tint, but let the system backdrop breathe through.
            .bg(root_bg)
            .relative()
            .text_color(fg)
            .text_sm()
            .font_family(self.state.settings.font_family.clone())
            .key_context(key_ctx)
            .track_focus(&self.focus_handle)
            .on_modifiers_changed({
                let view = view.clone();
                move |_ev, _window, cx| {
                    view.update(cx, |_, cx| cx.notify());
                }
            })
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                move |_ev, _window, cx| {
                    mouse_back_view.update(cx, |this, cx| {
                        let tile = this.state.focused_tile;
                        this.navigate_back(tile);
                        cx.notify();
                    });
                    cx.stop_propagation();
                },
            )
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Forward),
                move |_ev, _window, cx| {
                    mouse_forward_view.update(cx, |this, cx| {
                        let tile = this.state.focused_tile;
                        this.navigate_forward(tile);
                        cx.notify();
                    });
                    cx.stop_propagation();
                },
            )
            .on_action(cx.listener(|this, _: &crate::NavUp, _w, cx| {
                this.nav_up(false);
                this.notify_pane(this.state.focused_tile, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NavDown, _w, cx| {
                this.nav_down(false);
                this.notify_pane(this.state.focused_tile, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::ExtendUp, _w, cx| {
                this.nav_up(true);
                this.notify_pane(this.state.focused_tile, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::ExtendDown, _w, cx| {
                this.nav_down(true);
                this.notify_pane(this.state.focused_tile, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NavHome, _w, cx| {
                this.nav_home(false);
                this.notify_pane(this.state.focused_tile, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NavEnd, _w, cx| {
                this.nav_end(false);
                this.notify_pane(this.state.focused_tile, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NavPageUp, _w, cx| {
                this.nav_page(false, false);
                this.notify_pane(this.state.focused_tile, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NavPageDown, _w, cx| {
                this.nav_page(true, false);
                this.notify_pane(this.state.focused_tile, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::OpenSelected, _w, cx| {
                this.open_cursor();
                this.notify_pane(this.state.focused_tile, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::SelectAllItems, _w, cx| {
                this.process_actions(vec![Action::SelectAll]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::RenameSelected, window, cx| {
                this.begin_rename(window, cx);
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
            // Phase D: overlays / panels / toggles.
            .on_action(cx.listener(|this, _: &crate::OpenPalette, _w, cx| {
                this.process_actions(vec![Action::OpenPalette]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::OpenGotoBar, _w, cx| {
                this.process_actions(vec![Action::OpenGotoBar]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::OpenSearchInPlace, _w, cx| {
                this.process_actions(vec![Action::OpenSearchInPlace]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::ViewModeDetails, _w, cx| {
                this.process_actions(vec![Action::SetViewMode(ViewMode::Details)]);
                this.flush_dirty_panes(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::ViewModeGrid, _w, cx| {
                this.process_actions(vec![Action::SetViewMode(ViewMode::Grid)]);
                this.flush_dirty_panes(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::ViewModeList, _w, cx| {
                this.process_actions(vec![Action::SetViewMode(ViewMode::List)]);
                this.flush_dirty_panes(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::ViewModeMiller, _w, cx| {
                this.process_actions(vec![Action::SetViewMode(ViewMode::Miller)]);
                this.flush_dirty_panes(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::ZoomIn, _w, cx| {
                this.process_actions(vec![crate::input::action::Action::ZoomIn]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::ZoomOut, _w, cx| {
                this.process_actions(vec![crate::input::action::Action::ZoomOut]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::ZoomReset, _w, cx| {
                this.process_actions(vec![crate::input::action::Action::ZoomReset]);
                cx.notify();
            }))
            // Type-to-filter: plain characters typed while no text field has focus
            // open/extend the focused pane's local fuzzy filter. Captured at the
            // window root because keyboard focus often rests on untracked handles
            // (Dock panels), so per-pane listeners never see these keys.
            .capture_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, window, cx| {
                if this.handle_global_command_shortcut_key(ev, window, cx) {
                    cx.stop_propagation();
                    // Command shortcuts (selection.next/previous/etc.) mutate a pane's
                    // TabState directly and have no `cx` of their own — without this the
                    // focused pane's `.cached()` file view never lands in `dirty_views`,
                    // so keyboard nav appears to "lag" until an unrelated repaint catches
                    // the accumulated cursor moves up.
                    this.notify_pane(this.state.focused_tile, cx);
                    cx.notify();
                    return;
                }
                this.handle_global_type_to_filter(ev, window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &crate::TogglePreviewExpanded, _w, cx| {
                    // Space only acts when the file list is the active surface and the
                    // preview is visible — never while a text input/overlay has focus.
                    if this.preview_visible_for_focused()
                        && !this.state.overlay.show_goto_bar
                        && !this.state.overlay.show_palette
                        && !this.state.overlay.show_settings
                        && this.rename_edit.is_none()
                        && this.state.batch_rename.is_none()
                    {
                        this.toggle_preview_expanded();
                        cx.notify();
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &crate::ToggleInspector, _w, cx| {
                this.process_actions(vec![Action::ToggleInspector]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleSidebar, window, cx| {
                let _ = window;
                this.process_actions(vec![Action::ToggleSidebar]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleHidden, _w, cx| {
                this.process_actions(vec![Action::ToggleHidden]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::OpenSettings, _w, cx| {
                this.process_actions(vec![Action::OpenSettings]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::OpenExtManager, _w, cx| {
                this.process_actions(vec![Action::OpenExtManager]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::CloseOverlays, _w, cx| {
                this.process_actions(vec![Action::CloseOverlays]);
                this.notify_pane(this.state.focused_tile, cx);
                cx.notify();
            }))
            // Phase D: pane structure.
            .on_action(cx.listener(|this, _: &crate::CloseFocusedPane, _w, cx| {
                this.process_actions(vec![Action::CloseFocusedPane]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::FocusNextPane, _w, cx| {
                this.process_actions(vec![Action::FocusNextPane]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::SplitHorizontal, _w, cx| {
                this.process_actions(vec![Action::SplitHorizontalFocused]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::SplitVertical, _w, cx| {
                this.process_actions(vec![Action::SplitVerticalFocused]);
                cx.notify();
            }))
            // Phase D: history / location / refresh.
            .on_action(cx.listener(|this, _: &crate::Undo, _w, cx| {
                this.process_actions(vec![Action::Undo]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::Redo, _w, cx| {
                this.process_actions(vec![Action::Redo]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::RefreshTab, _w, cx| {
                let t = this.state.focused_tile;
                this.process_actions(vec![Action::Refresh(t)]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::EditLocation, _w, cx| {
                this.open_location_finder_or_editor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NavBack, _w, cx| {
                let t = this.state.focused_tile;
                this.process_actions(vec![Action::NavigateBack(t)]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NavForward, _w, cx| {
                let t = this.state.focused_tile;
                this.process_actions(vec![Action::NavigateForward(t)]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::NavParent, _w, cx| {
                let t = this.state.focused_tile;
                this.process_actions(vec![Action::NavigateUp(t)]);
                cx.notify();
            }))
            // Phase D: clipboard / delete on the current selection.
            .on_action(cx.listener(|this, _: &crate::DeleteToTrash, _w, cx| {
                if this
                    .state
                    .active_tab_data()
                    .map_or(false, |t| t.location.is_archive())
                {
                    this.toast_error(rust_i18n::t!("root_delete_archive_error").into());
                    cx.notify();
                    return;
                }
                let paths = this.selected_paths();
                if !paths.is_empty() {
                    this.process_actions(vec![Action::Delete {
                        paths,
                        to_trash: true,
                    }]);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::DeletePermanent, _w, cx| {
                if this
                    .state
                    .active_tab_data()
                    .map_or(false, |t| t.location.is_archive())
                {
                    this.toast_error(rust_i18n::t!("root_delete_archive_error").into());
                    cx.notify();
                    return;
                }
                let paths = this.selected_paths();
                if !paths.is_empty() {
                    this.process_actions(vec![Action::Delete {
                        paths,
                        to_trash: false,
                    }]);
                }
                cx.notify();
            }))
            .on_action(
                cx.listener(|this, _: &gpui_component::input::Copy, _w, cx| {
                    if this
                        .state
                        .active_tab_data()
                        .map_or(false, |t| t.location.is_archive())
                    {
                        this.toast_error(rust_i18n::t!("root_copy_archive_error").into());
                        cx.notify();
                        return;
                    }
                    let paths = this.selected_paths();
                    if !paths.is_empty() {
                        this.process_actions(vec![Action::Copy(paths)]);
                    }
                    cx.notify();
                }),
            )
            .on_action(cx.listener(|this, _: &gpui_component::input::Cut, _w, cx| {
                if this
                    .state
                    .active_tab_data()
                    .map_or(false, |t| t.location.is_archive())
                {
                    this.toast_error(rust_i18n::t!("root_cut_archive_error").into());
                    cx.notify();
                    return;
                }
                let paths = this.selected_paths();
                if !paths.is_empty() {
                    this.process_actions(vec![Action::Cut(paths)]);
                }
                cx.notify();
            }))
            .on_action(
                cx.listener(|this, _: &gpui_component::input::Paste, _w, cx| {
                    if this
                        .state
                        .active_tab_data()
                        .map_or(false, |t| t.location.is_archive())
                    {
                        this.toast_error(rust_i18n::t!("root_paste_archive_error").into());
                        cx.notify();
                        return;
                    }
                    let dest = this.state.active_tab_data().and_then(|t| {
                        // Local tab: use the real filesystem dir.
                        if let Some(p) = t.location.fs_dir() {
                            return Some(p.to_path_buf());
                        }
                        // Remote tab: use an empty sentinel; the action handler
                        // detects Location::Remote and routes to the cloud worker.
                        #[cfg(feature = "provider-host")]
                        if matches!(t.location, atlas_model::location::Location::Remote { .. }) {
                            return Some(PathBuf::new());
                        }
                        None
                    });
                    if let Some(dest) = dest {
                        this.process_actions(vec![Action::Paste { dest }]);
                        cx.notify();
                    }
                }),
            )
            // Custom title bar: reserve the same vertical space the old Dock shell
            // expected, so panes don't run under the window controls.
            .children(self.state.settings.show_titlebar.then(|| {
                let icon_dim = cx.theme().muted_foreground;
                let is_max = window.is_maximized();
                h_flex()
                    .id("titlebar")
                    .w_full()
                    .h(px(10.))
                    .items_center()
                    .px(px(4.))
                    .relative()
                    .window_control_area(WindowControlArea::Drag)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right(px(78.))
                            .h(px(22.))
                            .window_control_area(WindowControlArea::Drag),
                    )
                    .child(div().flex_1())
                    .child(
                        h_flex()
                            .gap(px(1.))
                            .items_center()
                            // Minimize
                            .child(
                                div()
                                    .id("wc-min")
                                    .flex_none()
                                    .w(px(24.))
                                    .h(px(10.))
                                    .rounded(px(3.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .hover(|s| s.bg(hsla(0., 0., 0.5, 0.10)))
                                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                    })
                                    .on_click(|_, window, cx| {
                                        cx.stop_propagation();
                                        window.minimize_window();
                                    })
                                    .child(icon(AtlasIcon::Minus, 7.0, icon_dim)),
                            )
                            // Maximize / Restore
                            .child(
                                div()
                                    .id("wc-max")
                                    .flex_none()
                                    .w(px(24.))
                                    .h(px(10.))
                                    .rounded(px(3.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .hover(|s| s.bg(hsla(0., 0., 0.5, 0.10)))
                                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                    })
                                    .on_click(|_, window, cx| {
                                        cx.stop_propagation();
                                        window.zoom_window();
                                    })
                                    .child(icon(
                                        if is_max {
                                            AtlasIcon::Copy
                                        } else {
                                            AtlasIcon::Square
                                        },
                                        7.0,
                                        icon_dim,
                                    )),
                            )
                            // Close
                            .child(
                                div()
                                    .id("wc-close")
                                    .flex_none()
                                    .w(px(24.))
                                    .h(px(10.))
                                    .rounded(px(3.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .hover(|s| s.bg(hsla(0., 0.65, 0.42, 0.75)))
                                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                    })
                                    .on_click(|_, window, cx| {
                                        cx.stop_propagation();
                                        window.remove_window();
                                    })
                                    .child(icon(AtlasIcon::X, 7.0, icon_dim)),
                            ),
                    )
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .px(px(if figma_glass { 10.0 } else { 3.0 }))
                    .py(px(if figma_glass { 5.0 } else { 3.0 }))
                    .gap(px(if figma_glass {
                        4.0
                    } else if windows_theme {
                        0.0
                    } else {
                        3.0
                    }))
                    .children(sidebar)
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .h_full()
                            .min_w(px(0.))
                            .children(sidebar_resizer)
                            .child(center_content),
                    )
                    .children(inspector_container),
            )
            // Phase5: overlay layers (goto bar, palette, settings, paste conflict, …).
            .children(overlays)
            // Phase C: the rubber-band selection rectangle, painted over the panes.
            .children(marquee)
            // Phase C: global rubber-band motor (invisible canvas, registers window-wide
            // mouse move/up listeners that fire even outside the pane hitbox).
            .child(rubber_layer(cx.entity()))
            .child(ctrl_scroll_layer(cx.entity()))
            .children(self.render_drag_hover_blink(cx))
            .child(drag_hover_layer(cx.entity()))
            // Floating glass search pill (Figma "Window/Search Global"), bottom-center.
            .children(search_pill)
    }
}

/// Shared Ctrl+scroll handler for every pane container. Plain scrolls are ignored
/// (the virtualized list scrolls itself); Ctrl+scroll routes to `pane_ctrl_scroll`.
pub(crate) fn ctrl_scroll(
    ev: &gpui::ScrollWheelEvent,
    pane_id: TabId,
    view: &Entity<App>,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    if !(ev.modifiers.control || ev.modifiers.platform) {
        return;
    }
    window.prevent_default();
    cx.stop_propagation();
    let dy = match ev.delta {
        gpui::ScrollDelta::Pixels(p) => f32::from(p.y),
        gpui::ScrollDelta::Lines(l) => l.y,
    };
    if dy == 0.0 {
        return;
    }
    view.update(cx, |this, cx| {
        this.pane_ctrl_scroll(pane_id, dy);
        cx.notify();
    });
}
