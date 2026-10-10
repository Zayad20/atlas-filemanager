//! Central drag-hover navigation.
//!
//! Renderers register window-space targets every frame; the global motor decides
//! when a held drag should navigate or autoscroll. This keeps Finder-style drag
//! affordances out of individual sidebar/chrome/view widgets.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::app::App;
use crate::input::action::Action;
use crate::ui::edge_autoscroll;
use atlas_model::layout_tree::TileId;
use atlas_model::location::Location;
use atlas_model::tag_cache::TagId;
use gpui::prelude::*;
use gpui::{
    canvas, div, AnyElement, Context, DispatchPhase, Entity, MouseMoveEvent, MouseUpEvent, Window,
};
use gpui_component::ActiveTheme;

pub(crate) const NAV_HOVER_DELAY: Duration = Duration::from_millis(350);
pub(crate) const FOLDER_HOVER_DELAY: Duration = Duration::from_millis(650);
const REPEAT_NAV_GUARD: Duration = Duration::from_millis(450);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum DragHoverId {
    Sidebar(String),
    Chrome(&'static str, TileId),
    Folder(TileId, PathBuf),
    Column(TileId, PathBuf),
    Breadcrumb(TileId, PathBuf),
    BreadcrumbMenu(TileId, PathBuf),
    Inspector(PathBuf),
    Tab(TileId),
}

#[derive(Clone)]
pub(crate) enum DragHoverCommand {
    NavigatePath(PathBuf),
    NavigatePathIn(TileId, PathBuf),
    NavigateLocation(Location),
    NavigateBack(TileId),
    NavigateForward(TileId),
    NavigateUp(TileId),
    OpenTag(TagId),
    Process(Action),
}

#[derive(Clone)]
pub(crate) enum DragHoverDropCommand {
    AddBookmarks,
    AssignTag(TagId),
    StageInSidebar,
    TransferTo(PathBuf),
    /// Send the dropped paths to the OS trash (drop on the Recycle Bin).
    TrashPaths,
    RejectArchive,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DragHoverKind {
    Nav,
    Folder,
}

#[derive(Clone)]
pub(crate) struct DragHoverTarget {
    pub(crate) id: DragHoverId,
    pub(crate) bounds: gpui::Bounds<gpui::Pixels>,
    pub(crate) kind: DragHoverKind,
    pub(crate) command: DragHoverCommand,
    pub(crate) drop_command: Option<DragHoverDropCommand>,
}

#[derive(Default)]
pub(crate) struct DragHoverState {
    active: bool,
    pub(crate) cursor: Option<gpui::Point<gpui::Pixels>>,
    active_id: Option<DragHoverId>,
    entered_at: Option<Instant>,
    last_fire: Option<(DragHoverId, Instant)>,
    fired_id: Option<DragHoverId>,
    last_scroll_frame: Option<Instant>,
    pub(crate) blink_id: Option<DragHoverId>,
    pub(crate) blink_started_at: Option<Instant>,
    pub(crate) blink_bounds: Option<gpui::Bounds<gpui::Pixels>>,
    /// Per-pane (location, history, forward) captured when the drag began, so an
    /// ESC-cancel (or drag-out without an in-Atlas drop) can undo any spring-load
    /// navigation that fired while dragging.
    nav_snapshot:
        Option<std::collections::HashMap<TileId, (Location, Vec<Location>, Vec<Location>)>>,
    /// Focused tile captured when the drag began, so an abandoned drag restores
    /// the pre-drag tab.
    initial_focused_tile: Option<TileId>,
    /// Set once a real drop lands inside Atlas; suppresses the cancel-revert.
    committed: bool,
    /// Time of the last fired nav, used to drive the confirmation pulse glow.
    pub(crate) confirm_pulse_at: Option<Instant>,
    pub(crate) confirm_pulse_bounds: Option<gpui::Bounds<gpui::Pixels>>,
}

impl DragHoverState {
    fn reset_target(&mut self, id: Option<DragHoverId>, now: Instant) {
        if self.active_id == id {
            return;
        }
        self.active_id = id;
        self.entered_at = now.into();
        self.fired_id = None;
        self.blink_id = None;
        self.blink_started_at = None;
        self.blink_bounds = None;
    }

    pub(crate) fn clear(&mut self) {
        self.active = false;
        self.cursor = None;
        self.active_id = None;
        self.entered_at = None;
        self.last_fire = None;
        self.fired_id = None;
        self.last_scroll_frame = None;
        self.blink_id = None;
        self.blink_started_at = None;
        self.blink_bounds = None;
        self.nav_snapshot = None;
        self.initial_focused_tile = None;
        self.committed = false;
        self.confirm_pulse_at = None;
        self.confirm_pulse_bounds = None;
    }
}

impl App {
    pub(crate) fn begin_drag_hover(&mut self) {
        self.drag_hover.active = true;
        self.drag_hover.committed = false;
        self.drag_hover.initial_focused_tile = Some(self.state.focused_tile);
        // Snapshot every pane's nav state so a cancel can undo spring-load nav.
        use atlas_model::layout_tree::Tile;
        let snap: std::collections::HashMap<_, _> = self
            .state
            .pane_tree
            .tiles
            .iter()
            .filter_map(|(id, tile)| match tile {
                Tile::Pane(p) => Some((
                    *id,
                    (
                        p.location.clone(),
                        p.history.clone(),
                        p.history_forward.clone(),
                    ),
                )),
                _ => None,
            })
            .collect();
        self.drag_hover.nav_snapshot = Some(snap);
    }

    /// Mark the drag as committed (a real drop landed inside Atlas), so the
    /// cancel-revert is suppressed when the drag ends.
    pub(crate) fn mark_drag_committed(&mut self) {
        self.drag_hover.committed = true;
    }

    /// Undo any spring-load navigation that fired during the drag, restoring each
    /// pane's location/history to the pre-drag snapshot. Marks affected panes
    /// dirty so the next flush repaints them. No-op once committed.
    fn revert_drag_nav(&mut self) {
        if self.drag_hover.committed {
            return;
        }
        if let Some(initial) = self.drag_hover.initial_focused_tile.take() {
            if self.state.focused_tile != initial {
                self.activate_tab(initial);
            }
        }
        let Some(snap) = self.drag_hover.nav_snapshot.take() else {
            return;
        };
        use atlas_model::layout_tree::Tile;
        for (id, (loc, history, forward)) in snap {
            if let Some(Tile::Pane(p)) = self.state.pane_tree.tiles.get_mut(id) {
                if p.location == loc {
                    continue; // never navigated; leave it (and its async listing) alone
                }
                p.location = loc;
                p.history = history;
                p.history_forward = forward;
                self.dirty_panes.insert(id);
                // Re-list the restored location so the view matches the route again.
                let restore = self.state.pane_tree.tiles.get(id).and_then(|t| match t {
                    Tile::Pane(p) => Some(p.location.clone()),
                    _ => None,
                });
                if let Some(loc) = restore {
                    if let Some(ts) = self.state.tab_states.get_mut(&id) {
                        ts.begin_navigation();
                    }
                    self.spawn_list(id, loc);
                }
            }
        }
    }

    pub(crate) fn register_drag_hover_target(&mut self, mut target: DragHoverTarget) {
        if let DragHoverId::Folder(pane, path) | DragHoverId::Column(pane, path) = &target.id {
            if let Some(tab) = self.tab_data_for(*pane) {
                let location = folder_location(&tab.location, path);
                target.drop_command = Some(location_drop_command(&location));
                target.command = DragHoverCommand::Process(Action::NavigateLocation { tab: *pane, location });
            }
        }
        if let Some(existing) = self
            .pending_drag_hover_targets
            .iter_mut()
            .find(|existing| existing.id == target.id)
        {
            *existing = target;
        } else {
            self.pending_drag_hover_targets.push(target);
        }
    }

    pub(crate) fn drag_hover_maybe_active(&self) -> bool {
        #[cfg(windows)]
        if self
            .dnd
            .as_ref()
            .and_then(|d| d.zones.lock().ok().map(|z| z.drag_active))
            .unwrap_or(false)
        {
            return true;
        }
        self.drag_hover.active
    }

    pub(crate) fn drag_hover_set_cursor(&mut self, pos: gpui::Point<gpui::Pixels>) {
        if !self.drag_hover.active && self.drag_hover_maybe_active() {
            self.begin_drag_hover();
        }
        self.drag_hover.cursor = Some(pos);
    }

    #[cfg(windows)]
    pub(crate) fn drag_hover_external_left(&mut self) {
        // OLE calls IDropTarget::DragLeave on ESC-cancel and when the drag leaves
        // our window. A real drop calls Drop (→ mark_drag_committed) instead, so a
        // DragLeave without a commit means the drag was abandoned → undo nav.
        self.revert_drag_nav();
        self.drag_hover.clear();
        self.active_external_drag = None;
    }

    pub(crate) fn tick_drag_hover(&mut self, cx: &mut Context<Self>) {
        #[cfg(windows)]
        if let Some((x, y)) = self
            .dnd
            .as_ref()
            .and_then(|d| d.zones.lock().ok().and_then(|z| z.cursor))
        {
            self.drag_hover_set_cursor(gpui::point(gpui::px(x), gpui::px(y)));
        }
        let mut dirty = self.tick_drag_hover_inner();
        #[cfg(windows)]
        {
            dirty |= self.drain_dnd_queue();
        }
        // During a Windows OLE drag the modal `DoDragDrop` loop starves gpui's
        // async 60 Hz drain, so spawn_list results and scroll offsets are applied
        // but never repainted. This canvas DOES run each frame (WM_PAINT is still
        // serviced), so pump the drain + flush here to keep the view live: the
        // spring-loaded folder shows its new contents and edge-scroll visibly moves.
        if dirty || !self.dirty_panes.is_empty() {
            self.drain_bg_results_state_only();
            self.flush_dirty_panes(cx);
            cx.notify();
        }
    }

    /// Drives one frame of hover navigation + edge-scroll. Returns `true` when a
    /// navigation fired, a scroll happened, or the blink state changed — i.e. the
    /// caller should repaint.
    fn tick_drag_hover_inner(&mut self) -> bool {
        let Some(pos) = self.drag_hover.cursor else {
            return false;
        };
        let now = Instant::now();
        let in_dropzone = self.dropzone_bounds.is_some_and(|b| contains(b, pos));
        let hit = hover_target_at(
            &self.pending_drag_hover_targets,
            pos,
            self.dropzone_bounds,
            self.sidebar_scroll_region.as_ref().map(|(bounds, _)| *bounds),
        )
        .cloned();

        // A drop over the fixed shelf must not scroll or activate sidebar rows
        // beneath it, even when their retained bounds overlap the shelf.
        let scrolled = !in_dropzone && self.tick_edge_scroll(pos, now);

        let Some(target) = hit else {
            self.drag_hover.reset_target(None, now);
            return scrolled;
        };
        if let DragHoverId::Tab(pane_id) = target.id {
            let is_already_active = self
                .state
                .find_tabs_container(pane_id)
                .and_then(|cid| match self.state.pane_tree.tiles.get(cid) {
                    Some(atlas_model::layout_tree::Tile::Container(
                        atlas_model::layout_tree::Container::Tabs(tabs),
                    )) => tabs.active,
                    _ => None,
                })
                .map_or(self.state.focused_tile == pane_id, |active_id| {
                    active_id == pane_id && self.state.focused_tile == pane_id
                });
            if is_already_active {
                self.drag_hover.reset_target(None, now);
                return scrolled;
            }
        }
        self.drag_hover.reset_target(Some(target.id.clone()), now);
        let delay = match target.kind {
            DragHoverKind::Nav => NAV_HOVER_DELAY,
            DragHoverKind::Folder => FOLDER_HOVER_DELAY,
        };
        let entered = self.drag_hover.entered_at.unwrap_or(now);
        if now.duration_since(entered) < delay {
            return scrolled;
        }
        if target.kind == DragHoverKind::Folder
            && self.drag_hover.blink_id.as_ref() != Some(&target.id)
        {
            self.drag_hover.blink_id = Some(target.id.clone());
            self.drag_hover.blink_started_at = Some(now);
            self.drag_hover.blink_bounds = Some(target.bounds);
            return true;
        }
        if target.kind == DragHoverKind::Folder
            && self
                .drag_hover
                .blink_started_at
                .is_some_and(|t| now.duration_since(t) < Duration::from_millis(170))
        {
            return scrolled;
        }
        if self
            .drag_hover
            .fired_id
            .as_ref()
            .is_some_and(|id| id == &target.id)
        {
            return scrolled;
        }
        if self
            .drag_hover
            .last_fire
            .as_ref()
            .is_some_and(|(id, at)| id == &target.id && now.duration_since(*at) < REPEAT_NAV_GUARD)
        {
            return scrolled;
        }

        self.execute_drag_hover_command(target.command);
        self.drag_hover.last_fire = Some((target.id.clone(), now));
        self.drag_hover.fired_id = Some(target.id.clone());
        // Hand the dwell glow off to a short confirmation pulse on the same rect.
        self.drag_hover.confirm_pulse_at = Some(now);
        self.drag_hover.confirm_pulse_bounds =
            self.drag_hover.blink_bounds.take().or(Some(target.bounds));
        self.drag_hover.blink_id = None;
        self.drag_hover.blink_started_at = None;
        true
    }

    /// Spring-load affordance: a rhythmically pulsing accent glow on the dwell
    /// target, plus a short confirmation pop when the navigation fires. Driven by
    /// elapsed time — the drag canvas re-renders every frame, so no `with_animation`.
    pub(crate) fn render_drag_hover_blink(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::ui::motion::ease_out_quart;
        let accent = cx.theme().accent;
        let now = Instant::now();

        // Confirmation pulse takes priority: a bright ring that fades out fast.
        if let (Some(at), Some(bounds)) = (
            self.drag_hover.confirm_pulse_at,
            self.drag_hover.confirm_pulse_bounds,
        ) {
            let t = now.duration_since(at).as_secs_f32() / 0.20;
            if t <= 1.0 {
                let e = ease_out_quart(t);
                let mut fill = accent;
                fill.alpha = 0.34 * (1.0 - e);
                let mut ring = accent;
                ring.alpha = 0.85 * (1.0 - e);
                let mut glow = accent;
                glow.alpha = 0.5 * (1.0 - e);
                return Some(glow_rect(bounds, fill, ring, glow, 8.0 + 22.0 * e, 2.0));
            }
        }

        let bounds = self.drag_hover.blink_bounds?;
        let started = self.drag_hover.blink_started_at.unwrap_or(now);
        let elapsed = now.duration_since(started).as_secs_f32();
        let fade_in = ease_out_quart((elapsed / 0.18).clamp(0.0, 1.0));
        // ~1.6 Hz pulse mapped to 0..1.
        let pulse = if cx.reduce_motion() {
            0.5
        } else {
            gpui::Motion::new(gpui::millis(312))
                .with_easing(gpui::ease_in_out)
                .repeat_forever()
                .alternate()
                .sample(now.duration_since(started))
                .progress
                .get()
        };
        let mut fill = accent;
        fill.alpha = (0.12 + 0.16 * pulse) * fade_in;
        let mut ring = accent;
        ring.alpha = (0.40 + 0.38 * pulse) * fade_in;
        let mut glow = accent;
        glow.alpha = (0.30 + 0.30 * pulse) * fade_in;
        Some(glow_rect(
            bounds,
            fill,
            ring,
            glow,
            10.0 + 12.0 * pulse,
            1.5,
        ))
    }

    pub(crate) fn tab_drop_target_at(&self, x: f32, y: f32) -> Option<TileId> {
        let pos = gpui::point(gpui::px(x), gpui::px(y));
        hover_target_at(
            &self.pending_drag_hover_targets,
            pos,
            self.dropzone_bounds,
            self.sidebar_scroll_region.as_ref().map(|(bounds, _)| *bounds),
        ).and_then(|target| match target.id {
            DragHoverId::Tab(tab) => Some(tab), _ => None,
        })
    }

    pub(crate) fn drag_hover_drop_command_at(
        &self,
        x: f32,
        y: f32,
    ) -> Option<DragHoverDropCommand> {
        let pos = gpui::Point {
            x: gpui::px(x),
            y: gpui::px(y),
        };
        if let Some(target) = hover_target_at(
            &self.pending_drag_hover_targets,
            pos,
            self.dropzone_bounds,
            self.sidebar_scroll_region.as_ref().map(|(bounds, _)| *bounds),
        ) {
            if let DragHoverId::Tab(tab) = target.id {
                let command = resolve_drop_command(target, |id| self.tab_drop_command(id));
                tracing::info!(tab = tab.0, target_count = self.pending_drag_hover_targets.len(), accepting = command.is_some(), "dnd tab drop resolved");
                return command;
            }
        }
        drop_command_at(
            &self.pending_drag_hover_targets,
            pos,
            self.dropzone_bounds,
            self.sidebar_scroll_region.as_ref().map(|(bounds, _)| *bounds),
        )
        .or_else(|| {
            self.pane_view_bounds
                .iter()
                .chain(self.pane_layout_bounds.iter())
                .find(|(pane, bounds)| contains(**bounds, pos) && self.pane_accepts_visible_drop(**pane))
                .and_then(|(pane, _)| {
                    self.tab_drop_command(*pane)
                })
        })
    }

    pub(crate) fn tab_drop_command(&self, pane_id: TileId) -> Option<DragHoverDropCommand> {
        if self.tab_closing.contains_key(&pane_id) || self.pane_closing.contains_key(&pane_id) {
            return None;
        }
        self.tab_data_for(pane_id).map(|tab| location_drop_command(&tab.location))
    }

    pub(crate) fn folder_drop_command(&self, pane: TileId, path: &std::path::Path) -> DragHoverDropCommand {
        self.tab_data_for(pane).map_or_else(
            || DragHoverDropCommand::TransferTo(path.to_path_buf()),
            |tab| location_drop_command(&folder_location(&tab.location, path)),
        )
    }

    pub(crate) fn active_drag_hover_drop_command(&self) -> Option<DragHoverDropCommand> {
        let pos = self.drag_hover.cursor?;
        self.drag_hover_drop_command_at(f32::from(pos.x), f32::from(pos.y))
    }

    pub(crate) fn pane_drop_dest_at(&self, x: f32, y: f32) -> Option<PathBuf> {
        let pos = gpui::Point {
            x: gpui::px(x),
            y: gpui::px(y),
        };
        for (pane, bounds) in self
            .pane_view_bounds
            .iter()
            .chain(self.pane_layout_bounds.iter())
        {
            if contains(*bounds, pos) && self.pane_accepts_visible_drop(*pane) {
                if let Some(dest) = self
                    .tab_data_for(*pane)
                    .and_then(|tab| tab.location.fs_dir().map(|path| path.to_path_buf()))
                {
                    return Some(dest);
                }
            }
        }
        None
    }

    pub(crate) fn pane_accepts_visible_drop(&self, pane: TileId) -> bool {
        if self.tab_closing.contains_key(&pane) || self.pane_closing.contains_key(&pane) {
            return false;
        }
        self.state.find_tabs_container(pane).is_some_and(|container| {
            matches!(self.state.pane_tree.tiles.get(container),
                Some(atlas_model::layout_tree::Tile::Container(atlas_model::layout_tree::Container::Tabs(tabs))) if tabs.active == Some(pane))
        })
    }

    pub(crate) fn execute_drag_hover_drop_command(
        &mut self,
        command: DragHoverDropCommand,
        paths: Vec<PathBuf>,
        from_self: bool,
    ) {
        self.execute_drag_hover_drop_with_modifiers(command, paths, from_self, false, false);
    }

    pub(crate) fn execute_drag_hover_drop_with_modifiers(
        &mut self,
        command: DragHoverDropCommand,
        paths: Vec<PathBuf>,
        from_self: bool,
        ctrl: bool,
        shift: bool,
    ) {
        self.mark_drag_committed();
        self.rubber = None;
        match command {
            DragHoverDropCommand::AddBookmarks => {
                let mut actions = Vec::new();
                for path in paths {
                    let bookmark_path = if path.is_dir() {
                        path
                    } else if let Some(parent) = path.parent() {
                        parent.to_path_buf()
                    } else {
                        path
                    };
                    let name = bookmark_path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| bookmark_path.display().to_string());
                    actions.push(Action::AddBookmark {
                        name,
                        path: bookmark_path,
                    });
                }
                self.process_actions(actions);
            }
            DragHoverDropCommand::AssignTag(tag) => {
                for path in &paths {
                    if self.state.sidebar_drop_zone.iter().any(|item| &item.path == path) {
                        self.dropzone_pending_tags.insert(path.clone(), tag);
                    }
                }
                self.process_actions(vec![Action::AssignTagToFiles { tag, paths }]);
            }
            DragHoverDropCommand::StageInSidebar => {
                stage_paths(&mut self.state.sidebar_drop_zone, paths, from_self);
                self.cancel_dropzone_file_drag();
            }
            DragHoverDropCommand::TransferTo(dest) => {
                let request = if ctrl || shift {
                    drop_transfer_with_modifiers(paths, dest, from_self, ctrl, shift)
                } else {
                    drop_transfer_request(paths, dest, from_self)
                };
                self.process_actions(vec![Action::Transfer(request)]);
            }
            DragHoverDropCommand::TrashPaths => {
                // Direct call (not Action::Delete): that action re-routes by the
                // *focused* tab and would purge instead of trash when a recycle
                // pane happens to hold focus.
                self.delete_files(paths, true);
            }
            DragHoverDropCommand::RejectArchive => {
                self.toast_error(rust_i18n::t!("dnd_archive_drop_blocked").into_owned());
            }
        }
        // Staged references are consumed by completion, never by submission.
        self.cancel_dropzone_file_drag();
    }

    pub(crate) fn drop_files_on_pane(
        &mut self,
        pane_id: TileId,
        paths: Vec<PathBuf>,
        external: bool,
    ) {
        if let Some(command) = self.tab_drop_command(pane_id) {
            self.execute_drag_hover_drop_command(command, paths, !external);
        }
    }

    pub(crate) fn begin_dropzone_file_drag(&mut self, paths: &[PathBuf]) {
        self.dropzone_drag_paths = paths.iter().cloned().collect();
    }

    pub(crate) fn cancel_dropzone_file_drag(&mut self) {
        self.dropzone_drag_paths.clear();
    }

    pub(crate) fn consume_completed_staged_paths(&mut self, paths: &[PathBuf]) {
        let committed = paths.iter().cloned().collect::<std::collections::HashSet<_>>();
        if committed.is_empty() {
            return;
        }
        consume_staged_paths(&mut self.state.sidebar_drop_zone, &committed);
        self.dropzone_selection.selected
            .retain(|path| !committed.contains(path));
        self.dropzone_rects
            .retain(|(path, _)| !committed.contains(path));
        self.dropzone_drag_paths.retain(|path| !committed.contains(path));
        self.dropzone_pending_tags.retain(|path, _| !committed.contains(path));
    }

    /// DnD uses proximity-only edge activation. The shared core integrates the
    /// velocity by elapsed time so a stationary cursor keeps scrolling smoothly.
    fn tick_edge_scroll(&mut self, pos: gpui::Point<gpui::Pixels>, now: Instant) -> bool {
        let elapsed = self
            .drag_hover
            .last_scroll_frame
            .replace(now)
            .map_or(1.0 / 60.0, |last| now.duration_since(last).as_secs_f32());
        // Panes first (row-based). The sidebar (pixel-based) is the fallback so a
        // pane that overlaps the sidebar bounds still wins.
        if let Some((&pane, bounds)) = self
            .pane_view_bounds
            .iter()
            .find(|(_, bounds)| contains(**bounds, pos))
        {
            return self.tick_pane_edge_scroll(pane, *bounds, pos, elapsed);
        }
        if let Some((bounds, handle)) = self.sidebar_scroll_region.clone() {
            if contains(bounds, pos) {
                return self.tick_pixel_edge_scroll(&handle, bounds, pos, elapsed);
            }
        }
        false
    }

    /// Edge-scroll a pane. All views (grid/list/details) drive a `uniform_list`
    /// whose underlying `base_handle` is a plain `ScrollHandle`, so we pixel-scroll
    /// it directly — the same proven path as the sidebar. The earlier row-based
    /// `scroll_to_item_strict` approach was flaky (deferred, sometimes a no-op).
    fn tick_pane_edge_scroll(
        &mut self,
        pane: TileId,
        bounds: gpui::Bounds<gpui::Pixels>,
        pos: gpui::Point<gpui::Pixels>,
        elapsed: f32,
    ) -> bool {
        let Some(handle) = self.scroll_handles.get(&pane) else {
            return false;
        };
        let base = handle.0.borrow().base_handle.clone();
        if self.tick_pixel_edge_scroll(&base, bounds, pos, elapsed) {
            // Separate PaneSignal entity: the canvas repaints App, not the pane.
            self.dirty_panes.insert(pane);
            true
        } else {
            false
        }
    }

    /// Pixel-based edge-scroll for a plain `ScrollHandle` (sidebar + pane base
    /// handle). gpui's scroll offset y is negative going down, so down subtracts.
    fn tick_pixel_edge_scroll(
        &mut self,
        handle: &gpui::ScrollHandle,
        bounds: gpui::Bounds<gpui::Pixels>,
        pos: gpui::Point<gpui::Pixels>,
        elapsed: f32,
    ) -> bool {
        let Some(activation) = edge_autoscroll::activation_at(bounds, pos) else {
            return false;
        };
        edge_autoscroll::tick_handle(handle, activation, elapsed)
    }

    fn execute_drag_hover_command(&mut self, command: DragHoverCommand) {
        match command {
            DragHoverCommand::NavigatePath(path) => {
                let tile = self.state.focused_tile;
                self.navigate(tile, path);
            }
            DragHoverCommand::NavigatePathIn(tile, path) => self.navigate(tile, path),
            DragHoverCommand::NavigateLocation(loc) => {
                let tile = self.state.focused_tile;
                self.navigate_to(tile, loc, crate::app::nav::NavKind::Push);
            }
            DragHoverCommand::NavigateBack(tile) => self.navigate_back(tile),
            DragHoverCommand::NavigateForward(tile) => self.navigate_forward(tile),
            DragHoverCommand::NavigateUp(tile) => self.navigate_up(tile),
            DragHoverCommand::OpenTag(tag) => {
                let tile = self.state.focused_tile;
                self.open_tag_results(tile, tag);
            }
            DragHoverCommand::Process(action) => self.process_actions(vec![action]),
        }
    }
}

pub(crate) fn drag_hover_recorder(
    id: DragHoverId,
    kind: DragHoverKind,
    command: DragHoverCommand,
    view: &Entity<App>,
) -> impl Fn(gpui::Bounds<gpui::Pixels>, &mut Window, &mut gpui::App) + 'static {
    let view = view.clone();
    move |bounds, window, cx| {
        let bounds = bounds.intersect(&window.content_mask().bounds);
        crate::perf::count("prepaint.drag_hover_target");
        let target = DragHoverTarget {
            id: id.clone(),
            bounds,
            kind,
            command: command.clone(),
            drop_command: None,
        };
        view.update(cx, |this, _| this.register_drag_hover_target(target));
    }
}

pub(crate) fn drag_hover_recorder_with_drop(
    id: DragHoverId,
    kind: DragHoverKind,
    command: DragHoverCommand,
    drop_command: DragHoverDropCommand,
    view: &Entity<App>,
) -> impl Fn(gpui::Bounds<gpui::Pixels>, &mut Window, &mut gpui::App) + 'static {
    let view = view.clone();
    move |bounds, window, cx| {
        let bounds = bounds.intersect(&window.content_mask().bounds);
        crate::perf::count("prepaint.drag_hover_drop_target");
        let target = DragHoverTarget {
            id: id.clone(),
            bounds,
            kind,
            command: command.clone(),
            drop_command: Some(drop_command.clone()),
        };
        view.update(cx, |this, _| this.register_drag_hover_target(target));
    }
}

/// `on_prepaint` recorder that publishes the sidebar's window-space bounds +
/// scroll handle so the drag edge-scroll engine can pixel-scroll it. The sidebar
/// is a separate entity, so it can't ride the per-pane row-based path.
pub(crate) fn sidebar_scroll_recorder(
    view: Entity<App>,
    handle: gpui::ScrollHandle,
) -> impl FnOnce(gpui::Bounds<gpui::Pixels>, &mut Window, &mut gpui::App) + 'static {
    move |bounds, _window, cx| {
        crate::perf::count("prepaint.sidebar_scroll");
        view.update(cx, |this, _| {
            this.sidebar_scroll_region = Some((bounds, handle.clone()));
        });
    }
}

pub(crate) fn drag_hover_layer(view: Entity<App>) -> impl gpui::IntoElement {
    let v = view.clone();
    canvas(
        |_, _, _| {},
        move |_, (), window, cx| {
            let active = v.read(cx).drag_hover_maybe_active();
            let vm = v.clone();
            window.on_mouse_event::<MouseMoveEvent>(move |ev, phase, _win, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let pos = ev.position;
                vm.update(cx, |this, _| this.drag_hover_set_cursor(pos));
            });
            let vu = v.clone();
            window.on_mouse_event::<MouseUpEvent>(move |_ev, phase, _win, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                // On Windows the drag ends through the OLE callbacks (DragLeave on
                // cancel → revert, Drop → commit), and DoDragDrop injects a synthetic
                // mouse-up afterwards. Clearing here would wipe the nav snapshot before
                // the queued DragLeave is drained, so the cancel-revert would no-op.
                #[cfg(not(windows))]
                vu.update(cx, |this, _| {
                    this.drag_hover.clear();
                    this.active_external_drag = None;
                });
                #[cfg(windows)]
                let _ = (&vu, cx);
            });
            if active {
                v.update(cx, |this, cx| this.tick_drag_hover(cx));
                crate::perf::event(|| "request_animation_frame drag_hover".to_string());
                window.request_animation_frame();
            }
        },
    )
    .absolute()
    .inset_0()
}

/// Build one absolutely-positioned spring-load highlight: a soft fill, an accent
/// ring, and an outer glow (box-shadow). `blur`/`border` are in logical pixels.
fn glow_rect(
    bounds: gpui::Bounds<gpui::Pixels>,
    fill: gpui::Hsla,
    ring: gpui::Hsla,
    glow: gpui::Hsla,
    blur: f32,
    border: f32,
) -> AnyElement {
    div()
        .absolute()
        .left(bounds.left())
        .top(bounds.top())
        .w(bounds.size.width)
        .h(bounds.size.height)
        .rounded(gpui::px(6.0))
        .bg(fill)
        .border(gpui::px(border))
        .border_color(ring)
        .shadow(vec![gpui::BoxShadow {
            color: glow.into(),
            offset: gpui::point(gpui::px(0.0), gpui::px(0.0)),
            blur_radius: gpui::px(blur),
            spread_radius: gpui::px(1.0),
            inset: false,
        }])
        .into_any_element()
}

fn contains(bounds: gpui::Bounds<gpui::Pixels>, pos: gpui::Point<gpui::Pixels>) -> bool {
    pos.x >= bounds.left()
        && pos.x < bounds.right()
        && pos.y >= bounds.top()
        && pos.y < bounds.bottom()
}

fn hover_target_at(
    targets: &[DragHoverTarget],
    pos: gpui::Point<gpui::Pixels>,
    dropzone: Option<gpui::Bounds<gpui::Pixels>>,
    sidebar_viewport: Option<gpui::Bounds<gpui::Pixels>>,
) -> Option<&DragHoverTarget> {
    if dropzone.is_some_and(|bounds| contains(bounds, pos)) {
        return None;
    }
    let visible_hit = |target: &&DragHoverTarget| {
        contains(target.bounds, pos)
            && (!matches!(target.id, DragHoverId::Sidebar(_))
                || sidebar_viewport.is_none_or(|bounds| contains(bounds, pos)))
    };
    // A cached/scrolled body row can retain bounds overlapping the tab strip.
    // The tab under the cursor owns the drop before any underlying row.
    targets.iter().rev().filter(|target| matches!(target.id, DragHoverId::BreadcrumbMenu(_, _))).find(visible_hit)
        .or_else(|| targets.iter().rev().filter(|target| matches!(target.id, DragHoverId::Tab(_))).find(visible_hit))
        .or_else(|| targets.iter().rev().filter(|target| !matches!(target.id, DragHoverId::Column(_, _))).find(visible_hit))
        .or_else(|| targets.iter().rev().find(visible_hit))
}

fn drop_command_at(
    targets: &[DragHoverTarget],
    pos: gpui::Point<gpui::Pixels>,
    dropzone: Option<gpui::Bounds<gpui::Pixels>>,
    sidebar_viewport: Option<gpui::Bounds<gpui::Pixels>>,
) -> Option<DragHoverDropCommand> {
    if dropzone.is_some_and(|bounds| contains(bounds, pos)) {
        return Some(DragHoverDropCommand::StageInSidebar);
    }
    hover_target_at(targets, pos, dropzone, sidebar_viewport)
        .and_then(|target| target.drop_command.clone())
}

/// Cached tab chrome may keep an older destination after spring navigation.
/// Read the current tab by ID at drop time instead of using the captured path.
fn resolve_drop_command(
    target: &DragHoverTarget,
    tab_command: impl FnOnce(TileId) -> Option<DragHoverDropCommand>,
) -> Option<DragHoverDropCommand> {
    match target.id {
        DragHoverId::Tab(tab) => tab_command(tab),
        _ => target.drop_command.clone(),
    }
}

pub(crate) fn retain_visible_drop_targets(
    targets: &mut Vec<DragHoverTarget>,
    exists: impl Fn(TileId) -> bool,
    pane_visible: impl Fn(TileId) -> bool,
    sidebar_visible: bool,
    inspector_visible: bool,
    menu_visible: bool,
) {
    targets.retain(|target| match &target.id {
        DragHoverId::Sidebar(_) => sidebar_visible,
        DragHoverId::Inspector(_) => inspector_visible,
        DragHoverId::Tab(tab) => exists(*tab),
        DragHoverId::BreadcrumbMenu(tab, _) => menu_visible && exists(*tab) && pane_visible(*tab),
        DragHoverId::Folder(tab, _) | DragHoverId::Column(tab, _) | DragHoverId::Breadcrumb(tab, _) | DragHoverId::Chrome(_, tab) => exists(*tab) && pane_visible(*tab),
    });
}

pub(crate) fn drop_transfer_request(
    paths: Vec<PathBuf>,
    dest: PathBuf,
    from_self: bool,
) -> atlas_model::ops::TransferRequest {
    drop_transfer_with_modifiers(paths, dest, from_self, false, false)
}

pub(crate) fn location_drop_command(location: &Location) -> DragHoverDropCommand {
    match location {
        Location::Directory(path) => DragHoverDropCommand::TransferTo(path.clone()),
        Location::Tag(tag) => DragHoverDropCommand::AssignTag(*tag),
        Location::RecycleBin => DragHoverDropCommand::TrashPaths,
        Location::Archive { .. } => DragHoverDropCommand::RejectArchive,
        #[cfg(feature = "provider-host")]
        Location::Remote { id, path } => DragHoverDropCommand::TransferTo(crate::app::cloud::remote_sentinel(id, path)),
    }
}

fn folder_location(parent: &Location, path: &std::path::Path) -> Location {
    match parent {
        Location::Archive { archive, .. } => Location::Archive { archive: archive.clone(), inner: path.to_path_buf() },
        #[cfg(feature = "provider-host")]
        Location::Remote { id, .. } => Location::Remote { id: id.clone(), path: path.to_string_lossy().replace('\\', "/") },
        _ => Location::Directory(path.to_path_buf()),
    }
}

pub(crate) fn drop_transfer_with_modifiers(
    paths: Vec<PathBuf>, dest: PathBuf, from_self: bool, ctrl: bool, shift: bool,
) -> atlas_model::ops::TransferRequest {
    use atlas_model::ops::{ConflictPolicy, TransferKind, TransferOrigin, TransferRequest};
    let effect = if dest.to_string_lossy().starts_with("remote:") {
        crate::dnd::DropEffect::Copy
    } else {
        crate::dnd::effect_with_modifiers(crate::dnd::default_effect_for_paths(&paths, &dest), ctrl, shift)
    };
    TransferRequest {
        sources: paths,
        target_names: None,
        dest,
        kind: if effect.is_move() { TransferKind::Move } else { TransferKind::Copy },
        policy: ConflictPolicy::Ask,
        origin: if from_self { TransferOrigin::DndInternal } else { TransferOrigin::DndExternal },
    }
}

fn stage_paths(staged: &mut Vec<atlas_model::app_state::StagedDrop>, paths: Vec<PathBuf>, from_self: bool) {
    let mut existing: std::collections::HashSet<_> = staged.iter().map(|item| item.path.clone()).collect();
    staged.extend(paths.into_iter()
        .filter(|path| path.exists() && !atlas_core::fs::paths::is_recycle_bin_path(path))
        .filter(|path| existing.insert(path.clone()))
        .map(|path| atlas_model::app_state::StagedDrop { path, from_self }));
}

fn consume_staged_paths(staged: &mut Vec<atlas_model::app_state::StagedDrop>, completed: &std::collections::HashSet<PathBuf>) {
    staged.retain(|item| !completed.contains(&item.path));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size, Bounds, Pixels};

    fn rect(y: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.0), px(y)), size(px(240.0), px(height)))
    }

    fn sidebar_row(id: &str, bounds: Bounds<Pixels>) -> DragHoverTarget {
        DragHoverTarget {
            id: DragHoverId::Sidebar(id.into()),
            bounds,
            kind: DragHoverKind::Nav,
            command: DragHoverCommand::NavigatePath(PathBuf::from(id)),
            drop_command: Some(DragHoverDropCommand::TransferTo(PathBuf::from(id))),
        }
    }

    #[test]
    fn dropzone_stages_instead_of_activating_overlapping_sidebar_rows() {
        let targets = [sidebar_row("drive", rect(640.0, 120.0))];
        let dropzone = Some(rect(660.0, 76.0));
        // Even stale row/viewport geometry that extends under the shelf cannot
        // navigate, assign a tag or transfer a drop to an underlying location.
        let viewport = Some(rect(0.0, 780.0));
        let pos = point(px(100.0), px(700.0));
        assert!(hover_target_at(&targets, pos, dropzone, viewport).is_none());
        assert!(matches!(
            drop_command_at(&targets, pos, dropzone, viewport),
            Some(DragHoverDropCommand::StageInSidebar)
        ));
    }

    #[test]
    fn clipped_sidebar_rows_cannot_receive_hover_or_drop() {
        let targets = [sidebar_row("bookmark", rect(500.0, 120.0))];
        let viewport = Some(rect(0.0, 550.0));
        let pos = point(px(100.0), px(580.0));
        assert!(hover_target_at(&targets, pos, None, viewport).is_none());
        assert!(drop_command_at(&targets, pos, None, viewport).is_none());

        let visible_pos = point(px(100.0), px(520.0));
        assert!(matches!(
            drop_command_at(&targets, visible_pos, None, viewport),
            Some(DragHoverDropCommand::TransferTo(path)) if path == PathBuf::from("bookmark")
        ));
    }

    #[test]
    fn topmost_visible_target_keeps_priority_outside_dropzone() {
        let targets = [
            sidebar_row("underneath", rect(0.0, 60.0)),
            sidebar_row("topmost", rect(0.0, 60.0)),
        ];
        let pos = point(px(100.0), px(30.0));
        let dropzone = Some(rect(660.0, 76.0));
        assert!(matches!(
            drop_command_at(&targets, pos, dropzone, Some(rect(0.0, 550.0))),
            Some(DragHoverDropCommand::TransferTo(path)) if path == PathBuf::from("topmost")
        ));
    }

    fn tab_target(tab: TileId, destination: PathBuf) -> DragHoverTarget {
        DragHoverTarget {
            id: DragHoverId::Tab(tab),
            bounds: rect(0.0, 36.0),
            kind: DragHoverKind::Nav,
            command: DragHoverCommand::Process(Action::FocusTab(tab)),
            drop_command: Some(DragHoverDropCommand::TransferTo(destination)),
        }
    }

    #[test]
    fn idle_renders_keep_cached_tabs_and_remove_closed_tabs_and_rows() {
        let live = TileId(1);
        let mut targets = vec![
            tab_target(live, "live".into()),
            tab_target(TileId(2), "closed".into()),
            sidebar_row("row", rect(0.0, 60.0)),
        ];
        retain_visible_drop_targets(&mut targets, |tab| tab == live, |_| true, false, false, false);
        assert_eq!(targets.len(), 1);
        let target = hover_target_at(&targets, point(px(100.0), px(18.0)), None, None).unwrap();
        assert_eq!(target.id, DragHoverId::Tab(live));
        // A tab that no longer accepts drops must not use its captured folder.
        assert!(resolve_drop_command(target, |_| None).is_none());
    }

    #[test]
    fn tab_header_wins_over_stale_body_geometry() {
        let tab = TileId(1);
        let targets = vec![tab_target(tab, "tab-folder".into()), sidebar_row("stale-body", rect(0.0, 60.0))];
        let hit = hover_target_at(&targets, point(px(100.0), px(18.0)), None, None).unwrap();
        assert_eq!(hit.id, DragHoverId::Tab(tab));
    }

    #[test]
    fn cached_geometry_survives_idle_but_hidden_owners_are_pruned() {
        let pane = TileId(1);
        let mut folder = sidebar_row("folder", rect(40.0, 30.0));
        folder.id = DragHoverId::Folder(pane, "folder".into());
        let mut menu = sidebar_row("menu", rect(0.0, 30.0));
        menu.id = DragHoverId::BreadcrumbMenu(pane, "menu".into());
        let mut inspector = sidebar_row("inspector", rect(0.0, 30.0));
        inspector.id = DragHoverId::Inspector("inspector".into());
        let mut targets = vec![tab_target(pane, "tab".into()), folder, sidebar_row("sidebar", rect(0.0, 30.0)), menu, inspector];
        retain_visible_drop_targets(&mut targets, |_| true, |_| true, true, true, true);
        assert_eq!(targets.len(), 5, "OLE may release between cached paints; retain all visible owners");
        retain_visible_drop_targets(&mut targets, |_| true, |_| false, false, false, false);
        assert_eq!(targets.len(), 1, "inactive pane bodies and closed panels/popups cannot receive drops");
        assert_eq!(targets[0].id, DragHoverId::Tab(pane));
        retain_visible_drop_targets(&mut targets, |_| false, |_| false, false, false, false);
        assert!(targets.is_empty());
    }

    #[test]
    fn pane_and_tab_location_matrix() {
        let directory = PathBuf::from(r"C:\destination");
        assert!(matches!(location_drop_command(&Location::Directory(directory.clone())), DragHoverDropCommand::TransferTo(path) if path == directory));
        let tag = TagId(7);
        assert!(matches!(location_drop_command(&Location::Tag(tag)), DragHoverDropCommand::AssignTag(id) if id == tag));
        assert!(matches!(location_drop_command(&Location::RecycleBin), DragHoverDropCommand::TrashPaths));
        assert!(matches!(location_drop_command(&Location::Archive { archive: "example.zip".into(), inner: "folder".into() }), DragHoverDropCommand::RejectArchive));
        #[cfg(feature = "provider-host")]
        assert!(matches!(location_drop_command(&Location::Remote { id: "drive".into(), path: "folder".into() }), DragHoverDropCommand::TransferTo(path) if path == PathBuf::from("remote:drive/folder")));
    }

    #[test]
    fn bookmarks_drop_command_variant() {
        let cmd = DragHoverDropCommand::AddBookmarks;
        assert!(matches!(cmd, DragHoverDropCommand::AddBookmarks));
    }

    #[test]
    fn every_filesystem_target_uses_volume_rules_and_modifiers() {
        use atlas_model::ops::{ConflictPolicy, TransferKind};
        for from_self in [false, true] {
            for (dest, ctrl, shift, expected) in [
                (r"C:\destination", false, false, TransferKind::Move),
                (r"D:\destination", false, false, TransferKind::Copy),
                (r"C:\destination", true, false, TransferKind::Copy),
                (r"D:\destination", false, true, TransferKind::Move),
                ("remote:drive/folder", false, true, TransferKind::Copy),
            ] {
                let request = drop_transfer_with_modifiers(vec![r"C:\source\file.txt".into()], dest.into(), from_self, ctrl, shift);
                assert_eq!(request.kind, expected);
                assert_eq!(request.policy, ConflictPolicy::Ask);
            }
        }
    }

    #[test]
    fn virtual_folder_rows_keep_their_provider_and_archive_semantics() {
        let archive = Location::Archive { archive: r"C:\data\example.zip".into(), inner: "parent".into() };
        let folder = folder_location(&archive, std::path::Path::new("parent/child"));
        assert!(matches!(folder, Location::Archive { inner, .. } if inner == PathBuf::from("parent/child")));
        assert!(matches!(location_drop_command(&archive), DragHoverDropCommand::RejectArchive));
        #[cfg(feature = "provider-host")]
        {
            let remote = Location::Remote { id: "drive".into(), path: "parent".into() };
            let folder = folder_location(&remote, std::path::Path::new(r"parent\child"));
            assert!(matches!(location_drop_command(&folder), DragHoverDropCommand::TransferTo(path) if path == PathBuf::from("remote:drive/parent/child")));
        }
    }

    #[test]
    fn clipped_inspector_folder_cannot_receive_a_drop() {
        let mut target = sidebar_row("inspector-folder", rect(500.0, 100.0));
        target.id = DragHoverId::Inspector("inspector-folder".into());
        target.bounds = target.bounds.intersect(&rect(0.0, 540.0));
        let targets = [target];
        assert!(drop_command_at(&targets, point(px(100.0), px(530.0)), None, None).is_some());
        assert!(drop_command_at(&targets, point(px(100.0), px(560.0)), None, None).is_none());
    }

    #[test]
    fn adjacent_targets_have_unambiguous_edges() {
        let targets = [sidebar_row("first", rect(0.0, 30.0)), sidebar_row("second", rect(30.0, 30.0))];
        let hit = hover_target_at(&targets, point(px(100.0), px(30.0)), None, None).unwrap();
        assert_eq!(hit.id, DragHoverId::Sidebar("second".into()));
        assert!(hover_target_at(&targets, point(px(100.0), px(60.0)), None, None).is_none());
    }

    #[test]
    fn miller_folder_row_wins_over_its_column_background() {
        let pane = TileId(1);
        let path = PathBuf::from("same-folder");
        let mut row = sidebar_row("row", rect(40.0, 30.0));
        row.id = DragHoverId::Folder(pane, path.clone());
        let mut column = sidebar_row("column", rect(0.0, 300.0));
        column.id = DragHoverId::Column(pane, path);
        assert_ne!(row.id, column.id);
        let targets = [row, column];
        assert!(matches!(hover_target_at(&targets, point(px(100.0), px(50.0)), None, None).unwrap().id, DragHoverId::Folder(_, _)));
        assert!(matches!(hover_target_at(&targets, point(px(100.0), px(200.0)), None, None).unwrap().id, DragHoverId::Column(_, _)));
    }

    #[test]
    fn cached_tab_drop_transfers_to_its_current_folder_on_disk() {
        use atlas_core::fs::transfer;
        use atlas_model::app_state::BgResult;
        use atlas_model::ops::{ConflictPolicy, TransferKind, TransferOrigin, TransferRequest};
        use std::sync::{atomic::AtomicBool, Arc};

        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("atlas-tab-drop-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        struct TempTree(PathBuf);
        impl Drop for TempTree {
            fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
        }
        let _cleanup = TempTree(root.clone());
        let source_dir = root.join("source");
        let current_dir = root.join("tab-current");
        let old_dir = root.join("tab-before-navigation");
        let active_dir = root.join("other-active-pane");
        for dir in [&source_dir, &current_dir, &old_dir, &active_dir] {
            std::fs::create_dir(dir).unwrap();
        }
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        for kind in [TransferKind::Move, TransferKind::Copy] {
            let name = format!("{kind:?}.txt");
            let source = source_dir.join(&name);
            std::fs::write(&source, b"tab drop payload").unwrap();
            let skipped = source_dir.join(format!("skipped-{name}"));
            std::fs::write(&skipped, b"keep source").unwrap();
            std::fs::write(current_dir.join(skipped.file_name().unwrap()), b"keep destination").unwrap();
            let mut staged = Vec::new();
            stage_paths(&mut staged, vec![source.clone(), source.clone(), skipped.clone()], true);
            stage_paths(&mut staged, vec![source.clone()], false);
            assert_eq!(staged.len(), 2, "duplicates within and across payloads must collapse");
            let tab = TileId(2);
            let mut targets = vec![tab_target(tab, old_dir.clone())];
            // The root renders idle after OLE's Drop; the tab strip stays cached.
            retain_visible_drop_targets(&mut targets, |_| true, |_| true, true, true, false);
            let hit = hover_target_at(&targets, point(px(100.0), px(18.0)), None, None).unwrap();
            let command = resolve_drop_command(hit, |id| {
                assert_eq!(id, tab);
                Some(DragHoverDropCommand::TransferTo(current_dir.clone()))
            }).unwrap();
            let DragHoverDropCommand::TransferTo(dest) = command else { panic!("expected a folder drop"); };
            let request = TransferRequest {
                sources: vec![source.clone(), skipped.clone()], target_names: None, dest,
                kind, policy: ConflictPolicy::SkipAll, origin: TransferOrigin::DndInternal,
            };
            let valid = transfer::validate(&request);
            assert!(valid.fatal.is_none() && valid.errors.is_empty());
            let (conflicts, clean) = transfer::detect_conflicts(&valid.valid, None, &request.dest);
            let built = transfer::build_pairs(&clean, &conflicts, &request.policy, &request.dest);
            let (tx, rx) = flume::unbounded();
            runtime.block_on(transfer::execute(built.pairs, kind, request.origin, 1, tx, Arc::new(AtomicBool::new(false)), vec![]));
            let report = rx.try_iter().find_map(|event| match event {
                BgResult::TransferComplete { report, .. } => Some(report), _ => None,
            }).unwrap();
            assert!(report.errors.is_empty() && !report.cancelled);
            assert_eq!(report.completed, vec![(source.clone(), current_dir.join(&name))]);
            consume_staged_paths(&mut staged, &report.completed.iter().map(|(src, _)| src.clone()).collect());
            assert_eq!(staged.len(), 1);
            assert_eq!(staged[0].path, skipped);
            assert_eq!(std::fs::read(&skipped).unwrap(), b"keep source");
            assert_eq!(std::fs::read(current_dir.join(skipped.file_name().unwrap())).unwrap(), b"keep destination");
            let pairs = transfer::build_pairs(&clean, &conflicts, &ConflictPolicy::RenameAll, &request.dest);
            let (tx, rx) = flume::unbounded();
            runtime.block_on(transfer::execute(pairs.pairs, kind, request.origin, 2, tx, Arc::new(AtomicBool::new(true)), vec![]));
            let report = rx.try_iter().find_map(|event| match event { BgResult::TransferComplete { report, .. } => Some(report), _ => None }).unwrap();
            assert!(report.cancelled && report.completed.is_empty());
            consume_staged_paths(&mut staged, &report.completed.iter().map(|(src, _)| src.clone()).collect());
            assert_eq!(staged.len(), 1, "cancelled operations must preserve staged sources");
            assert_eq!(std::fs::read(current_dir.join(&name)).unwrap(), b"tab drop payload");
            assert_eq!(source.exists(), kind == TransferKind::Copy);
            assert!(!old_dir.join(&name).exists());
            assert!(!active_dir.join(&name).exists());
        }
    }
}
