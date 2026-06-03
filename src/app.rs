use crate::state::layout_tree::{Tile, TileId};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{Context, Window};
use smol::Timer;

use crate::fs::list::list_dir;
use crate::state::app_state::{AppState, BgResult};
use crate::state::pane::TabData;
use crate::state::preview_cache::{ImageData, PreviewRequest};
use crate::state::tab::LoadState;
use crate::util::config::{
    load_profiles, load_session, load_settings, save_profiles, save_session, save_settings,
};

pub mod actions;
#[cfg(feature = "cloud")]
pub mod cloud;
pub mod devices;
pub mod drag;
pub mod ext;
pub mod fs_sync;
pub mod nav;
pub mod ops;
pub mod pane_ops;
pub mod profiles;

pub struct App {
    pub(crate) state: AppState,
    pub(crate) tx: flume::Sender<BgResult>,
    pub(crate) rx: flume::Receiver<BgResult>,
    pub(crate) rt: tokio::runtime::Handle,
    pub(crate) pending_preview: Option<PendingPreview>,
    pub(crate) pending_native_drag: Option<Vec<PathBuf>>,
    /// Channel to the SQLite tag-db worker (writes fire-and-forget, reads reply via `tx`).
    pub(crate) tag_db_tx: flume::Sender<crate::fs::tag_db::TagDbCmd>,
    /// Channel to the cloud worker (rclone sidecar + transfers). Replies via `tx`.
    #[cfg(feature = "cloud")]
    pub(crate) cloud_tx: flume::Sender<crate::cloud::CloudCmd>,
}

const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(90);
/// How often the foreground loop drains the background-result channel. ~60 Hz so
/// async dir/preview results land promptly; a repaint is requested only when the
/// drain actually changed state (see `drain_bg_results`).
const DRAIN_INTERVAL: Duration = Duration::from_millis(16);

pub(crate) struct PendingPreview {
    pub(crate) tab: TileId,
    pub(crate) request: PreviewRequest,
    pub(crate) ready_at: Instant,
}

impl App {
    pub fn new(rt: tokio::runtime::Handle, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = load_settings();
        let profiles = load_profiles();

        let (tx, rx) = flume::unbounded();

        // Restore previous session tree if available, else create fresh state
        let restored_tree = load_session();
        let (mut state, _initial_tile_id) = if let Some(tree) = restored_tree {
            let (s, id) = AppState::from_tree(tree, settings);
            // Discard a corrupted/empty session and start fresh
            if s.all_pane_tile_ids().is_empty() {
                AppState::new(s.settings)
            } else {
                (s, id)
            }
        } else {
            AppState::new(settings)
        };
        state.profiles = profiles;
        // Profiles saved before the cloud feature lack the Network section, which
        // would hide configured remotes — backfill it.
        #[cfg(feature = "cloud")]
        state.profiles.ensure_cloud_section();

        // Spawn the SQLite tag-db worker. On a brand-new database it imports the tags
        // that previously lived only in profile config (global + per-profile).
        let migration = collect_tag_migration(&state.profiles);
        let tag_db_tx = crate::fs::tag_db::spawn(
            &rt,
            crate::util::config::data_dir().join("tags.db"),
            migration,
            tx.clone(),
        );

        // Trigger directory loads for all panes (focused first, rest on demand)
        let pane_ids = state.all_pane_tile_ids();
        let show_hidden = state.settings.show_hidden;
        for tile_id in pane_ids {
            let (path, sort_col, sort_dir, source) = match state.pane_tree.tiles.get(tile_id) {
                Some(Tile::Pane(p)) => (
                    p.path.clone(),
                    p.sort_col.clone(),
                    p.sort_dir.clone(),
                    p.source.clone(),
                ),
                _ => continue,
            };
            // A restored Access-mode remote tab can't reconnect before rclone is up;
            // leave it idle (the user re-opens it from the sidebar).
            #[cfg(feature = "cloud")]
            if matches!(source, crate::state::pane::TabSource::Remote { .. }) {
                continue;
            }
            // A restored virtual tag tab re-queries its paths instead of listing a dir.
            if let crate::state::pane::TabSource::Tag(tag) = source {
                let op_id = state.next_op_id();
                let ts = state.tab_states.entry(tile_id).or_default();
                ts.load_state = LoadState::Loading;
                ts.last_nav_op_id = op_id;
                let _ = tag_db_tx.send(crate::fs::tag_db::TagDbCmd::QueryPathsForTag(tag));
                continue;
            }
            let op_id = state.next_op_id();
            let ts = state.tab_states.entry(tile_id).or_default();
            ts.load_state = LoadState::Loading;
            ts.last_nav_op_id = op_id;

            let tx_clone = tx.clone();
            rt.spawn(async move {
                match list_dir(&path, show_hidden, sort_col, sort_dir).await {
                    Ok(entries) => {
                        let _ = tx_clone.send(BgResult::DirListed {
                            tab: tile_id,
                            op_id,
                            entries,
                        });
                    }
                    Err(e) => {
                        let _ = tx_clone.send(BgResult::DirListError {
                            tab: tile_id,
                            op_id,
                            error: e.to_string(),
                        });
                    }
                }
            });
        }

        // Extensions: seed bundled ones, load enabled overrides, build the registry.
        let _ =
            crate::ext::install::ensure_bundled("file-tools", crate::app::ext::FILE_TOOLS_MANIFEST);
        state.ext.enabled = crate::util::config::load_ext_enabled();
        state.ext.reload();

        // Spawn the cloud worker (rclone sidecar is started lazily on first use)
        // and restore previously-configured remotes for the sidebar.
        #[cfg(feature = "cloud")]
        let cloud_tx = crate::cloud::spawn(&rt, tx.clone());
        #[cfg(feature = "cloud")]
        {
            state.cloud.remotes = crate::util::config::load_cloud_remotes();
            // Auto-mount remotes the user flagged. Not added to `navigate_on_mount`,
            // so they mount silently without hijacking the startup tab.
            for r in &state.cloud.remotes {
                if r.auto_mount && r.access == crate::cloud::AccessKind::Mount {
                    let _ = cloud_tx.send(crate::cloud::CloudCmd::Mount { id: r.id.clone() });
                }
            }
        }

        // Drain background results on a ~60 Hz foreground loop, repainting only
        // when the drain actually changed state. Breaks when the entity is gone.
        cx.spawn(async move |this, cx| loop {
            Timer::after(DRAIN_INTERVAL).await;
            let alive = this
                .update(cx, |app, cx| {
                    if app.drain_bg_results() {
                        cx.notify();
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        })
        .detach();

        Self {
            state,
            tx,
            rx,
            rt,
            pending_preview: None,
            pending_native_drag: None,
            tag_db_tx,
            #[cfg(feature = "cloud")]
            cloud_tx,
        }
    }

    /// Apply every queued background result to state. Returns `true` if at least
    /// one result was processed (i.e. a repaint is warranted).
    pub(crate) fn drain_bg_results(&mut self) -> bool {
        let results: Vec<BgResult> = self.rx.try_iter().collect();
        let changed = !results.is_empty();
        for result in results {
            match result {
                BgResult::DirListed {
                    tab,
                    op_id,
                    entries,
                } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        // Ignore stale results from a superseded navigation
                        if op_id == ts.last_nav_op_id {
                            // Preserve selection across refreshes (e.g. fs-watcher
                            // events) by remapping the previously-selected paths onto
                            // the new listing. A real navigation already cleared the
                            // selection via begin_navigation, so nothing is preserved
                            // there.
                            let prev_selected: HashSet<PathBuf> = ts
                                .selection
                                .selected
                                .iter()
                                .filter_map(|&i| ts.entries.get(i).map(|e| e.path.clone()))
                                .collect();
                            let prev_cursor = ts
                                .selection
                                .keyboard_cursor
                                .and_then(|i| ts.entries.get(i))
                                .map(|e| e.path.clone());
                            ts.set_entries(entries);
                            ts.load_state = LoadState::Loaded;
                            ts.file_list_hit_regions = None;
                            ts.selection.selected = ts
                                .entries
                                .iter()
                                .enumerate()
                                .filter(|(_, e)| prev_selected.contains(&e.path))
                                .map(|(i, _)| i)
                                .collect();
                            ts.selection.keyboard_cursor = prev_cursor
                                .and_then(|p| ts.entries.iter().position(|e| e.path == p));
                            ts.selection.selection_anchor = ts.selection.keyboard_cursor;
                            ts.selection.last_clicked_index = ts.selection.keyboard_cursor;
                        }
                    }
                }
                BgResult::MillerListed {
                    tab,
                    path,
                    op_id,
                    entries,
                } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if let Some(c) = ts
                            .miller
                            .columns
                            .iter_mut()
                            .find(|c| c.path == path && c.op_id == op_id)
                        {
                            c.entries = entries;
                            c.load_state = LoadState::Loaded;
                        }
                    }
                }
                BgResult::DirListError { tab, op_id, error } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if op_id == ts.last_nav_op_id {
                            ts.load_state = LoadState::Error(error);
                        }
                    }
                }
                BgResult::FsEvent(changed_path) => {
                    // Refresh any pane watching the directory containing the changed path
                    let parent = changed_path.parent().map(|p| p.to_path_buf());
                    let to_refresh: Vec<TileId> = self
                        .state
                        .watchers
                        .iter()
                        .filter(|(_, handle)| {
                            Some(&handle.watched_path) == parent.as_ref()
                                || handle.watched_path == changed_path
                        })
                        .map(|(&id, _)| id)
                        .collect();
                    for tile_id in to_refresh {
                        if let Some(Tile::Pane(p)) = self.state.pane_tree.tiles.get(tile_id) {
                            let path = p.path.clone();
                            let col = p.sort_col.clone();
                            let dir = p.sort_dir.clone();
                            self.spawn_list_dir(tile_id, path, col, dir);
                        }
                    }
                }
                BgResult::PreviewReady { tab, path, result } => {
                    if self.state.preview.paths.get(&tab) != Some(&path) {
                        continue;
                    }
                    // The decoded RGBA stays in `result`; the UI layer uploads it to a
                    // GPU texture and caches that handle on its own side (state holds no
                    // renderer texture).
                    self.state.preview.results.insert(tab, result);
                }
                BgResult::MultiPreviewThumbnailReady {
                    tab,
                    key,
                    path,
                    rgba,
                    width,
                    height,
                } => {
                    let Some(multi) = self.state.preview.multi.get_mut(&tab) else {
                        continue;
                    };
                    if multi.key != key
                        || !multi.paths.contains(&path)
                        || multi.images.iter().any(|(loaded, _)| loaded == &path)
                    {
                        continue;
                    }
                    multi.images.push((path, ImageData { rgba, width, height }));
                }
                BgResult::OpComplete {
                    id,
                    result,
                    affected_tabs,
                } => {
                    self.state.ops.active_ops.remove(&id);
                    if let Err(e) = result {
                        tracing::error!("Background operation failed: {}", e);
                    }
                    for tab_id in affected_tabs {
                        if let Some(tab_data) = self.tab_data_for(tab_id) {
                            let path = tab_data.path.clone();
                            let sort_col = tab_data.sort_col.clone();
                            let sort_dir = tab_data.sort_dir.clone();
                            self.spawn_list_dir(tab_id, path, sort_col, sort_dir);
                        }
                    }
                }
                BgResult::OpProgress {
                    id,
                    done,
                    total,
                    current_file,
                } => {
                    if let Some(op) = self.state.ops.active_ops.get_mut(&id) {
                        op.done = done;
                        op.total = total;
                        op.current_file = current_file;
                    }
                }
                BgResult::TagDefinitions(defs) => {
                    self.state.tag_cache.apply_definitions(defs);
                }
                BgResult::TagsForPaths { tags } => {
                    self.state.tag_cache.apply_for_paths(tags);
                }
                BgResult::PathsForTag { tag, paths } => {
                    self.populate_tag_tabs(tag, paths);
                }
                BgResult::DeviceMounted(path) => {
                    crate::fs::udisks::invalidate();
                    self.navigate(self.state.focused_tile, path);
                }
                BgResult::DeviceActionFailed(error) => {
                    tracing::warn!("device action failed: {error}");
                }
                BgResult::DevicesChanged => {
                    crate::fs::udisks::invalidate();
                }
                BgResult::ExtCommandDone {
                    ext_id,
                    error,
                    dirs,
                } => {
                    self.handle_ext_bg(ext_id, error, dirs);
                }
                #[cfg(feature = "cloud")]
                other => self.handle_cloud_bg(other),
            }
        }
        changed
    }

    fn tab_data_for(&self, tab_id: TileId) -> Option<&TabData> {
        match self.state.pane_tree.tiles.get(tab_id) {
            Some(Tile::Pane(p)) => Some(p),
            _ => None,
        }
    }

    /// Persist all UI state to disk. Called on window close.
    pub fn save(&self) {
        save_settings(&self.state.settings);
        save_session(&self.state.pane_tree);
        save_profiles(&self.state.profiles);
        crate::util::config::save_ext_enabled(&self.state.ext.enabled);
        #[cfg(feature = "cloud")]
        crate::util::config::save_cloud_remotes(&self.state.cloud.remotes);
    }

    #[cfg(feature = "cloud")]
    pub fn on_exit(&mut self) {
        self.cleanup_cloud_on_exit();
    }
}

/// Build the one-time migration payload for a fresh tag database: every tag that
/// previously lived only in profile config (globals + each profile's own tags).
fn collect_tag_migration(
    profiles: &crate::state::profiles::ProfileManager,
) -> Vec<crate::state::tag_cache::TagDef> {
    use crate::state::tag_cache::{TagDef, TagId};
    let mut out = Vec::new();
    for tag in &profiles.global_tags {
        out.push(TagDef {
            id: TagId(0),
            name: tag.name.clone(),
            color: tag.color,
            profile: None,
        });
    }
    for profile in &profiles.profiles {
        for tag in &profile.tags {
            out.push(TagDef {
                id: TagId(0),
                name: tag.name.clone(),
                color: tag.color,
                profile: Some(profile.name.clone()),
            });
        }
    }
    out
}
