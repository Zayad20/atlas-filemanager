#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use crate::layout_tree::{Container, Tile, TileId, Tree};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

use crate::file_entry::FileEntry;
use crate::pane::{TabData, TabId};
use crate::preview_cache::PreviewResult;
use crate::profiles::{ProfileManager, ProfileUiState};
use crate::settings::{Settings, ViewMode};
use crate::tab::{LoadState, TabState};

pub enum BgResult {
    DirListPreview {
        tab: TabId,
        op_id: u64,
        entries: Vec<FileEntry>,
    },
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
        location: crate::location::Location,
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
        key: crate::preview_cache::MultiPreviewKey,
        path: PathBuf,
        rgba: Vec<u8>,
        width: usize,
        height: usize,
    },
    /// A downscaled grid thumbnail finished decoding off-thread. `rgba` empty == the
    /// decode failed (cached as a permanent miss so it isn't re-requested).
    GridThumbnailReady {
        path: PathBuf,
        rgba: Vec<u8>,
        width: usize,
        height: usize,
    },
    /// Decode was skipped because the path left the visible viewport before the
    /// blocking task ran. Remove from thumb_textures so the cell can re-request.
    GridThumbnailSkipped {
        path: PathBuf,
    },

    /// Virtual listing of an archive directory completed.
    ArchiveListed {
        tab: TabId,
        op_id: u64,
        /// Archive file on disk.
        archive: PathBuf,
        /// Directory within the archive that was listed (empty = root).
        inner: PathBuf,
        entries: Vec<FileEntry>,
    },
    ArchiveListError {
        tab: TabId,
        op_id: u64,
        error: String,
    },
    SearchInPlaceResults {
        tab: TabId,
        gen: u64,
        entries: Vec<FileEntry>,
    },
    SearchInPlaceError {
        tab: TabId,
        gen: u64,
        error: String,
    },
    FsEvent(std::path::PathBuf),
    OpProgress {
        id: u64,
        done: u64,
        total: u64,
        bytes_done: u64,
        bytes_total: u64,
        current_file: PathBuf,
    },
    OpComplete {
        id: u64,
        /// Per-item failures; a user cancel is one `OpErrorKind::Cancelled` entry.
        result: Result<(), Vec<crate::ops::OpError>>,
        affected_tabs: Vec<TabId>,
    },
    /// A paste target wrote `Performed DropEffect` onto our clipboard data
    /// object (e.g. Explorer pasting an Atlas cut). `effect` is the raw
    /// `DROPEFFECT` value: 0 = optimized move (target moved the files itself),
    /// 1 = copy, 2 = move (we must delete the sources).
    ClipboardPerformed {
        effect: u32,
        paths: Vec<PathBuf>,
    },
    /// A unified transfer (copy/move via paste, drag-and-drop, or menu) finished —
    /// possibly partially. The report carries per-item errors, skips, the real
    /// `src→dst` pairs for undo, and whether it was cancelled.
    TransferComplete {
        id: u64,
        kind: crate::ops::TransferKind,
        origin: crate::ops::TransferOrigin,
        report: crate::ops::TransferReport,
        affected_tabs: Vec<TabId>,
    },
    /// Full tag-definition list pushed by the tag-db worker after any CRUD.
    TagDefinitions(Vec<crate::tag_cache::TagDef>),
    /// Cached tag associations for a set of queried paths.
    TagsForPaths {
        tags: HashMap<PathBuf, Vec<crate::tag_cache::TagId>>,
    },
    /// Paths carrying a given tag, used to populate a virtual tag-results tab.
    PathsForTag {
        tag: crate::tag_cache::TagId,
        paths: Vec<PathBuf>,
    },
    /// A removable device finished mounting at this path (navigate to it).
    DeviceMounted(PathBuf),
    /// A mount/unmount via udisksctl failed.
    DeviceActionFailed(String),
    /// The set of removable devices changed (e.g. after unmount); refresh sidebar.
    DevicesChanged,
    /// A batch of files whose contents matched the goto bar's content search. `gen`
    /// tags the search generation so stale batches (from an older query) are dropped.
    ContentResults {
        gen: u64,
        paths: Vec<PathBuf>,
    },
    ExtCommandDone {
        ext_id: String,
        error: Option<String>,
        dirs: Vec<PathBuf>,
    },
    /// Remote marketplace registry fetched from R2.
    ExtRegistryFetched(Result<crate::ext_model::RemoteRegistry, String>),
    /// A remote extension finished downloading and installing.
    ExtRemoteInstalled {
        id: String,
        result: Result<(), String>,
    },
    /// A local extension (zip/folder) finished installing on the background thread.
    ExtLocalInstalled(Result<String, String>),
    /// A runtime tool finished installing; retry the preview that needed it.
    ExtToolInstalled {
        tool: String,
        tab: TabId,
        path: PathBuf,
        result: Result<(), String>,
    },
    AppUpdateAvailable {
        version: String,
        url: String,
    },
    /// A WASM extension requested the fuzzy panel. The background worker blocks on `reply_tx`.
    ExtShowQuickPick {
        items: Vec<String>,
        reply_tx: std::sync::mpsc::Sender<String>,
    },
    // --- cloud extension ---
    /// rclone binary download progress while fetching it on first use.
    #[cfg(feature = "provider-host")]
    CloudRcloneProgress {
        done: u64,
        total: u64,
    },
    /// rclone daemon is up and the provider is ready.
    #[cfg(feature = "provider-host")]
    CloudReady,
    /// A non-fatal cloud error to surface in the UI.
    #[cfg(feature = "provider-host")]
    CloudError(String),
    /// WinFsp download progress `(bytes_done, bytes_total)`.
    #[cfg(all(feature = "provider-host", windows))]
    WinFspProgress {
        done: u64,
        total: u64,
    },
    /// WinFsp installed successfully; retry the pending mount.
    #[cfg(all(feature = "provider-host", windows))]
    WinFspInstalled {
        /// Remote to mount now that WinFsp is available (if any).
        retry_id: Option<crate::cloud_model::RemoteId>,
    },
    /// A remote's backend config was created successfully.
    #[cfg(feature = "provider-host")]
    RemoteConfigured(crate::cloud_model::CloudRemote),
    #[cfg(feature = "provider-host")]
    RemoteConfigFailed {
        id: crate::cloud_model::RemoteId,
        error: String,
    },
    #[cfg(feature = "provider-host")]
    RemoteRemoved(crate::cloud_model::RemoteId),
    /// A remote's runtime (mount/connection) status changed.
    #[cfg(feature = "provider-host")]
    RemoteStatusChanged {
        id: crate::cloud_model::RemoteId,
        status: crate::cloud_model::RemoteStatus,
    },
    /// Full transfer-list snapshot from the Transfer Manager.
    #[cfg(feature = "provider-host")]
    TransferUpdate(crate::cloud_model::TransferSnapshot),
    /// Storage usage `(used, total)` bytes reported for a remote.
    #[cfg(feature = "provider-host")]
    RemoteQuota {
        id: crate::cloud_model::RemoteId,
        used: u64,
        total: u64,
    },
    /// A remote directory's contents changed (e.g. after an upload); re-list any
    /// Access-mode tab showing `(id, path)`.
    #[cfg(feature = "provider-host")]
    RemoteChanged {
        id: crate::cloud_model::RemoteId,
        paths: Vec<String>,
    },
}

pub struct AppState {
    pub pane_tree: Tree<TabData>,
    /// Active Pane leaf `TileId` (the individual tab being viewed)
    pub focused_tile: TileId,
    /// Runtime per-tab state, keyed by Pane leaf `TileId`
    pub tab_states: HashMap<TileId, TabState>,
    pub settings: Settings,
    pub ops: crate::ops::OperationManager,
    pub overlay: crate::overlay::OverlayState,
    pub dragging_pane: Option<TileId>,
    pub drag_pending: bool,
    // Phase 5 — inspector preview
    pub preview: crate::preview_cache::PreviewState,
    pub batch_rename: Option<crate::batch_rename::BatchRenameState>,
    pub profiles: ProfileManager,
    pub profile_ui: ProfileUiState,
    pub sidebar_drop_zone: Vec<StagedDrop>,
    pub tag_cache: crate::tag_cache::TagCache,
    pub ext: crate::ext_state::ExtState,
    #[cfg(feature = "provider-host")]
    pub cloud: crate::cloud_state::CloudState,
}

#[derive(Clone)]
pub struct StagedDrop {
    pub path: PathBuf,
    pub from_self: bool,
}

impl AppState {
    pub fn container_contains_pane(&self, container_id: TileId, pane_id: TileId) -> bool {
        if container_id == pane_id {
            return true;
        }
        match self.pane_tree.tiles.get(container_id) {
            Some(Tile::Container(Container::Tabs(tabs))) => tabs.children.contains(&pane_id),
            Some(Tile::Container(Container::Linear(linear))) => linear
                .children
                .iter()
                .any(|c| self.container_contains_pane(*c, pane_id)),
            _ => false,
        }
    }

    pub fn new(settings: Settings) -> (Self, crate::pane::PaneId) {
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

    pub fn active_preview_request(&self) -> Option<crate::preview_cache::PreviewRequest> {
        use crate::preview_cache::{MultiPreviewKey, PreviewRequest, MULTI_PREVIEW_LIMIT};

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
                .get(col.primary_selected()?)
                .map(|entry| entry_to_preview_request(entry));
        }

        let mut selected: Vec<usize> = ts.selection.selected.iter().copied().collect();
        selected.sort_unstable();
        if selected.len() == 1 {
            return ts
                .entries
                .get(selected[0])
                .map(|entry| entry_to_preview_request(entry));
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
                .is_some_and(crate::preview_cache::is_image_ext);
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
            let Some(col) = ts.miller.columns.get(ts.miller.focused) else {
                return Vec::new();
            };
            let mut selected: Vec<usize> = col.selection.selected.iter().copied().collect();
            selected.sort_unstable();
            return selected
                .into_iter()
                .filter_map(|i| col.entries.get(i).map(|e| e.path.clone()))
                .collect();
        }
        let mut selected: Vec<usize> = ts.selection.selected.iter().copied().collect();
        selected.sort_unstable();
        selected
            .into_iter()
            .filter_map(|i| ts.entries.get(i).map(|e| e.path.clone()))
            .collect()
    }

    /// Like `selected_paths()` but returns `(archive, inner)` pairs for archive
    /// tabs, so callers can implement drag-out virtual (extract on drag).
    /// Returns `None` when the focused tab is not in archive mode.
    pub fn selected_archive_items(&self) -> Option<Vec<(PathBuf, PathBuf)>> {
        let td = self.active_tab_data()?;
        let (archive, _) = td.location.as_archive()?;
        let archive = archive.to_path_buf();
        let Some(ts) = self.active_tab_state() else {
            return Some(Vec::new());
        };
        let mut selected: Vec<usize> = ts.selection.selected.iter().copied().collect();
        selected.sort_unstable();
        let items = selected
            .into_iter()
            .filter_map(|i| ts.entries.get(i).map(|e| (archive.clone(), e.path.clone())))
            .collect();
        Some(items)
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

    /// Find the Tabs container `TileId` that contains the given pane tile.
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

    /// Find any Tabs container in the tree (fallback when `focused_tile` is stale).
    pub fn find_any_tabs_container(&self) -> Option<TileId> {
        self.pane_tree.tiles.iter().find_map(|(id, tile)| {
            if matches!(tile, Tile::Container(Container::Tabs(_))) {
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
                Tile::Container(Container::Linear(l)) => l.children.as_slice(),
                Tile::Container(Container::Tabs(t)) => t.children.as_slice(),
                _ => continue,
            };
            if children.contains(&tile_id) {
                return Some(*id);
            }
        }
        None
    }

    /// Restore from a persisted tree. Returns (state, `first_pane_to_load`).
    pub fn from_tree(mut tree: Tree<TabData>, settings: Settings) -> (Self, TileId) {
        tree.sanitize();

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
        tree.make_active(|id, _| id == focused);

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
            ops: crate::ops::OperationManager::default(),
            overlay: crate::overlay::OverlayState::default(),
            dragging_pane: None,
            preview: crate::preview_cache::PreviewState::default(),
            batch_rename: None,
            profiles: ProfileManager::default(),
            profile_ui: ProfileUiState::default(),
            sidebar_drop_zone: Vec::new(),
            drag_pending: false,
            tag_cache: crate::tag_cache::TagCache::default(),
            ext: crate::ext_state::ExtState::default(),
            #[cfg(feature = "provider-host")]
            cloud: crate::cloud_state::CloudState::default(),
        }
    }

    /// Remove all preview data for a tab.
    pub fn clear_preview(&mut self, tab: TileId) {
        self.preview.clear(tab);
    }

    /// All Pane leaf `TileIds` in the tree (for session restore / refresh all).
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

fn entry_to_preview_request(
    entry: &crate::file_entry::FileEntry,
) -> crate::preview_cache::PreviewRequest {
    use crate::preview_cache::PreviewRequest;
    if let Some(archive) = entry.archive_root.clone() {
        PreviewRequest::ArchiveSingle {
            archive,
            inner: entry.path.clone(),
        }
    } else {
        PreviewRequest::Single(entry.path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::AppState;
    use crate::file_entry::{FileEntry, FileKind};
    use crate::preview_cache::{PreviewRequest, MULTI_PREVIEW_LIMIT};
    use crate::settings::Settings;
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
            display_size: String::new().into(),
            display_date: String::new().into(),
            display_type: String::new().into(),
            _is_hidden: false,
            _is_symlink: false,
            archive_root: None,
            recycle_info: None,
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
        tab.set_entries(vec![entry("photo.png"), entry("notes.txt"), entry("other.jpg")].into());
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
