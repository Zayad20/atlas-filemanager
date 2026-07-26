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
    AssignTag(TagId),
    StageInSidebar,
    TransferTo(PathBuf),
    /// Send the dropped paths to the OS trash (drop on the Recycle Bin).
    TrashPaths,
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
        self.committed = false;
        self.confirm_pulse_at = None;
        self.confirm_pulse_bounds = None;
    }
}

impl App {
    pub(crate) fn begin_drag_hover(&mut self) {
        self.drag_hover.active = true;
        self.drag_hover.committed = false;
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

    pub(crate) fn register_drag_hover_target(&mut self, target: DragHoverTarget) {
        if self.drag_hover_maybe_active() {
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
        self.drag_hover.cursor = Some(pos);
    }

    #[cfg(windows)]
    pub(crate) fn drag_hover_external_left(&mut self) {
        // OLE calls IDropTarget::DragLeave on ESC-cancel and when the drag leaves
        // our window. A real drop calls Drop (→ mark_drag_committed) instead, so a
        // DragLeave without a commit means the drag was abandoned → undo nav.
        self.revert_drag_nav();
        self.drag_hover.clear();
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
        let hit = self
            .pending_drag_hover_targets
            .iter()
            .rev()
            .find(|target| {
                if !contains(target.bounds, pos) {
                    return false;
                }
                if in_dropzone {
                    return false;
                }
                true
            })
            .cloned();

        let scrolled = self.tick_edge_scroll(pos, now);

        let Some(target) = hit else {
            self.drag_hover.reset_target(None, now);
            return scrolled;
        };
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
        self.drag_hover.confirm_pulse_bounds = self.drag_hover.blink_bounds.take();
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
                fill.a = 0.34 * (1.0 - e);
                let mut ring = accent;
                ring.a = 0.85 * (1.0 - e);
                let mut glow = accent;
                glow.a = 0.5 * (1.0 - e);
                return Some(glow_rect(bounds, fill, ring, glow, 8.0 + 22.0 * e, 2.0));
            }
        }

        let bounds = self.drag_hover.blink_bounds?;
        let started = self.drag_hover.blink_started_at.unwrap_or(now);
        let elapsed = now.duration_since(started).as_secs_f32();
        let fade_in = ease_out_quart((elapsed / 0.18).clamp(0.0, 1.0));
        // ~1.6 Hz pulse mapped to 0..1.
        let pulse = 0.5 + 0.5 * (elapsed * std::f32::consts::TAU * 1.6).sin();
        let mut fill = accent;
        fill.a = (0.12 + 0.16 * pulse) * fade_in;
        let mut ring = accent;
        ring.a = (0.40 + 0.38 * pulse) * fade_in;
        let mut glow = accent;
        glow.a = (0.30 + 0.30 * pulse) * fade_in;
        Some(glow_rect(
            bounds,
            fill,
            ring,
            glow,
            10.0 + 12.0 * pulse,
            1.5,
        ))
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
        self.pending_drag_hover_targets
            .iter()
            .rev()
            .find(|target| contains(target.bounds, pos))
            .and_then(|target| target.drop_command.clone())
            .or_else(|| {
                self.pane_view_bounds
                    .iter()
                    .find(|(_, bounds)| contains(**bounds, pos))
                    .and_then(|(pane, _)| {
                        self.tab_data_for(*pane)
                            .and_then(|tab| match &tab.location {
                                Location::Tag(tag) => Some(DragHoverDropCommand::AssignTag(*tag)),
                                Location::RecycleBin => Some(DragHoverDropCommand::TrashPaths),
                                #[cfg(feature = "provider-host")]
                                Location::Remote { id, path } => {
                                    Some(DragHoverDropCommand::TransferTo(
                                        crate::app::cloud::remote_sentinel(id, path),
                                    ))
                                }
                                _ => None,
                            })
                    })
            })
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
            if contains(*bounds, pos) {
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

    pub(crate) fn execute_drag_hover_drop_command(
        &mut self,
        command: DragHoverDropCommand,
        paths: Vec<PathBuf>,
        from_self: bool,
    ) {
        self.mark_drag_committed();
        self.rubber = None;
        match command {
            DragHoverDropCommand::AssignTag(tag) => {
                self.clear_committed_dropzone_drag(&paths);
                self.process_actions(vec![Action::AssignTagToFiles { tag, paths }]);
            }
            DragHoverDropCommand::StageInSidebar => {
                let existing = self
                    .state
                    .sidebar_drop_zone
                    .iter()
                    .map(|item| item.path.clone())
                    .collect::<std::collections::HashSet<_>>();
                self.state.sidebar_drop_zone.extend(
                    paths
                        .into_iter()
                        .filter(|path| !existing.contains(path))
                        // Only stage real filesystem paths: virtual (archive/remote)
                        // entries make dead chips, and `$R` recycle payloads would
                        // later "place" with mangled names.
                        .filter(|path| {
                            path.exists() && !atlas_core::fs::paths::is_recycle_bin_path(path)
                        })
                        .map(|path| atlas_model::app_state::StagedDrop { path, from_self }),
                );
                self.cancel_dropzone_file_drag();
            }
            DragHoverDropCommand::TransferTo(dest) => {
                use atlas_model::ops::{
                    ConflictPolicy, TransferKind, TransferOrigin, TransferRequest,
                };
                let effect = crate::dnd::default_effect_for_paths(&paths, &dest);
                self.clear_committed_dropzone_drag(&paths);
                self.process_actions(vec![Action::Transfer(TransferRequest {
                    sources: paths,
                    target_names: None,
                    dest,
                    kind: if effect.is_move() {
                        TransferKind::Move
                    } else {
                        TransferKind::Copy
                    },
                    policy: ConflictPolicy::Ask,
                    origin: if from_self {
                        TransferOrigin::DndInternal
                    } else {
                        TransferOrigin::DndExternal
                    },
                })]);
            }
            DragHoverDropCommand::TrashPaths => {
                self.clear_committed_dropzone_drag(&paths);
                // Direct call (not Action::Delete): that action re-routes by the
                // *focused* tab and would purge instead of trash when a recycle
                // pane happens to hold focus.
                self.delete_files(paths, true);
            }
        }
    }

    pub(crate) fn drop_files_on_pane(
        &mut self,
        pane_id: TileId,
        paths: Vec<PathBuf>,
        external: bool,
    ) {
        self.mark_drag_committed();
        self.rubber = None;
        let Some(location) = self.tab_data_for(pane_id).map(|t| t.location.clone()) else {
            return;
        };
        match location {
            Location::Tag(tag) => {
                self.clear_committed_dropzone_drag(&paths);
                self.process_actions(vec![Action::AssignTagToFiles { tag, paths }]);
            }
            Location::Directory(dest) => {
                self.clear_committed_dropzone_drag(&paths);
                if external {
                    self.process_actions(vec![Action::CopyTo { paths, dest }]);
                } else {
                    self.process_actions(vec![Action::Move { paths, dest }]);
                }
            }
            Location::RecycleBin => {
                self.clear_committed_dropzone_drag(&paths);
                self.delete_files(paths, true);
            }
            Location::Archive { .. } => {
                // Writing into archives isn't supported yet — say so instead of
                // swallowing the drop.
                self.toast_error(rust_i18n::t!("dnd_archive_drop_blocked").into_owned());
            }
            #[cfg(feature = "provider-host")]
            Location::Remote { id, path } => {
                self.clear_committed_dropzone_drag(&paths);
                // Drop into a remote always uploads as copy; move-on-drop
                // (delete local source) when someone asks for it.
                let dest = crate::app::cloud::remote_sentinel(&id, &path);
                self.try_remote_upload(&paths, &dest, false);
            }
        }
    }

    pub(crate) fn begin_dropzone_file_drag(&mut self, paths: &[PathBuf]) {
        self.dropzone_drag_paths = paths.iter().cloned().collect();
    }

    pub(crate) fn cancel_dropzone_file_drag(&mut self) {
        self.dropzone_drag_paths.clear();
    }

    pub(crate) fn clear_committed_dropzone_drag(&mut self, paths: &[PathBuf]) {
        if self.dropzone_drag_paths.is_empty() {
            return;
        }
        let committed = paths
            .iter()
            .filter(|path| self.dropzone_drag_paths.contains(*path))
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        if committed.is_empty() {
            return;
        }
        self.state
            .sidebar_drop_zone
            .retain(|item| !committed.contains(&item.path));
        self.dropzone_selected
            .retain(|path| !committed.contains(path));
        self.dropzone_rects
            .retain(|(path, _)| !committed.contains(path));
        self.dropzone_drag_paths.clear();
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
    move |bounds, _window, cx| {
        if !view.read(cx).drag_hover_maybe_active() {
            return;
        }
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
    move |bounds, _window, cx| {
        if !view.read(cx).drag_hover_maybe_active() {
            return;
        }
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
                vu.update(cx, |this, _| this.drag_hover.clear());
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
            color: glow,
            offset: gpui::point(gpui::px(0.0), gpui::px(0.0)),
            blur_radius: gpui::px(blur),
            spread_radius: gpui::px(1.0),
            inset: false,
        }])
        .into_any_element()
}

fn contains(bounds: gpui::Bounds<gpui::Pixels>, pos: gpui::Point<gpui::Pixels>) -> bool {
    pos.x >= bounds.left()
        && pos.x <= bounds.right()
        && pos.y >= bounds.top()
        && pos.y <= bounds.bottom()
}
