//! Per-frame background drains: bg-result channel, palette close tick, DnD queue.
#![allow(clippy::too_many_lines)] // reason: cohesive startup/drain kept whole

use crate::state::layout_tree::{Tile, TileId};
use std::collections::HashSet;
use std::path::PathBuf;


use crate::state::app_state::BgResult;
use crate::state::preview_cache::ImageData;
use crate::state::tab::LoadState;
use super::*;

impl App {
    /// Tear down a dismissed palette once its exit animation has elapsed. Returns
    /// `true` when it finalized (so the caller repaints to drop the now-hidden view).
    pub(crate) fn tick_palette_close(&mut self) -> bool {
        if let Some((kind, started)) = self.palette_closing {
            if started.elapsed() >= PALETTE_CLOSE_FINALIZE {
                self.palette_closing = None;
                match kind {
                    crate::ui::palette::PaletteKind::Goto => self.goto_input = None,
                    crate::ui::palette::PaletteKind::Commands => self.palette_input = None,
                    crate::ui::palette::PaletteKind::Ext => self.ext_palette_input = None,
                }
                self.palette_host = None;
                return true;
            }
        }
        false
    }

    /// Apply every queued background result to state. Returns `true` if at least
    /// one result was processed (i.e. a repaint is warranted).
    /// Drain the Windows OLE drop-target action queue and route payloads to
    /// `process_actions`. Called in the foreground drain loop so drops appear
    /// on the next render after the COM vtable returns.
    #[cfg(windows)]
    pub(crate) fn drain_dnd_queue(&mut self) -> bool {
        use crate::dnd::windows::DndEvent;
        use crate::input::action::Action;
        use crate::state::ops::{ConflictPolicy, TransferKind, TransferOrigin, TransferRequest};

        let Some(dnd) = &self.dnd else { return false };
        let events = {
            let Ok(mut q) = dnd.action_queue.lock() else {
                return false;
            };
            std::mem::take(&mut *q)
        };
        if events.is_empty() {
            return false;
        }
        let mut had = false;
        let mut actions = Vec::with_capacity(events.len());
        for event in events {
            let p = match event {
                DndEvent::DropIn(p) => p,
                DndEvent::DragOutMoved { paths } => {
                    // Drag out of Atlas accepted as MOVE by the target: the
                    // sources go to the trash (recoverable). Files already gone
                    // surface as visible toasts via delete_files.
                    had = true;
                    self.delete_files(paths, true);
                    continue;
                }
                DndEvent::Error(msg) => {
                    had = true;
                    self.toast_error(msg);
                    continue;
                }
            };
            // Resolve destination: use the App's focused dir as fallback.
            let dest = if p.x >= 0.0 && p.y >= 0.0 {
                // Drop target already computed a zone-based dest inside the COM vtable;
                // we trust it. If it is empty, fall back to focused tab dir.
                if p.paths.is_empty() {
                    continue;
                }
                // Re-check zones: the payload carries the logical drop position.
                let dest = self.dnd.as_ref().and_then(|d| {
                    d.zones
                        .lock()
                        .ok()
                        .and_then(|z| z.hit(p.x, p.y).map(|z| z.dest.clone()))
                });
                dest.or_else(|| self.state.active_tab_data().and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf())))
            } else {
                self.state.active_tab_data().and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
            };
            let Some(dest) = dest else { continue };
            // Vtable always enqueues Copy; decide move vs copy here by volume.
            // Conflicts always go through the dialog (policy `Ask`) — a drop
            // must never overwrite or vanish silently.
            let vol_effect = crate::dnd::default_effect_for_paths(&p.paths, &dest);
            actions.push(Action::Transfer(TransferRequest {
                sources: p.paths,
                dest,
                kind: if vol_effect.is_move() {
                    TransferKind::Move
                } else {
                    TransferKind::Copy
                },
                policy: ConflictPolicy::Ask,
                origin: if p.from_self {
                    TransferOrigin::DndInternal
                } else {
                    TransferOrigin::DndExternal
                },
            }));
        }
        let had = had || !actions.is_empty();
        self.process_actions(actions);
        had
    }

    pub(crate) fn drain_bg_results(&mut self) -> bool {
        // Drain icon channel first (dedicated channel, Arc<RenderImage> direct).
        let icon_results: Vec<_> = self.icon_rx.try_iter().collect();
        let mut changed = !icon_results.is_empty();
        for (key, result) in icon_results {
            let state = match result {
                Some(tex) => crate::icon_theme::IconState::Ready(tex),
                None => crate::icon_theme::IconState::Missing,
            };
            self.icon_cache.insert(key, state);
        }

        let results: Vec<BgResult> = self.rx.try_iter().collect();
        changed |= !results.is_empty();
        for result in results {
            match result {
                BgResult::DirListed {
                    tab,
                    op_id,
                    entries,
                } => {
                    // Collect icon keys before moving `entries` so we can warm
                    // the icon cache for the new listing.
                    let icon_keys: Vec<String> = if self.icon_theme.is_some() {
                        crate::icon_theme::collect_keys(&entries)
                    } else {
                        vec![]
                    };
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
                    // Borrow of tab_states released — safe to request icons.
                    for key in icon_keys {
                        self.request_icon(key);
                    }
                }
                BgResult::MillerListed {
                    tab,
                    location,
                    op_id,
                    entries,
                } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if let Some(c) = ts
                            .miller
                            .columns
                            .iter_mut()
                            .find(|c| c.location == location && c.op_id == op_id)
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
                    let parent = changed_path.parent().map(std::path::Path::to_path_buf);
                    let to_refresh: Vec<TileId> = self
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
                            if let Some(path) = p.location.fs_dir().map(|p| p.to_path_buf()) {
                                let col = p.sort_col.clone();
                                let dir = p.sort_dir.clone();
                                self.spawn_list_dir(tile_id, path, col, dir);
                            }
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
                    multi.images.push((
                        path,
                        ImageData {
                            rgba,
                            width,
                            height,
                        },
                    ));
                }
                BgResult::GridThumbnailReady {
                    path,
                    rgba,
                    width,
                    height,
                } => {
                    self.apply_grid_thumbnail(path, rgba, width, height);
                }

                BgResult::SearchInPlaceResults { tab, gen, entries } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if ts.search.generation == gen
                            && ts.search.active
                            && ts.search.recursive
                            && ts.search.is_filtering()
                        {
                            ts.set_entries(entries);
                            ts.filter.clear();
                            ts.search.status = crate::state::tab::SearchStatus::Ready;
                            ts.search.error = None;
                            ts.selection.clear();
                            if !ts.entries.is_empty() {
                                ts.selection.selected.insert(0);
                                ts.selection.keyboard_cursor = Some(0);
                                ts.selection.selection_anchor = Some(0);
                                ts.selection.last_clicked_index = Some(0);
                                ts.selection.scroll_to_selected = true;
                            }
                        }
                    }
                }
                BgResult::SearchInPlaceError { tab, gen, error } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if ts.search.generation == gen && ts.search.active {
                            ts.search.status = crate::state::tab::SearchStatus::Error;
                            ts.search.error = Some(error);
                        }
                    }
                }
                BgResult::OpComplete {
                    id,
                    result,
                    affected_tabs,
                } => {
                    self.state.ops.active_ops.remove(&id);
                    if let Err(e) = result {
                        tracing::error!("Background operation failed: {}", e);
                        // Multi-line payloads are one error per item; toast each.
                        for line in e.lines().filter(|l| !l.trim().is_empty()) {
                            self.toast_error(line.to_string());
                        }
                    }
                    self.refresh_tabs(affected_tabs);
                }
                BgResult::ClipboardPerformed { effect, paths } => {
                    // A paste target consumed our clipboard cut/copy.
                    // effect 2 (MOVE) = unoptimized move: we must trash sources.
                    // effect 0 (NONE) = optimized move: target already moved them.
                    // effect 1 (COPY) = plain copy: nothing to do.
                    if effect == 2 || effect == 0 {
                        let was_cut = self.state.ops.clipboard.as_ref().is_some_and(|c| c.is_cut);
                        if was_cut {
                            if effect == 2 {
                                // Quietly skip already-gone files: an optimized
                                // mover may still report MOVE for some targets.
                                let existing: Vec<_> =
                                    paths.iter().filter(|p| p.exists()).cloned().collect();
                                if !existing.is_empty() {
                                    self.delete_files(existing, true);
                                }
                            }
                            self.state.ops.clipboard = None;
                            #[cfg(windows)]
                            crate::dnd::windows::ole_clipboard::clear();
                            let affected: std::collections::HashSet<_> = paths
                                .iter()
                                .flat_map(|p| self.affected_tabs_for_path(p))
                                .collect();
                            self.refresh_tabs(affected.into_iter().collect());
                        }
                    }
                }
                BgResult::TransferComplete {
                    id,
                    kind,
                    origin,
                    report,
                    affected_tabs,
                } => {
                    self.state.ops.active_ops.remove(&id);
                    for err in &report.errors {
                        tracing::error!("transfer: {}", err);
                        self.toast_error(err.to_string());
                    }
                    // Files cut in another app and pasted here: report the move
                    // back so the source clears its cut state — only when
                    // everything actually moved.
                    #[cfg(windows)]
                    if origin == crate::state::ops::TransferOrigin::PasteExternalCut
                        && report.errors.is_empty()
                        && !report.completed.is_empty()
                    {
                        crate::dnd::windows::ole_clipboard::acknowledge_external_cut();
                    }
                    if report.cancelled {
                        self.toast_error(format!(
                            "Operación cancelada ({} completados)",
                            report.completed.len()
                        ));
                    }
                    if !report.completed.is_empty() {
                        // Tags follow files moved within the app (real pairs,
                        // so this survives conflict renames).
                        if kind == crate::state::ops::TransferKind::Move {
                            for (src, dst) in &report.completed {
                                let _ =
                                    self.tag_db_tx
                                        .send(crate::fs::tag_db::TagDbCmd::RenamePath {
                                            old: src.clone(),
                                            new: dst.clone(),
                                        });
                            }
                        }
                        // Undo records what actually happened; undo/redo runs
                        // don't re-record themselves.
                        if origin != crate::state::ops::TransferOrigin::Undo {
                            let entry = match kind {
                                crate::state::ops::TransferKind::Move => {
                                    crate::state::undo::UndoEntry::Move {
                                        pairs: report.completed.clone(),
                                    }
                                }
                                crate::state::ops::TransferKind::Copy => {
                                    crate::state::undo::UndoEntry::Copy {
                                        pairs: report.completed.clone(),
                                    }
                                }
                            };
                            self.state.ops.undo_stack.push(entry);
                        }
                    }
                    self.refresh_tabs(affected_tabs);
                }
                BgResult::OpProgress {
                    id,
                    done,
                    total,
                    bytes_done,
                    bytes_total,
                    current_file,
                } => {
                    if let Some(op) = self.state.ops.active_ops.get_mut(&id) {
                        op.done = done;
                        op.total = total;
                        op.bytes_done = bytes_done;
                        op.bytes_total = bytes_total;
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
                BgResult::ContentResults { gen, mut paths } => {
                    // Drop stale batches from a superseded query generation.
                    if gen == self.content_gen {
                        self.content_results.append(&mut paths);
                    }
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
                BgResult::ExtShowQuickPick { items, reply_tx } => {
                    self.state.overlay.show_ext_palette = true;
                    self.state.overlay.ext_palette_items = items;
                    self.state.overlay.ext_palette_reply = Some(reply_tx);
                    self.state.overlay.ext_palette_state.reset();
                    // Fresh input + host so the pick opens empty, re-subscribed
                    // and with the open animation (mirrors OpenPalette/Goto).
                    self.ext_palette_input = None;
                    self.palette_host = None;
                    self.palette_closing = None;
                    changed = true;
                }
                BgResult::ArchiveListed {
                    tab,
                    op_id,
                    entries,
                    ..
                } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if op_id == ts.last_nav_op_id {
                            ts.set_entries(entries);
                            ts.load_state = LoadState::Loaded;
                        }
                    }
                }
                BgResult::ArchiveListError { tab, op_id, error } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if op_id == ts.last_nav_op_id {
                            ts.load_state = LoadState::Error(error);
                        }
                    }
                }
                #[cfg(feature = "cloud")]
                other => self.handle_cloud_bg(other),
            }
        }
        changed
    }

}
