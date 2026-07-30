//! `DockHost`: hosts the whole pane dock in its own gpui view.
//!
//! Split / close / resize animations drive themselves with
//! `window.request_animation_frame()`, which notifies the *current view* being
//! painted. While the dock was painted inside `App::render`, that current view was
//! `App` — so every animation frame re-rendered the entire App tree (sidebar,
//! toolbar, statusbar, and the dock's recursive O(N) tile walk). That is the
//! stutter that grew with the number of panes.
//!
//! Hosting the dock in this dedicated view moves the per-frame notify onto
//! `DockHost`: a split animation only re-renders this view, leaving `App`'s chrome
//! and the cached `FilePanel` entities untouched. We own no dock state — `render`
//! delegates straight to `App::render_custom_dock` through a weak handle (same
//! pattern as `PaletteHost` / `FilePanel`).
//!
//! `cx.observe(&app)` bridges App-state changes (keyboard split/close ops, focus,
//! streamed results) back into a dock re-render. The `is_rendering` guard drops the
//! re-entrant notifies that `render_custom_dock` itself fires (tab-close tick, drag
//! settle) — those self-drive the next frame via `request_animation_frame`, which
//! now targets this view.

use crate::app::App;
use gpui::prelude::*;
use gpui::{div, Context, Entity, WeakEntity, Window};

pub struct DockHost {
    app: WeakEntity<App>,
    is_rendering: bool,
}

impl DockHost {
    pub(crate) fn new(app: Entity<App>, cx: &mut Context<Self>) -> Self {
        // App repaint (keyboard ops, focus, streamed state) → re-render the dock.
        // Animation frames notify us directly via `request_animation_frame`, so they
        // are gated out here (and thus never re-render App or the panes).
        cx.observe(&app, |this: &mut Self, _, cx| {
            if !this.is_rendering {
                cx.notify();
            }
        })
        .detach();
        Self {
            app: app.downgrade(),
            is_rendering: false,
        }
    }
}

impl Render for DockHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.is_rendering = true;
        let el = match self.app.upgrade() {
            Some(app) => app.update(cx, |app, cx| {
                let dock_frame = crate::perf::next_dock_frame();
                let panes = app.file_panel_cache.len();
                let split_animations = app.dock_split_animations.len();
                let closing = app.pane_closing.len();
                let pane_animations = app.pane_animations.len();
                let tiles = app.state.pane_tree.tiles.iter().count();
                crate::perf::count("render.dock_host");
                crate::perf::event(|| {
                    format!(
                        "dock_frame frame={dock_frame} tiles={tiles} panes={panes} \
                         split_animations={split_animations} pane_animations={pane_animations} \
                         closing={closing}"
                    )
                });
                let _perf = crate::perf::span_detail("dock_host.render", || {
                    format!(
                        "frame={dock_frame} tiles={tiles} panes={panes} \
                         split_animations={split_animations} pane_animations={pane_animations} \
                         closing={closing}"
                    )
                });
                app.render_custom_dock(window, cx)
            }),
            None => div().into_any_element(),
        };
        self.is_rendering = false;
        el
    }
}

impl App {
    /// Get-or-create the cached dock host view (mirrors `ensure_palette_host`).
    /// Created once and kept for the App's lifetime — the dock is always shown.
    pub(crate) fn ensure_dock_host(&mut self, cx: &mut Context<Self>) -> Entity<DockHost> {
        if let Some(host) = &self.dock_host {
            return host.clone();
        }
        let app = cx.entity();
        let host = cx.new(|cx| DockHost::new(app, cx));
        self.dock_host = Some(host.clone());
        host
    }
}
