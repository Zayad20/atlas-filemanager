//! `PaletteHost`: hosts the open palette in its own gpui view.
//!
//! A palette's open animation (`with_animation`) calls
//! `window.request_animation_frame()` each frame. That notify targets the *current
//! view* being painted — so if the panel is painted inside `App`'s own render, every
//! animation frame re-renders `App` and, through `cx.observe(&app)` in `FilePanel`
//! (`pane_panel.rs`), every file pane rebuilds its thumbnail grid. That is what made
//! opening the goto feel laggy.
//!
//! Hosting the panel in this dedicated view moves the per-frame notify onto
//! `PaletteHost` instead: the open animation only re-renders this overlay, leaving
//! `App` and the panes untouched. We own no overlay state — `render` delegates
//! straight to `App::render_palette_layer` through a weak handle (same pattern as
//! `FilePanel`).
//!
//! Two things drive our re-renders so the list stays live:
//!   * `cx.subscribe` to the input's `InputEvent::Change` — typed queries refresh the
//!     results (the input lives in our element tree, so we own its refresh).
//!   * `cx.observe(&app)` — streamed index results / state changes stay visible.

use crate::app::App;
use crate::ui::palette::PaletteKind;
use gpui::prelude::*;
use gpui::{div, Context, Entity, WeakEntity, Window};
use gpui_component::input::{InputEvent, InputState};

pub struct PaletteHost {
    app: WeakEntity<App>,
    kind: PaletteKind,
    is_rendering: bool,
}

impl PaletteHost {
    pub(crate) fn new(
        app: Entity<App>,
        kind: PaletteKind,
        input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Typed character → re-render so the spec builder re-reads the query and
        // re-runs the match.
        cx.subscribe(input, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        // App repaint (streamed index results, state changes) → re-render. The open
        // animation notifies us directly via `request_animation_frame`, so it does
        // not go through here (and thus never re-renders App or the panes).
        cx.observe(&app, |this: &mut Self, _, cx| {
            if !this.is_rendering {
                cx.notify();
            }
        })
        .detach();
        Self {
            app: app.downgrade(),
            kind,
            is_rendering: false,
        }
    }
}

impl Render for PaletteHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        self.is_rendering = true;
        let el = match self.app.upgrade() {
            Some(app) => app
                .update(cx, |app, cx| app.render_palette_layer(kind, window, cx))
                .into_any_element(),
            None => div().into_any_element(),
        };
        self.is_rendering = false;
        el
    }
}

impl App {
    /// Get-or-create the cached palette host view (mirrors `ensure_dock`). Cleared
    /// to `None` on open and close (`actions.rs`) so each open remounts a fresh view
    /// and replays the open animation; reopening with a different kind replaces it.
    pub(crate) fn ensure_palette_host(
        &mut self,
        kind: PaletteKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<PaletteHost> {
        // The input is the source of truth the spec builder reads each frame; ensure
        // it exists before wiring the host's change-subscription to it.
        let input = self.palette_input_for(kind, window, cx);
        if let Some((cached_kind, host)) = &self.palette_host {
            if *cached_kind == kind {
                return host.clone();
            }
        }
        let app = cx.entity();
        let host = cx.new(|cx| PaletteHost::new(app, kind, &input, cx));
        self.palette_host = Some((kind, host.clone()));
        host
    }
}
