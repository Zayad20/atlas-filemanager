//! File-view renderers split out of `root.rs` by view mode.
use crate::app::App;
use crate::input::action::Action;
use crate::ui::drag_hover::{
    drag_hover_recorder_with_drop, DragHoverCommand, DragHoverDropCommand, DragHoverId,
    DragHoverKind,
};
use crate::ui::root::DraggedFiles;
use atlas_model::layout_tree::Tile;
use atlas_model::location::Location;
use atlas_model::pane::TabId;
use atlas_model::tab::LoadState;
use gpui::prelude::*;
use gpui::{ClickEvent, Div, Entity, Stateful, Window};
use gpui_component::ElementExt;
use std::path::PathBuf;

pub mod details;
pub mod grid;
pub mod list;
pub mod miller;
mod rename;
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
            let paths = this.select_context_menu_paths(pane_id, entry_idx);
            this.open_ctx_menu(paths, pos, window, cx);
            cx.notify();
        });
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
        .on_prepaint(drag_hover_recorder_with_drop(
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
                    DragHoverDropCommand::TransferTo(dest_internal.clone()),
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
                    DragHoverDropCommand::TransferTo(dest_external.clone()),
                    paths,
                    false,
                );
                cx.notify();
            });
        })
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
        match load_state {
            Some(LoadState::Loading) => {
                let local_dir = self
                    .state
                    .pane_tree
                    .tiles
                    .get(pane_id)
                    .and_then(|tile| match tile {
                        Tile::Pane(pane) => Some(&pane.location),
                        _ => None,
                    })
                    .is_some_and(should_suppress_loading);
                (!local_dir).then(|| rust_i18n::t!("miller_loading").into_owned())
            }
            Some(LoadState::Error(_)) => Some(rust_i18n::t!("miller_error").into_owned()),
            _ => Some(empty_message),
        }
    }
}
