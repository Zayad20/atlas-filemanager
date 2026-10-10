//! Per-frame background drains: bg-result channel, palette close tick, DnD queue.
#![allow(clippy::too_many_lines)] // reason: cohesive startup/drain kept whole

use atlas_model::layout_tree::{Tile, TileId};
use atlas_model::overlay::{Toast, ToastKind};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use super::*;
use atlas_model::app_state::BgResult;
use atlas_model::preview_cache::ImageData;
use atlas_model::tab::LoadState;

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
    pub(crate) fn tick_eco_mode(&mut self, window: &mut gpui::Window) {
        if !self.state.settings.eco_mode {
            return;
        }

        let is_active = window.is_window_active();

        if !is_active && !self.was_inactive {
            self.was_inactive = true;
            self.inactive_since = Some(std::time::Instant::now());
        } else if is_active {
            self.was_inactive = false;
            self.inactive_since = None;
            self.eco_cleared = false;
        }

        if let Some(since) = self.inactive_since {
            if since.elapsed() > std::time::Duration::from_secs(5) && !self.eco_cleared {
                self.eco_cleared = true;

                // Evict large GPU caches
                self.preview_textures.clear();
                self.multi_textures.clear();
                self.thumb_textures.clear();

                // Shrink FS allocations
                for tab in self.state.tab_states.values_mut() {
                    tab.entries = tab.entries.clone().into();
                }

                tracing::info!("eco_mode: Inactive for 5s. Evicted buffers & ran shrink_to_fit.");
            }
        }
    }
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
            .filter(|(id, started)| {
                started.elapsed() >= crate::app::PANE_CLOSE_FINALIZE
                    && self.state.find_tabs_container(**id)
                        .and_then(|tabs| self.state.find_parent_of(tabs))
                        .and_then(|parent| self.dock_split_animations.get(&parent).map(|animation| (parent, animation)))
                        .is_none_or(|(parent, animation)| {
                            animation.done() || matches!(self.state.pane_tree.tiles.get(parent),
                                Some(Tile::Container(Container::Linear(linear))) if
                                    linear.children.iter().position(|child| self.state.container_contains_pane(*child, **id))
                                        .and_then(|ix| animation.shares().get(ix).copied())
                                        .is_some_and(|share| share <= f32::EPSILON))
                        })
            })
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
            crate::perf::event(|| {
                format!(
                    "pane_close.finalize_ready done={} ages_ms={:?} pending_before={}",
                    done.len(),
                    ages,
                    self.pane_closing.len()
                )
            });
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

    pub(crate) fn tick_toasts(&mut self) -> bool {
        let before = self.state.overlay.op_errors.len();
        self.state.overlay.op_errors.retain(toast_alive);
        before != self.state.overlay.op_errors.len()
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
                    // target copied/moved the data; remove only sources that
                    // still exist. Already-gone paths mean the target optimized
                    // the move itself.
                    had = true;
                    let existing: Vec<_> = paths.into_iter().filter(|p| p.exists()).collect();
                    if !existing.is_empty() {
                        self.delete_files(existing, false);
                    }
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
                    self.cancel_dropzone_file_drag();
                    self.drag_hover_external_left();
                    continue;
                }
            };
            // A real drop landed inside Atlas: it's a deliberate release, not an
            // ESC-cancel, so keep any spring-load nav and tear the engine down here
            // (the synthetic mouse-up no longer clears it on Windows).
            tracing::info!(
                x = p.x,
                y = p.y,
                paths = p.paths.len(),
                from_self = p.from_self,
                "dnd drop in"
            );
            // Resolve before ending hover state. OLE has already ended its
            // drag; the target must still identify the tab under the cursor.
            let tab_target = self.tab_drop_target_at(p.x, p.y);
            let hover_command = self.drag_hover_drop_command_at(p.x, p.y);
            if tab_target.is_some() && hover_command.is_none() {
                tracing::warn!(tab = ?tab_target, "dnd drop ignored: tab no longer accepts files");
                self.cancel_dropzone_file_drag();
                self.drag_hover_external_left();
                had = true;
                continue;
            }
            self.mark_drag_committed();
            self.drag_hover.clear();
            self.active_external_drag = None;
            if self.dropzone_bounds.is_some_and(|bounds| {
                p.x >= f32::from(bounds.left())
                    && p.x <= f32::from(bounds.right())
                    && p.y >= f32::from(bounds.top())
                    && p.y <= f32::from(bounds.bottom())
            }) {
                tracing::debug!("dnd drop resolved by dropzone bounds");
                had = true;
                self.execute_drag_hover_drop_command(
                    crate::ui::drag_hover::DragHoverDropCommand::StageInSidebar,
                    p.paths,
                    p.from_self,
                );
                continue;
            }
            if let Some(command) = hover_command {
                tracing::debug!("dnd drop resolved by drag-hover command");
                had = true;
                self.execute_drag_hover_drop_with_modifiers(command, p.paths, p.from_self, p.ctrl, p.shift);
                continue;
            }
            let dest = if p.x >= 0.0 && p.y >= 0.0 {
                if p.paths.is_empty() {
                    continue;
                }
                // Zone-based dest computed from the drop position. A drop over empty
                // space (no zone, no drag-hover target) is a no-op — never fall back
                // to the active pane dir, which silently transferred to the wrong place.
                let pane_dest = self.pane_drop_dest_at(p.x, p.y);
                let (zone_pane, zone_count) = self
                    .dnd
                    .as_ref()
                    .and_then(|d| d.zones.lock().ok())
                    .map(|z| (z.hit(p.x, p.y).map(|z| TileId(z.pane_id)), z.len()))
                    .unwrap_or((None, 0));
                let zone_dest = zone_pane.filter(|pane| self.pane_accepts_visible_drop(*pane))
                    .and_then(|pane| self.tab_data_for(pane))
                    .and_then(|tab| tab.location.fs_dir().map(|path| path.to_path_buf()));
                tracing::info!(
                    pane_hit = pane_dest.is_some(),
                    zone_hit = zone_dest.is_some(),
                    zone_count,
                    pane_view_bounds = self.pane_view_bounds.len(),
                    pane_layout_bounds = self.pane_layout_bounds.len(),
                    "dnd drop destination resolved"
                );
                pane_dest.or(zone_dest)
            } else {
                // Synthetic position (no real cursor): fall back to the active dir.
                self.state
                    .active_tab_data()
                    .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
            };
            let Some(dest) = dest else {
                let dropzone = self.dropzone_bounds.map(|b| {
                    (
                        f32::from(b.left()),
                        f32::from(b.top()),
                        f32::from(b.right()),
                        f32::from(b.bottom()),
                    )
                });
                let nearest_pane = self
                    .pane_view_bounds
                    .iter()
                    .chain(self.pane_layout_bounds.iter())
                    .map(|(pane, b)| {
                        let left = f32::from(b.left());
                        let top = f32::from(b.top());
                        let right = f32::from(b.right());
                        let bottom = f32::from(b.bottom());
                        let dx = if p.x < left {
                            left - p.x
                        } else if p.x > right {
                            p.x - right
                        } else {
                            0.0
                        };
                        let dy = if p.y < top {
                            top - p.y
                        } else if p.y > bottom {
                            p.y - bottom
                        } else {
                            0.0
                        };
                        (dx + dy, pane.0, (left, top, right, bottom))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                tracing::warn!(
                    x = p.x,
                    y = p.y,
                    paths = p.paths.len(),
                    ?dropzone,
                    ?nearest_pane,
                    pane_view_bounds = self.pane_view_bounds.len(),
                    pane_layout_bounds = self.pane_layout_bounds.len(),
                    "dnd drop ignored: no destination"
                );
                continue;
            };
            // Vtable always enqueues Copy; decide move vs copy here by volume.
            // Conflicts always go through the dialog (policy `Ask`) — a drop
            // must never overwrite or vanish silently.
            self.cancel_dropzone_file_drag();
            actions.push(Action::Transfer(crate::ui::drag_hover::drop_transfer_with_modifiers(p.paths, dest, p.from_self, p.ctrl, p.shift)));
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
        let start = std::time::Instant::now();
        let max_duration = if self.is_pane_or_dock_animating() {
            std::time::Duration::from_micros(500)
        } else {
            std::time::Duration::from_millis(2)
        };

        // Drain icon channel first.
        let mut icon_results = Vec::new();
        for res in self.icon_rx.try_iter() {
            if start.elapsed() > max_duration {
                break;
            }
            icon_results.push(res);
        }
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

        let mut results_len = 0;
        let mut info = BgDrainInfo {
            changed: false,
            notify_all: false,
            notify_panes: Vec::new(),
        };
        if start.elapsed() < max_duration {
            while let Ok(res) = self.rx.try_recv() {
                results_len += 1;
                let sub_info = self.process_bg_results(vec![res]);
                info.changed |= sub_info.changed;
                info.notify_all |= sub_info.notify_all;
                info.notify_panes.extend(sub_info.notify_panes);
                if start.elapsed() > max_duration {
                    break;
                }
            }
        }
        changed |= info.changed;
        let notify_panes_len = info.notify_panes.len();
        crate::perf::event(|| {
            format!(
                "bg.drain icons={icon_len} results={results_len} changed={changed} notify_all={} notify_panes={notify_panes_len} dirty={}",
                info.notify_all, self.dirty_panes.len()
            )
        });

        // Apply pane notifications now that state mutation is done.
        if info.notify_all {
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
        if !self.is_pane_or_dock_animating() && !self.pending_column_reflow.is_empty() {
            self.flush_pending_column_reflows(cx);
        }
        changed
    }

    /// Drain background results and update App state without calling
    /// `signal.update`. Safe to call inside `App::render` because it never
    /// re-enters gpui's entity lease machinery.
    pub(crate) fn drain_bg_results_state_only(&mut self) -> bool {
        let _perf = crate::perf::span("bg.drain_state_only");
        let start = std::time::Instant::now();
        let max_duration = if self.is_pane_or_dock_animating() {
            std::time::Duration::from_micros(500)
        } else {
            std::time::Duration::from_millis(2)
        };

        let mut icon_results = Vec::new();
        for res in self.icon_rx.try_iter() {
            if start.elapsed() > max_duration {
                break;
            }
            icon_results.push(res);
        }
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
        let mut results_len = 0;
        let mut info = BgDrainInfo {
            changed: false,
            notify_all: false,
            notify_panes: Vec::new(),
        };
        if start.elapsed() < max_duration {
            while let Ok(res) = self.rx.try_recv() {
                results_len += 1;
                let sub_info = self.process_bg_results(vec![res]);
                info.changed |= sub_info.changed;
                info.notify_all |= sub_info.notify_all;
                info.notify_panes.extend(sub_info.notify_panes);
                if start.elapsed() > max_duration {
                    break;
                }
            }
        }
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
                BgResult::DirListPreview {
                    tab,
                    op_id,
                    entries,
                } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if op_id == ts.last_nav_op_id && ts.entries.is_empty() {
                            ts.set_entries(entries.into());
                            info.notify_panes.push(tab);
                        }
                    }
                }
                BgResult::DirListed {
                    tab,
                    op_id,
                    entries,
                } => {
                    crate::perf::count_by("bg.entries", entries.len() as u64);
                    // display_* fields are pre-formatted by the producing task
                    // (see `details::format_entries`); keep the drain allocation-free.
                    // Collect icon keys before moving `entries` so we can warm
                    // the icon cache for the new listing.
                    let icon_keys: Vec<String> = if self.icon_theme.is_some() {
                        let _perf = crate::perf::span("bg.collect_icon_keys");
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
                            ts.set_entries(entries.into());
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
                            if self
                                .pending_select
                                .as_ref()
                                .is_some_and(|(pending_tab, _)| *pending_tab == tab)
                            {
                                if let Some((_, pending_path)) = self.pending_select.as_ref() {
                                    if let Some(idx) =
                                        ts.entries.iter().position(|e| e.path == *pending_path)
                                    {
                                        self.pending_select = None;
                                        ts.selection.selected.clear();
                                        ts.selection.selected.insert(idx);
                                        ts.selection.keyboard_cursor = Some(idx);
                                        ts.selection.selection_anchor = Some(idx);
                                        ts.selection.last_clicked_index = Some(idx);
                                        ts.selection.scroll_to_selected = true;
                                    }
                                }
                            }
                            // Fresh navigation (not a watcher refresh) landed with no
                            // selection (no preserved path, no pending_select match) →
                            // select the first entry so a folder is never entered "blind".
                            if armed_appear
                                && ts.selection.selected.is_empty()
                                && !ts.entries.is_empty()
                            {
                                ts.selection.selected.insert(0);
                                ts.selection.keyboard_cursor = Some(0);
                                ts.selection.selection_anchor = Some(0);
                                ts.selection.last_clicked_index = Some(0);
                                ts.selection.scroll_to_selected = true;
                            }
                        }
                    }
                    // Borrow of tab_states released — safe to request icons.
                    if armed_appear {
                        self.pane_items_appear
                            .insert(tab, std::time::Instant::now());
                    }
                    {
                        let _perf = crate::perf::span_detail("bg.request_icons", || {
                            format!("n={}", icon_keys.len())
                        });
                        for key in icon_keys {
                            self.request_icon(key);
                        }
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
                            c.entries = entries.into();
                            c.load_state = LoadState::Loaded;
                        }
                    }
                    info.notify_panes.push(tab);
                }
                BgResult::DirListError { tab, op_id, error } => {
                    tracing::warn!(tab = tab.0, op_id, %error, "directory listing failed");
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if op_id == ts.last_nav_op_id {
                            ts.load_state = LoadState::Error(error);
                        }
                    }
                    info.notify_panes.push(tab);
                }
                BgResult::FsEvent(changed_path) => {
                    self.nucleo.reset();
                    self.goto_index_root = None;
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
                    // Miller cascades: re-list any column showing the changed dir.
                    // Only the pane root is watched, so deep-column
                    // changes from *external* programs still go unseen — per-column
                    // watchers if that ever matters.
                    let mut miller_refresh = Vec::new();
                    for (&tid, ts) in &self.state.tab_states {
                        for col in &ts.miller.columns {
                            let Some(dir) = col.location.fs_dir() else {
                                continue;
                            };
                            if Some(dir) == parent.as_deref() || dir == changed_path.as_path() {
                                miller_refresh.push((tid, col.location.clone()));
                            }
                        }
                    }
                    for (tid, loc) in miller_refresh {
                        self.spawn_miller_list(tid, loc);
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
                    // Only the panes showing this path care about the texture;
                    // notify_all here repainted every pane per decoded thumbnail.
                    let tabs = self.affected_tabs_for_path(&path);
                    self.apply_grid_thumbnail(path, rgba, width, height);
                    info.notify_panes.extend(tabs);
                }

                BgResult::SearchInPlaceResults { tab, gen, entries } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if ts.search.generation == gen
                            && ts.search.active
                            && ts.search.recursive
                            && ts.search.is_filtering()
                        {
                            ts.set_entries(entries.into());
                            ts.filter.clear();
                            ts.search.status = atlas_model::tab::SearchStatus::Ready;
                            ts.search.error = None;
                            self.select_first_visible(tab);
                        }
                    }
                    info.notify_panes.push(tab);
                }
                BgResult::SearchInPlaceError { tab, gen, error } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if ts.search.generation == gen && ts.search.active {
                            ts.search.status = atlas_model::tab::SearchStatus::Error;
                            ts.search.error = Some(error);
                        }
                    }
                    info.notify_panes.push(tab);
                }
                BgResult::FolderFilterResults {
                    tab,
                    gen,
                    query,
                    visible,
                    counts,
                } => {
                    if let Some(ts) = self.state.tab_states.get_mut(&tab) {
                        if ts.search.generation == gen
                            && ts.search.active
                            && !ts.search.recursive
                            && ts.search.query.trim() == query
                        {
                            ts.filter.clone_from(&query);
                            ts.set_visible_filter_result(query, visible, counts);
                            ts.search.status = atlas_model::tab::SearchStatus::Ready;
                            ts.search.error = None;
                            self.select_first_visible(tab);
                        }
                    }
                    info.notify_panes.push(tab);
                }
                BgResult::PathsDeleted(paths) => {
                    self.consume_completed_staged_paths(&paths);
                    info.notify_all = true;
                }
                BgResult::OpComplete {
                    id,
                    result,
                    affected_tabs,
                } => {
                    self.state.ops.active_ops.remove(&id);
                    if let Err(errors) = result {
                        for err in &errors {
                            tracing::error!("Background operation failed: {}", err);
                            self.toast_error(crate::app::ops::op_error_text(err));
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
                                    self.delete_files(existing, false);
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
                        self.toast_error(crate::app::ops::op_error_text(err));
                    }
                    // Files cut in another app and pasted here: report the move
                    // back so the source clears its cut state — only when
                    // everything actually moved.
                    #[cfg(windows)]
                    if origin == atlas_model::ops::TransferOrigin::PasteExternalCut
                        && report.errors.is_empty()
                        && !report.completed.is_empty()
                    {
                        crate::dnd::windows::ole_clipboard::acknowledge_external_cut();
                    }
                    if report.cancelled {
                        self.toast_error(
                            rust_i18n::t!("op_cancelled_partial", done = report.completed.len())
                                .into_owned(),
                        );
                    }
                    if !report.cancelled && report.errors.is_empty() && !report.completed.is_empty() {
                        self.toast_info(
                            rust_i18n::t!("op_transfer_complete", count = report.completed.len())
                                .into_owned(),
                        );
                    }
                    if !report.completed.is_empty() {
                        if matches!(origin, atlas_model::ops::TransferOrigin::DndInternal | atlas_model::ops::TransferOrigin::DndExternal) {
                            let sources = report.completed.iter().map(|(src, _)| src.clone()).collect::<Vec<_>>();
                            self.consume_completed_staged_paths(&sources);
                        }
                        // Tags follow files moved within the app (real pairs,
                        // so this survives conflict renames).
                        if kind == atlas_model::ops::TransferKind::Move {
                            for (src, dst) in &report.completed {
                                let _ = self.tag_db_tx.send(
                                    atlas_core::fs::tag_db::TagDbCmd::RenamePath {
                                        old: src.clone(),
                                        new: dst.clone(),
                                    },
                                );
                            }
                        }
                        // Undo records what actually happened; undo/redo runs
                        // don't re-record themselves.
                        if origin != atlas_model::ops::TransferOrigin::Undo {
                            let entry = match kind {
                                atlas_model::ops::TransferKind::Move => {
                                    atlas_model::undo::UndoEntry::Move {
                                        pairs: report.completed.clone(),
                                    }
                                }
                                atlas_model::ops::TransferKind::Copy => {
                                    atlas_model::undo::UndoEntry::Copy {
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
                    let completed = self.dropzone_pending_tags.iter()
                        .filter(|(path, tag)| tags.get(*path).is_some_and(|assigned| assigned.contains(tag)))
                        .map(|(path, _)| path.clone()).collect::<Vec<_>>();
                    if !completed.is_empty() {
                        self.consume_completed_staged_paths(&completed);
                        info.notify_all = true;
                    }
                    // Scope the repaint to panes whose dir contains a tagged path.
                    let mut tabs = std::collections::HashSet::new();
                    for path in tags.keys() {
                        tabs.extend(self.affected_tabs_for_path(path));
                    }
                    self.state.tag_cache.apply_for_paths(tags);
                    info.notify_panes.extend(tabs);
                }
                BgResult::PathsForTag { tag, paths } => {
                    let completed = paths.iter().filter(|path| self.dropzone_pending_tags.get(*path) == Some(&tag)).cloned().collect::<Vec<_>>();
                    self.consume_completed_staged_paths(&completed);
                    self.populate_tag_tabs(tag, paths);
                    info.notify_all = true;
                }
                BgResult::ContentResults { gen, mut paths } => {
                    // Drop stale batches from a superseded query generation.
                    if gen == self.content_gen {
                        self.content_results.append(&mut paths);
                    }
                }
                BgResult::GotoServiceResults { gen, query, results, complete } => {
                    if gen == self.goto_service_generation.load(std::sync::atomic::Ordering::SeqCst) {
                        let entries: Vec<atlas_core::search::nucleo_pool::IndexEntry> = results
                            .into_iter()
                            .map(|(path, is_dir)| atlas_core::search::nucleo_pool::IndexEntry { path, is_dir })
                            .collect();
                        self.goto_service_cache = Some((query, std::sync::Arc::new(entries), complete));
                        if complete {
                            self.goto_service_inflight = None;
                        }
                        info.changed = true;
                    }
                }
                BgResult::GotoServiceFailed { gen, query } => {
                    if gen == self.goto_service_generation.load(std::sync::atomic::Ordering::SeqCst) {
                        self.goto_service_failed = Some(query);
                        self.goto_service_inflight = None;
                        info.changed = true;
                    }
                }
                BgResult::DeviceMounted(path) => {
                    atlas_core::fs::udisks::invalidate();
                    self.navigate(self.state.focused_tile, path);
                }
                BgResult::DeviceActionFailed(error) => {
                    tracing::warn!("device action failed: {error}");
                }
                BgResult::DevicesChanged => {
                    atlas_core::fs::udisks::invalidate();
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
                            ts.set_entries(entries.into());
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
                    let tabs = self.affected_tabs_for_path(&path);
                    self.apply_thumb_skipped(path);
                    info.notify_panes.extend(tabs);
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
                            atlas_model::preview_cache::PreviewResult::Error(format!(
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
                BgResult::ExtRegistryFetched(result) => {
                    self.handle_ext_registry_fetched(result);
                }
                BgResult::ExtRemoteInstalled { id, result } => {
                    self.handle_ext_remote_installed(id, result);
                }
                BgResult::ExtLocalInstalled(result) => {
                    self.handle_ext_local_installed(result);
                }
                #[cfg(feature = "provider-host")]
                other => self.handle_cloud_bg(other),
            }
        }
        info
    }
}

fn toast_alive(toast: &Toast) -> bool {
    toast.created_at.elapsed() < toast_ttl(toast.kind)
}

fn toast_ttl(kind: ToastKind) -> Duration {
    match kind {
        ToastKind::Info => Duration::from_secs(4),
        ToastKind::Error => Duration::from_secs(10),
    }
}

#[cfg(test)]
mod tests {
    use super::{toast_alive, Toast, ToastKind};
    use std::time::{Duration, Instant};

    #[test]
    fn toast_lifetimes_match_kind() {
        assert!(!toast_alive(&Toast {
            message: String::new(),
            kind: ToastKind::Info,
            created_at: Instant::now() - Duration::from_secs(5),
        }));
        assert!(toast_alive(&Toast {
            message: String::new(),
            kind: ToastKind::Error,
            created_at: Instant::now() - Duration::from_secs(5),
        }));
    }
}
