use egui_tiles::TileId;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use crate::fs::preview::PreviewResult;
use crate::search::nucleo_pool::NucleoPool;
use crate::state::file_entry::FileEntry;
use crate::state::pane::{TabData, TabId};
use crate::state::profiles::{ProfileManager, ProfileUiState};
use crate::state::settings::Settings;
use crate::state::tab::{LoadState, TabState};
use crate::state::undo::UndoStack;
use crate::ui::fuzzy_overlay::FuzzyOverlayState;

#[allow(dead_code)]
pub enum BgResult {
    DirListed {
        tab: TabId,
        op_id: u64,
        entries: Vec<FileEntry>,
    },
    DirListError {
        tab: TabId,
        op_id: u64,
        error: String,
    },
    PreviewReady {
        tab: TabId,
        path: PathBuf,
        result: PreviewResult,
    },
    FsEvent(std::path::PathBuf),
    OpProgress {
        id: u64,
        done: u64,
        total: u64,
        current_file: PathBuf,
    },
    OpComplete {
        id: u64,
        result: Result<(), String>,
        affected_tabs: Vec<TabId>,
    },
    IndexChunk(Vec<PathBuf>),
    IndexComplete,
    DirSize {
        path: PathBuf,
        bytes: u64,
    },
}

pub struct ClipboardState {
    pub paths: Vec<PathBuf>,
    pub is_cut: bool,
}

pub struct OpState {
    pub done: u64,
    pub total: u64,
    pub current_file: PathBuf,
}

#[derive(Clone)]
pub struct DraggedFiles {
    pub source_tab: TabId,
    pub paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragHoverTarget {
    Folder { tab: TabId, path: PathBuf },
    Tab(TabId),
    Location { tab: TabId, path: PathBuf },
}

#[derive(Default)]
pub struct DragHoverState {
    pub observed_target: Option<DragHoverTarget>,
    pub candidate: Option<DragHoverTarget>,
    pub candidate_since: f64,
    pub confirmation_target: Option<DragHoverTarget>,
    pub confirmation_started_at: f64,
    pub triggered: bool,
    pub original_paths: HashMap<TabId, PathBuf>,
    pub original_focused_tile: Option<TabId>,
}

impl DragHoverState {
    pub fn confirmation_visible(&self, target: &DragHoverTarget, now: f64) -> bool {
        const BLINK_INTERVAL: f64 = 0.08;
        const BLINK_DURATION: f64 = BLINK_INTERVAL * 4.0;

        if self.confirmation_target.as_ref() != Some(target) {
            return false;
        }
        let elapsed = now - self.confirmation_started_at;
        (0.0..BLINK_DURATION).contains(&elapsed)
            && (elapsed / BLINK_INTERVAL).floor() as u64 % 2 == 0
    }
}

#[derive(Clone)]
pub struct ExternalDropTarget {
    pub tab: TabId,
    pub dest: PathBuf,
}

pub struct AppState {
    pub pane_tree: egui_tiles::Tree<TabData>,
    /// Active Pane leaf TileId (the individual tab being viewed)
    pub focused_tile: TileId,
    /// Runtime per-tab state, keyed by Pane leaf TileId
    pub tab_states: HashMap<TileId, TabState>,
    pub settings: Settings,
    pub undo_stack: UndoStack,
    pub op_id_counter: Arc<AtomicU64>,
    pub clipboard: Option<ClipboardState>,
    pub active_ops: HashMap<u64, OpState>,
    pub show_goto_bar: bool,
    pub show_palette: bool,
    pub show_settings: bool,
    pub nucleo: NucleoPool,
    pub goto_state: FuzzyOverlayState,
    pub palette_state: FuzzyOverlayState,
    pub nucleo_results: Vec<std::path::PathBuf>,
    // Phase 5 — inspector preview
    pub preview_paths: HashMap<TileId, PathBuf>,
    pub preview_results: HashMap<TileId, PreviewResult>,
    pub preview_textures: HashMap<TileId, egui::TextureHandle>,
    // Phase 7 — filesystem watchers keyed by Pane leaf TileId
    pub watchers: HashMap<TileId, crate::fs::watcher::WatcherHandle>,
    pub batch_rename: Option<crate::ui::batch_rename::BatchRenameState>,
    pub tabs_popup: crate::ui::tabs_popup::TabsPopupState,
    pub dragged_files: Option<DraggedFiles>,
    pub drag_hover: DragHoverState,
    pub external_drop_target: Option<ExternalDropTarget>,
    pub drop_shelf_open: bool,
    pub drop_shelf_files: Vec<PathBuf>,
    pub profiles: ProfileManager,
    pub profile_ui: ProfileUiState,
}

impl AppState {
    pub fn new(settings: Settings) -> (Self, TileId) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let tab_data = TabData::with_view(
            home,
            settings.view_mode.clone(),
            settings.thumbnail_size,
            settings.show_inspector,
        );

        let mut tree: egui_tiles::Tree<TabData> =
            egui_tiles::Tree::empty(egui::Id::new("Atlas"));
        let pane_tile_id = tree.tiles.insert_pane(tab_data);
        let tabs_id = tree.tiles.insert_tab_tile(vec![pane_tile_id]);
        tree.root = Some(tabs_id);

        let mut tab_states = HashMap::new();
        tab_states.insert(pane_tile_id, TabState::default());

        let state = Self {
            pane_tree: tree,
            focused_tile: pane_tile_id,
            tab_states,
            settings,
            undo_stack: UndoStack::default(),
            op_id_counter: Arc::new(AtomicU64::new(1)),
            clipboard: None,
            active_ops: HashMap::new(),
            show_goto_bar: false,
            show_palette: false,
            show_settings: false,
            nucleo: NucleoPool::new(),
            goto_state: FuzzyOverlayState::default(),
            palette_state: FuzzyOverlayState::default(),
            nucleo_results: Vec::new(),
            preview_paths: HashMap::new(),
            preview_results: HashMap::new(),
            preview_textures: HashMap::new(),
            watchers: HashMap::new(),
            batch_rename: None,
            tabs_popup: crate::ui::tabs_popup::TabsPopupState::default(),
            dragged_files: None,
            drag_hover: DragHoverState::default(),
            external_drop_target: None,
            drop_shelf_open: false,
            drop_shelf_files: Vec::new(),
            profiles: ProfileManager::default(),
            profile_ui: ProfileUiState::default(),
        };
        (state, pane_tile_id)
    }

    pub fn active_tab_state(&self) -> Option<&TabState> {
        self.tab_states.get(&self.focused_tile)
    }

    pub fn active_tab_state_mut(&mut self) -> Option<&mut TabState> {
        self.tab_states.get_mut(&self.focused_tile)
    }

    #[allow(dead_code)]
    pub fn active_tab_data(&self) -> Option<&TabData> {
        match self.pane_tree.tiles.get(self.focused_tile) {
            Some(egui_tiles::Tile::Pane(p)) => Some(p),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn active_tab_data_mut(&mut self) -> Option<&mut TabData> {
        match self.pane_tree.tiles.get_mut(self.focused_tile) {
            Some(egui_tiles::Tile::Pane(p)) => Some(p),
            _ => None,
        }
    }

    pub fn any_loading(&self) -> bool {
        self.tab_states
            .values()
            .any(|s| s.load_state == LoadState::Loading)
    }

    /// Find the Tabs container TileId that contains the given pane tile.
    pub fn find_tabs_container(&self, tile_id: TileId) -> Option<TileId> {
        for (id, tile) in self.pane_tree.tiles.iter() {
            if let egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs)) = tile {
                if tabs.children.contains(&tile_id) {
                    return Some(*id);
                }
            }
        }
        None
    }

    /// Find any Tabs container in the tree (fallback when focused_tile is stale).
    pub fn find_any_tabs_container(&self) -> Option<TileId> {
        self.pane_tree.tiles.iter().find_map(|(id, tile)| {
            if matches!(
                tile,
                egui_tiles::Tile::Container(egui_tiles::Container::Tabs(_))
            ) {
                Some(*id)
            } else {
                None
            }
        })
    }

    /// Find the direct parent container of a tile.
    pub fn find_parent_of(&self, tile_id: TileId) -> Option<TileId> {
        for (id, tile) in self.pane_tree.tiles.iter() {
            let children = match tile {
                egui_tiles::Tile::Container(egui_tiles::Container::Linear(l)) => {
                    l.children.as_slice()
                }
                egui_tiles::Tile::Container(egui_tiles::Container::Tabs(t)) => {
                    t.children.as_slice()
                }
                _ => continue,
            };
            if children.contains(&tile_id) {
                return Some(*id);
            }
        }
        None
    }

    /// Restore from a persisted tree. Returns (state, first_pane_to_load).
    pub fn from_tree(tree: egui_tiles::Tree<TabData>, settings: Settings) -> (Self, TileId) {
        let pane_ids = reachable_pane_tile_ids(&tree);
        let focused = pane_ids.first().copied().unwrap_or(egui_tiles::TileId(0));
        let tab_states = pane_ids
            .into_iter()
            .map(|id| (id, TabState::default()))
            .collect();

        let state = Self {
            pane_tree: tree,
            focused_tile: focused,
            tab_states,
            settings,
            undo_stack: UndoStack::default(),
            op_id_counter: Arc::new(AtomicU64::new(1)),
            clipboard: None,
            active_ops: HashMap::new(),
            show_goto_bar: false,
            show_palette: false,
            show_settings: false,
            nucleo: NucleoPool::new(),
            goto_state: FuzzyOverlayState::default(),
            palette_state: FuzzyOverlayState::default(),
            nucleo_results: Vec::new(),
            preview_paths: HashMap::new(),
            preview_results: HashMap::new(),
            preview_textures: HashMap::new(),
            watchers: HashMap::new(),
            batch_rename: None,
            tabs_popup: crate::ui::tabs_popup::TabsPopupState::default(),
            dragged_files: None,
            drag_hover: DragHoverState::default(),
            external_drop_target: None,
            drop_shelf_open: false,
            drop_shelf_files: Vec::new(),
            profiles: ProfileManager::default(),
            profile_ui: ProfileUiState::default(),
        };
        (state, focused)
    }

    /// All Pane leaf TileIds in the tree (for session restore / refresh all).
    pub fn all_pane_tile_ids(&self) -> Vec<TileId> {
        reachable_pane_tile_ids(&self.pane_tree)
    }

    pub fn next_op_id(&self) -> u64 {
        use std::sync::atomic::Ordering;
        self.op_id_counter.fetch_add(1, Ordering::Relaxed)
    }
}

fn reachable_pane_tile_ids(tree: &egui_tiles::Tree<TabData>) -> Vec<TileId> {
    let Some(root) = tree.root else {
        return Vec::new();
    };
    let mut panes = Vec::new();
    let mut pending = vec![root];
    let mut visited = std::collections::HashSet::new();
    while let Some(tile_id) = pending.pop() {
        if !visited.insert(tile_id) {
            continue;
        }
        match tree.tiles.get(tile_id) {
            Some(egui_tiles::Tile::Pane(_)) => panes.push(tile_id),
            Some(egui_tiles::Tile::Container(container)) => {
                pending.extend(container.children().copied());
            }
            None => {}
        }
    }
    panes
}

#[cfg(test)]
mod tests {
    use super::{AppState, DragHoverState, DragHoverTarget};
    use crate::state::pane::TabData;
    use crate::state::settings::{Settings, ViewMode};
    use egui_tiles::TileId;
    use std::path::PathBuf;

    #[test]
    fn drag_hover_confirmation_blinks_twice_then_stops() {
        let target = DragHoverTarget::Tab(TileId(7));
        let state = DragHoverState {
            confirmation_target: Some(target.clone()),
            confirmation_started_at: 10.0,
            ..Default::default()
        };

        assert!(state.confirmation_visible(&target, 10.01));
        assert!(!state.confirmation_visible(&target, 10.09));
        assert!(state.confirmation_visible(&target, 10.17));
        assert!(!state.confirmation_visible(&target, 10.25));
        assert!(!state.confirmation_visible(&target, 10.33));
    }

    #[test]
    fn restored_state_ignores_orphaned_panes() {
        let mut tree = egui_tiles::Tree::empty("restore_test");
        let reachable = tree.tiles.insert_pane(TabData::with_view(
            PathBuf::from("/reachable"),
            ViewMode::Details,
            128.0,
            false,
        ));
        let orphan = tree.tiles.insert_pane(TabData::with_view(
            PathBuf::from("/orphan"),
            ViewMode::Details,
            128.0,
            false,
        ));
        tree.root = Some(tree.tiles.insert_tab_tile(vec![reachable]));

        let (state, focused) = AppState::from_tree(tree, Settings::default());

        assert_eq!(focused, reachable);
        assert_eq!(state.all_pane_tile_ids(), vec![reachable]);
        assert!(state.tab_states.contains_key(&reachable));
        assert!(!state.tab_states.contains_key(&orphan));
    }
}
