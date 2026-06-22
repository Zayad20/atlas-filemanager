//! Per-frame background drains: bg-result channel, palette close tick, DnD queue.
#![allow(clippy::too_many_lines)] // reason: cohesive startup/drain kept whole

use crate::state::layout_tree::{Tile, TileId};
use std::collections::HashSet;
use std::path::PathBuf;

use super::*;
use crate::state::app_state::BgResult;
use crate::state::preview_cache::ImageData;
use crate::state::tab::LoadState;

/// Return value from `process_bg_results`: tracks what needs re-rendering
/// without holding a `Context` reference. Callers apply notifications after
/// all state mutations are done and only when a Context is safely available.
pub(crate) struct BgDrainInfo {
    /// Whether the App shell (sidebar, toolbars, overlays) needs a re-render.
    pub changed: bool,
    /// Set to true when all panes need notification (overrides `notify_panes`).
    pub notify_all: bool,
    /// Specific pane IDs that should be notified (used when `notify_all` is false).
    pub notify_panes: Vec<TileId>,
}

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

    /// Remove panes whose close-out animation has elapsed.
    pub(crate) fn tick_pane_close(&mut self) -> bool {
        if self.pane_closing.is_empty() {
            return false;
        }
        let done: Vec<TileId> = self
            .pane_closing
            .iter()
            .filter(|(_, started)| started.elapsed() >= crate::app::PANE_CLOSE_FINALIZE)
            .map(|(id, _)| *id)
            .collect();
        if !done.is_empty() {
            let ages = done
                .iter()
                .filter_map(|id| {
                    self.pane_closing
                        .get(id)
                        .map(|started| started.elapsed().as_millis())
                })
                .collect::<Vec<_>>();
            eprintln!(
                "[atlas close] finalize_ready done={} ages_ms={:?} pending_before={}",
                done.len(),
                ages,
                self.pane_closing.len()
            );
        }
        for id in &done {
            self.pane_closing.remove(id);
        }
        if !done.is_empty() {
            crate::perf::event(|| format!("pane_close.finalize done={}", done.len()));
            self.remove_panes_from_tree(&done);
        }
        !done.is_empty()
    }

    /// Remove inner tabs whose close-out chip animation has elapsed.
    pub(crate) fn tick_tab_close(&mut self) -> bool {
        if self.tab_closing.is_empty() {
            return false;
        }
        let done: Vec<TileId> = self
            .tab_closing
            .iter()
            .filter(|(_, started)| started.elapsed() >= crate::app::TAB_CLOSE_FINALIZE)
            .map(|(id, _)| *id)
            .collect();
        for id in &done {
            self.tab_closing.remove(id);
        }
        if !done.is_empty() {
            self.remove_panes_from_tree(&done);
        }
        !done.is_empty()
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
                DndEvent::DragOver { x, y } => {
                    had = true;
                    self.drag_hover_set_cursor(gpui::point(gpui::px(x), gpui::px(y)));
                    continue;
                }
                DndEvent::DragLeave => {
                    had = true;
                    self.drag_hover_external_left();
                    continue;
                }
            };
            // A real drop landed inside Atlas: it's a deliberate release, not an
            // ESC-cancel, so keep any spring-load nav and tear the engine down here
            // (the synthetic mouse-up no longer clears it on Windows).
            self.mark_drag_committed();
            self.drag_hover.clear();
            if let Some(command) = self.drag_hover_drop_command_at(p.x, p.y) {
                had = true;
                self.execute_drag_hover_drop_command(command, p.paths, p.from_self);
                continue;
            }
            let dest = if p.x >= 0.0 && p.y >= 0.0 {
                if p.paths.is_empty() {
                    continue;
                }
                // Zone-based dest computed from the drop position. A drop over empty
                // space (no zone, no drag-hover target) is a no-op — never fall back
                // to the active pane dir, which silently transferred to the wrong place.
                self.dnd.as_ref().and_then(|d| {
                    d.zones
                        .lock()
                        .ok()
                        .and_then(|z| z.hit(p.x, p.y).map(|z| z.dest.clone()))
                })
            } else {
                // Synthetic position (no real cursor): fall back to the active dir.
                self.state
                    .active_tab_data()
                    .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
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

    /// Drain background results, update App state, then dispatch per-pane
    /// notifications via `signal.update`. Call this ONLY from outside a
    /// render cycle (e.g., the 60 Hz async drain loop). During render use
    /// `drain_bg_results_state_only` to avoid re-entering gpui's entity lease.
    pub(crate) fn drain_bg_results(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        let _perf = crate::perf::span("bg.drain");
        // Drain icon channel first.
        let icon_results: Vec<_> = self.icon_rx.try_iter().collect();
        let icon_len = icon_results.len();
        let icon_changed = !icon_results.is_empty();
        let mut changed = icon_changed;
        for (key, result) in icon_results {
            let state = match result {
                Some(tex) => crate::icon_theme::IconState::Ready(tex),
                None => crate::icon_theme::IconState::Missing,
            };
            self.icon_cache.insert(key, state);
        }

        let results: Vec<BgResult> = self.rx.try_iter().collect();
        let results_len = results.len();
        let info = self.process_bg_results(results);
        changed |= info.changed;
        let notify_all = self.dirty_all_panes || info.notify_all;
        let notify_panes_len = info.notify_panes.len();
        crate::perf::event(|| {
            format!(
                "bg.drain icons={icon_len} results={results_len} changed={changed} notify_all={notify_all} notify_panes={notify_panes_len} dirty={}",
                self.dirty_panes.len()
            )
        });

        // Apply pane notifications now that state mutation is done.
        if notify_all {
            self.dirty_all_panes = false;
            self.dirty_panes.clear();
            self.notify_all_panes(cx);
        } else if !info.notify_panes.is_empty() {
            let mut seen = std::collections::HashSet::new();
            for id in info.notify_panes {
                if seen.insert(id) {
                    self.notify_pane(id, cx);
                }
            }
        }
        // Flush any pane navigated this cycle whose listing isn't back yet, so the
        // route and the file view never drift apart (the normal-use path; the
        // drag-hover tick handles the modal-loop case where this drain is starved).
        if !self.dirty_panes.is_empty() {
            let dirty: Vec<TileId> = self.dirty_panes.drain().collect();
            for id in dirty {
                self.notify_pane(id, cx);
            }
        }
        changed
    }

    /// Drain background results and update App state without calling
    /// `signal.update`. Safe to call inside `App::render` because it never
    /// re-enters gpui's entity lease machinery.
    pub(crate) fn drain_bg_results_state_only(&mut self) -> bool {
        let _perf = crate::perf::span("bg.drain_state_only");
        let icon_results: Vec<_> = self.icon_rx.try_iter().collect();
        let icon_len = icon_results.len();
        let icon_changed = !icon_results.is_empty();
        let mut changed = icon_changed;
        for (key, result) in icon_results {
            let state = match result {
                Some(tex) => crate::icon_theme::IconState::Ready(tex),
                None => crate::icon_theme::IconState::Missing,
            };
            self.icon_cache.insert(key, state);
        }
        let results: Vec<BgResult> = self.rx.try_iter().collect();
        let results_len = results.len();
        let info = self.process_bg_results(results);
        changed |= info.changed;
        let notify_panes_len = info.notify_panes.len();
        crate::perf::event(|| {
            format!(
                "bg.drain_state_only icons={icon_len} results={results_len} changed={changed} notify_all={} notify_panes={notify_panes_len}",
                info.notify_all
            )
        });
        self.dirty_all_panes |= info.notify_all;
        self.dirty_panes.extend(info.notify_panes);
        changed
    }

    pub(crate) fn process_bg_results(&mut self, results: Vec<BgResult>) -> BgDrainInfo {
        let mut info = BgDrainInfo {
            changed: !results.is_empty(),
            notify_all: false,
            notify_panes: Vec::new(),
        };
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
                    let mut armed_appear = false;
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
                            // Fresh navigation/create load → arm the item appear anim.
                            if ts.pending_appear {
                                ts.appear_seq = ts.appear_seq.wrapping_add(1);
                                ts.pending_appear = false;
                                armed_appear = true;
                            }
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
                    if armed_appear {
                        self.pane_items_appear
                            .insert(tab, std::time::Instant::now());
                    }
                    for key in icon_keys {
                        self.request_icon(key);
                    }
                    info.notify_panes.push(tab);
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
                    info.notify_panes.push(tab);
                }
                BgResult::DirListError { tab, op_id, error } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if op_id == ts.last_nav_op_id {
                            ts.load_state = LoadState::Error(error);
                        }
                    }
                    info.notify_panes.push(tab);
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
                    info.notify_panes.push(tab);
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
                    info.notify_panes.push(tab);
                }
                BgResult::GridThumbnailReady {
                    path,
                    rgba,
                    width,
                    height,
                } => {
                    self.apply_grid_thumbnail(path, rgba, width, height);
                    info.notify_all = true;
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
                    info.notify_panes.push(tab);
                }
                BgResult::SearchInPlaceError { tab, gen, error } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if ts.search.generation == gen && ts.search.active {
                            ts.search.status = crate::state::tab::SearchStatus::Error;
                            ts.search.error = Some(error);
                        }
                    }
                    info.notify_panes.push(tab);
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
                    // refresh_tabs spawns new list_dir ops that produce DirListed
                    // events; notify all panes now so they can show loading state.
                    info.notify_all = true;
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
                            info.notify_all = true;
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
                    info.notify_all = true;
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
                    info.notify_all = true;
                }
                BgResult::PathsForTag { tag, paths } => {
                    self.populate_tag_tabs(tag, paths);
                    info.notify_all = true;
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
                    info.changed = true;
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
                    info.notify_panes.push(tab);
                }
                BgResult::ArchiveListError { tab, op_id, error } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if op_id == ts.last_nav_op_id {
                            ts.load_state = LoadState::Error(error);
                        }
                    }
                    info.notify_panes.push(tab);
                }
                BgResult::GridThumbnailSkipped { path } => {
                    self.apply_thumb_skipped(path);
                    info.notify_all = true;
                }
                BgResult::ExtToolInstalled {
                    tool,
                    tab,
                    path,
                    result,
                } => match result {
                    Ok(()) => {
                        self.reload_ext_registry();
                        self.state.preview.paths.remove(&tab);
                        self.load_preview_gpui(tab, path);
                        info.notify_panes.push(tab);
                    }
                    Err(error) => {
                        self.state.preview.results.insert(
                            tab,
                            crate::state::preview_cache::PreviewResult::Error(format!(
                                "Could not install {}: {}",
                                crate::ui::inspector::tool_label(&tool),
                                error
                            )),
                        );
                        info.notify_panes.push(tab);
                    }
                },
                BgResult::AppUpdateAvailable { version, url } => {
                    self.toast_info(format!("Atlas {version} is available: {url}"));
                }
                BgResult::ExtRegistryFetched(_)
                | BgResult::ExtRemoteInstalled { .. }
                | BgResult::ExtLocalInstalled(_) => {}
                #[cfg(feature = "cloud")]
                other => self.handle_cloud_bg(other),
            }
        }
        info
    }
}
