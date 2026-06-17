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

use crate::app::{App as AtlasApp, PaneAnimation};
use crate::input::action::Action;
use crate::state::pane::TabId;
use crate::ui::icons::{icon, AtlasIcon};
use crate::ui::motion;
use crate::ui::tooltips::shortcut_tooltip;
use crate::util::display::display_name;
use gpui::prelude::*;
use gpui::{
    div, point, px, AnimationExt, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, MouseButton, Render, SharedString, WeakEntity, Window,
};
use gpui_component::dock::{Panel, PanelEvent, PanelInfo, PanelState};
use gpui_component::{h_flex, ActiveTheme as _, ElementExt};

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
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |app, cx| app.render_pane(self.pane_id, window, cx))
        } else {
            div().into_any_element()
        }
    }
}

impl FilePanel {
    pub fn new(app: Entity<AtlasApp>, pane_id: TabId, cx: &mut Context<Self>) -> Self {
        let content = cx.new(|cx| {
            cx.observe(&app, |_, _, cx| cx.notify()).detach();
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
                (
                    app.tab_data_for(self.pane_id)
                        .map(|t| display_name(&t.location.display_path()))
                        .unwrap_or_else(|| rust_i18n::t!("fallback_tab_title").to_string()),
                    app.pane_animation_for(self.pane_id),
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
            title
                .with_animation(
                    animation_key("file-tab-title-intro", pane_id, animation),
                    motion::animation(motion::TAB_MS),
                    move |this, t| {
                        let inset = 7.0 * (1.0 - t);
                        this.opacity(t)
                            .ml(px(inset))
                            .bg(accent.opacity(0.12 * (1.0 - t)))
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
        match self.app.upgrade() {
            Some(app) => {
                let animation = {
                    let app = app.read(cx);
                    app.pane_animation_for(pane_id)
                };
                let accent = cx.theme().accent;
                let glow = animation.map(|animation| {
                    div()
                        .absolute()
                        .top(px(0.))
                        .right(px(0.))
                        .bottom(px(0.))
                        .left(px(0.))
                        .rounded(px(crate::ui::RADIUS + 2.0))
                        .bg(accent.opacity(0.08))
                        .with_animation(
                            animation_key("file-pane-glow", pane_id, animation),
                            motion::animation(motion::GLOW_MS),
                            move |this, t| motion::glow(this, t, accent),
                        )
                });
                let bounds_app = app.clone();
                let panel = div()
                    .size_full()
                    .relative()
                    .child(self.content.clone())
                    .children(glow)
                    .on_prepaint(move |bounds, window, cx| {
                        bounds_app.update(cx, |app, cx| {
                            if app.record_pane_layout_bounds(pane_id, bounds)
                                && app.request_pane_layout_frame()
                            {
                                window.request_animation_frame();
                                cx.notify();
                            }
                        });
                    });

                if animation.is_some() {
                    panel
                        .with_animation(
                            animation_key("file-pane-motion", pane_id, animation.unwrap()),
                            motion::animation(motion::PANE_MS),
                            move |this, t| {
                                // Cubic ease-out for a snappier, premium feel
                                let ease_out = 1.0 - (1.0 - t).powi(3);
                                let inv = 1.0 - ease_out;

                                let mut x = 0.0;
                                let mut y = 0.0;
                                match animation.unwrap().kind {
                                    crate::app::PaneAnimationKind::NewTab => y = 6.0 * inv,
                                    crate::app::PaneAnimationKind::SplitHorizontal => {
                                        x = 12.0 * inv
                                    }
                                    crate::app::PaneAnimationKind::SplitVertical => y = 12.0 * inv,
                                }

                                this.opacity(ease_out).paint_offset(point(px(x), px(y)))
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
