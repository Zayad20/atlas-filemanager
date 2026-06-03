use crate::state::layout_tree::{Container, Tile, Tree, TileId};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

use crate::fs::preview::PreviewResult;
use crate::search::nucleo_pool::NucleoPool;
use crate::state::file_entry::FileEntry;
use crate::state::pane::{TabData, TabId};
use crate::state::profiles::{ProfileManager, ProfileUiState};
use crate::state::settings::{Settings, ViewMode};
use crate::state::tab::{LoadState, TabState};

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
    MillerListed {
        tab: TabId,
        path: PathBuf,
        op_id: u64,
        entries: Vec<FileEntry>,
    },
    PreviewReady {
        tab: TabId,
        path: PathBuf,
        result: PreviewResult,
    },
    MultiPreviewThumbnailReady {
        tab: TabId,
        key: crate::state::preview_cache::MultiPreviewKey,
        path: PathBuf,
        rgba: Vec<u8>,
        width: usize,
        height: usize,
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
    /// Full tag-definition list pushed by the tag-db worker after any CRUD.
    TagDefinitions(Vec<crate::state::tag_cache::TagDef>),
    /// Cached tag associations for a set of queried paths.
    TagsForPaths {
        tags: HashMap<PathBuf, Vec<crate::state::tag_cache::TagId>>,
    },
    /// Paths carrying a given tag, used to populate a virtual tag-results tab.
    PathsForTag {
        tag: crate::state::tag_cache::TagId,
        paths: Vec<PathBuf>,
    },
    /// A removable device finished mounting at this path (navigate to it).
    DeviceMounted(PathBuf),
    /// A mount/unmount via udisksctl failed.
    DeviceActionFailed(String),
    /// The set of removable devices changed (e.g. after unmount); refresh sidebar.
    DevicesChanged,
    /// A declarative extension command finished; refresh `dirs` and report `error`.
    ExtCommandDone {
        ext_id: String,
        error: Option<String>,
        dirs: Vec<PathBuf>,
    },
    // --- cloud extension ---
    /// rclone binary download progress while fetching it on first use.
    #[cfg(feature = "cloud")]
    CloudRcloneProgress { done: u64, total: u64 },
    /// rclone daemon is up and the provider is ready.
    #[cfg(feature = "cloud")]
    CloudReady,
    /// A non-fatal cloud error to surface in the UI.
    #[cfg(feature = "cloud")]
    CloudError(String),
    /// A remote's backend config was created successfully.
    #[cfg(feature = "cloud")]
    RemoteConfigured(crate::cloud::CloudRemote),
    #[cfg(feature = "cloud")]
    RemoteConfigFailed {
        id: crate::cloud::RemoteId,
        error: String,
    },
    #[cfg(feature = "cloud")]
    RemoteRemoved(crate::cloud::RemoteId),
    /// A remote's runtime (mount/connection) status changed.
    #[cfg(feature = "cloud")]
    RemoteStatusChanged {
        id: crate::cloud::RemoteId,
        status: crate::cloud::RemoteStatus,
    },
    /// Full transfer-list snapshot from the Transfer Manager.
    #[cfg(feature = "cloud")]
    TransferUpdate(crate::cloud::TransferSnapshot),
    /// Storage usage `(used, total)` bytes reported for a remote.
    #[cfg(feature = "cloud")]
    RemoteQuota {
        id: crate::cloud::RemoteId,
        used: u64,
        total: u64,
    },
    /// A remote directory's contents changed (e.g. after an upload); re-list any
    /// Access-mode tab showing `(id, path)`.
    #[cfg(feature = "cloud")]
    RemoteChanged {
        id: crate::cloud::RemoteId,
        path: String,
    },
}

pub struct AppState {
    pub pane_tree: Tree<TabData>,
    /// Active Pane leaf TileId (the individual tab being viewed)
    pub focused_tile: TileId,
    /// Runtime per-tab state, keyed by Pane leaf TileId
    pub tab_states: HashMap<TileId, TabState>,
    pub settings: Settings,
    pub ops: crate::state::ops::OperationManager,
    pub overlay: crate::state::overlay::OverlayState,
    pub nucleo: NucleoPool,
    pub nucleo_results: Vec<std::path::PathBuf>,
    // Phase 5 — inspector preview
    pub preview: crate::state::preview_cache::PreviewState,
    // Phase 7 — filesystem watchers keyed by Pane leaf TileId
    pub watchers: HashMap<TileId, crate::fs::watcher::WatcherHandle>,
    pub batch_rename: Option<crate::state::batch_rename::BatchRenameState>,
    pub tabs_popup: crate::ui::tabs_popup::TabsPopupState,
    pub drag_drop: crate::state::drag::DragDropState,
    pub profiles: ProfileManager,
    pub profile_ui: ProfileUiState,
    pub space_lens: crate::state::space_lens::SpaceLensState,
    pub tag_cache: crate::state::tag_cache::TagCache,
    pub ext: crate::state::ext_state::ExtState,
    #[cfg(feature = "cloud")]
    pub cloud: crate::state::cloud_state::CloudState,
}

impl AppState {
    pub fn new(settings: Settings) -> (Self, crate::state::pane::PaneId) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let tab_data = TabData::with_view(
            home,
            settings.view_mode.clone(),
            settings.thumbnail_size,
            settings.show_inspector,
        );

        let mut tree: Tree<TabData> = Tree::empty();
        let pane_tile_id = tree.tiles.insert_pane(tab_data);
        let tabs_id = tree.tiles.insert_tab_tile(vec![pane_tile_id]);
        tree.root = Some(tabs_id);

        let mut tab_states = HashMap::new();
        tab_states.insert(pane_tile_id, TabState::default());

        let state = Self::build(tree, pane_tile_id, tab_states, settings);
        (state, pane_tile_id)
    }

    pub fn active_tab_state(&self) -> Option<&TabState> {
        self.tab_states.get(&self.focused_tile)
    }

    pub fn active_preview_request(&self) -> Option<crate::state::preview_cache::PreviewRequest> {
        use crate::state::preview_cache::{MultiPreviewKey, PreviewRequest, MULTI_PREVIEW_LIMIT};

        let tab = self.focused_tile;
        let view = match self.pane_tree.tiles.get(tab) {
            Some(Tile::Pane(p)) => p.view_mode.clone(),
            _ => return None,
        };
        let ts = self.tab_states.get(&tab)?;
        if view == ViewMode::Miller {
            let col = ts.miller.columns.get(ts.miller.focused)?;
            return col
                .entries
                .get(col.selected?)
                .map(|entry| PreviewRequest::Single(entry.path.clone()));
        }

        let mut selected: Vec<usize> = ts.selection.selected.iter().copied().collect();
        selected.sort_unstable();
        if selected.len() == 1 {
            return ts
                .entries
                .get(selected[0])
                .map(|entry| PreviewRequest::Single(entry.path.clone()));
        }
        if selected.is_empty() {
            return None;
        }

        let mut hasher = DefaultHasher::new();
        let mut paths = Vec::with_capacity(MULTI_PREVIEW_LIMIT);
        for idx in selected {
            let Some(entry) = ts.entries.get(idx) else {
                continue;
            };
            idx.hash(&mut hasher);
            entry.path.hash(&mut hasher);
            let is_image = entry
                .extension
                .as_deref()
                .is_some_and(crate::fs::preview::is_image_ext);
            if paths.len() < MULTI_PREVIEW_LIMIT && is_image {
                paths.push(entry.path.clone());
            }
        }

        Some(PreviewRequest::Multiple {
            key: MultiPreviewKey {
                fingerprint: hasher.finish(),
                selected_count: ts.selection.selected.len(),
            },
            paths,
        })
    }

    pub fn active_tab_state_mut(&mut self) -> Option<&mut TabState> {
        self.tab_states.get_mut(&self.focused_tile)
    }

    /// Paths of the currently selected entries in the focused tab (handles Miller).
    /// Mirrors what the inspector's `SelectionSummary` displays.
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        let Some(ts) = self.active_tab_state() else {
            return Vec::new();
        };
        let is_miller = matches!(
            self.pane_tree.tiles.get(self.focused_tile),
            Some(Tile::Pane(p)) if p.view_mode == ViewMode::Miller
        );
        if is_miller {
            return ts
                .miller
                .columns
                .get(ts.miller.focused)
                .and_then(|c| c.entries.get(c.selected?))
                .map(|e| vec![e.path.clone()])
                .unwrap_or_default();
        }
        let mut selected: Vec<usize> = ts.selection.selected.iter().copied().collect();
        selected.sort_unstable();
        selected
            .into_iter()
            .filter_map(|i| ts.entries.get(i).map(|e| e.path.clone()))
            .collect()
    }

    pub fn active_tab_data(&self) -> Option<&TabData> {
        match self.pane_tree.tiles.get(self.focused_tile) {
            Some(Tile::Pane(p)) => Some(p),
            _ => None,
        }
    }

    pub fn active_tab_data_mut(&mut self) -> Option<&mut TabData> {
        match self.pane_tree.tiles.get_mut(self.focused_tile) {
            Some(Tile::Pane(p)) => Some(p),
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
            if let Tile::Container(Container::Tabs(tabs)) = tile {
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
                Tile::Container(Container::Tabs(_))
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
                Tile::Container(Container::Linear(l)) => {
                    l.children.as_slice()
                }
                Tile::Container(Container::Tabs(t)) => {
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
    pub fn from_tree(tree: Tree<TabData>, settings: Settings) -> (Self, TileId) {
        // Collect all Pane leaf IDs to initialize tab_states
        let mut tab_states = HashMap::new();
        let mut first_tile = None;

        for (id, tile) in tree.tiles.iter() {
            if matches!(tile, Tile::Pane(_)) {
                tab_states.insert(*id, TabState::default());
                if first_tile.is_none() {
                    first_tile = Some(*id);
                }
            }
        }

        let focused = first_tile.unwrap_or(TileId(0));

        let state = Self::build(tree, focused, tab_states, settings);
        (state, focused)
    }

    fn build(
        pane_tree: Tree<TabData>,
        focused_tile: TileId,
        tab_states: HashMap<TileId, TabState>,
        settings: Settings,
    ) -> Self {
        Self {
            pane_tree,
            focused_tile,
            tab_states,
            settings,
            ops: crate::state::ops::OperationManager::default(),
            overlay: crate::state::overlay::OverlayState::default(),
            nucleo: NucleoPool::new(),
            nucleo_results: Vec::new(),
            preview: crate::state::preview_cache::PreviewState::default(),
            watchers: HashMap::new(),
            batch_rename: None,
            tabs_popup: crate::ui::tabs_popup::TabsPopupState::default(),
            drag_drop: crate::state::drag::DragDropState::default(),
            profiles: ProfileManager::default(),
            profile_ui: ProfileUiState::default(),
            space_lens: crate::state::space_lens::SpaceLensState::default(),
            tag_cache: crate::state::tag_cache::TagCache::default(),
            ext: crate::state::ext_state::ExtState::default(),
            #[cfg(feature = "cloud")]
            cloud: crate::state::cloud_state::CloudState::default(),
        }
    }

    /// Remove all preview data for a tab.
    pub fn clear_preview(&mut self, tab: TileId) {
        self.preview.clear(tab);
    }

    /// Register a new background operation and return its ID.
    pub fn start_op(&mut self, count: usize) -> u64 {
        self.ops.start_op(count)
    }

    /// All Pane leaf TileIds in the tree (for session restore / refresh all).
    pub fn all_pane_tile_ids(&self) -> Vec<TileId> {
        self.pane_tree
            .tiles
            .iter()
            .filter_map(|(id, tile)| {
                if matches!(tile, Tile::Pane(_)) {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn next_op_id(&self) -> u64 {
        self.ops.next_op_id()
    }
}

#[cfg(test)]
mod tests {
    use super::AppState;
    use crate::state::file_entry::{FileEntry, FileKind};
    use crate::state::preview_cache::{PreviewRequest, MULTI_PREVIEW_LIMIT};
    use crate::state::settings::Settings;
    use std::path::PathBuf;

    fn entry(name: &str) -> FileEntry {
        let path = PathBuf::from(name);
        FileEntry {
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            extension: path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(str::to_string),
            path,
            kind: FileKind::File,
            size: Some(1),
            modified: None,
            _is_hidden: false,
            _is_symlink: false,
        }
    }

    #[test]
    fn multi_preview_limits_images_in_visible_order() {
        let (mut state, tile) = AppState::new(Settings::default());
        let tab = state.tab_states.get_mut(&tile).unwrap();
        tab.set_entries((0..8).map(|idx| entry(&format!("{idx}.png"))).collect());
        tab.selection.selected.extend([7, 2, 5, 1, 3, 6, 4, 0]);

        let Some(PreviewRequest::Multiple { key, paths }) = state.active_preview_request() else {
            panic!("expected a multi-preview request");
        };
        assert_eq!(key.selected_count, 8);
        assert_eq!(paths.len(), MULTI_PREVIEW_LIMIT);
        assert_eq!(
            paths,
            ["0.png", "1.png", "2.png", "3.png"]
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn mixed_selection_schedules_only_image_thumbnails() {
        let (mut state, tile) = AppState::new(Settings::default());
        let tab = state.tab_states.get_mut(&tile).unwrap();
        tab.set_entries(vec![
            entry("photo.png"),
            entry("notes.txt"),
            entry("other.jpg"),
        ]);
        tab.selection.selected.extend([0, 1, 2]);

        let Some(PreviewRequest::Multiple { paths, .. }) = state.active_preview_request() else {
            panic!("expected a multi-preview request");
        };
        assert_eq!(
            paths,
            ["photo.png", "other.jpg"]
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        );
    }
}
