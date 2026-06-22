//! `FilePanel`: a gpui-component `Panel` wrapping one file-view pane.
//!
//! FASE A: the central tabs/splits are now a `gpui_component::dock::DockArea`. The
//! Dock owns the interactive layout (tab drag/reorder, drag-to-split, close), so
//! each pane must be its own `Entity` implementing the `Panel` trait. `FilePanel`
//! holds a weak handle back to `App` and renders the pane by delegating to the
//! existing `App::render_pane` (which dispatches Details/Grid/List/Miller).
//!
//! The Dock caches the active panel's content (`.cached()`), so the surrounding
//! DockArea drives invalidation. Avoid observing every `App` repaint here: doing so
//! invalidates every file panel on each app notify and makes window movement stutter.

use crate::app::{App as AtlasApp, PaneAnimation, PaneSignal};
use crate::input::action::Action;
use crate::state::pane::TabId;
use crate::ui::icons::{icon, AtlasIcon};
use crate::ui::motion;
use crate::ui::tooltips::shortcut_tooltip;
use crate::util::display::display_name;
use gpui::prelude::*;
use gpui::{
    div, point, px, AnimationExt, AnyView, App, Context, Entity, EventEmitter, FocusHandle,
    Focusable, IntoElement, MouseButton, Render, SharedString, StyleRefinement, WeakEntity, Window,
};
use gpui_component::dock::{Panel, PanelEvent, PanelInfo, PanelState};
use gpui_component::{h_flex, ActiveTheme as _, ElementExt as _, StyledExt as _};

fn animation_key(prefix: &str, pane_id: TabId, animation: PaneAnimation) -> SharedString {
    motion::keyed(prefix, pane_id.0, (animation.kind, animation.started))
}

pub struct FilePanel {
    /// Weak handle to the orchestrator that owns `AppState`.
    app: WeakEntity<AtlasApp>,
    /// Stable identity of the pane this panel renders (also its layout-tree tile id).
    pub(crate) pane_id: TabId,
    focus_handle: FocusHandle,
    content: Entity<PaneContentView>,
}

pub struct PaneContentView {
    app: WeakEntity<AtlasApp>,
    pane_id: TabId,
}

impl Render for PaneContentView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf::count("render.pane_content");
        let _perf =
            crate::perf::span_detail("pane.content.render", || format!("pane={}", self.pane_id.0));
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |app, cx| app.render_pane(self.pane_id, window, cx))
        } else {
            div().into_any_element()
        }
    }
}

impl FilePanel {
    /// `signal` must be pre-computed by the caller using the App context BEFORE
    /// `cx.new(|cx| FilePanel::new(...))` is called, because at that point the App
    /// entity is already leased and `app.update(cx, ...)` would double-borrow it.
    pub fn new(
        app: Entity<AtlasApp>,
        pane_id: TabId,
        signal: Entity<PaneSignal>,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = cx.new(|cx| {
            cx.observe(&signal, |_, _, cx| cx.notify()).detach();
            PaneContentView {
                app: app.downgrade(),
                pane_id,
            }
        });
        Self {
            app: app.downgrade(),
            pane_id,
            focus_handle: cx.focus_handle(),
            content,
        }
    }
}

impl EventEmitter<PanelEvent> for FilePanel {}

impl Focusable for FilePanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for FilePanel {
    fn panel_name(&self) -> &'static str {
        "FilePanel"
    }

    /// Hide the trailing "..." toolbar menu (and its grey suffix box) on file panes.
    fn show_toolbar(&self, _cx: &gpui::App) -> bool {
        false
    }

    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (label, animation) = self
            .app
            .upgrade()
            .map(|a| {
                let app = a.read(cx);
                let anim = if app.state.settings.disable_animations {
                    None
                } else {
                    app.pane_animation_for(self.pane_id)
                };
                (
                    app.tab_data_for(self.pane_id)
                        .map(|t| display_name(&t.location.display_path()))
                        .unwrap_or_else(|| rust_i18n::t!("fallback_tab_title").to_string()),
                    anim,
                )
            })
            .unwrap_or_else(|| (rust_i18n::t!("fallback_tab_title").to_string(), None));
        let pane_id = self.pane_id;
        let app = self.app.clone();
        let muted = cx.theme().muted_foreground;
        let hover_bg = cx.theme().border.opacity(0.45);
        let danger = cx.theme().danger;
        let accent = cx.theme().accent;

        let title = h_flex()
            .id(SharedString::from(format!("file-tab-label-{}", pane_id.0)))
            .relative()
            .min_w(px(0.))
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(SharedString::from(label)),
            )
            .child(
                div()
                    .id(SharedString::from(format!("file-tab-close-{}", pane_id.0)))
                    .flex_none()
                    .size(px(20.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(muted)
                    .hover(|this| this.bg(hover_bg).text_color(danger))
                    .child(icon(AtlasIcon::X, 11.0, muted))
                    .tooltip(|window, cx| {
                        shortcut_tooltip(
                            rust_i18n::t!("tooltip_tab_close_title").to_string(),
                            rust_i18n::t!("tooltip_tab_close_desc").to_string(),
                            &["ctrl-w"],
                            window,
                            cx,
                        )
                    })
                    .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                        if let Some(app) = app.upgrade() {
                            app.update(cx, |app, cx| {
                                app.process_actions(vec![Action::CloseTab(pane_id)]);
                                cx.notify();
                            });
                        }
                        cx.stop_propagation();
                    }),
            );

        if let Some(animation) = animation {
            let settle_secs = motion::PANE_SPRING.settle().as_secs_f32();
            title
                .with_animation(
                    animation_key("file-tab-title-intro", pane_id, animation),
                    motion::linear_anim(motion::PANE_SPRING.settle()),
                    move |this, t| {
                        let s = motion::PANE_SPRING.value(t * settle_secs);
                        let inset = 8.0 * (1.0 - s);
                        this.opacity(s.clamp(0.0, 1.0))
                            .ml(px(inset))
                            .bg(accent.opacity(0.12 * (1.0 - s).clamp(0.0, 1.0)))
                    },
                )
                .into_any_element()
        } else {
            title.into_any_element()
        }
    }

    fn title_suffix(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let pane_id = self.pane_id;
        let app = self.app.clone();
        let muted = cx.theme().muted_foreground;
        let hover_bg = cx.theme().border.opacity(0.45);
        let accent = cx.theme().accent;

        Some(
            div()
                .id(SharedString::from(format!("file-tab-new-{}", pane_id.0)))
                .flex_none()
                .size(px(24.))
                .rounded(px(7.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(muted)
                .hover(|this| this.bg(hover_bg).text_color(accent))
                .child(icon(AtlasIcon::Plus, 14.0, muted))
                .tooltip(|window, cx| {
                    shortcut_tooltip(
                        rust_i18n::t!("tooltip_tab_new_title").to_string(),
                        rust_i18n::t!("tooltip_tab_new_desc").to_string(),
                        &["ctrl-t"],
                        window,
                        cx,
                    )
                })
                .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                    if let Some(app) = app.upgrade() {
                        app.update(cx, |app, cx| {
                            app.process_actions(vec![Action::NewTab(pane_id)]);
                            cx.notify();
                        });
                    }
                    cx.stop_propagation();
                }),
        )
    }

    /// Serialize the pane identity into the dumped layout so the reconcile path can
    /// map a dumped `FilePanel` node back to its `TileId`.
    fn dump(&self, _cx: &App) -> PanelState {
        let mut state = PanelState::new(self);
        state.info = PanelInfo::Panel(serde_json::json!({ "pane_id": self.pane_id.0 }));
        state
    }
}

impl Render for FilePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pane_id = self.pane_id;
        crate::perf::count("render.file_panel");
        let _perf = crate::perf::span_detail("file_panel.render", || format!("pane={}", pane_id.0));
        match self.app.upgrade() {
            Some(app) => {
                let (animation, closing, animations_enabled) = {
                    let app = app.read(cx);
                    (
                        app.pane_animation_for(pane_id),
                        app.pane_closing_for(pane_id),
                        !app.state.settings.disable_animations,
                    )
                };
                // Suppress animation when the setting is off — the pane renders at its
                // final state immediately, saving all per-frame repaint cost.
                let animation = animation.filter(|_| animations_enabled);
                let closing = closing.filter(|_| animations_enabled);
                let content = AnyView::from(self.content.clone())
                    .cached(StyleRefinement::default().v_flex().size_full());
                let bounds_app = self.app.clone();
                let panel = div().size_full().relative().child(content).on_prepaint(
                    move |bounds, _window, cx| {
                        if let Some(app) = bounds_app.upgrade() {
                            app.update(cx, |app, _cx| {
                                if app.state.tab_states.contains_key(&pane_id) {
                                    app.pane_layout_bounds.insert(pane_id, bounds);
                                }
                            });
                        }
                    },
                );

                if let Some((dx, dy)) = closing {
                    // Close-out: accelerate away into the seam (ease-in) while fading —
                    // the physical counterpart of the spring intro. Self-driving tween
                    // over PANE_CLOSE_FINALIZE; the finalize tick removes the pane when
                    // it ends, then the neighbor expands (FLIP). Stable key = plays once.
                    let dist = 14.0;
                    panel
                        .with_animation(
                            SharedString::from(format!("file-pane-close-{}", pane_id.0)),
                            motion::animation(crate::app::PANE_CLOSE_FINALIZE.as_millis() as u64),
                            move |this, t| {
                                let a = t * t; // ease-in: starts slow, falls away fast
                                this.opacity(1.0 - a * 0.85)
                                    .paint_offset(point(px(dx * dist * a), px(dy * dist * a)))
                            },
                        )
                        .into_any_element()
                } else if let Some(animation) = animation {
                    // Spring intro: pane glides in from the entrance direction with
                    // momentum and a soft overshoot, settling exactly as the (linear)
                    // timeline ends. The spring curve lives in the closure, not the
                    // easing, so it can overshoot past 1.0 without tripping gpui's assert.
                    let settle_secs = motion::PANE_SPRING.settle().as_secs_f32();
                    panel
                        .with_animation(
                            animation_key("file-pane-motion", pane_id, animation),
                            motion::linear_anim(motion::PANE_SPRING.settle()),
                            move |this, t| {
                                let s = motion::PANE_SPRING.value(t * settle_secs);
                                let inv = 1.0 - s; // overshoots slightly negative
                                let mut y = 0.0;
                                let mut scale = 1.0;
                                if matches!(animation.kind, crate::app::PaneAnimationKind::NewTab) {
                                    y = 10.0 * inv;
                                } else {
                                    scale = (0.92 + 0.08 * s).clamp(0.92, 1.04);
                                }
                                // Opacity ramps ahead of position so content is readable
                                // while the pane is still settling.
                                this.opacity((0.35 + 0.9 * s).clamp(0.0, 1.0))
                                    .paint_offset(point(px(0.), px(y)))
                                    .scale(scale)
                            },
                        )
                        .into_any_element()
                } else {
                    panel.into_any_element()
                }
            }
            None => div().into_any_element(),
        }
    }
}
