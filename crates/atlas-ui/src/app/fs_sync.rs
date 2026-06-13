#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use crate::app::App;
use crate::fs::list::{entries_for_paths, list_dir};
use crate::state::app_state::BgResult;
use crate::state::layout_tree::{Tile, TileId};
use crate::state::location::Location;
use crate::state::tab::LoadState;
use crate::state::tag_cache::TagId;
use std::path::PathBuf;

impl App {
    pub(crate) fn spawn_list_dir(
        &mut self,
        tab_id: TileId,
        path: PathBuf,
        sort_col: crate::state::file_entry::SortColumn,
        sort_dir: crate::state::file_entry::SortDir,
    ) {
        let folders_first = self
            .tab_data_for(tab_id)
            .is_none_or(|td| td.folders_first);
        let op_id = self.state.next_op_id();
        let ts = self.state.tab_states.entry(tab_id).or_default();
        ts.load_state = LoadState::Loading;
        ts.last_nav_op_id = op_id;

        let tx = self.tx.clone();
        let show_hidden = self.state.settings.show_hidden;
        self.rt.spawn(async move {
            match list_dir(&path, show_hidden, sort_col, sort_dir, folders_first).await {
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

    /// Re-sort the already-loaded listing of `tab_id` in place (no re-listing),
    /// remapping the selection by path. Used by the folders-first toggle so the
    /// pane reorders instantly.
    pub(crate) fn resort_entries_in_place(
        &mut self,
        tab_id: TileId,
        sort_col: &crate::state::file_entry::SortColumn,
        sort_dir: &crate::state::file_entry::SortDir,
        folders_first: bool,
    ) {
        let Some(ts) = self.state.tab_states.get_mut(&tab_id) else {
            return;
        };
        let prev_selected: std::collections::HashSet<PathBuf> = ts
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

        let mut entries = std::mem::take(&mut ts.entries);
        crate::fs::list::sort_entries(&mut entries, sort_col, sort_dir, folders_first);
        ts.set_entries(entries);

        ts.selection.selected = ts
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| prev_selected.contains(&e.path))
            .map(|(i, _)| i)
            .collect();
        ts.selection.keyboard_cursor =
            prev_cursor.and_then(|p| ts.entries.iter().position(|e| e.path == p));
        ts.selection.selection_anchor = ts.selection.keyboard_cursor;
        ts.selection.last_clicked_index = ts.selection.keyboard_cursor;
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
            bool,
        )> = self
            .state
            .pane_tree
            .tiles
            .iter()
            .filter_map(|(id, tile)| match tile {
                Tile::Pane(p) if p.location == Location::Tag(tag) => {
                    Some((*id, p.sort_col.clone(), p.sort_dir.clone(), p.folders_first))
                }
                _ => None,
            })
            .collect();

        for (tab_id, sort_col, sort_dir, folders_first) in targets {
            let op_id = self.state.next_op_id();
            let ts = self.state.tab_states.entry(tab_id).or_default();
            ts.load_state = LoadState::Loading;
            ts.last_nav_op_id = op_id;

            let tx = self.tx.clone();
            let show_hidden = self.state.settings.show_hidden;
            let paths = paths.clone();
            self.rt.spawn(async move {
                let entries =
                    entries_for_paths(&paths, show_hidden, sort_col, sort_dir, folders_first)
                        .await;
                let _ = tx.send(BgResult::DirListed {
                    tab: tab_id,
                    op_id,
                    entries,
                });
            });
        }
    }

    /// Load one Miller column's listing in the background.
    pub(crate) fn spawn_miller_list(&mut self, tile: TileId, location: Location) {
        let op_id = self.state.next_op_id();
        let show_hidden = self.state.settings.show_hidden;
        let (col, dir, folders_first) = self
            .tab_data_for(tile)
            .map(|td| (td.sort_col.clone(), td.sort_dir.clone(), td.folders_first))
            .unwrap_or((Default::default(), Default::default(), true));
        if let Some(ts) = self.state.tab_states.get_mut(&tile) {
            if let Some(c) = ts.miller.columns.iter_mut().find(|c| c.location == location) {
                c.op_id = op_id;
                c.load_state = LoadState::Loading;
            }
        }
        let tx = self.tx.clone();
        let loc = location.clone();
        self.rt.spawn(async move {
            let entries = match loc.clone() {
                Location::Directory(path) => {
                    list_dir(&path, show_hidden, col, dir, folders_first)
                        .await
                        .unwrap_or_default()
                }
                Location::Archive { archive, inner } => {
                    tokio::task::spawn_blocking(move || {
                        crate::fs::archive::open(&archive)
                            .and_then(|p| p.list(&inner).ok())
                            .unwrap_or_default()
                    })
                    .await
                    .unwrap_or_default()
                }
                _ => Vec::new(),
            };
            let _ = tx.send(BgResult::MillerListed {
                tab: tile,
                location,
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
            .map(|e| (e.path.clone(), e.kind.clone(), e.archive_root.clone()));
        let Some((path, kind, archive_root)) = entry else {
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
            ts.miller.columns[column].selection.selected.clear();
            ts.miller.columns[column].selection.selected.insert(idx);
            ts.miller.columns[column].selection.keyboard_cursor = Some(idx);
            ts.miller.columns.truncate(column + 1);
            ts.selection.scroll_to_selected = true;
            if kind == crate::state::file_entry::FileKind::Dir {
                let child_loc = if let Some(archive) = archive_root {
                    // Dir inside an archive — navigate as virtual path.
                    Location::Archive { archive, inner: path.clone() }
                } else {
                    Location::Directory(path.clone())
                };
                ts.miller
                    .columns
                    .push(crate::state::tab::MillerColumn::new(child_loc.clone()));
                Some(child_loc)
            } else if crate::fs::archive::is_archive_path(&path) {
                let child_loc = Location::Archive {
                    archive: path.clone(),
                    inner: std::path::PathBuf::new(),
                };
                ts.miller
                    .columns
                    .push(crate::state::tab::MillerColumn::new(child_loc.clone()));
                Some(child_loc)
            } else {
                None
            }
        };
        if let Some(child_loc) = child_to_load {
            self.spawn_miller_list(tile, child_loc);
        }
    }

    /// List a directory inside an archive file in the background.
    pub(crate) fn spawn_list_archive(
        &mut self,
        tab_id: TileId,
        archive: PathBuf,
        inner: PathBuf,
        sort_col: crate::state::file_entry::SortColumn,
        sort_dir: crate::state::file_entry::SortDir,
    ) {
        let folders_first = self
            .tab_data_for(tab_id)
            .is_none_or(|td| td.folders_first);
        let op_id = self.state.next_op_id();
        let ts = self.state.tab_states.entry(tab_id).or_default();
        ts.load_state = LoadState::Loading;
        ts.last_nav_op_id = op_id;

        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = tokio::task::spawn_blocking({
                let archive = archive.clone();
                let inner = inner.clone();
                move || -> Result<Vec<atlas_model::file_entry::FileEntry>, String> {
                    let provider = crate::fs::archive::open(&archive)
                        .ok_or_else(|| rust_i18n::t!("app_archive_unsupported").to_string())?;
                    let mut entries = provider.list(&inner).map_err(|e| e.to_string())?;
                    crate::fs::list::sort_entries(&mut entries, &sort_col, &sort_dir, folders_first);
                    Ok(entries)
                }
            })
            .await;

            let send_result = match result {
                Ok(Ok(entries)) => crate::state::app_state::BgResult::ArchiveListed {
                    tab: tab_id,
                    op_id,
                    archive,
                    inner,
                    entries,
                },
                Ok(Err(e)) => crate::state::app_state::BgResult::ArchiveListError {
                    tab: tab_id,
                    op_id,
                    error: e,
                },
                Err(e) => crate::state::app_state::BgResult::ArchiveListError {
                    tab: tab_id,
                    op_id,
                    error: e.to_string(),
                },
            };
            let _ = tx.send(send_result);
        });
    }

    /// Unified listing dispatcher used by `navigate_to`. Gets sort from pane data.
    pub(crate) fn spawn_list(&mut self, tab_id: TileId, loc: Location) {
        let (col, dir) = self
            .tab_data_for(tab_id)
            .map(|td| (td.sort_col.clone(), td.sort_dir.clone()))
            .unwrap_or_default();
        self.dispatch_list(tab_id, loc, col, dir);
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

    /// Extract selected virtual archive entries to `dest`.
    /// `items` = (`archive_path`, `inner_path`) pairs.
    ///
    /// Runs as a tracked operation: per-item progress, cancellation, and every
    /// failure reported via `OpComplete`. Dirs are extracted recursively.
    /// Existing names at `dest` are auto-renamed ("name (1)") — extraction
    /// never overwrites silently.
    pub(crate) fn extract_archive_items(
        &mut self,
        _tab_id: TileId,
        items: Vec<(PathBuf, PathBuf)>,
        dest: PathBuf,
    ) {
        let affected = self.affected_tabs_for_path(&dest);
        let (op_id, cancel) = self
            .state
            .ops
            .start_op_labeled(items.len(), rust_i18n::t!("fs_sync_extracting").to_string());
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                let total = items.len() as u64;
                let mut errors: Vec<String> = Vec::new();
                let mut bytes_done = 0u64;
                // Group by archive to open each file only once.
                let mut by_archive: std::collections::HashMap<PathBuf, Vec<PathBuf>> =
                    std::collections::HashMap::new();
                for (archive, inner) in items {
                    by_archive.entry(archive).or_default().push(inner);
                }
                let mut done = 0u64;
                'outer: for (archive_path, inners) in by_archive {
                    let Some(provider) = crate::fs::archive::open(&archive_path) else {
                        errors.push(rust_i18n::t!(
                            "fs_sync_open_err",
                            path = archive_path.file_name().unwrap_or_default().to_string_lossy()
                        ).to_string());
                        continue;
                    };
                    for inner in inners {
                        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                            errors.push(rust_i18n::t!("fs_sync_extract_cancelled", done = done, total = total).to_string());
                            break 'outer;
                        }
                        let name = inner
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "extracted".to_string());
                        let _ = tx.send(BgResult::OpProgress {
                            id: op_id,
                            done,
                            total,
                            bytes_done,
                            bytes_total: 0,
                            current_file: PathBuf::from(&name),
                        });
                        let out_base = crate::fs::transfer::unique_target(
                            &dest,
                            std::ffi::OsStr::new(&name),
                        );
                        let written = extract_entry_recursive(
                            provider.as_ref(),
                            &inner,
                            &out_base,
                            &cancel,
                            &mut errors,
                        );
                        bytes_done += written;
                        done += 1;
                    }
                }
                let result = if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors.join("\n"))
                };
                let _ = tx.send(BgResult::OpComplete {
                    id: op_id,
                    result,
                    affected_tabs: affected,
                });
            })
            .await;
        });
    }
}

/// Recursively extract `inner` from `provider` into `dest_path`.
/// If `inner` is a directory, creates `dest_path/` and extracts all children.
/// Returns total bytes written.
fn extract_entry_recursive(
    provider: &dyn crate::fs::archive::ArchiveProvider,
    inner: &std::path::Path,
    dest_path: &std::path::Path,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    errors: &mut Vec<String>,
) -> u64 {
    use std::io::Read as _;
    use crate::fs::archive::sanitize::sanitize_inner_path;
    use crate::fs::archive::bomb_guard::BombGuard;

    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return 0;
    }

    // Check if this is a directory by listing it.
    match provider.list(inner) {
        Ok(children) if !children.is_empty() => {
            // It's a non-empty directory — create it and recurse.
            #[allow(clippy::disallowed_methods)]
            let _ = std::fs::create_dir_all(dest_path);
            let mut total = 0u64;
            for child in children {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let child_name = sanitize_inner_path(std::path::Path::new(&child.name))
                    .and_then(|p| p.file_name().map(|n| n.to_os_string()))
                    .unwrap_or_else(|| child.name.clone().into());
                let child_dest = dest_path.join(&child_name);
                total += extract_entry_recursive(
                    provider,
                    &child.path,
                    &child_dest,
                    cancel,
                    errors,
                );
            }
            total
        }
        _ => {
            // Treat as a file (or empty dir — attempt extract, create dir if that fails).
            let mut reader = match provider.read_bytes(inner) {
                Ok(r) => r,
                Err(_) => {
                    // Likely an empty directory entry — create it and move on.
                    #[allow(clippy::disallowed_methods)]
                    let _ = std::fs::create_dir_all(dest_path);
                    return 0;
                }
            };
            if let Some(parent) = dest_path.parent() {
                #[allow(clippy::disallowed_methods)]
                let _ = std::fs::create_dir_all(parent);
            }
            let mut out = match std::fs::File::create(dest_path) {
                Ok(f) => f,
                Err(e) => {
                    errors.push(rust_i18n::t!(
                        "fs_sync_create_err",
                        path = dest_path.file_name().unwrap_or_default().to_string_lossy(),
                        e = e
                    ).to_string());
                    return 0;
                }
            };
            let mut guard = BombGuard::default();
            let mut buf = [0u8; 65536];
            let mut total = 0u64;
            loop {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    drop(out);
                    #[allow(clippy::disallowed_methods)]
                    let _ = std::fs::remove_file(dest_path);
                    return total;
                }
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if guard.check_chunk(n as u64).is_err() {
                            errors.push(rust_i18n::t!(
                                "fs_sync_zip_bomb",
                                path = dest_path.file_name().unwrap_or_default().to_string_lossy()
                            ).to_string());
                            drop(out);
                            #[allow(clippy::disallowed_methods)]
                            let _ = std::fs::remove_file(dest_path);
                            return total;
                        }
                        use std::io::Write as _;
                        if let Err(e) = out.write_all(&buf[..n]) {
                            errors.push(rust_i18n::t!(
                                "fs_sync_write_err",
                                path = dest_path.file_name().unwrap_or_default().to_string_lossy(),
                                e = e
                            ).to_string());
                            return total;
                        }
                        total += n as u64;
                    }
                    Err(e) => {
                        errors.push(rust_i18n::t!(
                            "fs_sync_read_err",
                            path = dest_path.file_name().unwrap_or_default().to_string_lossy(),
                            e = e
                        ).to_string());
                        return total;
                    }
                }
            }
            total
        }
    }
}
