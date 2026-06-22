use gpui::prelude::*;
use gpui::{
    div, px, AnimationExt, AnyElement, Context, Entity, Focusable, MouseButton, SharedString,
    Window,
};
use gpui_component::{h_flex, v_flex, ActiveTheme as _, ElementExt as _};

use crate::app::{App, PaneAnimation, PaneAnimationKind};
use crate::input::action::Action;
use crate::state::layout_tree::{Container, LinearDir, Tile, TileId};
use crate::ui::dock::drag::{edge_zone, tab_drag_entity, TabDrag};
use crate::ui::glass::backdrop_tint;
use crate::ui::icons::{icon, AtlasIcon};
use crate::ui::pane_panel::FilePanel;
use crate::util::display::display_name;
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
enum SplitMotion {
    Opening(PaneAnimation),
    Closing(Instant),
}

impl App {
    pub(crate) fn render_custom_dock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        crate::perf::count("render.dock");
        let _perf = crate::perf::span_detail("dock.render", || {
            format!(
                "tiles={} cached_panels={} focused={}",
                self.state.pane_tree.tiles.iter().count(),
                self.file_panel_cache.len(),
                self.state.focused_tile.0
            )
        });
        {
            let _perf = crate::perf::span("dock.sync_focus");
            self.sync_pane_focus(window, cx);
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
                Some(self.render_tabs_like(tabs.children, tabs.active, window, cx))
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
                let mut row = if linear.dir == LinearDir::Horizontal {
                    h_flex()
                } else {
                    v_flex()
                }
                .size_full()
                .min_w(px(0.))
                .min_h(px(0.))
                .items_stretch();
                let final_shares = linear.shares;
                let sum: f32 = final_shares.iter().copied().filter(|v| *v > 0.0).sum();
                let shares = if final_shares.len() == children.len() && sum > f32::EPSILON {
                    self.dock_split_shares(id, final_shares, window, cx)
                } else {
                    vec![1.0; children.len()]
                };
                let view = cx.entity();
                for (ix, (child_id, child, intro)) in children.into_iter().enumerate() {
                    if ix > 0 {
                        row = row.child(crate::ui::dock::resize::splitter(
                            id,
                            ix - 1,
                            linear.dir,
                            &view,
                            cx,
                        ));
                    }
                    row = row.child(
                        div()
                            .size_full()
                            .flex_grow(shares[ix].max(0.01))
                            .flex_basis(px(0.))
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .child(self.render_split_intro(child_id, child, intro)),
                    );
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
                    let hidden = (1.0 - visible).clamp(0.0, 1.0);
                    this.opacity((0.25 + 0.9 * visible).clamp(0.0, 1.0))
                        .p(px(42.0 * hidden))
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
        cx: &mut Context<Self>,
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
        window.request_animation_frame();
        cx.notify();
        let shares = anim.shares();
        let elapsed_ms = anim.started.elapsed().as_millis();
        if anim.closing && (elapsed_ms < 20 || elapsed_ms % 80 < 18) {
            eprintln!(
                "[atlas close] split_frame parent={} elapsed_ms={} shares={:?}",
                id.0, elapsed_ms, shares
            );
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
            .map(|id| self.get_or_make_file_panel(id, cx).into_any_element())
            .unwrap_or_else(|| div().size_full().into_any_element());
        let content = if let Some(active) = active {
            self.with_split_drop_zones(active, content, cx)
        } else {
            content
        };
        let panel_bg = backdrop_tint(cx.theme().secondary, 0.24);
        v_flex()
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
            .rounded(cx.theme().radius_lg)
            .bg(panel_bg)
            .border_1()
            .border_color(cx.theme().border.opacity(0.55))
            .child(self.render_tab_strip(children, active, window, cx))
            .child(div().flex_1().min_h(px(0.)).min_w(px(0.)).child(content))
            .into_any_element()
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
        let tab_bar_bg = backdrop_tint(cx.theme().tab_bar, 0.16);
        let active_bg = backdrop_tint(cx.theme().tab_active, 0.22);
        let active_fg = cx.theme().tab_active_foreground;
        let tab_fg = cx.theme().tab_foreground;
        let app = cx.entity();
        let mut strip = h_flex()
            .h(px(34.))
            .min_w(px(0.))
            .items_center()
            .gap(px(4.))
            .px_2()
            .bg(tab_bar_bg)
            .border_b_1()
            .border_color(border.opacity(0.7));

        for pane_id in children {
            let label = self
                .tab_data_for(pane_id)
                .map(|t| display_name(&t.location.display_path()))
                .unwrap_or_else(|| rust_i18n::t!("fallback_tab_title").to_string());
            let is_active = Some(pane_id) == active;
            let animation = (!self.state.settings.disable_animations)
                .then(|| self.pane_animation_for(pane_id))
                .flatten()
                .filter(|animation| matches!(animation.kind, PaneAnimationKind::NewTab));
            let activate_app = app.clone();
            let close_app = app.clone();
            let tab_glow = animation.map(|animation| {
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(2.))
                    .rounded(px(2.))
                    .bg(accent)
                    .with_animation(
                        crate::ui::motion::keyed("custom-tab-glow", pane_id.0, animation.started),
                        crate::ui::motion::animation(crate::ui::motion::PANEL_MS),
                        move |this, t| this.opacity((1.0 - t).clamp(0.0, 1.0)),
                    )
            });
            let tab = h_flex()
                .id(SharedString::from(format!("custom-tab-{}", pane_id.0)))
                .relative()
                .max_w(px(220.))
                .min_w(px(56.))
                .h(px(26.))
                .items_center()
                .gap(px(6.))
                .px_2()
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
                        s.bg(border.opacity(0.22))
                    }
                })
                .on_drag(TabDrag { pane_id }, move |_d, _pos, _window, cx| {
                    tab_drag_entity(pane_id, cx)
                })
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
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .child(SharedString::from(label)),
                )
                .child(
                    div()
                        .size(px(18.))
                        .rounded(px(5.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(muted)
                        .hover(|s| s.bg(border.opacity(0.5)))
                        .child(icon(AtlasIcon::X, 10.0, muted))
                        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                            close_app.update(cx, |this, cx| {
                                this.process_actions(vec![Action::CloseTab(pane_id)]);
                                cx.notify();
                            });
                            cx.stop_propagation();
                        }),
                )
                .children(tab_glow);
            let tab = if let Some(animation) = animation {
                tab.with_animation(
                    crate::ui::motion::keyed("custom-tab-intro", pane_id.0, animation.started),
                    crate::ui::motion::animation(crate::ui::motion::PANEL_MS),
                    move |this, t| {
                        this.opacity(0.20 + 0.80 * t)
                            .paint_offset(gpui::point(px(0.), px(3.0 * (1.0 - t))))
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
                    .size(px(24.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(border.opacity(0.35)))
                    .child(icon(AtlasIcon::Plus, 13.0, muted))
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
        let left_app = app.clone();
        let right_app = app.clone();
        let top_app = app.clone();
        let bottom_app = app;
        div()
            .size_full()
            .relative()
            .child(content)
            .child(
                edge_zone()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(18.))
                    .on_drop::<TabDrag>(move |drag, _window, cx| {
                        left_app.update(cx, |this, cx| {
                            this.split_existing_tab(target, drag.pane_id, false, true);
                            cx.notify();
                        });
                    }),
            )
            .child(
                edge_zone()
                    .right_0()
                    .top_0()
                    .bottom_0()
                    .w(px(18.))
                    .on_drop::<TabDrag>(move |drag, _window, cx| {
                        right_app.update(cx, |this, cx| {
                            this.split_existing_tab(target, drag.pane_id, false, false);
                            cx.notify();
                        });
                    }),
            )
            .child(
                edge_zone()
                    .left_0()
                    .right_0()
                    .top_0()
                    .h(px(18.))
                    .on_drop::<TabDrag>(move |drag, _window, cx| {
                        top_app.update(cx, |this, cx| {
                            this.split_existing_tab(target, drag.pane_id, true, true);
                            cx.notify();
                        });
                    }),
            )
            .child(
                edge_zone()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(18.))
                    .on_drop::<TabDrag>(move |drag, _window, cx| {
                        bottom_app.update(cx, |this, cx| {
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

    fn sync_pane_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_overlay_active() || self.pane_closing.contains_key(&self.state.focused_tile) {
            return;
        }
        if let Some(panel) = self.file_panel_cache.get(&self.state.focused_tile) {
            window.focus(&panel.read(cx).focus_handle(cx), cx);
        }
    }
}
