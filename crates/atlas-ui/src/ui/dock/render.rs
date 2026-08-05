use gpui::prelude::*;
use gpui::{
    div, px, relative, AnimationExt, AnyElement, Context, Entity, Focusable, MouseButton,
    SharedString, Window,
};
use gpui_component::{h_flex, v_flex, ActiveTheme as _, ElementExt as _};

use crate::app::{App, PaneAnimationKind};
use crate::input::action::Action;
use crate::ui::dock::drag::{tab_drag_entity, TabDrag};
use crate::ui::glass::{backdrop_tint, glass_chip_fill, glass_fill, GlassExt};
use crate::ui::icons::{icon, is_windows_theme, theme_icon, AtlasIcon};
use crate::ui::layout_region::{layout_region, timed_layout_region, LayoutRegionTimings};
use crate::ui::pane_panel::FilePanel;
use atlas_core::util::display::display_name;
use atlas_model::layout_tree::{Container, LinearDir, Tile, TileId};
use atlas_model::location::Location;
use std::time::Instant;

const DOCK_FLAT_HOST_TIMINGS: LayoutRegionTimings = LayoutRegionTimings {
    request_layout: "dock.flat_host.request_layout",
    prepaint: "dock.flat_host.prepaint",
    paint: "dock.flat_host.paint",
};
const DOCK_SPLITTER_TIMINGS: LayoutRegionTimings = LayoutRegionTimings {
    request_layout: "dock.splitters.request_layout",
    prepaint: "dock.splitters.prepaint",
    paint: "dock.splitters.paint",
};
const PANE_CACHED_HOST_TIMINGS: LayoutRegionTimings = LayoutRegionTimings {
    request_layout: "pane.cached_host.request_layout",
    prepaint: "pane.cached_host.prepaint",
    paint: "pane.cached_host.paint",
};
const PANE_SHELL_TIMINGS: LayoutRegionTimings = LayoutRegionTimings {
    request_layout: "pane.shell.request_layout",
    prepaint: "pane.shell.prepaint",
    paint: "pane.shell.paint",
};
const PANE_TABS_BAR_TIMINGS: LayoutRegionTimings = LayoutRegionTimings {
    request_layout: "pane.tabs_bar.request_layout",
    prepaint: "pane.tabs_bar.prepaint",
    paint: "pane.tabs_bar.paint",
};
const PANE_CONTENT_SHELL_TIMINGS: LayoutRegionTimings = LayoutRegionTimings {
    request_layout: "pane.content_shell.request_layout",
    prepaint: "pane.content_shell.prepaint",
    paint: "pane.content_shell.paint",
};
const PANE_CONTENT_LAYERS_TIMINGS: LayoutRegionTimings = LayoutRegionTimings {
    request_layout: "pane.content_layers.request_layout",
    prepaint: "pane.content_layers.prepaint",
    paint: "pane.content_layers.paint",
};
const PANE_BODY_TIMINGS: LayoutRegionTimings = LayoutRegionTimings {
    request_layout: "pane.body.request_layout",
    prepaint: "pane.body.prepaint",
    paint: "pane.body.paint",
};
const PANE_DROP_ZONES_TIMINGS: LayoutRegionTimings = LayoutRegionTimings {
    request_layout: "pane.drop_zones.request_layout",
    prepaint: "pane.drop_zones.prepaint",
    paint: "pane.drop_zones.paint",
};

#[derive(Clone, Copy, Debug)]
enum SplitMotion {
    Opening,
    Closing(Instant),
}

#[derive(Clone, Copy)]
struct FlatRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

struct FlatPane {
    tile: TileId,
    rect: FlatRect,
    /// Paint-only FLIP delta accumulated from animated linear ancestors.
    paint_offset: (f32, f32),
    /// Direction in which an exiting pane should slide beneath its survivor.
    close_axis: (f32, f32),
}

struct FlatSplitter {
    container: TileId,
    divider: usize,
    dir: LinearDir,
    rect: FlatRect,
    paint_offset: (f32, f32),
}

fn split_child_span(share: f32, share_sum: f32, available: f32, child_count: usize) -> f32 {
    if share_sum > f32::EPSILON {
        share / share_sum * available
    } else {
        available / child_count.max(1) as f32
    }
}

fn closing_splitter_index(pane_ix: usize, num_splitters: usize) -> Option<usize> {
    (num_splitters > 0).then(|| pane_ix.min(num_splitters - 1))
}

fn flip_axis_offset(old: f32, final_position: f32, progress: f32) -> f32 {
    (old - final_position) * (1.0 - progress)
}

/// Tabs-like tile (tab strip + body) hosted in its own entity so it can be embedded
/// with `AnyView::cached` (Zed's `dock.rs:1185` pattern: the whole pane, including
/// the tab bar, is cached). Rendering delegates to `App::render_tabs_like`;
/// invalidation is decided by the skeleton fingerprint in `render_cached_tabs`.
pub(crate) struct PaneTabsView {
    app: gpui::WeakEntity<App>,
    tile: TileId,
}

impl gpui::Render for PaneTabsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf::count("render.pane_tabs_view");
        let tile = self.tile;
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        app.update(cx, |app, cx| {
            let (children, active) = match app.state.pane_tree.tiles.get(tile) {
                Some(Tile::Pane(_)) => (vec![tile], Some(tile)),
                Some(Tile::Container(Container::Tabs(tabs))) => {
                    let children = tabs.children.clone();
                    let active = tabs
                        .active
                        .filter(|a| children.contains(a))
                        .or_else(|| children.first().copied());
                    (children, active)
                }
                _ => return div().size_full().into_any_element(),
            };
            if children.is_empty() {
                return div().size_full().into_any_element();
            }
            app.render_tabs_like(children, active, window, cx)
        })
    }
}

fn split_drop_zone(kind: AtlasIcon) -> gpui::Div {
    div()
        .absolute()
        .opacity(0.0)
        .bg(gpui::transparent_black())
        .drag_over::<TabDrag>(|s, _, _, _| s.opacity(1.0).bg(gpui::hsla(0.58, 0.70, 0.42, 0.24)))
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size(crate::ui::rem_px(58.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(gpui::hsla(0.58, 0.84, 0.58, 0.78))
                        .bg(gpui::hsla(0.58, 0.78, 0.30, 0.58))
                        .shadow_lg()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(kind, 26.0, gpui::hsla(0.58, 0.90, 0.84, 1.0))),
                ),
        )
}

impl App {
    pub(crate) fn render_custom_dock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.tick_tab_close() {
            // Self-drive the next frame on the host view (DockHost), not App.
            window.request_animation_frame();
        }
        crate::perf::count("render.dock");
        let _perf = crate::perf::span_detail("dock.render", || {
            format!(
                "tiles={} cached_panels={} focused={}",
                self.state.pane_tree.tiles.iter().count(),
                self.file_panel_cache.len(),
                self.state.focused_tile.0
            )
        });
        // Focus is synced from `App::render` (root.rs), not here: hosting the dock in
        // its own `DockHost` view means this render no longer runs every App frame, and
        // pane focus must be re-asserted every frame (see `sync_overlay_focus`).
        let is_dragging = cx.has_active_drag();
        if !is_dragging && self.state.dragging_pane.is_some() {
            if !self.state.drag_pending {
                let pane_id = self.state.dragging_pane.unwrap();
                self.state.dragging_pane = None;
                self.animate_pane_drag(pane_id, false);
                // Settle frame on the host view (DockHost), not App.
                window.request_animation_frame();
            }
        } else if is_dragging {
            self.state.drag_pending = false;
        }

        // Poda de views/fingerprints de tiles que ya no existen (split/close/dnd).
        let tiles = &self.state.pane_tree.tiles;
        self.pane_tabs_cache
            .retain(|id, _| tiles.get(*id).is_some());
        self.pane_tabs_fp.retain(|id, _| tiles.get(*id).is_some());
        self.pane_tabs_bounds
            .retain(|id, _| tiles.get(*id).is_some());
        self.details_row_rings
            .retain(|id, _| tiles.get(*id).is_some());

        let root = self.state.pane_tree.root;
        let content = root
            .and_then(|root| {
                self.render_flat_tree(root, window, cx)
                    .or_else(|| self.render_tree(root, window, cx))
            })
            .unwrap_or_else(|| div().size_full().into_any_element());
        let view = cx.entity();
        div()
            .relative()
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
            .child(content)
            .on_prepaint(move |bounds, window, cx| {
                view.update(cx, |this, _cx| {
                    let old = this
                        .dock_root_bounds
                        .map(|b| (f32::from(b.size.width), f32::from(b.size.height)));
                    let new = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                    if old != Some(new) {
                        this.dock_root_bounds = Some(bounds);
                        window.request_animation_frame();
                    }
                });
            })
            .into_any_element()
    }

    fn render_flat_tree(
        &mut self,
        root: TileId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let bounds = self.dock_root_bounds?;
        let w = f32::from(bounds.size.width);
        let h = f32::from(bounds.size.height);
        if w <= 1.0 || h <= 1.0 {
            return None;
        }

        let mut panes = Vec::new();
        let mut splitters = Vec::new();
        let rect = FlatRect {
            x: 0.0,
            y: 0.0,
            w,
            h,
        };
        // Only the newest interrupted/restarted split supplies visual motion.
        // Older entries may still be settling, but combining axes makes a lateral
        // split travel diagonally.
        let active_animation = self
            .dock_split_animations
            .values()
            .filter(|animation| !animation.done())
            .max_by_key(|animation| animation.started)
            .map(|animation| animation.started);
        if !self.collect_flat_layout(
            root,
            rect,
            window,
            cx,
            &mut panes,
            &mut splitters,
            (0.0, 0.0),
            (0.0, 0.0),
            active_animation,
        ) {
            return None;
        }

        let view = cx.entity();
        let flip_active = active_animation.is_some();
        let mut host = div().absolute().inset_0().overflow_hidden();
        let mut closing_panes = Vec::new();
        let mut opening_panes = Vec::new();
        let mut live_panes = Vec::new();

        for pane in panes {
            let motion = self.split_motion_for_tile(pane.tile);
            let is_closing = matches!(motion, Some(SplitMotion::Closing(_)));
            let previous = self.pane_tabs_bounds.get(&pane.tile).map(|old| FlatRect {
                x: f32::from(old.origin.x - bounds.origin.x),
                y: f32::from(old.origin.y - bounds.origin.y),
                w: f32::from(old.size.width),
                h: f32::from(old.size.height),
            });
            // The tree keeps an exiting pane until the close timer elapses, but its
            // final share is zero. Paint that one cached view at its previous bounds
            // as an overlay while every survivor uses final layout immediately.
            let rect = if is_closing {
                previous.unwrap_or(pane.rect)
            } else {
                pane.rect
            };
            let content = if rect.w <= 4.0 || rect.h <= 4.0 {
                div().size_full().into_any_element()
            } else {
                self.render_cached_tabs(pane.tile, window, cx)
            };
            let flip_offset = if is_closing {
                gpui::point(px(0.0), px(0.0))
            } else {
                gpui::point(px(pane.paint_offset.0), px(pane.paint_offset.1))
            };
            let animated = div()
                .size_full()
                .paint_offset(flip_offset)
                .child(self.render_split_intro(pane.tile, content, motion, pane.close_axis));
            let pane = div()
                .absolute()
                .left(px(rect.x))
                .top(px(rect.y))
                .w(px(rect.w.max(0.0)))
                .h(px(rect.h.max(0.0)))
                .min_w(px(0.))
                .min_h(px(0.))
                // A final-bounds clip can hide most of a translated survivor.
                // Let panes overlap while FLIP is active; the dock root remains clipped.
                .when(!is_closing && !flip_active, |this| this.overflow_hidden())
                .child(animated);
            if is_closing {
                closing_panes.push(pane);
            } else if matches!(motion, Some(SplitMotion::Opening)) {
                opening_panes.push(pane);
            } else {
                live_panes.push(pane);
            }
        }

        // Transition panes stay underneath. Survivors slide over them from their
        // previous origins, which reveals an opening pane and covers a closing one
        // without a second pop/scale/exit animation on the pane itself.
        for pane in closing_panes
            .into_iter()
            .chain(opening_panes)
            .chain(live_panes)
        {
            host = host.child(layout_region("dock.pane_slot", pane));
        }

        for splitter in splitters {
            let thickness = match splitter.dir {
                LinearDir::Horizontal => splitter.rect.w,
                LinearDir::Vertical => splitter.rect.h,
            };
            let splitter_el = crate::ui::dock::resize::splitter(
                splitter.container,
                splitter.divider,
                splitter.dir,
                &view,
                cx,
                thickness,
            );
            host = host.child(timed_layout_region(
                "dock.splitters",
                DOCK_SPLITTER_TIMINGS,
                div()
                    .absolute()
                    .left(px(splitter.rect.x))
                    .top(px(splitter.rect.y))
                    .w(px(splitter.rect.w.max(0.0)))
                    .h(px(splitter.rect.h.max(0.0)))
                    .paint_offset(gpui::point(
                        px(splitter.paint_offset.0),
                        px(splitter.paint_offset.1),
                    ))
                    .child(splitter_el),
            ));
        }

        Some(timed_layout_region("dock.flat_host", DOCK_FLAT_HOST_TIMINGS, host).into_any_element())
    }

    fn collect_flat_layout(
        &mut self,
        id: TileId,
        rect: FlatRect,
        window: &mut Window,
        cx: &mut Context<Self>,
        panes: &mut Vec<FlatPane>,
        splitters: &mut Vec<FlatSplitter>,
        close_axis: (f32, f32),
        paint_offset: (f32, f32),
        active_animation: Option<Instant>,
    ) -> bool {
        match self.state.pane_tree.tiles.get(id).cloned() {
            Some(Tile::Pane(_)) => {
                panes.push(FlatPane {
                    tile: id,
                    rect,
                    paint_offset,
                    close_axis,
                });
                true
            }
            Some(Tile::Container(Container::Tabs(tabs))) => {
                if tabs.children.is_empty() {
                    false
                } else {
                    panes.push(FlatPane {
                        tile: id,
                        rect,
                        paint_offset,
                        close_axis,
                    });
                    true
                }
            }
            Some(Tile::Container(Container::Linear(linear))) => {
                if linear.children.is_empty() {
                    return false;
                }
                self.dock_linear_bounds.insert(
                    id,
                    gpui::Bounds {
                        origin: gpui::point(gpui::px(rect.x), gpui::px(rect.y)),
                        size: gpui::size(gpui::px(rect.w), gpui::px(rect.h)),
                    },
                );

                let mut final_shares = linear.shares.clone();
                if cx.has_active_drag() {
                    if let Some(dragged) = self.state.dragging_pane {
                        for (ix, child_id) in linear.children.iter().enumerate() {
                            if ix < final_shares.len()
                                && self.state.container_contains_pane(*child_id, dragged)
                            {
                                final_shares[ix] = 0.0;
                            }
                        }
                    }
                }
                let closing_layout = self
                    .dock_split_animations
                    .get(&id)
                    .is_some_and(|animation| animation.closing)
                    || linear.children.iter().any(|child| {
                        matches!(
                            self.split_motion_for_tile(*child),
                            Some(SplitMotion::Closing(_))
                        )
                    });
                if closing_layout {
                    crate::app::normalize_split_shares_allow_zero(&mut final_shares);
                } else {
                    crate::app::normalize_split_shares(&mut final_shares);
                }
                let flip_animation = self
                    .dock_split_animations
                    .get(&id)
                    .filter(|animation| {
                        Some(animation.started) == active_animation
                            && !animation.done()
                            && animation.from.len() == linear.children.len()
                    })
                    .cloned();
                let sum: f32 = final_shares.iter().copied().filter(|v| *v > 0.0).sum();
                let shares = if final_shares.len() == linear.children.len() && sum > f32::EPSILON {
                    self.dock_split_shares(id, final_shares, window, cx)
                } else {
                    vec![1.0; linear.children.len()]
                };
                let share_sum = shares.iter().copied().filter(|v| *v > 0.0).sum::<f32>();

                let splitter_size = 5.0;
                let span = match linear.dir {
                    LinearDir::Horizontal => rect.w,
                    LinearDir::Vertical => rect.h,
                };
                let num_splitters = linear.children.len().saturating_sub(1);
                let closing_ix = self
                    .dock_split_animations
                    .get(&id)
                    .filter(|animation| animation.closing)
                    .and_then(|animation| animation.closing_ix)
                    .or_else(|| {
                        linear.children.iter().position(|child| {
                            matches!(
                                self.split_motion_for_tile(*child),
                                Some(SplitMotion::Closing(_))
                            )
                        })
                    });
                let closing_splitter_ix =
                    closing_ix.and_then(|pane_ix| closing_splitter_index(pane_ix, num_splitters));
                let mut splitter_gaps = vec![splitter_size; num_splitters];
                if let Some(splitter_ix) = closing_splitter_ix {
                    splitter_gaps[splitter_ix] = 0.0;
                }
                let start_splitter_gaps = vec![splitter_size; num_splitters];
                let available = (span - splitter_gaps.iter().sum::<f32>()).max(0.0);
                let start_available = (span - start_splitter_gaps.iter().sum::<f32>()).max(0.0);
                let (start_shares, progress) = if let Some(animation) = flip_animation {
                    let progress = animation.progress();
                    (animation.from, progress)
                } else {
                    (shares.clone(), 1.0)
                };
                let start_share_sum = start_shares
                    .iter()
                    .copied()
                    .filter(|share| *share > 0.0)
                    .sum::<f32>();
                let mut offset = match linear.dir {
                    LinearDir::Horizontal => rect.x,
                    LinearDir::Vertical => rect.y,
                };
                let mut start_offset = offset;
                let mut any = false;

                for (ix, child_id) in linear.children.iter().copied().enumerate() {
                    if ix > 0 {
                        let gap = splitter_gaps[ix - 1];
                        let start_gap = start_splitter_gaps[ix - 1];
                        let splitter_axis_offset = flip_axis_offset(start_offset, offset, progress);
                        let splitter_paint_offset = match linear.dir {
                            LinearDir::Horizontal => {
                                (paint_offset.0 + splitter_axis_offset, paint_offset.1)
                            }
                            LinearDir::Vertical => {
                                (paint_offset.0, paint_offset.1 + splitter_axis_offset)
                            }
                        };
                        let srect = match linear.dir {
                            LinearDir::Horizontal => FlatRect {
                                x: offset,
                                y: rect.y,
                                w: gap,
                                h: rect.h,
                            },
                            LinearDir::Vertical => FlatRect {
                                x: rect.x,
                                y: offset,
                                w: rect.w,
                                h: gap,
                            },
                        };
                        splitters.push(FlatSplitter {
                            container: id,
                            divider: ix - 1,
                            dir: linear.dir,
                            rect: srect,
                            paint_offset: splitter_paint_offset,
                        });
                        offset += gap;
                        start_offset += start_gap;
                    }

                    let share = if closing_layout {
                        shares.get(ix).copied().unwrap_or(1.0).max(0.0)
                    } else {
                        shares.get(ix).copied().unwrap_or(1.0).max(0.01)
                    };
                    let child_span =
                        split_child_span(share, share_sum, available, linear.children.len());
                    let start_share = if closing_layout {
                        start_shares.get(ix).copied().unwrap_or(1.0).max(0.0)
                    } else {
                        start_shares.get(ix).copied().unwrap_or(1.0).max(0.01)
                    };
                    let start_child_span = split_child_span(
                        start_share,
                        start_share_sum,
                        start_available,
                        linear.children.len(),
                    );
                    let child_axis_offset = flip_axis_offset(start_offset, offset, progress);
                    let child_paint_offset = match linear.dir {
                        LinearDir::Horizontal => {
                            (paint_offset.0 + child_axis_offset, paint_offset.1)
                        }
                        LinearDir::Vertical => (paint_offset.0, paint_offset.1 + child_axis_offset),
                    };
                    let child_rect = match linear.dir {
                        LinearDir::Horizontal => FlatRect {
                            x: offset,
                            y: rect.y,
                            w: child_span,
                            h: rect.h,
                        },
                        LinearDir::Vertical => FlatRect {
                            x: rect.x,
                            y: offset,
                            w: rect.w,
                            h: child_span,
                        },
                    };
                    offset += child_span;
                    start_offset += start_child_span;
                    let child_close_axis = if matches!(
                        self.split_motion_for_tile(child_id),
                        Some(SplitMotion::Closing(_))
                    ) {
                        match linear.dir {
                            LinearDir::Horizontal => {
                                if ix + 1 < linear.children.len() {
                                    (1.0, 0.0)
                                } else {
                                    (-1.0, 0.0)
                                }
                            }
                            LinearDir::Vertical => {
                                if ix + 1 < linear.children.len() {
                                    (0.0, 1.0)
                                } else {
                                    (0.0, -1.0)
                                }
                            }
                        }
                    } else {
                        close_axis
                    };
                    any |= self.collect_flat_layout(
                        child_id,
                        child_rect,
                        window,
                        cx,
                        panes,
                        splitters,
                        child_close_axis,
                        child_paint_offset,
                        active_animation,
                    );
                }
                any
            }
            None => false,
        }
    }

    fn render_tree(
        &mut self,
        id: TileId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        crate::perf::count("dock.render_tree");
        match self.state.pane_tree.tiles.get(id)?.clone() {
            Tile::Pane(_) => Some(self.render_cached_tabs(id, window, cx)),
            Tile::Container(Container::Tabs(tabs)) => {
                if tabs.children.is_empty() {
                    return None;
                }
                Some(self.render_cached_tabs(id, window, cx))
            }
            Tile::Container(Container::Linear(linear)) => {
                let mut children = Vec::new();
                for child in &linear.children {
                    if let Some(el) = self.render_tree(*child, window, cx) {
                        children.push((*child, el, self.split_motion_for_tile(*child)));
                    }
                }
                if children.is_empty() {
                    return None;
                }
                let container_bounds = self.dock_linear_bounds.get(&id).copied();
                let exact_span = if linear.dir == LinearDir::Horizontal {
                    container_bounds.map(|b| f32::from(b.size.width))
                } else {
                    container_bounds.map(|b| f32::from(b.size.height))
                };

                let mut row = if exact_span.is_some() {
                    div().relative()
                } else {
                    if linear.dir == LinearDir::Horizontal {
                        h_flex()
                    } else {
                        v_flex()
                    }
                }
                .size_full()
                .min_w(px(0.))
                .min_h(px(0.));

                if exact_span.is_none() {
                    row = row.items_stretch();
                }

                let mut final_shares = linear.shares.clone();
                if cx.has_active_drag() {
                    if let Some(dragged) = self.state.dragging_pane {
                        for (ix, child_id) in linear.children.iter().enumerate() {
                            if self.state.container_contains_pane(*child_id, dragged) {
                                final_shares[ix] = 0.0;
                            }
                        }
                    }
                }
                let closing_layout = self
                    .dock_split_animations
                    .get(&id)
                    .is_some_and(|animation| animation.closing)
                    || linear.children.iter().any(|child| {
                        matches!(
                            self.split_motion_for_tile(*child),
                            Some(SplitMotion::Closing(_))
                        )
                    });
                if closing_layout {
                    crate::app::normalize_split_shares_allow_zero(&mut final_shares);
                } else {
                    crate::app::normalize_split_shares(&mut final_shares);
                }
                let sum: f32 = final_shares.iter().copied().filter(|v| *v > 0.0).sum();
                let shares = if final_shares.len() == children.len() && sum > f32::EPSILON {
                    self.dock_split_shares(id, final_shares, window, cx)
                } else {
                    vec![1.0; children.len()]
                };
                let share_sum = shares.iter().copied().filter(|v| *v > 0.0).sum::<f32>();

                let view = cx.entity();
                let child_count = children.len();
                let num_splitters = child_count.saturating_sub(1);
                let splitter_size = 5.0; // from resize.rs splitter thickness
                let closing_ix = self
                    .dock_split_animations
                    .get(&id)
                    .filter(|animation| animation.closing)
                    .and_then(|animation| animation.closing_ix)
                    .or_else(|| {
                        linear.children.iter().position(|child| {
                            matches!(
                                self.split_motion_for_tile(*child),
                                Some(SplitMotion::Closing(_))
                            )
                        })
                    });
                let closing_splitter_ix =
                    closing_ix.and_then(|pane_ix| closing_splitter_index(pane_ix, num_splitters));
                let mut splitter_gaps = vec![splitter_size; num_splitters];
                if let Some(splitter_ix) = closing_splitter_ix {
                    splitter_gaps[splitter_ix] = 0.0;
                }
                let total_splitter_size = splitter_gaps.iter().sum::<f32>();
                let mut current_offset = 0.0;

                for (ix, (child_id, child, intro)) in children.into_iter().enumerate() {
                    let share = if closing_layout {
                        shares[ix].max(0.0)
                    } else {
                        shares[ix].max(0.01)
                    };

                    if ix > 0 {
                        let gap = splitter_gaps[ix - 1];
                        let splitter_el = crate::ui::dock::resize::splitter(
                            id,
                            ix - 1,
                            linear.dir,
                            &view,
                            cx,
                            gap,
                        );
                        if exact_span.is_some() {
                            let mut s_div = div().absolute();
                            if linear.dir == LinearDir::Horizontal {
                                s_div =
                                    s_div.left(px(current_offset)).top_0().bottom_0().w(px(gap));
                            } else {
                                s_div = s_div.top(px(current_offset)).left_0().right_0().h(px(gap));
                            }
                            row = row.child(s_div.child(splitter_el));
                            current_offset += gap;
                        } else {
                            row = row.child(splitter_el);
                        }
                    }

                    let child_span = if let Some(span) = exact_span {
                        let available = (span - total_splitter_size).max(0.0);
                        Some(split_child_span(share, share_sum, available, child_count))
                    } else {
                        None
                    };

                    let mut pane_div = div().size_full().min_w(px(0.)).min_h(px(0.));

                    if let Some(span) = child_span {
                        pane_div = pane_div.absolute();
                        if linear.dir == LinearDir::Horizontal {
                            pane_div = pane_div
                                .left(px(current_offset))
                                .top_0()
                                .bottom_0()
                                .w(px(span));
                        } else {
                            pane_div = pane_div
                                .top(px(current_offset))
                                .left_0()
                                .right_0()
                                .h(px(span));
                        }
                        current_offset += span;
                    } else {
                        pane_div = pane_div.flex_grow(share).flex_basis(px(0.));
                    }

                    // A closing pane whose span has already collapsed paints no content:
                    // this avoids the 0.01-share sliver between seam animation end
                    // (~170ms), fade end (~250ms), and unmount (~200ms).
                    let collapsed_closing = matches!(intro, Some(SplitMotion::Closing(_)))
                        && child_span.is_some_and(|s| s <= 6.0);
                    let content = if collapsed_closing {
                        div().size_full().into_any_element()
                    } else {
                        let close_axis = if matches!(intro, Some(SplitMotion::Closing(_))) {
                            match linear.dir {
                                LinearDir::Horizontal => {
                                    if ix + 1 < child_count {
                                        (1.0, 0.0)
                                    } else {
                                        (-1.0, 0.0)
                                    }
                                }
                                LinearDir::Vertical => {
                                    if ix + 1 < child_count {
                                        (0.0, 1.0)
                                    } else {
                                        (0.0, -1.0)
                                    }
                                }
                            }
                        } else {
                            (0.0, 0.0)
                        };
                        self.render_split_intro(child_id, child, intro, close_axis)
                    };
                    row = row.child(pane_div.child(content));
                }

                let view = cx.entity();
                Some(
                    row.on_prepaint(move |bounds, _window, cx| {
                        crate::perf::count("prepaint.dock_linear");
                        view.update(cx, |this, _cx| {
                            this.dock_linear_bounds.insert(id, bounds);
                        });
                    })
                    .into_any_element(),
                )
            }
        }
    }

    fn split_motion_for_tile(&self, id: TileId) -> Option<SplitMotion> {
        match self.state.pane_tree.tiles.get(id)? {
            Tile::Pane(_) => self
                .pane_closing
                .get(&id)
                .copied()
                .map(SplitMotion::Closing)
                .or_else(|| {
                    self.pane_animation_for(id)
                        .filter(|animation| {
                            matches!(
                                animation.kind,
                                PaneAnimationKind::SplitHorizontal
                                    | PaneAnimationKind::SplitVertical
                            )
                        })
                        .map(|_| SplitMotion::Opening)
                }),
            Tile::Container(Container::Tabs(tabs)) => tabs
                .children
                .iter()
                .find_map(|child| self.split_motion_for_tile(*child)),
            Tile::Container(Container::Linear(_)) => None,
        }
    }

    fn render_split_intro(
        &self,
        id: TileId,
        child: AnyElement,
        motion: Option<SplitMotion>,
        close_axis: (f32, f32),
    ) -> AnyElement {
        let is_closing = matches!(motion, Some(SplitMotion::Closing(_)));
        let inner = div()
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .when(!is_closing, |this| this.overflow_hidden())
            .child(child);
        let Some(motion) = motion else {
            return inner.into_any_element();
        };
        let SplitMotion::Closing(started) = motion else {
            // Opening is revealed exclusively by the surrounding panes' FLIP.
            // Adding a second offset here is the bottom/right pop-in the user saw.
            return inner.into_any_element();
        };
        inner
            .with_animation(
                crate::ui::motion::keyed("dock-split-pop", id.0, ("close", started)),
                crate::ui::motion::linear_anim(crate::app::DockSplitAnimation::DURATION),
                move |this, t| {
                    let progress = crate::app::DockSplitAnimation::closing_progress(t);
                    let slide = 10.0 * progress;
                    this.paint_offset(gpui::point(
                        px(close_axis.0 * slide),
                        px(close_axis.1 * slide),
                    ))
                },
            )
            .into_any_element()
    }

    fn dock_split_shares(
        &mut self,
        id: TileId,
        final_shares: Vec<f32>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Vec<f32> {
        let Some(anim) = self.dock_split_animations.get(&id) else {
            return final_shares;
        };
        if anim.to.len() != final_shares.len() {
            self.dock_split_animations.remove(&id);
            return final_shares;
        }
        if anim.done() || self.state.settings.disable_animations {
            self.dock_split_animations.remove(&id);
            return final_shares;
        }
        // FLIP: layout jumps to final shares once. Cached paint moves from the
        // previous origins, so Taffy/prepaint do not repeat on every animation frame.
        window.request_animation_frame();
        let progress = anim.progress();
        let elapsed_ms = anim.started.elapsed().as_millis();
        if anim.closing && (elapsed_ms < 20 || elapsed_ms % 80 < 18) {
            crate::perf::event(|| {
                format!(
                    "pane_close.flip_frame parent={} elapsed_ms={} progress={:.3}",
                    id.0, elapsed_ms, progress
                )
            });
        }
        final_shares
    }

    /// Embed the tabs-like tile as a cached `PaneTabsView`. The fingerprint captures
    /// every bit of state read by `render_tabs_like`/`render_tab_strip`; if it did not
    /// change, the view is not notified and GPUI reuses the previous layout, hitboxes,
    /// and paint (pane chrome costs one Taffy node).
    fn render_cached_tabs(
        &mut self,
        id: TileId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fp = self.tabs_fingerprint(id, window, cx);
        let view = if let Some(view) = self.pane_tabs_cache.get(&id) {
            view.clone()
        } else {
            let app = cx.entity().downgrade();
            let view = cx.new(|_| PaneTabsView { app, tile: id });
            self.pane_tabs_cache.insert(id, view.clone());
            view
        };
        if self.pane_tabs_fp.insert(id, fp) != Some(fp) {
            view.update(cx, |_, cx| cx.notify());
        }
        let style = gpui::StyleRefinement::default().size_full();

        let app = cx.entity();
        let host = div()
            .size_full()
            .overflow_hidden()
            .child(layout_region(
                "pane.cached_view",
                gpui::AnyView::from(view).cached(style),
            ))
            .on_prepaint(move |bounds, _window, cx| {
                let should_update = {
                    let app_ref = app.read(cx);
                    app_ref.dock_split_animations.is_empty()
                        && app_ref.state.pane_tree.tiles.get(id).is_some()
                        && app_ref.pane_tabs_bounds.get(&id) != Some(&bounds)
                };
                if should_update {
                    app.update(cx, |this, _cx| {
                        this.pane_tabs_bounds.insert(id, bounds);
                    });
                }
            });
        timed_layout_region("pane.cached_host", PANE_CACHED_HOST_TIMINGS, host).into_any_element()
    }

    fn tabs_fingerprint(&self, id: TileId, window: &Window, cx: &Context<Self>) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let epoch = self.startup_started;
        let hash_instant = |h: &mut std::collections::hash_map::DefaultHasher, i: Instant| {
            i.saturating_duration_since(epoch).as_nanos().hash(h);
        };

        let children: Vec<TileId> = match self.state.pane_tree.tiles.get(id) {
            Some(Tile::Pane(_)) => vec![id],
            Some(Tile::Container(Container::Tabs(tabs))) => {
                tabs.active.map(|a| a.0).hash(&mut h);
                tabs.children.clone()
            }
            _ => Vec::new(),
        };
        for child in &children {
            child.0.hash(&mut h);
            if let Some(t) = self.tab_data_for(*child) {
                t.location.display_path().hash(&mut h);
                if let Some(p) = t.location.fs_dir() {
                    self.grid_thumbnail(&p.to_path_buf()).is_some().hash(&mut h);
                }
            }
            if let Some(started) = self.tab_closing.get(child) {
                hash_instant(&mut h, *started);
            }
            self.pane_closing.contains_key(child).hash(&mut h);
            if let Some(anim) = (!self.state.settings.disable_animations)
                .then(|| self.pane_animation_for(*child))
                .flatten()
            {
                std::mem::discriminant(&anim.kind).hash(&mut h);
                hash_instant(&mut h, anim.started);
            }
        }
        // Global state that changes chrome, Alt overlay for alt-drag
        // drag, and active drag state for body drop zones.
        self.state.profiles.active().theme.hash(&mut h);
        self.state.settings.disable_animations.hash(&mut h);
        self.state.settings.dim_inactive_panes.hash(&mut h);
        if self.state.settings.dim_inactive_panes {
            self.state.focused_tile.0.hash(&mut h);
        }
        window.modifiers().alt.hash(&mut h);
        cx.has_active_drag().hash(&mut h);
        self.state.dragging_pane.map(|p| p.0).hash(&mut h);
        self.state.drag_pending.hash(&mut h);
        self.rename_edit
            .as_ref()
            .map(|edit| edit.pane.0)
            .hash(&mut h);
        h.finish()
    }

    fn render_tabs_like(
        &mut self,
        children: Vec<TileId>,
        active: Option<TileId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        crate::perf::count("render.tabs_like");
        let _perf = crate::perf::span_detail("dock.tabs", || {
            format!(
                "children={} active={}",
                children.len(),
                active.map_or(0, |id| id.0)
            )
        });
        let active = active
            .filter(|id| children.contains(id))
            .or_else(|| children.first().copied());
        let active_theme = self.state.profiles.active().theme.as_str();
        let figma_glass = active_theme == "Atlas Figma Glass";
        let windows_glass = active_theme == "Atlas Windows Glass";
        let glass_theme = active_theme.ends_with(" Glass") || active_theme == "Atlas Frost";
        let panel_bg = if figma_glass {
            glass_fill()
        } else if windows_glass {
            cx.theme().secondary
        } else if is_windows_theme(active_theme) {
            gpui::hsla(0., 0., 1., 0.20)
        } else if glass_theme {
            backdrop_tint(cx.theme().secondary, 0.06)
        } else {
            backdrop_tint(cx.theme().secondary, 0.24)
        };
        let active_id = active.unwrap_or_else(|| *children.first().unwrap());
        let content = active
            .map(|id| {
                if self.should_render_pane_body(id) {
                    // Zed pattern (pane_group.rs:554): pane cached as AnyView. If the
                    // pane was not notified and its bounds did not change, GPUI reuses
                    // the previous layout, hitboxes, and primitives: one Taffy node
                    // instead of roughly 250.
                    gpui::AnyView::from(self.get_or_make_file_panel(id, cx))
                        .cached(gpui::StyleRefinement::default().size_full())
                        .into_any_element()
                } else {
                    crate::perf::count("render.pane_body_skipped");
                    div().size_full().bg(panel_bg).into_any_element()
                }
            })
            .unwrap_or_else(|| div().size_full().bg(panel_bg).into_any_element());
        let content = if let Some(active) = active {
            self.with_split_drop_zones(active, content, cx)
        } else {
            content
        };

        let panel = FastPaneShell::new(
            active_id,
            self.render_tab_strip(children, active, window, cx),
            content,
        )
        .size_full()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_hidden()
        .when(
            self.state.settings.dim_inactive_panes && self.state.focused_tile != active_id,
            |s| s.opacity(0.65),
        )
        .when(figma_glass, |s| s.glass_surface(panel_bg))
        .when(!figma_glass, |s| {
            s.rounded(cx.theme().radius_lg)
                .bg(panel_bg)
                .border_1()
                .border_color(cx.theme().border.opacity(0.55))
        });

        let is_mod = window.modifiers().alt;
        let activate_drag_app = cx.entity().clone();
        let activate_drag_app_down = cx.entity().clone();
        let activate_drag_app_up = cx.entity().clone();
        let app_for_resize = cx.entity().clone();

        let panel = if is_mod {
            div()
                .size_full()
                .relative()
                .child(panel)
                .child(layout_region(
                    "pane.drag_overlay",
                    div()
                        .id(gpui::SharedString::from(format!(
                            "alt-drag-{}",
                            active_id.0
                        )))
                        .absolute()
                        .inset_0()
                        .cursor_grab()
                        .on_drag(
                            TabDrag {
                                pane_id: active_id,
                                content: self.get_or_make_file_panel(active_id, cx).clone().into(),
                                size: self.pane_view_bounds.get(&active_id).map(|b| b.size),
                            },
                            move |drag, _pos, _window, cx| {
                                activate_drag_app.update(cx, |this, cx| {
                                    if this.state.dragging_pane != Some(drag.pane_id) {
                                        this.animate_pane_drag(drag.pane_id, true);
                                        this.state.dragging_pane = Some(drag.pane_id);
                                        this.state.drag_pending = false;
                                        cx.notify();
                                    }
                                });
                                tab_drag_entity(drag.pane_id, drag.content.clone(), drag.size, cx)
                            },
                        )
                        .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                            activate_drag_app_down.update(cx, |this, cx| {
                                if this.state.dragging_pane != Some(active_id) {
                                    this.animate_pane_drag(active_id, true);
                                    this.state.dragging_pane = Some(active_id);
                                    this.state.drag_pending = true;
                                    cx.notify();
                                }
                            });
                        })
                        .on_mouse_up(MouseButton::Left, move |_, _window, cx| {
                            activate_drag_app_up.update(cx, |this, cx| {
                                if this.state.drag_pending {
                                    this.state.drag_pending = false;
                                    if this.state.dragging_pane == Some(active_id) {
                                        this.state.dragging_pane = None;
                                        this.animate_pane_drag(active_id, false);
                                        cx.notify();
                                    }
                                }
                            });
                        })
                        .on_mouse_down(MouseButton::Right, move |ev, _window, cx| {
                            app_for_resize.update(cx, |this, _cx| {
                                this.start_alt_resize(active_id, ev.position);
                            });
                            cx.stop_propagation();
                        }),
                ))
                .into_any_element()
        } else {
            panel.into_any_element()
        };

        timed_layout_region("pane.shell", PANE_SHELL_TIMINGS, panel).into_any_element()
    }

    fn should_render_pane_body(&self, pane_id: TileId) -> bool {
        !self.pane_closing.contains_key(&pane_id)
    }

    fn render_tab_strip(
        &mut self,
        children: Vec<TileId>,
        active: Option<TileId>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let active_theme = self.state.profiles.active().theme.as_str();
        let figma_glass = active_theme == "Atlas Figma Glass";
        let windows_glass = active_theme == "Atlas Windows Glass";
        let glass_theme = active_theme.ends_with(" Glass") || active_theme == "Atlas Frost";
        let tab_bar_bg = if figma_glass {
            glass_fill()
        } else if windows_glass {
            cx.theme().tab_bar
        } else if glass_theme {
            backdrop_tint(cx.theme().tab_bar, 0.06)
        } else {
            backdrop_tint(cx.theme().tab_bar, 0.16)
        };
        let active_bg = if figma_glass {
            glass_chip_fill()
        } else if windows_glass {
            cx.theme().tab_active
        } else if glass_theme {
            backdrop_tint(cx.theme().tab_active, 0.12)
        } else {
            backdrop_tint(cx.theme().tab_active, 0.22)
        };
        let active_fg = cx.theme().tab_active_foreground;
        let tab_fg = cx.theme().tab_foreground;
        let windows_theme = is_windows_theme(active_theme);
        let hover_bg = if figma_glass {
            glass_chip_fill().opacity(0.36)
        } else {
            border.opacity(0.22)
        };

        let app = cx.entity();
        let tab_min = crate::ui::rem_px(if windows_theme { 118.0 } else { 56.0 });
        let tab_max = crate::ui::rem_px(if windows_theme { 210.0 } else { 220.0 });
        let tab_gap = crate::ui::rem_px(4.0);
        let mut strip = h_flex()
            .h(crate::ui::rem_px(if windows_theme { 46.0 } else { 34.0 }))
            .min_w(px(0.))
            .items_center()
            .px_2()
            .bg(tab_bar_bg)
            .rounded_tl(px(12.0))
            .rounded_tr(px(12.0))
            .border_b_1()
            .border_color(border.opacity(0.42));

        for pane_id in children {
            let (label, tag_color) = self
                .tab_data_for(pane_id)
                .map(|t| {
                    let tag_color = match &t.location {
                        Location::Tag(id) => Some(
                            self.state
                                .tag_cache
                                .def(*id)
                                .map_or([128, 128, 128], |def| def.color),
                        ),
                        _ => None,
                    };
                    let label = match &t.location {
                        Location::Tag(id) => self.state.tag_cache.def(*id).map_or_else(
                            || display_name(&t.location.display_path()),
                            |def| def.name.to_string(),
                        ),
                        _ => display_name(&t.location.display_path()),
                    };
                    (label, tag_color)
                })
                .unwrap_or_else(|| (rust_i18n::t!("fallback_tab_title").to_string(), None));
            let is_active = Some(pane_id) == active;
            let activate_app = app.clone();
            let close_app = app.clone();
            let drag_app = app.clone();
            let tab_motion = (!self.state.settings.disable_animations)
                .then(|| {
                    self.tab_closing
                        .get(&pane_id)
                        .copied()
                        .map(|started| (true, started))
                        .or_else(|| {
                            self.pane_animation_for(pane_id)
                                .filter(|animation| animation.kind == PaneAnimationKind::NewTab)
                                .map(|animation| (false, animation.started))
                        })
                })
                .flatten();
            let tab = h_flex()
                .id(SharedString::from(format!("tab-chip-{}", pane_id.0)))
                .relative()
                .min_w(tab_min)
                .max_w(tab_max)
                .h(crate::ui::rem_px(if windows_theme { 38.0 } else { 26.0 }))
                .items_center()
                .gap(crate::ui::rem_px(if windows_theme { 8.0 } else { 6.0 }))
                .px_3()
                .overflow_hidden()
                .rounded(cx.theme().radius)
                .cursor_pointer()
                .bg(if is_active {
                    active_bg
                } else {
                    cx.theme().transparent
                })
                .text_color(if is_active { active_fg } else { tab_fg })
                .hover(move |s| {
                    if is_active {
                        s.bg(active_bg)
                    } else {
                        s.bg(hover_bg)
                    }
                })
                .on_drag(
                    TabDrag {
                        pane_id,
                        content: self.get_or_make_file_panel(pane_id, cx).clone().into(),
                        size: self.pane_view_bounds.get(&pane_id).map(|b| b.size),
                    },
                    move |drag, _pos, _window, cx| {
                        drag_app.update(cx, |this, cx| {
                            if this.state.dragging_pane != Some(drag.pane_id) {
                                this.animate_pane_drag(drag.pane_id, true);
                                this.state.dragging_pane = Some(drag.pane_id);
                                cx.notify();
                            }
                        });
                        tab_drag_entity(drag.pane_id, drag.content.clone(), drag.size, cx)
                    },
                )
                .on_drop::<TabDrag>({
                    let drop_app = app.clone();
                    move |drag, _window, cx| {
                        drop_app.update(cx, |this, cx| {
                            this.move_tab_before(drag.pane_id, pane_id);
                            cx.notify();
                        });
                    }
                })
                .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                    activate_app.update(cx, |this, cx| {
                        this.process_actions(vec![Action::FocusTab(pane_id)]);
                        cx.notify();
                    });
                })
                .when_some(tag_color, |s, color| {
                    s.child(
                        div()
                            .flex_none()
                            .size(crate::ui::rem_px(9.))
                            .rounded_full()
                            .bg(gpui::rgb(
                                (u32::from(color[0]) << 16)
                                    | (u32::from(color[1]) << 8)
                                    | u32::from(color[2]),
                            )),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .when(!windows_theme, |s| s.text_xs())
                        .when(windows_theme, |s| s.text_sm())
                        .child(SharedString::from(label)),
                )
                .child(
                    div()
                        .size(crate::ui::rem_px(if windows_theme { 22.0 } else { 18.0 }))
                        .rounded(px(5.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(muted)
                        .hover(move |s| s.bg(border.opacity(0.5)))
                        .child(theme_icon(
                            AtlasIcon::X,
                            if windows_theme { 11.0 } else { 10.0 },
                            muted,
                            windows_theme,
                        ))
                        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                            close_app.update(cx, |this, cx| {
                                this.process_actions(vec![Action::CloseTab(pane_id)]);
                                cx.notify();
                            });
                            cx.stop_propagation();
                        }),
                );
            let slot = h_flex()
                .min_w(tab_min)
                .max_w(tab_max)
                .mr(tab_gap)
                .overflow_hidden()
                .child(tab);
            let slot = if let Some((closing, started)) = tab_motion {
                let phase = if closing { "close" } else { "open" };
                slot.with_animation(
                    crate::ui::motion::keyed("tab-chip-motion", pane_id.0, (phase, started)),
                    crate::ui::motion::animation(crate::ui::motion::TAB_MS),
                    move |this, t| {
                        let openness = if closing { 1.0 - t } else { t };
                        this.min_w(tab_min * openness)
                            .max_w(tab_max * openness)
                            .mr(tab_gap * openness)
                            .opacity((openness * 1.2).clamp(0.0, 1.0))
                            .scale(0.88 + 0.12 * openness)
                    },
                )
                .into_any_element()
            } else {
                slot.into_any_element()
            };
            strip = strip.child(layout_region("pane.tabs_chips", slot));
        }

        if let Some(anchor) = active {
            let new_app = app;
            strip = strip.child(layout_region(
                "pane.tabs_actions",
                div()
                    .id(SharedString::from(format!("tab-new-{}", anchor.0)))
                    .size(crate::ui::rem_px(if windows_theme { 32.0 } else { 24.0 }))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover_bg))
                    .child(theme_icon(
                        AtlasIcon::Plus,
                        if windows_theme { 15.0 } else { 13.0 },
                        muted,
                        windows_theme,
                    ))
                    .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                        new_app.update(cx, |this, cx| {
                            this.process_actions(vec![Action::NewTab(anchor)]);
                            cx.notify();
                        });
                    }),
            ));
        }
        timed_layout_region("pane.tabs_bar", PANE_TABS_BAR_TIMINGS, strip).into_any_element()
    }

    fn with_split_drop_zones(
        &self,
        target: TileId,
        content: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = cx.entity();
        let overlay_top_app = app.clone();
        let overlay_left_app = app.clone();
        let overlay_right_app = app.clone();
        let overlay_bottom_app = app;
        div()
            .size_full()
            .relative()
            .group("tab-split-zone")
            .child(content)
            .child(timed_layout_region(
                "pane.drop_zones",
                PANE_DROP_ZONES_TIMINGS,
                split_drop_zone(AtlasIcon::SplitHorizontal)
                    .left_0()
                    .top(relative(0.25))
                    .bottom(relative(0.25))
                    .w(relative(0.35))
                    .on_drop::<TabDrag>(move |drag, _window, cx| {
                        overlay_left_app.update(cx, |this, cx| {
                            this.split_existing_tab(target, drag.pane_id, false, true);
                            cx.notify();
                        });
                    }),
            ))
            .child(timed_layout_region(
                "pane.drop_zones",
                PANE_DROP_ZONES_TIMINGS,
                split_drop_zone(AtlasIcon::SplitHorizontal)
                    .right_0()
                    .top(relative(0.25))
                    .bottom(relative(0.25))
                    .w(relative(0.35))
                    .on_drop::<TabDrag>(move |drag, _window, cx| {
                        overlay_right_app.update(cx, |this, cx| {
                            this.split_existing_tab(target, drag.pane_id, false, false);
                            cx.notify();
                        });
                    }),
            ))
            .child(timed_layout_region(
                "pane.drop_zones",
                PANE_DROP_ZONES_TIMINGS,
                split_drop_zone(AtlasIcon::SplitVertical)
                    .left_0()
                    .right_0()
                    .top_0()
                    .h(relative(0.25))
                    .on_drop::<TabDrag>(move |drag, _window, cx| {
                        overlay_top_app.update(cx, |this, cx| {
                            this.split_existing_tab(target, drag.pane_id, true, true);
                            cx.notify();
                        });
                    }),
            ))
            .child(timed_layout_region(
                "pane.drop_zones",
                PANE_DROP_ZONES_TIMINGS,
                split_drop_zone(AtlasIcon::SplitVertical)
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(relative(0.25))
                    .on_drop::<TabDrag>(move |drag, _window, cx| {
                        overlay_bottom_app.update(cx, |this, cx| {
                            this.split_existing_tab(target, drag.pane_id, true, false);
                            cx.notify();
                        });
                    }),
            ))
            .into_any_element()
    }

    pub(crate) fn get_or_make_file_panel(
        &mut self,
        id: TileId,
        cx: &mut Context<Self>,
    ) -> Entity<FilePanel> {
        if let Some(panel) = self.file_panel_cache.get(&id) {
            return panel.clone();
        }
        let app = cx.entity();
        let signal = self.pane_signal_for(id, cx);
        let panel = cx.new(|cx| FilePanel::new(app, id, signal, cx));
        crate::perf::event(|| format!("new FilePanel pane={}", id.0));
        self.file_panel_cache.insert(id, panel.clone());
        panel
    }

    pub(crate) fn sync_pane_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_overlay_active() || self.pane_closing.contains_key(&self.state.focused_tile) {
            return;
        }
        if let Some(panel) = self.file_panel_cache.get(&self.state.focused_tile) {
            window.focus(&panel.read(cx).focus_handle(cx), cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_child_span_uses_share_ratio() {
        assert_eq!(split_child_span(1.0, 2.0, 100.0, 2), 50.0);
        assert_eq!(split_child_span(3.0, 4.0, 100.0, 2), 75.0);
    }

    #[test]
    fn split_child_span_falls_back_to_even_width() {
        assert_eq!(split_child_span(1.0, 0.0, 90.0, 3), 30.0);
    }

    #[test]
    fn collapses_the_divider_adjacent_to_the_closing_pane() {
        assert_eq!(closing_splitter_index(0, 2), Some(0));
        assert_eq!(closing_splitter_index(1, 2), Some(1));
        assert_eq!(closing_splitter_index(2, 2), Some(1));
        assert_eq!(closing_splitter_index(0, 0), None);
    }

    #[test]
    fn flip_offset_moves_from_previous_to_final_position() {
        assert_eq!(flip_axis_offset(100.0, 40.0, 0.0), 60.0);
        assert_eq!(flip_axis_offset(100.0, 40.0, 0.5), 30.0);
        assert_eq!(flip_axis_offset(100.0, 40.0, 1.0), 0.0);
    }
}

pub(crate) struct FastPaneShell {
    id: gpui::ElementId,
    tab_strip: gpui::AnyElement,
    content: gpui::AnyElement,
    style: gpui::StyleRefinement,
}

pub(crate) struct FastPaneShellPrepaint {
    tab_layout: gpui::LayoutId,
    content_layout: gpui::LayoutId,
}

impl FastPaneShell {
    pub(crate) fn new(
        pane_id: TileId,
        tab_strip: gpui::AnyElement,
        content: gpui::AnyElement,
    ) -> Self {
        Self {
            id: gpui::ElementId::Name(gpui::SharedString::from(format!("pane-shell-{}", pane_id.0))),
            tab_strip,
            content,
            style: gpui::StyleRefinement::default(),
        }
    }
}

impl gpui::IntoElement for FastPaneShell {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Styled for FastPaneShell {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl gpui::Element for FastPaneShell {
    type RequestLayoutState = FastPaneShellPrepaint;
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        crate::perf::count("pane.shell.layout");
        let tab_layout = self.tab_strip.request_layout(window, cx);
        let content_layout = self.content.request_layout(window, cx);

        let mut style = gpui::Style::default();
        style.refine(&self.style);

        let layout_id = window.request_layout(style, vec![tab_layout, content_layout], cx);

        (
            layout_id,
            FastPaneShellPrepaint {
                tab_layout,
                content_layout,
            },
        )
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        crate::perf::count("pane.shell.prepaint");
        
        let tab_bounds = window.layout_bounds(request_layout.tab_layout);
        let content_bounds = window.layout_bounds(request_layout.content_layout);
        
        let tab_offset = bounds.origin - tab_bounds.origin;
        let content_offset = bounds.origin + gpui::Point { x: gpui::px(0.0), y: tab_bounds.size.height } - content_bounds.origin;

        window.with_element_offset(tab_offset, |window| {
            self.tab_strip.prepaint(window, cx);
        });
        window.with_element_offset(content_offset, |window| {
            self.content.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        crate::perf::count("pane.shell.paint");
        
        let tab_bounds = window.layout_bounds(request_layout.tab_layout);
        let content_bounds = window.layout_bounds(request_layout.content_layout);
        
        let tab_offset = bounds.origin - tab_bounds.origin;
        let content_offset = bounds.origin + gpui::Point { x: gpui::px(0.0), y: tab_bounds.size.height } - content_bounds.origin;

        window.with_element_offset(tab_offset, |window| {
            self.tab_strip.paint(window, cx);
        });
        window.with_element_offset(content_offset, |window| {
            self.content.paint(window, cx);
        });
    }
}
