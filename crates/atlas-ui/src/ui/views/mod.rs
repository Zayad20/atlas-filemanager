//! File-view renderers split out of `root.rs` by view mode.
use crate::app::App;
use crate::state::layout_tree::Tile;
use crate::state::location::Location;
use crate::state::pane::TabId;
use crate::state::tab::LoadState;

pub mod details;
pub mod grid;
pub mod list;
pub mod miller;
mod rename;
pub mod virtual_grid;

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
