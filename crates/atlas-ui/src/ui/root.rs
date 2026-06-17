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
use crate::state::pane::TabId;
use crate::state::settings::ViewMode;
use crate::ui::drag_hover::drag_hover_layer;
use crate::ui::glass::glass_chrome_fill;
use crate::ui::icons::{icon, AtlasIcon};
use crate::ui::interactions::rubber_layer;
use crate::util::display::format_size;
use gpui::prelude::*;
use gpui::{
    div, hsla, px, Context, Entity, MouseButton, NavigationDirection, SharedString, Window,
    WindowControlArea,
};
use gpui_component::dock::DockPlacement;
use gpui_component::{h_flex, v_flex, ActiveTheme};
use std::path::PathBuf;

/// A small clickable glyph in the tab toolbar, bound to an `App` mutation.
/// FASE F: payload for an internal file drag (paths dragged between panes / onto
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

/// FASE F (Windows): placeholder drag view used instead of `DraggedFiles` when the
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

/// FASE C: record a visible item's window-space rect for rubber-band hit-testing.
/// Returns an `on_prepaint` closure to spread onto the item element. The per-pane
/// `row_rects` vec is cleared at the start of each view's render.
pub(crate) fn rect_recorder(
    pane: TabId,
    idx: usize,
    view: &Entity<App>,
) -> impl Fn(gpui::Bounds<gpui::Pixels>, &mut Window, &mut gpui::App) + 'static {
    let view = view.clone();
    move |bounds, _window, cx| {
        view.update(cx, |this, _| {
            this.row_rects.entry(pane).or_default().push((idx, bounds));
        });
    }
}

pub(crate) fn pane_bounds_recorder(
    pane: TabId,
    view: &Entity<App>,
) -> impl Fn(gpui::Bounds<gpui::Pixels>, &mut Window, &mut gpui::App) + 'static {
    let view = view.clone();
    move |bounds, _window, cx| {
        view.update(cx, |this, _| {
            this.pane_view_bounds.insert(pane, bounds);
        });
    }
}

/// FASE F: record the pane's window-space rect every frame so the OLE drop target
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
        view.update(cx, |this, _cx| {
            this.push_pane_zone(pane_id, bounds);
        });
    }
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.drain_bg_results() {
            cx.notify();
        }
        // FASE C: reset per-frame item rects; each visible row/cell re-records via
        // on_prepaint while painting this frame (used for rubber-band hit-testing).
        self.row_rects.clear();
        self.pane_view_bounds.clear();
        self.pending_drag_hover_targets.clear();
        self.begin_pane_layout_frame();
        self.prune_pane_animations();
        self.prune_pane_layout_animations();
        if !self.pane_animations.is_empty() || !self.pane_layout_animations.is_empty() {
            window.request_animation_frame();
        }
        // FASE F (Windows): publish last frame's pane drop-zones to the OLE drop
        // target, then clear the vec so this frame re-populates it via on_prepaint.
        // One frame of lag is harmless — layout is stable between frames.
        #[cfg(windows)]
        if let Some(dnd) = &self.dnd {
            if let Ok(mut z) = dnd.zones.lock() {
                z.update(std::mem::take(&mut self.pending_pane_zones));
            }
        }
        // Apply a pending theme switch (profile restore / switch / user selection)
        // before reading colors so this frame paints with the new palette.
        if self.theme_dirty {
            self.theme_dirty = false;
            self.apply_active_theme(window, cx);
        }
        // (Rename-profile is now a normal overlay layer rendered by `render_overlays`
        // + focused via the overlay-focus arbiter; no render-time dialog open.)
        // Apply the active profile's sidebar width to the left dock (needs the dock
        // entity + window, which `switch_profile` lacks). One-shot per change.
        if self.sidebar_width_dirty {
            self.sidebar_width_dirty = false;
            let width = self.state.settings.sidebar_width;
            if let Some(left) = self
                .dock
                .as_ref()
                .and_then(|d| d.read(cx).left_dock().cloned())
            {
                left.update(cx, |dock, cx| dock.set_size(px(width), window, cx));
            }
        }
        let (fg, muted, status_border) = {
            let t = cx.theme();
            (t.foreground, t.muted_foreground, t.border)
        };
        // Most recent background operation, rendered as a floating overlay pill
        // bar with a cancel button. `active_ops` updates via OpProgress events.
        let op_progress = self
            .state
            .ops
            .active_ops
            .iter()
            .max_by_key(|(id, _)| **id)
            .map(|(id, op)| {
                let file = op
                    .current_file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let mut text = if op.label.is_empty() {
                    format!("{file} — {}/{}", op.done.min(op.total), op.total)
                } else {
                    format!(
                        "{} {file} — {}/{}",
                        op.label,
                        op.done.min(op.total),
                        op.total
                    )
                };
                if op.bytes_total > 0 {
                    text.push_str(&format!(
                        " · {} / {}",
                        format_size(op.bytes_done),
                        format_size(op.bytes_total)
                    ));
                }
                let n_ops = self.state.ops.active_ops.len();
                if n_ops > 1 {
                    text.push_str(&format!(
                        " ({})",
                        rust_i18n::t!("root_operations", count = n_ops)
                    ));
                }
                (*id, text, op.cancel.clone())
            });

        // Diagnostic fast path: bypass the gpui-component Dock/FilePanel bridge to
        // isolate whether window-move lag comes from the dock layer or from pane UI.
        let center_content = if std::env::var("ATLAS_BYPASS_DOCK").is_ok_and(|v| v != "0") {
            self.render_pane(self.state.focused_tile, window, cx)
        } else {
            // FASE A: the central tabs/splits are a gpui-component DockArea built from
            // `pane_tree` (tab drag/reorder + drag-to-split come from the Dock).
            self.ensure_dock(window, cx).into_any_element()
        };

        // FASE5: inspector panel (right of body) + overlay layers (above root).
        let inspector = if self.preview_visible_for_focused() {
            Some(
                div()
                    .h_full()
                    .py(px(crate::ui::SIDE_PANEL_VERTICAL_INSET))
                    .child(self.render_inspector(window, cx))
                    .into_any_element(),
            )
        } else {
            None
        };
        let overlays = self.render_overlays(window, cx);
        let marquee = self.render_rubber_marquee(cx);
        let search_pill =
            (!self.state.overlay.show_goto_bar).then(|| self.render_floating_search_pill(cx));

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
            // Opaque themes paint their real window background. Transparent themes
            // like Atlas Frost still allow acrylic through when ATLAS_ACRYLIC=1.
            .bg(cx.theme().background)
            .relative()
            .text_color(fg)
            .text_sm()
            .key_context(key_ctx)
            .track_focus(&self.focus_handle)
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
            // FASE D: overlays / panels / toggles.
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
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::ViewModeGrid, _w, cx| {
                this.process_actions(vec![Action::SetViewMode(ViewMode::Grid)]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::ViewModeList, _w, cx| {
                this.process_actions(vec![Action::SetViewMode(ViewMode::List)]);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::ViewModeMiller, _w, cx| {
                this.process_actions(vec![Action::SetViewMode(ViewMode::Miller)]);
                cx.notify();
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
                if let Some(dock) = this.dock.clone() {
                    dock.update(cx, |da, cx| da.toggle_dock(DockPlacement::Left, window, cx));
                }
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
                cx.notify();
            }))
            // FASE D: pane structure.
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
            // FASE D: history / location / refresh.
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
                // Ctrl+L: open the fuzzy goto pre-filled with the pane's absolute
                // path (copy it, or Tab into subfolders). Non-filesystem
                // locations fall back to the inline path editor.
                let t = this.state.focused_tile;
                let dir = this
                    .tab_data_for(t)
                    .and_then(|td| td.location.fs_dir().map(|p| p.to_path_buf()));
                if let Some(dir) = dir {
                    this.goto_prefill =
                        Some(format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR));
                    this.process_actions(vec![Action::OpenGotoBar]);
                } else {
                    this.process_actions(vec![Action::EditLocation(t)]);
                }
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
            // FASE D: clipboard / delete on the current selection.
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
                    let dest = this.state.active_tab_data().and_then(|t| {
                        // Local tab: use the real filesystem dir.
                        if let Some(p) = t.location.fs_dir() {
                            return Some(p.to_path_buf());
                        }
                        // Remote tab: use an empty sentinel; the action handler
                        // detects Location::Remote and routes to the cloud worker.
                        #[cfg(feature = "cloud")]
                        if matches!(t.location, crate::state::location::Location::Remote { .. }) {
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
            // Custom title bar: ultra-thin draggable strip + grouped phosphor controls.
            // Sits flush to the top edge; controls cluster right with 2px outer padding.
            .child({
                let icon_dim = cx.theme().muted_foreground;
                let is_max = window.is_maximized();
                h_flex()
                    .id("titlebar")
                    .w_full()
                    .h(px(28.))
                    .items_center()
                    .px(px(4.))
                    .window_control_area(WindowControlArea::Drag)
                    .child(div().flex_1())
                    // Grouped cluster: 3 controls in a tight h_flex, 2px gap
                    .child(
                        h_flex()
                            .gap(px(1.))
                            .items_center()
                            .py(px(4.))
                            // Minimize
                            .child(
                                div()
                                    .id("wc-min")
                                    .flex_none()
                                    .w(px(30.))
                                    .h(px(20.))
                                    .rounded(px(5.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .hover(|s| s.bg(hsla(0., 0., 0.5, 0.10)))
                                    .window_control_area(WindowControlArea::Min)
                                    .child(icon(AtlasIcon::Minus, 10.0, icon_dim)),
                            )
                            // Maximize / Restore
                            .child(
                                div()
                                    .id("wc-max")
                                    .flex_none()
                                    .w(px(30.))
                                    .h(px(20.))
                                    .rounded(px(5.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .hover(|s| s.bg(hsla(0., 0., 0.5, 0.10)))
                                    .window_control_area(WindowControlArea::Max)
                                    .child(icon(
                                        if is_max {
                                            AtlasIcon::Copy
                                        } else {
                                            AtlasIcon::Square
                                        },
                                        10.0,
                                        icon_dim,
                                    )),
                            )
                            // Close
                            .child(
                                div()
                                    .id("wc-close")
                                    .flex_none()
                                    .w(px(30.))
                                    .h(px(20.))
                                    .rounded(px(5.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .hover(|s| s.bg(hsla(0., 0.65, 0.42, 0.75)))
                                    .window_control_area(WindowControlArea::Close)
                                    .child(icon(AtlasIcon::X, 10.0, icon_dim)),
                            ),
                    )
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .p_2()
                    .gap_2()
                    .child(div().flex_1().h_full().min_w(px(0.)).child(center_content))
                    // FASE5: inspector panel as the third child of the central row.
                    .children(inspector),
            )
            // FASE5: overlay layers (goto bar, palette, settings, paste conflict, …).
            .children(overlays)
            // FASE C: the rubber-band selection rectangle, painted over the panes.
            .children(marquee)
            // FASE C: global rubber-band motor (invisible canvas, registers window-wide
            // mouse move/up listeners that fire even outside the pane hitbox).
            .child(rubber_layer(cx.entity()))
            .child(drag_hover_layer(cx.entity()))
            // Floating glass search pill (Figma "Window/Search Global"), bottom-center.
            .children(search_pill)
            // Operation progress toast: appears bottom-right while a transfer is active.
            .children(op_progress.map(|(op_id, text, cancel)| {
                div()
                    .absolute()
                    .bottom_4()
                    .right_4()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .bg(glass_chrome_fill())
                    .text_xs()
                    .text_color(muted)
                    .shadow_md()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().child(text))
                            .child(
                                div()
                                    .id(SharedString::from(format!("op-cancel-{op_id}")))
                                    .px_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .border_1()
                                    .border_color(status_border)
                                    .hover(|h| h.bg(status_border))
                                    .child(rust_i18n::t!("root_cancel"))
                                    .on_mouse_down(MouseButton::Left, move |_ev, _w, _cx| {
                                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                    }),
                            ),
                    )
            }))
    }
}

/// Shared Ctrl+scroll handler for every pane container. Plain scrolls are ignored
/// (the virtualized list scrolls itself); Ctrl+scroll routes to `pane_ctrl_scroll`.
pub(crate) fn ctrl_scroll(
    ev: &gpui::ScrollWheelEvent,
    pane_id: TabId,
    view: &Entity<App>,
    cx: &mut gpui::App,
) {
    if !ev.modifiers.control {
        return;
    }
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
