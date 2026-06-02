use crate::app::{App, PendingPreview, PREVIEW_DEBOUNCE};
use crate::fs::list::{entries_for_paths, list_dir};
use crate::state::app_state::BgResult;
use crate::state::pane::TabSource;
use crate::state::preview_cache::{MultiPreviewState, PreviewRequest};
use crate::state::tab::LoadState;
use crate::state::tag_cache::TagId;
use egui_tiles::TileId;
use std::path::PathBuf;
use std::time::Instant;

impl App {
    pub(crate) fn spawn_list_dir(
        &mut self,
        tab_id: TileId,
        path: PathBuf,
        sort_col: crate::state::file_entry::SortColumn,
        sort_dir: crate::state::file_entry::SortDir,
    ) {
        let op_id = self.state.next_op_id();
        let ts = self.state.tab_states.entry(tab_id).or_default();
        ts.load_state = LoadState::Loading;
        ts.last_nav_op_id = op_id;

        let tx = self.tx.clone();
        let show_hidden = self.state.settings.show_hidden;
        self.rt.spawn(async move {
            match list_dir(&path, show_hidden, sort_col, sort_dir).await {
                Ok(entries) => {
                    let _ = tx.send(BgResult::DirListed {
                        tab: tab_id,
                        op_id,
                        entries,
                    });
                }
                Err(e) => {
                    let _ = tx.send(BgResult::DirListError {
                        tab: tab_id,
                        op_id,
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    /// Ensure the tag cache has association data for every currently-selected path, so
    /// the inspector can show their tags. Queries are sent only for paths we don't yet
    /// know about; an empty placeholder is inserted immediately to avoid re-querying
    /// the same paths every frame while the worker's answer is in flight.
    pub(crate) fn sync_selection_tags(&mut self) {
        let unknown: Vec<PathBuf> = self
            .state
            .selected_paths()
            .into_iter()
            .filter(|p| !self.state.tag_cache.knows(p))
            .collect();
        if unknown.is_empty() {
            return;
        }
        for p in &unknown {
            self.state.tag_cache.for_path.entry(p.clone()).or_default();
        }
        let _ = self
            .tag_db_tx
            .send(crate::fs::tag_db::TagDbCmd::QueryTagsForPaths(unknown));
    }

    /// Rebuild the listing of every tab showing results for `tag` from the paths the
    /// tag-db worker just returned. Mirrors `spawn_list_dir` but sources entries from an
    /// explicit path set instead of a `read_dir`, and reuses `BgResult::DirListed` so the
    /// views update through the exact same path as a normal directory load.
    pub(crate) fn populate_tag_tabs(&mut self, tag: TagId, paths: Vec<PathBuf>) {
        let targets: Vec<(
            TileId,
            crate::state::file_entry::SortColumn,
            crate::state::file_entry::SortDir,
        )> = self
            .state
            .pane_tree
            .tiles
            .iter()
            .filter_map(|(id, tile)| match tile {
                egui_tiles::Tile::Pane(p) if p.source == TabSource::Tag(tag) => {
                    Some((*id, p.sort_col.clone(), p.sort_dir.clone()))
                }
                _ => None,
            })
            .collect();

        for (tab_id, sort_col, sort_dir) in targets {
            let op_id = self.state.next_op_id();
            let ts = self.state.tab_states.entry(tab_id).or_default();
            ts.load_state = LoadState::Loading;
            ts.last_nav_op_id = op_id;

            let tx = self.tx.clone();
            let show_hidden = self.state.settings.show_hidden;
            let paths = paths.clone();
            self.rt.spawn(async move {
                let entries = entries_for_paths(&paths, show_hidden, sort_col, sort_dir).await;
                let _ = tx.send(BgResult::DirListed {
                    tab: tab_id,
                    op_id,
                    entries,
                });
            });
        }
    }

    /// Load one Miller column's directory listing in the background.
    pub(crate) fn spawn_miller_list(&mut self, tile: TileId, column_path: PathBuf) {
        let op_id = self.state.next_op_id();
        let show_hidden = self.state.settings.show_hidden;
        let (col, dir) = self
            .tab_data_for(tile)
            .map(|td| (td.sort_col.clone(), td.sort_dir.clone()))
            .unwrap_or_default();
        if let Some(ts) = self.state.tab_states.get_mut(&tile) {
            if let Some(c) = ts.miller.columns.iter_mut().find(|c| c.path == column_path) {
                c.op_id = op_id;
                c.load_state = LoadState::Loading;
            }
        }
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let entries = list_dir(&column_path, show_hidden, col, dir)
                .await
                .unwrap_or_default();
            let _ = tx.send(BgResult::MillerListed {
                tab: tile,
                path: column_path,
                op_id,
                entries,
            });
        });
    }

    /// Select an item in a Miller column: focus that column, truncate deeper
    /// columns, and (for a folder) open a new child column.
    pub(crate) fn miller_select(&mut self, tile: TileId, column: usize, idx: usize) {
        self.state.focused_tile = tile;
        let entry = self
            .state
            .tab_states
            .get(&tile)
            .and_then(|ts| ts.miller.columns.get(column))
            .and_then(|c| c.entries.get(idx))
            .map(|e| (e.path.clone(), e.kind.clone()));
        let Some((path, kind)) = entry else {
            return;
        };
        let child_to_load = {
            let Some(ts) = self.state.tab_states.get_mut(&tile) else {
                return;
            };
            if column >= ts.miller.columns.len() {
                return;
            }
            ts.miller.focused = column;
            ts.miller.columns[column].selected = Some(idx);
            ts.miller.columns.truncate(column + 1);
            ts.selection.scroll_to_selected = true;
            if kind == crate::state::file_entry::FileKind::Dir {
                ts.miller
                    .columns
                    .push(crate::state::tab::MillerColumn::new(path.clone()));
                Some(path)
            } else {
                None
            }
        };
        if let Some(child) = child_to_load {
            self.spawn_miller_list(tile, child);
        }
    }

    pub(crate) fn load_preview(&mut self, ctx: &egui::Context, tab: TileId, path: PathBuf) {
        if self.state.preview.paths.get(&tab).map(PathBuf::as_path) == Some(path.as_path()) {
            return; // already loaded
        }
        self.state.preview.paths.insert(tab, path.clone());
        self.state.preview.results.remove(&tab);
        self.state.preview.textures.remove(&tab);
        self.state.preview.multi.remove(&tab);
        // Note: preview_paths was just inserted above; only results/textures cleared here
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        self.rt.spawn(async move {
            crate::fs::preview::load_preview(tab, path, tx).await;
            ctx.request_repaint();
        });
    }

    pub(crate) fn load_multi_preview(
        &mut self,
        ctx: &egui::Context,
        tab: TileId,
        key: crate::state::preview_cache::MultiPreviewKey,
        paths: Vec<PathBuf>,
    ) {
        if self
            .state
            .preview
            .multi
            .get(&tab)
            .is_some_and(|multi| multi.key == key)
        {
            return;
        }
        self.state.clear_preview(tab);
        self.state.preview.multi.insert(
            tab,
            MultiPreviewState {
                key: key.clone(),
                paths: paths.clone(),
                textures: Vec::with_capacity(paths.len()),
            },
        );

        for path in paths {
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            let key = key.clone();
            self.rt.spawn(async move {
                crate::fs::preview::load_multi_thumbnail(tab, key, path, tx).await;
                ctx.request_repaint();
            });
        }
    }

    pub(crate) fn sync_active_preview(&mut self, ctx: &egui::Context) {
        let tab = self.state.focused_tile;
        if !self
            .state
            .active_tab_data()
            .is_some_and(|data| data.preview_enabled)
        {
            self.pending_preview = None;
            return;
        }
        let request = self.state.active_preview_request();
        if let Some(request) = request {
            let already_loaded = match &request {
                PreviewRequest::Single(path) => self.state.preview.paths.get(&tab) == Some(path),
                PreviewRequest::Multiple { key, .. } => self
                    .state
                    .preview
                    .multi
                    .get(&tab)
                    .is_some_and(|multi| &multi.key == key),
            };
            if already_loaded {
                self.pending_preview = None;
                return;
            }

            let now = Instant::now();
            let is_same_pending = self
                .pending_preview
                .as_ref()
                .is_some_and(|pending| pending.tab == tab && pending.request == request);
            if !is_same_pending {
                self.state.clear_preview(tab);
                self.pending_preview = Some(PendingPreview {
                    tab,
                    request,
                    ready_at: now + PREVIEW_DEBOUNCE,
                });
            }

            let pending = self
                .pending_preview
                .as_ref()
                .expect("pending preview was just created");
            if pending.ready_at <= now {
                let pending = self
                    .pending_preview
                    .take()
                    .expect("pending preview still exists");
                match pending.request {
                    PreviewRequest::Single(path) => self.load_preview(ctx, pending.tab, path),
                    PreviewRequest::Multiple { key, paths } => {
                        self.load_multi_preview(ctx, pending.tab, key, paths);
                    }
                }
            } else {
                ctx.request_repaint_after(pending.ready_at - now);
            }
        } else {
            self.pending_preview = None;
            self.state.clear_preview(tab);
        }
    }

    pub(crate) fn spawn_open_files(&self, paths: Vec<PathBuf>) {
        self.rt.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                for path in paths {
                    let _ = open::that(path);
                }
            })
            .await;
        });
    }
}
