//! Root gpui view: the window shell and the recursive render of the pane/tab/split
//! tree (`state::layout_tree`), replacing `egui_tiles`.
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
use crate::state::layout_tree::Tile;
use crate::state::pane::TabId;
use crate::state::settings::ViewMode;
use crate::state::tab::LoadState;
use crate::ui::glass::{
    glass_border, glass_chrome_fill, glass_fill, glass_shadows_enabled, GLASS_RADIUS,
};
use crate::ui::icons::{file_type_icon, icon, AtlasIcon};
use crate::ui::search_shell::search_icon;
use crate::util::display::{display_name, format_date, format_size};
use gpui::prelude::*;
use gpui::{
    canvas, deferred, div, hsla, img, px, uniform_list, AnyElement, Context, DispatchPhase, Div,
    Entity, Focusable, Hsla, MouseButton, MouseMoveEvent, MouseUpEvent, NavigationDirection,
    RenderImage, SharedString, Stateful, Window,
};
use gpui_component::dock::DockPlacement;
use gpui_component::input::InputState;
use gpui_component::menu::{ContextMenuExt, PopupMenu};
use gpui_component::{h_flex, v_flex, ActiveTheme, ElementExt};
use std::path::PathBuf;
use std::sync::Arc;

/// A small clickable glyph in the tab toolbar, bound to an `App` mutation.
fn toolbar_btn(
    id: String,
    glyph: AtlasIcon,
    color: Hsla,
    view: Entity<App>,
    action: impl Fn(&mut App, &mut Context<App>) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(id))
        .px_1()
        .cursor_pointer()
        .child(icon(glyph, 16.0, color))
        .on_click(move |_ev, _window, cx| {
            view.update(cx, |this, cx| action(this, cx));
        })
}

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
            .child(SharedString::from(format!("{n} elemento(s)")))
    }
}

/// FASE F (Windows): placeholder drag view used instead of `DraggedFiles` when the
/// OLE drag-and-drop path (`DoDragDrop`) takes over. Returns an invisible zero-size
/// element so gpui has no ghost to paint. The OLE drag loop provides its own OS
/// cursor feedback.  Using a *distinct type* ensures no `on_drop::<DraggedFiles>`
/// fires when the synthetic `WM_LBUTTONUP` is injected to clear `cx.active_drag`.
#[cfg(windows)]
#[derive(Clone)]
struct DragNoop;

#[cfg(windows)]
impl Render for DragNoop {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// FASE G: right-click context menu for a file item. Items dispatch crate actions
/// that the root `Atlas` key context handles, operating on the current selection
/// (right-click first selects the item under the cursor).
fn file_context_menu(
    menu: PopupMenu,
    _window: &mut Window,
    _cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    menu.menu("Abrir", Box::new(crate::OpenSelected))
        .menu("Renombrar", Box::new(crate::RenameSelected))
        .separator()
        .menu("Copiar", Box::new(crate::CopySel))
        .menu("Cortar", Box::new(crate::CutSel))
        .menu("Pegar aquí", Box::new(crate::PasteHere))
        .separator()
        .menu("Mover a papelera", Box::new(crate::DeleteToTrash))
        .menu("Eliminar permanentemente", Box::new(crate::DeletePermanent))
}

/// FASE C: record a visible item's window-space rect for rubber-band hit-testing.
/// Returns an `on_prepaint` closure to spread onto the item element. The per-pane
/// `row_rects` vec is cleared at the start of each view's render.
fn rect_recorder(
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

/// FASE F: record the pane's window-space rect every frame so the OLE drop target
/// can route external file drops to the correct pane directory.  The recorded zones
/// are committed to `dnd.zones` at the start of the next render frame (one-frame lag
/// is invisible; layout is stable between frames).
///
/// On non-Windows this is a no-op closure (`push_pane_zone` has a `#[cfg(windows)]`
/// body) so the `when(cfg!(windows), …)` gate in each chain is purely for clarity.
fn zone_recorder(
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

/// A clickable path breadcrumb: each segment navigates the focused pane to that
/// ancestor directory. Replaces the old plain truncated path label.
fn breadcrumb(
    view: &Entity<App>,
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
                .id(SharedString::from(format!("crumb-{i}")))
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .text_color(fg)
                .truncate()
                .child(SharedString::from(label))
                .on_click(move |_ev, _window, cx| {
                    let target = target.clone();
                    v.update(cx, |this, cx| {
                        let tile = this.state.focused_tile;
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
                .id(SharedString::from(format!("crumb-r{i}")))
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .text_color(fg)
                .truncate()
                .child(SharedString::from(label))
                .on_click(move |_ev, _window, cx| {
                    let target = target.clone();
                    v.update(cx, |this, cx| {
                        let tile = this.state.focused_tile;
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
                .id(SharedString::from(format!("crumb-a{j}")))
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
                        let tile = this.state.focused_tile;
                        this.navigate_into_archive(tile, arc, inn);
                        cx.notify();
                    });
                }),
        );
    }

    row
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // FASE C: reset per-frame item rects; each visible row/cell re-records via
        // on_prepaint while painting this frame (used for rubber-band hit-testing).
        self.row_rects.clear();
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
        let (fg, muted, status_border) = {
            let t = cx.theme();
            (t.foreground, t.muted_foreground, t.border)
        };
        let active_path = self.state.active_tab_data().map(|t| t.location.display_path());
        let active_archive_inner = self.state.active_tab_data().and_then(|t| {
            t.location.as_archive().map(|(archive, inner)| {
                (archive.to_path_buf(), inner.to_path_buf())
            })
        });

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
                        format!(
                            "{items} elementos · {sel} seleccionados · {}",
                            format_size(bytes)
                        )
                    } else {
                        format!("{items} elementos")
                    }
                }
            },
            None => String::new(),
        };

        // Most recent background operation, rendered at the right of the status
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
                    text.push_str(&format!(" ({n_ops} operaciones)"));
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
            Some(self.render_inspector(window, cx))
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
        let modal_active = self.state.overlay.show_goto_bar
            || self.goto_closing.is_some()
            || self.state.overlay.show_palette
            || self.state.overlay.show_settings
            || self.state.overlay.transfer_conflict.is_some()
            || self.state.profile_ui.creating_tag
            || self.state.batch_rename.is_some()
            || self.rename_edit.is_some()
            || self
                .state
                .active_tab_state()
                .is_some_and(|ts| ts.search.active);
        let key_ctx = if modal_active { "AtlasModal" } else { "Atlas" };

        // FASE G: clickable breadcrumb for the focused tab's path.
        let view = cx.entity();
        let crumbs: Option<gpui::AnyElement> =
            if let Some((archive, inner)) = active_archive_inner.as_ref() {
                Some(archive_breadcrumb(&view, archive, inner, fg, muted).into_any_element())
            } else {
                active_path
                    .as_ref()
                    .map(|p| breadcrumb(&view, p, fg, muted).into_any_element())
            };
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
                let t = this.state.focused_tile;
                this.process_actions(vec![Action::EditLocation(t)]);
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
                if this.state.active_tab_data().map_or(false, |t| t.location.is_archive()) {
                    this.toast_error("Eliminar dentro de archivos comprimidos no disponible todavía".into());
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
                if this.state.active_tab_data().map_or(false, |t| t.location.is_archive()) {
                    this.toast_error("Eliminar dentro de archivos comprimidos no disponible todavía".into());
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
            .on_action(cx.listener(|this, _: &crate::CopySel, _w, cx| {
                if this.state.active_tab_data().map_or(false, |t| t.location.is_archive()) {
                    this.toast_error("Copiar dentro de archivos comprimidos no disponible todavía".into());
                    cx.notify();
                    return;
                }
                let paths = this.selected_paths();
                if !paths.is_empty() {
                    this.process_actions(vec![Action::Copy(paths)]);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::CutSel, _w, cx| {
                if this.state.active_tab_data().map_or(false, |t| t.location.is_archive()) {
                    this.toast_error("Cortar dentro de archivos comprimidos no disponible todavía".into());
                    cx.notify();
                    return;
                }
                let paths = this.selected_paths();
                if !paths.is_empty() {
                    this.process_actions(vec![Action::Cut(paths)]);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::PasteHere, _w, cx| {
                if let Some(dest) = this.state.active_tab_data().and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf())) {
                    this.process_actions(vec![Action::Paste { dest }]);
                    cx.notify();
                }
            }))
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .bg(glass_chrome_fill())
                    .child("Atlas")
                    .children(crumbs)
                    .child(toolbar_btn(
                        "vm-details".into(),
                        AtlasIcon::Details,
                        muted,
                        cx.entity(),
                        |this, cx| {
                            this.process_actions(vec![Action::SetViewMode(ViewMode::Details)]);
                            cx.notify();
                        },
                    ))
                    .child(toolbar_btn(
                        "vm-grid".into(),
                        AtlasIcon::Grid,
                        muted,
                        cx.entity(),
                        |this, cx| {
                            this.process_actions(vec![Action::SetViewMode(ViewMode::Grid)]);
                            cx.notify();
                        },
                    ))
                    .child(toolbar_btn(
                        "vm-list".into(),
                        AtlasIcon::List,
                        muted,
                        cx.entity(),
                        |this, cx| {
                            this.process_actions(vec![Action::SetViewMode(ViewMode::List)]);
                            cx.notify();
                        },
                    ))
                    .child(toolbar_btn(
                        "vm-miller".into(),
                        AtlasIcon::Columns,
                        muted,
                        cx.entity(),
                        |this, cx| {
                            this.process_actions(vec![Action::SetViewMode(ViewMode::Miller)]);
                            cx.notify();
                        },
                    )),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    // Floating gutter so the glass panes sit over the blurred wallpaper
                    // with a margin (Figma panels float, not edge-to-edge).
                    .p_2()
                    .gap_2()
                    .child(div().flex_1().h_full().min_w(px(0.)).child(center_content))
                    // FASE5: inspector panel as the third child of the central row.
                    .children(inspector),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1()
                    .bg(glass_chrome_fill())
                    .text_xs()
                    .text_color(muted)
                    .gap_2()
                    .child(div().flex_1().child(status_text))
                    .children(op_progress.map(|(op_id, text, cancel)| {
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
                                    .child("Cancelar")
                                    .on_mouse_down(MouseButton::Left, move |_ev, _w, _cx| {
                                        // The worker checks this per file and per
                                        // 1 MiB chunk; partial files are removed.
                                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                    }),
                            )
                    })),
            )
            // FASE5: overlay layers (goto bar, palette, settings, paste conflict, …).
            .children(overlays)
            // FASE C: the rubber-band selection rectangle, painted over the panes.
            .children(marquee)
            // FASE C: global rubber-band motor (invisible canvas, registers window-wide
            // mouse move/up listeners that fire even outside the pane hitbox).
            .child(rubber_layer(cx.entity()))
            // Floating glass search pill (Figma "Window/Search Global"), bottom-center.
            .children(search_pill)
    }
}

/// Global rubber-band motor: a full-size invisible canvas painted last so its
/// `window.on_mouse_event` listeners fire even when the cursor leaves the pane.
/// Handles move (threshold → started, apply), up (clear on empty click), and
/// drives `request_animation_frame` while rubber is active for continuous autoscroll.
fn rubber_layer(view: Entity<App>) -> impl gpui::IntoElement {
    let v = view.clone();
    canvas(
        |_, _, _| {},
        move |_, (), window, cx| {
            let has_rubber = v.read(cx).rubber.is_some();

            // ── Global move ──────────────────────────────────────────────────
            let vm = v.clone();
            window.on_mouse_event::<MouseMoveEvent>(move |ev, phase, _win, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let pos = ev.position;
                vm.update(cx, |this, cx| {
                    let Some(r) = this.rubber.as_mut() else {
                        return;
                    };
                    r.current = pos;
                    if !r.started {
                        let dx = f32::from(pos.x) - f32::from(r.origin.x);
                        let dy = f32::from(pos.y) - f32::from(r.origin.y);
                        r.started = dx * dx + dy * dy >= 16.0;
                    }
                    if r.started {
                        this.apply_rubber();
                        let pane = this.rubber.as_ref().map(|r| r.pane);
                        let cur = this.rubber.as_ref().map(|r| r.current);
                        if let (Some(pane), Some(cur)) = (pane, cur) {
                            let (cols, rows) = this.rubber_scroll_dims(pane);
                            this.rubber_autoscroll_rows(pane, cur, cols, rows);
                        }
                    }
                    cx.notify();
                });
            });

            // ── Global up ────────────────────────────────────────────────────
            let vu = v.clone();
            window.on_mouse_event::<MouseUpEvent>(move |ev, phase, _win, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let pos = ev.position;
                vu.update(cx, |this, cx| {
                    if let Some(r) = this.rubber.take() {
                        if !r.started && !r.additive {
                            // Plain click on empty space → clear selection.
                            // If pos hit an item rect, that item's on_click already handled it.
                            let hit = this.row_rects.get(&r.pane).is_some_and(|rects| {
                                rects.iter().any(|(_, b)| {
                                    pos.x >= b.left()
                                        && pos.x <= b.right()
                                        && pos.y >= b.top()
                                        && pos.y <= b.bottom()
                                })
                            });
                            if !hit {
                                if let Some(ts) = this.state.tab_states.get_mut(&r.pane) {
                                    ts.selection.clear();
                                }
                            }
                        }
                        cx.notify();
                    }
                });
            });

            // Keep repainting while rubber is active (continuous autoscroll at edges).
            if has_rubber {
                window.request_animation_frame();
            }
        },
    )
    .absolute()
    .inset_0()
}

impl App {
    /// Compute (cols, row_count) for autoscroll in the given pane. Uses tab_state
    /// pixel width for Grid/List; returns (1, count) for Details; (1, 0) for Miller.
    pub(crate) fn rubber_scroll_dims(&self, pane: TabId) -> (usize, usize) {
        let Some(ts) = self.state.tab_states.get(&pane) else {
            return (1, 0);
        };
        let Some(td) = self.tab_data_for(pane) else {
            return (1, 0);
        };
        let count = ts.visible.len();
        match &td.view_mode {
            crate::state::settings::ViewMode::Details => (1, count),
            crate::state::settings::ViewMode::Grid => {
                let thumb = td.thumbnail_size;
                const GAP: f32 = 8.0;
                const PAD: f32 = 8.0;
                let pane_w = ts.last_pane_width.max(thumb + 1.0);
                let inner = (pane_w - 2.0 * PAD).max(thumb);
                let cols = (((inner + GAP) / (thumb + GAP)).floor() as usize).max(1);
                (cols, count.div_ceil(cols))
            }
            crate::state::settings::ViewMode::List => {
                const CELL_W: f32 = 220.0;
                let stride = CELL_W + 6.0;
                let avail = ts.last_pane_width.max(stride);
                let cols = ((avail / stride).floor() as usize).max(1);
                (cols, count.div_ceil(cols))
            }
            crate::state::settings::ViewMode::Miller => (1, 0),
        }
    }

    fn render_floating_search_pill(&self, cx: &mut Context<Self>) -> AnyElement {
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

    /// FASE C: recompute the focused rubber-band selection by intersecting its
    /// rectangle (window coords) against the recorded visible-item rects.
    pub(crate) fn apply_rubber(&mut self) {
        let Some(r) = self.rubber.as_ref() else {
            return;
        };
        if !r.started {
            return;
        }
        let pane = r.pane;
        let (ox, oy) = (f32::from(r.origin.x), f32::from(r.origin.y));
        let (cx_, cy) = (f32::from(r.current.x), f32::from(r.current.y));
        let (rl, rr) = (ox.min(cx_), ox.max(cx_));
        let (rt, rb) = (oy.min(cy), oy.max(cy));
        let mut hits = std::collections::HashSet::new();
        let mut visible = std::collections::HashSet::new();
        if let Some(rects) = self.row_rects.get(&pane) {
            for (idx, b) in rects {
                visible.insert(*idx);
                let (bl, br) = (f32::from(b.left()), f32::from(b.right()));
                let (bt, bb) = (f32::from(b.top()), f32::from(b.bottom()));
                if rl < br && bl < rr && rt < bb && bt < rb {
                    hits.insert(*idx);
                }
            }
        }
        let Some(r) = self.rubber.as_mut() else {
            return;
        };
        r.swept.extend(hits.iter().copied());
        // Items visible but no longer under the rect get deselected; items that
        // scrolled off-screen stay in swept (can't re-check their position).
        r.swept.retain(|idx| !visible.contains(idx) || hits.contains(idx));
        let mut sel: std::collections::HashSet<usize> = if r.additive {
            r.base.clone()
        } else {
            std::collections::HashSet::new()
        };
        sel.extend(r.swept.iter().copied());
        if let Some(ts) = self.state.tab_states.get_mut(&pane) {
            ts.selection.selected = sel;
        }
    }

    /// Nudge a virtualized file view while rubber-band selecting near the visible
    /// top/bottom edge. `cols` maps file positions to uniform-list rows.
    fn rubber_autoscroll_rows(
        &mut self,
        pane: TabId,
        pos: gpui::Point<gpui::Pixels>,
        cols: usize,
        row_count: usize,
    ) {
        if row_count == 0 || self.rubber.as_ref().is_none_or(|r| r.pane != pane) {
            return;
        }
        let Some(rects) = self.row_rects.get(&pane) else {
            return;
        };
        let Some(ts) = self.state.tab_states.get(&pane) else {
            return;
        };

        let mut top = f32::INFINITY;
        let mut bottom = f32::NEG_INFINITY;
        let mut top_row = usize::MAX;
        let mut bottom_row = 0usize;

        for (idx, b) in rects {
            let Some(visible_pos) = ts.visible.iter().position(|v| v == idx) else {
                continue;
            };
            let row = visible_pos / cols.max(1);
            let item_top = f32::from(b.top());
            let item_bottom = f32::from(b.bottom());
            top = top.min(item_top);
            bottom = bottom.max(item_bottom);
            top_row = top_row.min(row);
            bottom_row = bottom_row.max(row);
        }

        if !top.is_finite() || !bottom.is_finite() {
            return;
        }

        const EDGE: f32 = 44.0;
        let y = f32::from(pos.y);
        let target = if y < top + EDGE {
            top_row.saturating_sub(1)
        } else if y > bottom - EDGE {
            (bottom_row + 1).min(row_count.saturating_sub(1))
        } else {
            return;
        };

        if let Some(handle) = self.scroll_handles.get(&pane) {
            let strategy = if target <= top_row {
                gpui::ScrollStrategy::Top
            } else {
                gpui::ScrollStrategy::Bottom
            };
            handle.scroll_to_item_strict(target, strategy);
        }
    }

    /// Begin inline rename of the focused pane's cursor item: pre-fill a native input
    /// with its current name and stash it in `rename_edit` (rendered by `render_row`).
    pub(crate) fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane = self.state.focused_tile;
        let Some(ts) = self.state.tab_states.get(&pane) else {
            return;
        };
        let idx = ts
            .selection
            .keyboard_cursor
            .or_else(|| ts.selection.selected.iter().copied().next());
        let Some(idx) = idx else { return };
        let Some(entry) = ts.entries.get(idx) else {
            return;
        };
        let path = entry.path.clone();
        let name = entry.name.clone();
        let input = crate::ui::text_input::single_line(window, cx, "", name);
        self.rename_edit = Some(crate::app::RenameEdit {
            pane,
            idx,
            path,
            input,
        });
    }

    /// Confirm the inline rename: validate the new name (cross-platform: reject path
    /// separators and the `.`/`..` specials) and dispatch the FS rename.
    pub(crate) fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.rename_edit.take() else {
            return;
        };
        let new_name = edit.input.read(cx).value().to_string();
        let new_name = new_name.trim();
        if new_name.is_empty()
            || new_name == "."
            || new_name == ".."
            || new_name.contains('/')
            || new_name.contains('\\')
        {
            return;
        }
        if let Some(parent) = edit.path.parent() {
            let to = parent.join(new_name);
            if to != edit.path {
                self.process_actions(vec![Action::Rename {
                    from: edit.path,
                    to,
                }]);
            }
        }
    }

    /// Cancel the inline rename without touching the filesystem.
    pub(crate) fn cancel_rename(&mut self) {
        self.rename_edit = None;
    }

    /// FASE C: start a rubber-band drag in `pane` at `pos` (window coords), capturing
    /// the current selection as the additive base. Shared by the pane's empty area and
    /// the Details "dead zone" column so the gesture begins the same way from either.
    pub(crate) fn begin_rubber(
        &mut self,
        pane: TabId,
        pos: gpui::Point<gpui::Pixels>,
        additive: bool,
    ) {
        let base = self
            .state
            .tab_states
            .get(&pane)
            .map(|ts| ts.selection.selected.clone())
            .unwrap_or_default();
        self.rubber = Some(crate::app::RubberDrag {
            pane,
            origin: pos,
            current: pos,
            base,
            swept: std::collections::HashSet::new(),
            additive,
            started: false,
        });
    }

    /// FASE C: the visible selection rectangle for the active drag, in window coords
    /// (same space as `row_rects`), painted as a deferred layer over the panes. Hidden
    /// until the drag actually moves, so a plain click doesn't flash a box.
    fn render_rubber_marquee(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
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

    fn render_search_in_place_bar(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let active = self
            .state
            .tab_states
            .get(&pane_id)
            .is_some_and(|ts| ts.search.active);
        if !active {
            return None;
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
                crate::ui::text_input::single_line(window, cx, "Buscar en esta carpeta", initial)
            })
            .clone();

        let value = input.read(cx).value().to_string();
        let mut needs_refresh = false;
        if let Some(ts) = self.state.tab_states.get_mut(&pane_id) {
            if ts.search.focus {
                let handle = input.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
                ts.search.focus = false;
            }
            if value != ts.search.query {
                ts.search.query.clone_from(&value);
                needs_refresh = true;
            }
        }
        if needs_refresh {
            self.refresh_search_in_place(pane_id);
        }

        let (recursive, status, count, error) = self
            .state
            .tab_states
            .get_mut(&pane_id)
            .map(|ts| {
                (
                    ts.search.recursive,
                    ts.search.status,
                    ts.visible_indices().len(),
                    ts.search.error.clone(),
                )
            })
            .unwrap_or_default();
        let (border, muted, accent, accent_fg, secondary) = {
            let t = cx.theme();
            (
                t.border,
                t.muted_foreground,
                t.accent,
                t.accent_foreground,
                t.secondary,
            )
        };
        let view = cx.entity();
        let status_text = if let Some(error) = error {
            error
        } else if status == crate::state::tab::SearchStatus::Searching {
            "Buscando...".to_string()
        } else if value.trim().is_empty() {
            "Escribe para filtrar".to_string()
        } else {
            format!("{count} resultados")
        };

        let toggle_view = view.clone();
        let close_view = view;
        let key_input = input.clone();
        Some(
            h_flex()
                .id(SharedString::from(format!("search-in-place-{}", pane_id.0)))
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .items_center()
                .border_b_1()
                .border_color(border)
                .bg(secondary)
                .capture_key_down(
                    cx.listener(move |this, ev: &gpui::KeyDownEvent, window, cx| {
                        let handled = match ev.keystroke.key.as_str() {
                            "escape" => {
                                let current = key_input.read(cx).value().to_string();
                                if current.is_empty() {
                                    this.process_actions(vec![Action::CloseSearchInPlace]);
                                } else {
                                    key_input
                                        .update(cx, |state, cx| state.set_value("", window, cx));
                                    if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                                        ts.search.query.clear();
                                    }
                                    this.refresh_search_in_place(pane_id);
                                }
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
                .child(div().text_color(muted).child("Buscar"))
                .child(div().flex_1().child(crate::ui::text_input::flat(&input)))
                .child(
                    div()
                        .px_2()
                        .py(px(2.))
                        .rounded_md()
                        .border_1()
                        .border_color(if recursive { accent } else { border })
                        .when(recursive, |s| s.bg(accent).text_color(accent_fg))
                        .when(!recursive, |s| s.text_color(muted))
                        .cursor_pointer()
                        .text_xs()
                        .child("Subfolders")
                        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                            toggle_view.update(cx, |this, cx| {
                                this.process_actions(vec![Action::ToggleSearchInSubfolders]);
                                cx.notify();
                            });
                        }),
                )
                .child(div().text_xs().text_color(muted).child(status_text))
                .child(
                    div()
                        .px_2()
                        .py(px(2.))
                        .rounded_md()
                        .cursor_pointer()
                        .text_color(muted)
                        .child("x")
                        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                            close_view.update(cx, |this, cx| {
                                this.process_actions(vec![Action::CloseSearchInPlace]);
                                cx.notify();
                            });
                        }),
                )
                .into_any_element(),
        )
    }

    fn open_search_in_place_seeded(
        &mut self,
        pane_id: TabId,
        seed: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.focused_tile = pane_id;
        let input = self
            .search_inputs
            .entry(pane_id)
            .or_insert_with(|| crate::ui::text_input::single_line(window, cx, "", ""))
            .clone();
        input.update(cx, |state, cx| state.set_value(seed.clone(), window, cx));
        if let Some(ts) = self.state.tab_states.get_mut(&pane_id) {
            ts.search.active = true;
            ts.search.focus = true;
            ts.search.query = seed;
        }
        self.refresh_search_in_place(pane_id);
    }

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

        let search_bar = self.render_search_in_place_bar(pane_id, window, cx);
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
            .capture_key_down(
                cx.listener(move |this, ev: &gpui::KeyDownEvent, window, cx| {
                    if let Some(seed) = search_seed_from_key(ev) {
                        let already_active = this
                            .state
                            .tab_states
                            .get(&pane_id)
                            .is_some_and(|ts| ts.search.active);
                        if !already_active {
                            this.open_search_in_place_seeded(pane_id, seed, window, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }
                    }
                }),
            )
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
            .children(search_bar)
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
            (
                t.accent,
                t.accent_foreground,
                t.list_active,
                t.muted_foreground,
            )
        };
        let thumb = self
            .tab_data_for(pane_id)
            .map_or(128.0, |t| t.thumbnail_size);
        let count = self
            .state
            .tab_states
            .get_mut(&pane_id)
            .map_or(0, |ts| ts.visible_indices().len());
        // Grid layout constants matching the row's gap_2 / px_2 styling.
        const GRID_GAP: f32 = 8.0; // gap_2 between cells
        const GRID_PAD: f32 = 8.0; // px_2 on each side of the row
                                   // Use the width captured by on_prepaint from the previous frame — this
                                   // reflects the real pane size and adapts to preview panels and splitter
                                   // drags. Fall back to a viewport estimate on the very first frame before
                                   // any paint has run.
        let pane_w = self
            .state
            .tab_states
            .get(&pane_id)
            .map(|ts| ts.last_pane_width)
            .filter(|&w| w > 1.0)
            .unwrap_or_else(|| (f32::from(window.viewport_size().width) - 217.0).max(0.0));
        let inner_avail = (pane_w - 2.0 * GRID_PAD).max(thumb);
        // n*(thumb+GAP) - GAP = inner_avail  →  n = ⌊(inner_avail + GAP) / (thumb + GAP)⌋
        let cols = (((inner_avail + GRID_GAP) / (thumb + GRID_GAP)).floor() as usize).max(1);
        // Distribute remaining space so every cell fills exactly its column width.
        let fill_w = ((inner_avail - (cols - 1) as f32 * GRID_GAP) / cols as f32).max(thumb);
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
                    // Phase 1: collect visible paths still missing a thumbnail. Native
                    // images decode in-process; other types (video/3D/psd) go through an
                    // extension thumbnailer if one accepts the extension.
                    let (to_request, to_request_ext): (Vec<PathBuf>, Vec<(PathBuf, String)>) = {
                        let app = list_view.read(cx);
                        let Some(ts) = app.state.tab_states.get(&pane_id) else {
                            return Vec::new();
                        };
                        let mut imgs = Vec::new();
                        let mut exts: Vec<(PathBuf, String)> = Vec::new();
                        for row in range.clone() {
                            for c in 0..cols {
                                let vi = row * cols + c;
                                let Some(&ei) = ts.visible.get(vi) else { break };
                                let Some(entry) = ts.entries.get(ei) else {
                                    continue;
                                };
                                if app.thumb_textures.contains_key(&entry.path) {
                                    continue;
                                }
                                if is_thumbnailable(entry) {
                                    imgs.push(entry.path.clone());
                                } else if entry.kind == FileKind::File {
                                    if let Some(ext) = entry.extension.as_deref() {
                                        if let Some(cmd) = app.ext_registry.thumbnailer_for(ext) {
                                            exts.push((
                                                entry.path.clone(),
                                                cmd.command.run.clone(),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        (imgs, exts)
                    };
                    // Phase 2: enqueue off-thread decodes / thumbnailer runs.
                    if !to_request.is_empty() || !to_request_ext.is_empty() {
                        list_view.update(cx, |this, _| {
                            for p in to_request {
                                this.request_thumbnail(p);
                            }
                            for (p, run) in to_request_ext {
                                this.request_ext_thumbnail(p, run);
                            }
                        });
                    }
                    // Phase 3: build the rows, painting ready thumbnails.
                    let app = list_view.read(cx);
                    let Some(ts) = app.state.tab_states.get(&pane_id) else {
                        return Vec::new();
                    };
                    let record_rects = app.rubber.as_ref().is_some_and(|r| r.pane == pane_id);
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
                                let thumb_tex = app.grid_thumbnail(&entry.path);
                                let icon_tex =
                                    app.icon_tex_for_entry(&entry.kind, entry.extension.as_deref());
                                cells.push(render_cell(
                                    entry,
                                    entry_idx,
                                    selected,
                                    pane_id,
                                    &list_view,
                                    thumb,
                                    fill_w,
                                    thumb_tex,
                                    icon_tex,
                                    accent,
                                    accent_fg,
                                    hover,
                                    record_rects,
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

        let search_bar = self.render_search_in_place_bar(pane_id, window, cx);
        let focus_view = view.clone();
        let rubber_view = view.clone();
        let prepaint_view = view.clone();
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
            .capture_key_down(
                cx.listener(move |this, ev: &gpui::KeyDownEvent, window, cx| {
                    if let Some(seed) = search_seed_from_key(ev) {
                        let already_active = this
                            .state
                            .tab_states
                            .get(&pane_id)
                            .is_some_and(|ts| ts.search.active);
                        if !already_active {
                            this.open_search_in_place_seeded(pane_id, seed, window, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }
                    }
                }),
            )
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
            // Capture the real pane pixel width every frame so the next frame's
            // column calculation uses the actual container size (handles preview
            // panel open/close and splitter drags, not just window resize).
            .on_prepaint(move |bounds: gpui::Bounds<gpui::Pixels>, _window, cx| {
                let new_w = f32::from(bounds.size.width);
                prepaint_view.update(cx, |this, cx| {
                    if let Some(ts) = this.state.tab_states.get_mut(&pane_id) {
                        if (ts.last_pane_width - new_w).abs() > 0.5 {
                            ts.last_pane_width = new_w;
                            cx.notify();
                        }
                    }
                    // FASE F: register this pane's bounds for OLE drop routing.
                    this.push_pane_zone(pane_id, bounds);
                });
            })
            .children(search_bar)
            .child(div().flex_1().min_h(px(0.)).child(inner))
            .into_any_element()
    }

    /// Compact List view: small `icon + name` cells flowing row-major into as many
    /// columns as fit the width — the classic Windows "List" layout. Virtualized by
    /// row (`uniform_list`) like the grid, so huge dirs stay smooth.
    fn render_list_compact(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let focused = self.state.focused_tile == pane_id;
        let (accent, accent_fg, hover, muted) = {
            let t = cx.theme();
            (
                t.accent,
                t.accent_foreground,
                t.list_active,
                t.muted_foreground,
            )
        };
        let count = self
            .state
            .tab_states
            .get_mut(&pane_id)
            .map_or(0, |ts| ts.visible_indices().len());
        #[allow(clippy::items_after_statements)] // local constant kept next to its use
        const CELL_W: f32 = 220.0;
        let cell_stride = CELL_W + 6.0;
        let avail = (f32::from(window.viewport_size().width) - 217.0).max(cell_stride);
        let cols = ((avail / cell_stride).floor() as usize).max(1);
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
                SharedString::from(format!("listc-{}", pane_id.0)),
                row_count,
                move |range, _window, cx| {
                    let app = list_view.read(cx);
                    let Some(ts) = app.state.tab_states.get(&pane_id) else {
                        return Vec::new();
                    };
                    let record_rects = app.rubber.as_ref().is_some_and(|r| r.pane == pane_id);
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
                                let icon_tex =
                                    app.icon_tex_for_entry(&entry.kind, entry.extension.as_deref());
                                cells.push(render_cell_compact(
                                    entry,
                                    entry_idx,
                                    selected,
                                    pane_id,
                                    &list_view,
                                    accent,
                                    accent_fg,
                                    hover,
                                    record_rects,
                                    icon_tex,
                                ));
                            }
                            h_flex().w_full().gap(px(6.)).px_2().children(cells)
                        })
                        .collect::<Vec<_>>()
                },
            )
            .track_scroll(&scroll)
            .h_full()
            .into_any_element()
        };

        let search_bar = self.render_search_in_place_bar(pane_id, window, cx);
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
            .capture_key_down(
                cx.listener(move |this, ev: &gpui::KeyDownEvent, window, cx| {
                    if let Some(seed) = search_seed_from_key(ev) {
                        let already_active = this
                            .state
                            .tab_states
                            .get(&pane_id)
                            .is_some_and(|ts| ts.search.active);
                        if !already_active {
                            this.open_search_in_place_seeded(pane_id, seed, window, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }
                    }
                }),
            )
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
            .children(search_bar)
            .child(div().flex_1().min_h(px(0.)).child(inner))
            .into_any_element()
    }

    /// Miller (columns) view: a horizontal row of directory columns. Selecting a
    /// folder in a column opens its contents in the next column to the right. The
    /// cascade lives in `TabState.miller` (loaded via `BgResult::MillerListed`); this
    /// only paints it and routes clicks through the `Miller*` actions.
    fn render_miller(
        &mut self,
        pane_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Lazily seed the first column from the pane path on entry.
        let need_init = self
            .state
            .tab_states
            .get(&pane_id)
            .is_none_or(|ts| ts.miller.columns.is_empty());
        if need_init {
            if let Some(loc) = self.tab_data_for(pane_id).map(|t| t.location.clone()) {
                self.process_actions(vec![Action::MillerInit {
                    tile: pane_id,
                    location: loc,
                }]);
            }
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

        // Snapshot each column's rows up-front; Miller dirs are browsed one level at a
        // time, so a column is rarely huge — a scrollable v_flex per column is enough.
        let mut columns: Vec<AnyElement> = Vec::new();
        if let Some(ts) = self.state.tab_states.get(&pane_id) {
            for (ci, col) in ts.miller.columns.iter().enumerate() {
                let title = display_name(&col.location.display_path());
                let body: AnyElement = if col.entries.is_empty() {
                    let msg = match col.load_state {
                        LoadState::Loading => "Cargando…",
                        LoadState::Error(_) => "Error",
                        _ => "Vacío",
                    };
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_color(muted)
                        .child(msg)
                        .into_any_element()
                } else {
                    let count = col.entries.len();
                    let list_view = view.clone();
                    uniform_list(
                        SharedString::from(format!("mcol-{}-{}", pane_id.0, ci)),
                        count,
                        move |range, _window, cx| {
                            let app = list_view.read(cx);
                            let Some(col) = app
                                .state
                                .tab_states
                                .get(&pane_id)
                                .and_then(|ts| ts.miller.columns.get(ci))
                            else {
                                return Vec::new();
                            };
                            range
                                .filter_map(|i| {
                                    let entry = col.entries.get(i)?;
                                    let selected = col.selection.selected.contains(&i);
                                    let icon_tex = app.icon_tex_for_entry(
                                        &entry.kind,
                                        entry.extension.as_deref(),
                                    );
                                    Some(render_miller_row(
                                        entry, ci, i, selected, pane_id, &list_view, accent,
                                        accent_fg, hover, icon_tex,
                                    ))
                                })
                                .collect::<Vec<_>>()
                        },
                    )
                    .flex_1()
                    .h_full()
                    .min_h(px(0.))
                    .into_any_element()
                };
                columns.push(
                    v_flex()
                        .w(px(240.))
                        .h_full()
                        .border_r_1()
                        .border_color(header_bg)
                        .child(
                            div()
                                .w_full()
                                .px_2()
                                .py_1()
                                .bg(header_bg)
                                .text_xs()
                                .text_color(muted)
                                .truncate()
                                .child(title),
                        )
                        .child(body)
                        .into_any_element(),
                );
            }
        }

        let focus_view = view.clone();
        let rubber_view = view.clone();
        let zone_view = view.clone();
        let scroll_view = view;
        v_flex()
            .id(SharedString::from(format!("pane-{}", pane_id.0)))
            .size_full()
            .border_2()
            .border_color(if focused {
                accent
            } else {
                hsla(0., 0., 0., 0.)
            })
            .on_mouse_down(MouseButton::Left, {
                let v = rubber_view.clone();
                move |ev, _window, cx| {
                    let (pos, additive) =
                        (ev.position, ev.modifiers.control || ev.modifiers.platform);
                    v.update(cx, |this, _| this.begin_rubber(pane_id, pos, additive));
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
            .child(
                h_flex()
                    .id(SharedString::from(format!("miller-{}", pane_id.0)))
                    .size_full()
                    .overflow_x_scroll()
                    .children(columns),
            )
            .into_any_element()
    }

    /// Ctrl+scroll on a pane: in Grid it zooms the thumbnail size; in every other
    /// view it cycles to the next/previous view mode.
    fn pane_ctrl_scroll(&mut self, pane_id: TabId, dy: f32) {
        let mode = self
            .tab_data_for(pane_id)
            .map(|t| t.view_mode.clone())
            .unwrap_or_default();
        if matches!(mode, ViewMode::Grid) {
            if let Some(Tile::Pane(p)) = self.state.pane_tree.tiles.get_mut(pane_id) {
                let step = if dy > 0.0 { 16.0 } else { -16.0 };
                p.thumbnail_size = (p.thumbnail_size + step).clamp(64.0, 384.0);
            }
            return;
        }
        #[allow(clippy::items_after_statements)] // local constant kept next to its use
        const ORDER: [ViewMode; 4] = [
            ViewMode::Details,
            ViewMode::List,
            ViewMode::Grid,
            ViewMode::Miller,
        ];
        let cur = ORDER.iter().position(|m| *m == mode).unwrap_or(0);
        let next = if dy > 0.0 {
            (cur + 1) % ORDER.len()
        } else {
            (cur + ORDER.len() - 1) % ORDER.len()
        };
        let next_mode = ORDER[next].clone();
        self.state.focused_tile = pane_id;
        if let Some(Tile::Pane(p)) = self.state.pane_tree.tiles.get_mut(pane_id) {
            p.view_mode = next_mode.clone();
        }
        // Leaving Miller can leave the canonical entries empty; reload for the others.
        if next_mode != ViewMode::Miller {
            let needs_load = self
                .state
                .tab_states
                .get(&pane_id)
                .is_some_and(|ts| ts.entries.is_empty());
            if needs_load {
                if let Some(td) = self.tab_data_for(pane_id) {
                    let loc = td.location.clone();
                    let col = td.sort_col.clone();
                    let dir = td.sort_dir.clone();
                    self.dispatch_list(pane_id, loc, col, dir);
                }
            }
        }
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
            move |_ev, _window, cx| {
                rv.update(cx, |this, cx| {
                    this.process_actions(vec![Action::Select {
                        tile: pane_id,
                        idx: entry_idx,
                        shift: false,
                        ctrl: false,
                    }]);
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
                {
                    let drag_view = view.clone();
                    let anchor = entry.path.clone();
                    move |_d, _pos, _window, cx| {
                        // Full multi-selection at drag-start time (fix: was single entry).
                        let (paths, archive_items) = {
                            let app = drag_view.read(&*cx);
                            let mut arc = app.state.selected_archive_items();
                            // Fallback: single-entry drag when nothing is in the selection set.
                            if let Some(ref mut items) = arc {
                                if items.is_empty() {
                                    if let Some(archive) =
                                        app.state.active_tab_data().and_then(|t| t.location.as_archive().map(|(a, _)| a.to_path_buf()))
                                    {
                                        items.push((archive, anchor.clone()));
                                    }
                                }
                            }
                            let p = app.state.selected_paths();
                            let p = if p.is_empty() {
                                vec![anchor.clone()]
                            } else {
                                p
                            };
                            (p, arc)
                        };
                        // Windows: schedule DoDragDrop via PostMessage so it runs
                        // AFTER this closure returns and gpui's App RefCell is
                        // no longer borrowed (prevents "RefCell already borrowed"
                        // panic from DoDragDrop's nested Win32 message loop).
                        #[cfg(windows)]
                        {
                            drag_view
                                .read(&*cx)
                                .trigger_os_drag(paths.clone(), archive_items);
                            // Return an invisible zero-size placeholder. OLE provides
                            // cursor feedback; a synthetic WM_LBUTTONUP injected after
                            // DoDragDrop clears gpui's active_drag without firing any
                            // on_drop handler (DragNoop has no matching handlers).
                            return cx.new(|_| DragNoop);
                        }
                        // Non-Windows: OLE path absent; gpui renders the ghost itself.
                        #[cfg(not(windows))]
                        {
                            let _ = archive_items;
                        }
                        #[cfg(not(windows))]
                        cx.new(|_| DraggedFiles { paths })
                    }
                },
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
        .context_menu(file_context_menu)
        .into_any_element()
}

/// Extensions worth decoding into a grid thumbnail.
fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff"
    )
}

/// Whether this entry should get a downscaled thumbnail (an image file).
fn is_thumbnailable(entry: &FileEntry) -> bool {
    matches!(entry.kind, FileKind::File) && entry.extension.as_deref().is_some_and(is_image_ext)
}

/// One grid cell: thumbnail (or a scaled glyph fallback while it loads / for
/// non-images) + truncated name, fixed to the thumbnail size. The texture is a
/// cached, downscaled `RenderImage` (see `ui::thumbs`) — never a full-res decode.
#[allow(clippy::too_many_arguments)]
fn render_cell(
    entry: &FileEntry,
    entry_idx: usize,
    selected: bool,
    pane_id: TabId,
    view: &Entity<App>,
    thumb: f32,
    cell_w: f32,
    thumb_tex: Option<Arc<RenderImage>>,
    icon_tex: Option<Arc<RenderImage>>,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    record_rects: bool,
) -> AnyElement {
    let name = entry.name.clone();
    let v = view.clone();
    v_flex()
        .id(SharedString::from(format!(
            "cell-{}-{}",
            pane_id.0, entry_idx
        )))
        .when(record_rects, |s| {
            s.on_prepaint(rect_recorder(pane_id, entry_idx, view))
        })
        .w(px(cell_w))
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
            match thumb_tex {
                Some(tex) => area.child(
                    img(tex)
                        .max_h(px(thumb * 0.6))
                        .max_w(px(thumb))
                        .rounded_sm(),
                ),
                None => match icon_tex {
                    Some(tex) => area.child(
                        img(tex)
                            .max_h(px(thumb * 0.56))
                            .max_w(px(thumb * 0.56))
                            .flex_none(),
                    ),
                    None => area.child(file_type_icon(
                        &entry.kind,
                        entry.extension.as_deref(),
                        thumb * 0.42,
                    )),
                },
            }
        })
        .child(
            div()
                .w_full()
                .text_center()
                .text_xs()
                .truncate()
                .child(name),
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
        .on_mouse_down(MouseButton::Right, {
            let rv = view.clone();
            move |_ev, _window, cx| {
                rv.update(cx, |this, cx| {
                    this.process_actions(vec![Action::Select {
                        tile: pane_id,
                        idx: entry_idx,
                        shift: false,
                        ctrl: false,
                    }]);
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
                {
                    let drag_view = view.clone();
                    let anchor = entry.path.clone();
                    move |_d, _pos, _window, cx| {
                        // Full multi-selection at drag-start time (fix: was single entry).
                        let (paths, archive_items) = {
                            let app = drag_view.read(&*cx);
                            let mut arc = app.state.selected_archive_items();
                            // Fallback: single-entry drag when nothing is in the selection set.
                            if let Some(ref mut items) = arc {
                                if items.is_empty() {
                                    if let Some(archive) =
                                        app.state.active_tab_data().and_then(|t| t.location.as_archive().map(|(a, _)| a.to_path_buf()))
                                    {
                                        items.push((archive, anchor.clone()));
                                    }
                                }
                            }
                            let p = app.state.selected_paths();
                            let p = if p.is_empty() {
                                vec![anchor.clone()]
                            } else {
                                p
                            };
                            (p, arc)
                        };
                        // Windows: schedule DoDragDrop via PostMessage so it runs
                        // AFTER this closure returns and gpui's App RefCell is
                        // no longer borrowed (prevents "RefCell already borrowed"
                        // panic from DoDragDrop's nested Win32 message loop).
                        #[cfg(windows)]
                        {
                            drag_view
                                .read(&*cx)
                                .trigger_os_drag(paths.clone(), archive_items);
                            // Return an invisible zero-size placeholder. OLE provides
                            // cursor feedback; a synthetic WM_LBUTTONUP injected after
                            // DoDragDrop clears gpui's active_drag without firing any
                            // on_drop handler (DragNoop has no matching handlers).
                            return cx.new(|_| DragNoop);
                        }
                        // Non-Windows: OLE path absent; gpui renders the ghost itself.
                        #[cfg(not(windows))]
                        {
                            let _ = archive_items;
                        }
                        #[cfg(not(windows))]
                        cx.new(|_| DraggedFiles { paths })
                    }
                },
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
        .context_menu(file_context_menu)
        .into_any_element()
}

/// Shared Ctrl+scroll handler for every pane container. Plain scrolls are ignored
/// (the virtualized list scrolls itself); Ctrl+scroll routes to `pane_ctrl_scroll`.
fn ctrl_scroll(
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

/// One compact List cell: 16px icon + truncated name in a fixed 220px row. Click
/// routing mirrors the Details rows (shift = range, ctrl = toggle, dbl = open).
#[allow(clippy::too_many_arguments)]
fn render_cell_compact(
    entry: &FileEntry,
    entry_idx: usize,
    selected: bool,
    pane_id: TabId,
    view: &Entity<App>,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    record_rects: bool,
    icon_tex: Option<Arc<RenderImage>>,
) -> AnyElement {
    let name = entry.name.clone();
    let v = view.clone();
    h_flex()
        .id(SharedString::from(format!(
            "lc-{}-{}",
            pane_id.0, entry_idx
        )))
        .when(record_rects, |s| {
            s.on_prepaint(rect_recorder(pane_id, entry_idx, view))
        })
        .w(px(220.))
        .px_1()
        .py(px(2.))
        .gap_1()
        .items_center()
        .rounded_sm()
        .cursor_pointer()
        .when(selected, |s| s.bg(accent).text_color(accent_fg))
        .when(!selected, |s| s.hover(|h| h.bg(hover)))
        .child(
            div().w(px(16.)).flex_none().child(match icon_tex {
                Some(tex) => img(tex)
                    .w(px(14.))
                    .h(px(14.))
                    .flex_none()
                    .into_any_element(),
                None => file_type_icon(&entry.kind, entry.extension.as_deref(), 14.0),
            }),
        )
        .child(div().flex_1().min_w(px(0.)).truncate().child(name))
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
            move |_ev, _window, cx| {
                rv.update(cx, |this, cx| {
                    this.process_actions(vec![Action::Select {
                        tile: pane_id,
                        idx: entry_idx,
                        shift: false,
                        ctrl: false,
                    }]);
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
                {
                    let drag_view = view.clone();
                    let anchor = entry.path.clone();
                    move |_d, _pos, _window, cx| {
                        // Full multi-selection at drag-start time (fix: was single entry).
                        let (paths, archive_items) = {
                            let app = drag_view.read(&*cx);
                            let mut arc = app.state.selected_archive_items();
                            // Fallback: single-entry drag when nothing is in the selection set.
                            if let Some(ref mut items) = arc {
                                if items.is_empty() {
                                    if let Some(archive) =
                                        app.state.active_tab_data().and_then(|t| t.location.as_archive().map(|(a, _)| a.to_path_buf()))
                                    {
                                        items.push((archive, anchor.clone()));
                                    }
                                }
                            }
                            let p = app.state.selected_paths();
                            let p = if p.is_empty() {
                                vec![anchor.clone()]
                            } else {
                                p
                            };
                            (p, arc)
                        };
                        // Windows: schedule DoDragDrop via PostMessage so it runs
                        // AFTER this closure returns and gpui's App RefCell is
                        // no longer borrowed (prevents "RefCell already borrowed"
                        // panic from DoDragDrop's nested Win32 message loop).
                        #[cfg(windows)]
                        {
                            drag_view
                                .read(&*cx)
                                .trigger_os_drag(paths.clone(), archive_items);
                            // Return an invisible zero-size placeholder. OLE provides
                            // cursor feedback; a synthetic WM_LBUTTONUP injected after
                            // DoDragDrop clears gpui's active_drag without firing any
                            // on_drop handler (DragNoop has no matching handlers).
                            return cx.new(|_| DragNoop);
                        }
                        // Non-Windows: OLE path absent; gpui renders the ghost itself.
                        #[cfg(not(windows))]
                        {
                            let _ = archive_items;
                        }
                        #[cfg(not(windows))]
                        cx.new(|_| DraggedFiles { paths })
                    }
                },
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
        .context_menu(file_context_menu)
        .into_any_element()
}

/// One row in a Miller column. Single click selects (cascading the next column);
/// double click on a file opens it. Routed through the `Miller*` actions.
#[allow(clippy::too_many_arguments)]
fn render_miller_row(
    entry: &FileEntry,
    column: usize,
    idx: usize,
    selected: bool,
    pane_id: TabId,
    view: &Entity<App>,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    icon_tex: Option<Arc<RenderImage>>,
) -> AnyElement {
    let is_dir = matches!(entry.kind, FileKind::Dir);
    let trailing = if is_dir {
        icon(
            AtlasIcon::CaretRight,
            12.0,
            if selected { accent_fg } else { accent },
        )
    } else {
        div().w(px(12.)).flex_none().into_any_element()
    };
    let name = entry.name.clone();
    let v = view.clone();
    h_flex()
        .id(SharedString::from(format!(
            "mrow-{}-{}-{}",
            pane_id.0, column, idx
        )))
        .w_full()
        .px_2()
        .py(px(3.))
        .gap_2()
        .items_center()
        .cursor_pointer()
        .when(selected, |s| s.bg(accent).text_color(accent_fg))
        .when(!selected, |s| s.hover(|h| h.bg(hover)))
        .child(
            div().w(px(16.)).flex_none().child(match icon_tex {
                Some(tex) => img(tex)
                    .w(px(14.))
                    .h(px(14.))
                    .flex_none()
                    .into_any_element(),
                None => file_type_icon(&entry.kind, entry.extension.as_deref(), 14.0),
            }),
        )
        .child(div().flex_1().min_w(px(0.)).truncate().child(name))
        .child(trailing)
        .on_click(move |ev, _window, cx| {
            let activate = ev.click_count() >= 2;
            v.update(cx, |this, cx| {
                if activate {
                    this.process_actions(vec![Action::MillerActivate {
                        tile: pane_id,
                        column,
                        idx,
                    }]);
                } else {
                    this.process_actions(vec![Action::MillerSelect {
                        tile: pane_id,
                        column,
                        idx,
                    }]);
                }
                cx.notify();
            });
        })
        .into_any_element()
}

fn search_seed_from_key(ev: &gpui::KeyDownEvent) -> Option<String> {
    if ev.keystroke.modifiers.control
        || ev.keystroke.modifiers.alt
        || ev.keystroke.modifiers.platform
    {
        return None;
    }
    let key = ev.keystroke.key.as_str();
    if key == "space" || key.chars().count() != 1 {
        return None;
    }
    Some(key.to_string())
}
