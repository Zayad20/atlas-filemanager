//! File-view renderers split out of `root.rs` by view mode.
use crate::app::App;
use crate::input::action::Action;
use crate::ui::drag_hover::{
    drag_hover_recorder_with_drop, DragHoverCommand, DragHoverDropCommand, DragHoverId,
    DragHoverKind,
};
use crate::ui::root::{ctrl_scroll, DraggedFiles};
use atlas_model::layout_tree::Tile;
use atlas_model::location::Location;
use atlas_model::pane::TabId;
use atlas_model::tab::LoadState;
use gpui::prelude::*;
use gpui::{ClickEvent, Div, Entity, MouseButton, MouseDownEvent, Stateful, Window};
use gpui_component::ElementExt;
use std::path::PathBuf;

pub mod crumb_strip_element;
pub mod details;
pub mod details_element;
pub mod details_header_element;
pub mod details_ring;
pub mod grid;
pub mod grid_surface_element;
pub mod list;
pub mod miller;
mod rename;
pub mod space_lens;
pub mod tabs_bar_element;
pub mod virtual_grid;

/// Shared entry click handler: double-click activates, single click selects
/// (shift/ctrl aware), and an in-flight rename is committed first. Used by the
/// grid/list/details views (miller has its own column-based actions).
pub(super) fn on_entry_click(
    view: &Entity<App>,
    pane_id: TabId,
    entry_idx: usize,
) -> impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static {
    let v = view.clone();
    move |ev, _window, cx| {
        let m = ev.modifiers();
        let (shift, ctrl) = (m.shift, m.control || m.platform);
        let activate = ev.click_count() >= 2;
        v.update(cx, |this, cx| {
            if this.rename_edit.is_some() {
                this.commit_rename(cx);
            }
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
    }
}

pub(super) fn on_pane_rubber_mouse_down(
    view: &Entity<App>,
    pane_id: TabId,
) -> impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static {
    let rubber_view = view.clone();
    move |ev, _window, cx| {
        let (pos, additive) = (
            ev.position,
            ev.modifiers.control || ev.modifiers.platform || ev.modifiers.shift,
        );
        rubber_view.update(cx, |this, cx| {
            if this.rename_edit.is_some() {
                this.commit_rename(cx);
            }
            this.begin_rubber(pane_id, pos, additive);
            cx.notify();
        });
        cx.stop_propagation();
    }
}

/// Shared right-click handler: select the target (keeping multi-selection when
/// clicked inside it) and open the shell context menu at the cursor.
pub(super) fn on_entry_ctx_menu(
    view: &Entity<App>,
    pane_id: TabId,
    entry_idx: usize,
) -> impl Fn(&gpui::MouseDownEvent, &mut Window, &mut gpui::App) + 'static {
    let v = view.clone();
    move |ev, window, cx| {
        let pos = ev.position;
        v.update(cx, |this, cx| {
            this.activate_tab(pane_id);
            let paths = this.select_context_menu_paths(pane_id, entry_idx);
            this.open_ctx_menu(paths, pos, window, cx);
            cx.notify();
        });
        cx.stop_propagation();
    }
}

/// Shared folder drop target: dim on drag-over, record the hover zone for
/// spring-loaded navigation, and accept both intra-Atlas and external drops
/// as a transfer into `dest`. Apply behind a `.when(is_dir, …)` at the call site.
pub(super) fn folder_drop_target(
    el: Stateful<Div>,
    view: &Entity<App>,
    pane_id: TabId,
    dest: PathBuf,
) -> Stateful<Div> {
    let hover_id = DragHoverId::Folder(pane_id, dest.clone());
    let dest_internal = dest.clone();
    let dest_external = dest.clone();
    let mv = view.clone();
    let cpv = view.clone();
    el.drag_over::<DraggedFiles>(|s, _, _, _| s.opacity(0.6))
        .on_prepaint_child(drag_hover_recorder_with_drop(
            hover_id,
            DragHoverKind::Folder,
            DragHoverCommand::NavigatePathIn(pane_id, dest.clone()),
            DragHoverDropCommand::TransferTo(dest),
            view,
        ))
        .on_drop::<DraggedFiles>(move |d, _window, cx| {
            let paths = d.paths.clone();
            mv.update(cx, |this, cx| {
                this.execute_drag_hover_drop_command(
                    this.folder_drop_command(pane_id, &dest_internal),
                    paths,
                    true,
                );
                cx.notify();
            });
        })
        .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
            let paths = paths.paths().to_vec();
            cpv.update(cx, |this, cx| {
                this.execute_drag_hover_drop_command(
                    this.folder_drop_command(pane_id, &dest_external),
                    paths,
                    false,
                );
                cx.notify();
            });
        })
}

/// Shared empty-space / background interactions for a pane container, applied
/// identically by every view renderer (grid, list, details, miller):
/// - Left mousedown: commit any in-flight rename, start rubber-band selection
///   (no-op in Miller; a plain click with no drag clears the selection).
/// - Right mousedown: background context menu for filesystem locations.
/// - Drops on empty space: routed through `drop_files_on_pane` (which handles
///   Directory/Tag/RecycleBin/Archive/Remote semantics centrally).
/// - Click: focus the pane. Scroll: ctrl-wheel zoom.
pub(super) fn pane_background_interactions(
    el: Stateful<Div>,
    view: &Entity<App>,
    pane_id: TabId,
) -> Stateful<Div> {
    let menu_view = view.clone();
    let drop_view = view.clone();
    let drop_ext_view = view.clone();
    let focus_view = view.clone();
    let scroll_view = view.clone();
    el.on_mouse_down(MouseButton::Left, on_pane_rubber_mouse_down(view, pane_id))
        .on_mouse_down(MouseButton::Right, move |ev, window, cx| {
            let pos = ev.position;
            menu_view.update(cx, |this, cx| {
                if let Some(path) = this
                    .tab_data_for(pane_id)
                    .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
                {
                    this.open_background_ctx_menu(Some(path), pos, window, cx);
                    cx.notify();
                }
            });
        })
        .on_drop::<DraggedFiles>(move |d, _window, cx| {
            let paths = d.paths.clone();
            drop_view.update(cx, |this, cx| {
                if let Some(command) = this.active_drag_hover_drop_command() {
                    this.execute_drag_hover_drop_command(command, paths, true);
                } else {
                    this.drop_files_on_pane(pane_id, paths, false);
                }
                cx.notify();
            });
        })
        .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
            let paths = paths.paths().to_vec();
            drop_ext_view.update(cx, |this, cx| {
                if let Some(command) = this.active_drag_hover_drop_command() {
                    this.execute_drag_hover_drop_command(command, paths, false);
                } else {
                    this.drop_files_on_pane(pane_id, paths, true);
                }
                cx.notify();
            });
        })
        .on_click(move |_ev, _window, cx| {
            focus_view.update(cx, |this, cx| {
                this.activate_tab(pane_id);
                cx.notify();
            });
        })
        .on_scroll_wheel(move |ev, window, cx| ctrl_scroll(ev, pane_id, &scroll_view, window, cx))
}

pub(super) fn should_suppress_loading(location: &Location) -> bool {
    matches!(location, Location::Directory(_))
}

impl App {
    pub(super) fn empty_pane_message(
        &self,
        pane_id: TabId,
        empty_message: String,
    ) -> Option<String> {
        let load_state = self.state.tab_states.get(&pane_id).map(|t| &t.load_state);
        let is_local_dir = self
            .state
            .pane_tree
            .tiles
            .get(pane_id)
            .and_then(|tile| match tile {
                Tile::Pane(pane) => Some(&pane.location),
                _ => None,
            })
            .is_some_and(should_suppress_loading);
        match load_state {
            // Still loading or not yet started — suppress for local dirs (avoids
            // a flash of "Empty folder" before the async listing arrives).
            Some(LoadState::Loading) | Some(LoadState::NotLoaded) => {
                (!is_local_dir).then(|| rust_i18n::t!("miller_loading").into_owned())
            }
            Some(LoadState::Error(error)) => Some(if error.is_empty() {
                rust_i18n::t!("miller_error").into_owned()
            } else {
                error.clone()
            }),
            _ => Some(empty_message),
        }
    }
}
