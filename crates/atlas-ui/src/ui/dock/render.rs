use gpui::prelude::*;
use gpui::{
    div, px, relative, AnimationExt, AnyElement, Context, Entity, Focusable, MouseButton,
    SharedString, Window,
};
use gpui_component::{h_flex, v_flex, ActiveTheme as _, ElementExt as _};

use crate::app::{App, PaneAnimation, PaneAnimationKind};
use crate::input::action::Action;
use crate::state::layout_tree::{Container, LinearDir, Tile, TileId};
use crate::ui::dock::drag::{tab_drag_entity, TabDrag};
use crate::ui::glass::{backdrop_tint, glass_chip_fill, glass_fill, GlassExt};
use crate::ui::icons::{icon, is_windows_theme, theme_icon, AtlasIcon};
use crate::ui::pane_panel::FilePanel;
use crate::util::display::display_name;
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
enum SplitMotion {
    Opening(PaneAnimation),
    Closing(Instant),
}

#[derive(Clone, Copy, Debug)]
enum TabMotion {
    Opening(Instant),
    Closing(Instant),
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
                        .size(px(58.))
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

        self.state
            .pane_tree
            .root
            .and_then(|root| self.render_tree(root, window, cx))
            .unwrap_or_else(|| div().size_full().into_any_element())
    }

    fn render_tree(
        &mut self,
        id: TileId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        crate::perf::count("dock.render_tree");
        match self.state.pane_tree.tiles.get(id)?.clone() {
            Tile::Pane(_) => Some(self.render_tabs_like(vec![id], Some(id), window, cx)),
            Tile::Container(Container::Tabs(tabs)) => {
                let children = tabs.children;
                if children.is_empty() {
                    return None;
                }
                let active = tabs
                    .active
                    .filter(|a| children.contains(a))
                    .or_else(|| children.first().copied());
                Some(self.render_tabs_like(children, active, window, cx))
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

                let mut row = if exact_span.is_some() { div().relative() } else { if linear.dir == LinearDir::Horizontal { h_flex() } else { v_flex() } }
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
                crate::app::normalize_split_shares(&mut final_shares);
                let sum: f32 = final_shares.iter().copied().filter(|v| *v > 0.0).sum();
                let shares = if final_shares.len() == children.len() && sum > f32::EPSILON {
                    self.dock_split_shares(id, final_shares, window, cx)
                } else {
                    vec![1.0; children.len()]
                };
                
                let view = cx.entity();
                let num_splitters = children.len().saturating_sub(1);
                let splitter_size = 5.0; // from resize.rs splitter thickness
                let mut current_offset = 0.0;

                for (ix, (child_id, child, intro)) in children.into_iter().enumerate() {
                    let share = shares[ix].max(0.01);
                    
                    if ix > 0 {
                        let splitter_el = crate::ui::dock::resize::splitter(id, ix - 1, linear.dir, &view, cx);
                        if exact_span.is_some() {
                            let mut s_div = div().absolute();
                            if linear.dir == LinearDir::Horizontal {
                                s_div = s_div.left(px(current_offset)).top_0().bottom_0();
                            } else {
                                s_div = s_div.top(px(current_offset)).left_0().right_0();
                            }
                            row = row.child(s_div.child(splitter_el));
                            current_offset += splitter_size;
                        } else {
                            row = row.child(splitter_el);
                        }
                    }

                    let child_span = if let Some(span) = exact_span {
                        let available = (span - (num_splitters as f32 * splitter_size)).max(0.0);
                        Some(share * available)
                    } else {
                        None
                    };

                    let mut pane_div = div().size_full().min_w(px(0.)).min_h(px(0.));
                    
                    if let Some(span) = child_span {
                        pane_div = pane_div.absolute();
                        if linear.dir == LinearDir::Horizontal {
                            pane_div = pane_div.left(px(current_offset)).top_0().bottom_0().w(px(span));
                        } else {
                            pane_div = pane_div.top(px(current_offset)).left_0().right_0().h(px(span));
                        }
                        current_offset += span;
                    } else {
                        pane_div = pane_div.flex_grow(share).flex_basis(px(0.));
                    }
                    
                    row = row.child(pane_div.child(self.render_split_intro(child_id, child, intro)));
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
                        .map(SplitMotion::Opening)
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
    ) -> AnyElement {
        let inner = div()
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
            .child(child);
        let Some(motion) = motion else {
            return inner.into_any_element();
        };
        let settle_secs = crate::ui::motion::PANE_SPRING.settle().as_secs_f32();
        let key = match motion {
            SplitMotion::Opening(animation) => ("open", animation.started),
            SplitMotion::Closing(started) => ("close", started),
        };
        inner
            .with_animation(
                crate::ui::motion::keyed("dock-split-pop", id.0, key),
                crate::ui::motion::linear_anim(crate::ui::motion::PANE_SPRING.settle()),
                move |this, t| {
                    let s = crate::ui::motion::PANE_SPRING.value(t * settle_secs);
                    let visible = match motion {
                        SplitMotion::Opening(_) => s,
                        SplitMotion::Closing(_) => 1.0 - s,
                    }
                    .clamp(0.0, 1.0);
                    this.opacity((0.25 + 0.9 * visible).clamp(0.0, 1.0))
                        .scale((0.48 + 0.52 * visible).clamp(0.48, 1.10))
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
        // The resize animation interpolates EVERY sibling's width each frame, which
        // repaints all of them ~15× over the animation. With many panes in one
        // container that is the stutter when rapidly splitting or closing under a held
        // shortcut (perf.log: 40-54ms frames at 10+ panes). Past a small count, snap to
        // the final layout (open still fades via PaneAnimation, close via the pop anim).
        // heuristic cap; raise if paint per pane gets cheap enough.
        if anim.to.len() > 4 {
            self.dock_split_animations.remove(&id);
            return final_shares;
        }
        window.request_animation_frame();
        let shares = anim.shares();
        let elapsed_ms = anim.started.elapsed().as_millis();
        if anim.closing && (elapsed_ms < 20 || elapsed_ms % 80 < 18) {
            crate::perf::event(|| {
                format!(
                    "pane_close.split_frame parent={} elapsed_ms={} shares={:?}",
                    id.0, elapsed_ms, shares
                )
            });
        }
        shares
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
        let content = active
            .map(|id| {
                if self.should_render_pane_body(id) {
                    // Patrón Zed (pane_group.rs:554): pane cacheado como AnyView. Si el
                    // pane no fue notificado y sus bounds no cambiaron, gpui recicla su
                    // layout, hitboxes y primitivas del frame anterior — el pane cuesta
                    // 1 nodo taffy en vez de ~250.
                    gpui::AnyView::from(self.get_or_make_file_panel(id, cx))
                        .cached(gpui::StyleRefinement::default().size_full())
                        .into_any_element()
                } else {
                    crate::perf::count("render.pane_body_skipped");
                    div().size_full().into_any_element()
                }
            })
            .unwrap_or_else(|| div().size_full().into_any_element());
        let content = if let Some(active) = active {
            self.with_split_drop_zones(active, content, cx)
        } else {
            content
        };
        let active_theme = self.state.profiles.active().theme.as_str();
        let figma_glass = active_theme == "Atlas Figma Glass";
        let windows_glass = active_theme == "Atlas Windows Glass"
            || (active_theme == "Atlas Windows" && self.state.settings.mica);
        let panel_bg = if figma_glass {
            glass_fill()
        } else if windows_glass {
            cx.theme().secondary
        } else if is_windows_theme(active_theme) {
            gpui::hsla(0., 0., 1., 0.20)
        } else if self.state.settings.mica {
            backdrop_tint(cx.theme().secondary, 0.06)
        } else {
            backdrop_tint(cx.theme().secondary, 0.24)
        };
        let active_id = active.unwrap_or_else(|| *children.first().unwrap());
        let panel = v_flex()
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
            .when(figma_glass, |s| s.glass_surface(panel_bg))
            .when(!figma_glass, |s| {
                s.rounded(cx.theme().radius_lg)
                    .bg(panel_bg)
                    .border_1()
                    .border_color(cx.theme().border.opacity(0.55))
            })
            .child(self.render_tab_strip(children, active, window, cx))
            .child(div().flex_1().min_h(px(0.)).min_w(px(0.)).child(content));

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
                .child(
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
                )
                .into_any_element()
        } else {
            panel.into_any_element()
        };

        panel
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
        let accent = cx.theme().accent;
        let active_theme = self.state.profiles.active().theme.as_str();
        let figma_glass = active_theme == "Atlas Figma Glass";
        let windows_glass = active_theme == "Atlas Windows Glass"
            || (active_theme == "Atlas Windows" && self.state.settings.mica);
        let tab_bar_bg = if figma_glass {
            glass_fill()
        } else if windows_glass {
            cx.theme().tab_bar
        } else if self.state.settings.mica {
            backdrop_tint(cx.theme().tab_bar, 0.06)
        } else {
            backdrop_tint(cx.theme().tab_bar, 0.16)
        };
        let active_bg = if figma_glass {
            glass_chip_fill()
        } else if windows_glass {
            cx.theme().tab_active
        } else if self.state.settings.mica {
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
        let strip_h = if windows_theme { 46.0 } else { 34.0 };
        let tab_h = if windows_theme { 38.0 } else { 26.0 };
        let tab_min_w = if windows_theme { 118.0 } else { 56.0 };
        let tab_max_w = if windows_theme { 210.0 } else { 220.0 };
        let tab_gap = if windows_theme { 8.0 } else { 6.0 };
        let close_size = if windows_theme { 22.0 } else { 18.0 };
        let close_icon = if windows_theme { 11.0 } else { 10.0 };
        let app = cx.entity();
        let mut strip = h_flex()
            .h(px(strip_h))
            .min_w(px(0.))
            .items_center()
            .gap(px(4.))
            .px_2()
            .bg(tab_bar_bg)
            .rounded_tl(px(12.0))
            .rounded_tr(px(12.0))
            .border_b_1()
            .border_color(border.opacity(0.42));

        for pane_id in children {
            let (label, path) = self
                .tab_data_for(pane_id)
                .map(|t| {
                    (
                        display_name(&t.location.display_path()),
                        t.location.fs_dir().map(|p| p.to_path_buf()),
                    )
                })
                .unwrap_or_else(|| (rust_i18n::t!("fallback_tab_title").to_string(), None));

            let tex = if windows_theme {
                path.as_ref().and_then(|p| self.grid_thumbnail(p))
            } else {
                None
            };
            let is_active = Some(pane_id) == active;
            let animation = (!self.state.settings.disable_animations)
                .then(|| self.pane_animation_for(pane_id))
                .flatten()
                .filter(|animation| matches!(animation.kind, PaneAnimationKind::NewTab));
            let motion = if let Some(started) = self.tab_closing.get(&pane_id).copied() {
                Some(TabMotion::Closing(started))
            } else {
                animation.map(|animation| TabMotion::Opening(animation.started))
            };
            let activate_app = app.clone();
            let close_app = app.clone();
            let tab_glow = motion.map(|motion| {
                let (key, color) = match motion {
                    TabMotion::Opening(started) => (("open", started), accent),
                    TabMotion::Closing(started) => (("close", started), cx.theme().danger),
                };
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(2.))
                    .rounded(px(2.))
                    .bg(color)
                    .with_animation(
                        crate::ui::motion::keyed("custom-tab-glow", pane_id.0, key),
                        crate::ui::motion::animation(crate::ui::motion::PANEL_MS),
                        move |this, t| this.opacity((1.0 - t).clamp(0.0, 1.0)),
                    )
            });
            let tab = h_flex()
                .id(SharedString::from(format!("custom-tab-{}", pane_id.0)))
                .relative()
                .max_w(px(tab_max_w))
                .min_w(px(tab_min_w))
                .h(px(tab_h))
                .items_center()
                .gap(px(tab_gap))
                .px_3()
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
                    {
                        let drag_app = app.clone();
                        move |drag, _pos, _window, cx| {
                            drag_app.update(cx, |this, cx| {
                                if this.state.dragging_pane != Some(drag.pane_id) {
                                    this.animate_pane_drag(drag.pane_id, true);
                                    this.state.dragging_pane = Some(drag.pane_id);
                                    cx.notify();
                                }
                            });
                            tab_drag_entity(drag.pane_id, drag.content.clone(), drag.size, cx)
                        }
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
                .when_some(path.clone(), |s, drop_dest| {
                    s.on_prepaint(crate::ui::drag_hover::drag_hover_recorder_with_drop(
                        crate::ui::drag_hover::DragHoverId::Chrome("Tab", pane_id),
                        crate::ui::drag_hover::DragHoverKind::Nav,
                        crate::ui::drag_hover::DragHoverCommand::Process(
                            crate::input::action::Action::FocusTab(pane_id),
                        ),
                        crate::ui::drag_hover::DragHoverDropCommand::TransferTo(drop_dest),
                        &app,
                    ))
                })
                .when(path.is_none(), |s| {
                    s.on_prepaint(crate::ui::drag_hover::drag_hover_recorder(
                        crate::ui::drag_hover::DragHoverId::Chrome("Tab", pane_id),
                        crate::ui::drag_hover::DragHoverKind::Nav,
                        crate::ui::drag_hover::DragHoverCommand::Process(
                            crate::input::action::Action::FocusTab(pane_id),
                        ),
                        &app,
                    ))
                })
                .when_some(tex, |s, tex| s.child(gpui::img(tex).size(px(16.0))))
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
                        .size(px(close_size))
                        .rounded(px(5.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(muted)
                        .hover(|s| s.bg(border.opacity(0.5)))
                        .child(theme_icon(AtlasIcon::X, close_icon, muted, windows_theme))
                        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                            close_app.update(cx, |this, cx| {
                                this.process_actions(vec![Action::CloseTab(pane_id)]);
                                cx.notify();
                            });
                            cx.stop_propagation();
                        }),
                )
                .children(tab_glow);
            let tab = if let Some(motion) = motion {
                let (key, closing) = match motion {
                    TabMotion::Opening(started) => (("open", started), false),
                    TabMotion::Closing(started) => (("close", started), true),
                };
                tab.with_animation(
                    crate::ui::motion::keyed("custom-tab-intro", pane_id.0, key),
                    crate::ui::motion::animation(crate::ui::motion::PANEL_MS),
                    move |this, t| {
                        let visible = if closing { 1.0 - t } else { t };
                        this.opacity(0.20 + 0.80 * visible)
                            .paint_offset(gpui::point(px(0.), px(3.0 * (1.0 - visible))))
                    },
                )
                .into_any_element()
            } else {
                tab.into_any_element()
            };
            strip = strip.child(tab);
        }

        if let Some(anchor) = active {
            let new_app = app;
            strip = strip.child(
                div()
                    .id(SharedString::from(format!("custom-tab-new-{}", anchor.0)))
                    .size(px(if windows_theme { 32.0 } else { 24.0 }))
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
            );
        }
        strip.into_any_element()
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
            .child(
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
            )
            .child(
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
            )
            .child(
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
            )
            .child(
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
            )
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
