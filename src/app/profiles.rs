use crate::app::App;
use crate::state::layout_tree::{Tile, Tree};
use crate::state::pane::TabData;
use std::path::PathBuf;

impl App {
    pub(crate) fn switch_profile(&mut self, idx: usize) {
        if idx == self.state.profiles.active_idx || idx >= self.state.profiles.profiles.len() {
            return;
        }

        // Snapshot current tree into the active profile before leaving
        self.state.profiles.active_mut().tree = Some(self.state.pane_tree.clone());

        // Drop all FS watchers from the current session
        self.state.watchers.clear();

        self.state.profiles.active_idx = idx;
        self.state.profile_ui.adding_bookmark = false;
        self.state.profile_ui.adding_tag = false;

        // Restore the target profile's tree, or create a fresh single-tab home pane
        let new_tree = self
            .state
            .profiles
            .active()
            .tree
            .clone()
            .unwrap_or_else(|| {
                let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                let tab_data = TabData::with_view(
                    home,
                    self.state.settings.view_mode.clone(),
                    self.state.settings.thumbnail_size,
                    self.state.settings.show_inspector,
                );
                let mut tree = Tree::empty();
                let pane_id = tree.tiles.insert_pane(tab_data);
                let tabs_id = tree.tiles.insert_tab_tile(vec![pane_id]);
                tree.root = Some(tabs_id);
                tree
            });

        self.state.pane_tree = new_tree;
        self.state.tab_states.clear();
        self.state.preview.clear_all();
        self.pending_preview = None;

        let pane_ids = self.state.all_pane_tile_ids();
        if let Some(&first) = pane_ids.first() {
            self.state.focused_tile = first;
        }

        // Kick off directory loads for every pane in the restored tree
        let show_hidden = self.state.settings.show_hidden;
        for tile_id in pane_ids {
            let (path, sort_col, sort_dir) = match self.state.pane_tree.tiles.get(tile_id) {
                Some(Tile::Pane(p)) => {
                    (p.path.clone(), p.sort_col.clone(), p.sort_dir.clone())
                }
                _ => continue,
            };
            let op_id = self.state.next_op_id();
            let ts = self.state.tab_states.entry(tile_id).or_default();
            ts.load_state = crate::state::tab::LoadState::Loading;
            ts.last_nav_op_id = op_id;

            let tx_clone = self.tx.clone();
            self.rt.spawn(async move {
                match crate::fs::list::list_dir(&path, show_hidden, sort_col, sort_dir).await {
                    Ok(entries) => {
                        let _ = tx_clone.send(crate::state::app_state::BgResult::DirListed {
                            tab: tile_id,
                            op_id,
                            entries,
                        });
                    }
                    Err(e) => {
                        let _ = tx_clone.send(crate::state::app_state::BgResult::DirListError {
                            tab: tile_id,
                            op_id,
                            error: e.to_string(),
                        });
                    }
                }
            });
        }
    }

    pub(crate) fn delete_active_profile(&mut self) {
        if self.state.profiles.profiles.len() <= 1 {
            return;
        }
        let del_idx = self.state.profiles.active_idx;
        self.state.profiles.profiles.remove(del_idx);
        let new_idx = if del_idx >= self.state.profiles.profiles.len() {
            self.state.profiles.profiles.len() - 1
        } else {
            del_idx
        };
        // active_idx must differ from new_idx to trigger a real switch,
        // so temporarily place it out of range before calling switch_profile.
        self.state.profiles.active_idx = usize::MAX;
        self.switch_profile(new_idx);
    }
}
