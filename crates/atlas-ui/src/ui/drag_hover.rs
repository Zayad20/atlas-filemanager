//! Central drag-hover navigation.
//!
//! Renderers register window-space targets every frame; the global motor decides
//! when a held drag should navigate or autoscroll. This keeps Finder-style drag
//! affordances out of individual sidebar/chrome/view widgets.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::app::App;
use crate::input::action::Action;
use crate::state::layout_tree::TileId;
use crate::state::location::Location;
use crate::state::tag_cache::TagId;
use gpui::prelude::*;
use gpui::{canvas, Context, DispatchPhase, Entity, MouseMoveEvent, MouseUpEvent, Window};

pub(crate) const NAV_HOVER_DELAY: Duration = Duration::from_millis(350);
pub(crate) const FOLDER_HOVER_DELAY: Duration = Duration::from_millis(650);
const EDGE_SCROLL_ZONE: f32 = 44.0;
const EDGE_SCROLL_MIN_MS: u64 = 38;
const EDGE_SCROLL_MAX_MS: u64 = 105;
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
    TransferTo(PathBuf),
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
    next_scroll_at: Option<Instant>,
    pub(crate) blink_id: Option<DragHoverId>,
    pub(crate) blink_started_at: Option<Instant>,
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
    }

    fn clear(&mut self) {
        self.active = false;
        self.cursor = None;
        self.active_id = None;
        self.entered_at = None;
        self.last_fire = None;
        self.fired_id = None;
        self.next_scroll_at = None;
        self.blink_id = None;
        self.blink_started_at = None;
    }
}

impl App {
    pub(crate) fn begin_drag_hover(&mut self) {
        self.drag_hover.active = true;
    }

    pub(crate) fn register_drag_hover_target(&mut self, target: DragHoverTarget) {
        if self.drag_hover_maybe_active() {
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
        self.drag_hover.cursor = Some(pos);
    }

    #[cfg(windows)]
    pub(crate) fn drag_hover_external_left(&mut self) {
        self.drag_hover.clear();
    }

    pub(crate) fn tick_drag_hover(&mut self, cx: &mut Context<Self>) {
        let Some(pos) = self.drag_hover.cursor else {
            return;
        };
        let now = Instant::now();
        let hit = self
            .pending_drag_hover_targets
            .iter()
            .rev()
            .find(|target| contains(target.bounds, pos))
            .cloned();

        self.tick_edge_scroll(pos, now);

        let Some(target) = hit else {
            self.drag_hover.reset_target(None, now);
            return;
        };
        self.drag_hover.reset_target(Some(target.id.clone()), now);
        let delay = match target.kind {
            DragHoverKind::Nav => NAV_HOVER_DELAY,
            DragHoverKind::Folder => FOLDER_HOVER_DELAY,
        };
        let entered = self.drag_hover.entered_at.unwrap_or(now);
        if now.duration_since(entered) < delay {
            return;
        }
        if target.kind == DragHoverKind::Folder
            && self.drag_hover.blink_id.as_ref() != Some(&target.id)
        {
            self.drag_hover.blink_id = Some(target.id.clone());
            self.drag_hover.blink_started_at = Some(now);
            cx.notify();
            return;
        }
        if target.kind == DragHoverKind::Folder
            && self
                .drag_hover
                .blink_started_at
                .is_some_and(|t| now.duration_since(t) < Duration::from_millis(170))
        {
            return;
        }
        if self
            .drag_hover
            .fired_id
            .as_ref()
            .is_some_and(|id| id == &target.id)
        {
            return;
        }
        if self
            .drag_hover
            .last_fire
            .as_ref()
            .is_some_and(|(id, at)| id == &target.id && now.duration_since(*at) < REPEAT_NAV_GUARD)
        {
            return;
        }

        self.execute_drag_hover_command(target.command);
        self.drag_hover.last_fire = Some((target.id.clone(), now));
        self.drag_hover.fired_id = Some(target.id.clone());
        self.drag_hover.blink_id = None;
        self.drag_hover.blink_started_at = None;
        cx.notify();
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
                                _ => None,
                            })
                    })
            })
    }

    pub(crate) fn execute_drag_hover_drop_command(
        &mut self,
        command: DragHoverDropCommand,
        paths: Vec<PathBuf>,
        from_self: bool,
    ) {
        match command {
            DragHoverDropCommand::AssignTag(tag) => {
                self.process_actions(vec![Action::AssignTagToFiles { tag, paths }]);
            }
            DragHoverDropCommand::TransferTo(dest) => {
                use crate::state::ops::{
                    ConflictPolicy, TransferKind, TransferOrigin, TransferRequest,
                };
                let effect = crate::dnd::default_effect_for_paths(&paths, &dest);
                self.process_actions(vec![Action::Transfer(TransferRequest {
                    sources: paths,
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
        }
    }

    pub(crate) fn drop_files_on_pane(
        &mut self,
        pane_id: TileId,
        paths: Vec<PathBuf>,
        external: bool,
    ) {
        let Some(location) = self.tab_data_for(pane_id).map(|t| t.location.clone()) else {
            return;
        };
        match location {
            Location::Tag(tag) => {
                self.process_actions(vec![Action::AssignTagToFiles { tag, paths }]);
            }
            Location::Directory(dest) => {
                if external {
                    self.process_actions(vec![Action::CopyTo { paths, dest }]);
                } else {
                    self.process_actions(vec![Action::Move { paths, dest }]);
                }
            }
            _ => {}
        }
    }

    fn tick_edge_scroll(&mut self, pos: gpui::Point<gpui::Pixels>, now: Instant) {
        if self
            .drag_hover
            .next_scroll_at
            .is_some_and(|next| now < next)
        {
            return;
        }
        let Some((&pane, bounds)) = self
            .pane_view_bounds
            .iter()
            .find(|(_, bounds)| contains(**bounds, pos))
        else {
            return;
        };
        let y = f32::from(pos.y);
        let top = f32::from(bounds.top());
        let bottom = f32::from(bounds.bottom());
        let edge = if y < top + EDGE_SCROLL_ZONE {
            Some((
                true,
                ((top + EDGE_SCROLL_ZONE - y) / EDGE_SCROLL_ZONE).clamp(0.0, 1.0),
            ))
        } else if y > bottom - EDGE_SCROLL_ZONE {
            Some((
                false,
                ((y - (bottom - EDGE_SCROLL_ZONE)) / EDGE_SCROLL_ZONE).clamp(0.0, 1.0),
            ))
        } else {
            None
        };
        let Some((up, strength)) = edge else {
            return;
        };
        let (cols, rows) = self.rubber_scroll_dims(pane);
        if rows == 0 {
            return;
        }
        let current_row = self.visible_edge_row(pane, cols, up);
        let Some(current_row) = current_row else {
            return;
        };
        let step = if strength > 0.72 {
            3
        } else if strength > 0.42 {
            2
        } else {
            1
        };
        let target = if up {
            current_row.saturating_sub(step)
        } else {
            (current_row + step).min(rows.saturating_sub(1))
        };
        if target == current_row {
            return;
        }
        let strategy = if up {
            gpui::ScrollStrategy::Top
        } else {
            gpui::ScrollStrategy::Bottom
        };
        if self.scroll_grid_row_to(pane, target, strategy) {
            self.drag_hover.next_scroll_at =
                Some(now + Duration::from_millis(EDGE_SCROLL_MIN_MS.max(1)));
            return;
        }
        if let Some(handle) = self.scroll_handles.get(&pane) {
            handle.scroll_to_item_strict(target, strategy);
        }
        let ms = (EDGE_SCROLL_MAX_MS as f32
            - (EDGE_SCROLL_MAX_MS - EDGE_SCROLL_MIN_MS) as f32 * strength) as u64;
        self.drag_hover.next_scroll_at = Some(now + Duration::from_millis(ms.max(1)));
    }

    fn visible_edge_row(&self, pane: TileId, cols: usize, top: bool) -> Option<usize> {
        let rects = self.row_rects.get(&pane)?;
        let ts = self.state.tab_states.get(&pane)?;
        let mut row = if top { usize::MAX } else { 0 };
        for (idx, _) in rects {
            let Some(pos) = ts.visible.iter().position(|v| v == idx) else {
                continue;
            };
            let r = pos / cols.max(1);
            if top {
                row = row.min(r);
            } else {
                row = row.max(r);
            }
        }
        if top && row == usize::MAX {
            None
        } else {
            Some(row)
        }
    }

    fn execute_drag_hover_command(&mut self, command: DragHoverCommand) {
        match command {
            DragHoverCommand::NavigatePath(path) => {
                let tile = self.state.focused_tile;
                self.navigate(tile, path);
            }
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
                vu.update(cx, |this, _| this.drag_hover.clear());
            });
            if active {
                v.update(cx, |this, cx| this.tick_drag_hover(cx));
                window.request_animation_frame();
            }
        },
    )
    .absolute()
    .inset_0()
}

fn contains(bounds: gpui::Bounds<gpui::Pixels>, pos: gpui::Point<gpui::Pixels>) -> bool {
    pos.x >= bounds.left()
        && pos.x <= bounds.right()
        && pos.y >= bounds.top()
        && pos.y <= bounds.bottom()
}
