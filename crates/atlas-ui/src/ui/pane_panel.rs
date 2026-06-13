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

use crate::app::App as AtlasApp;
use crate::state::pane::TabId;
use crate::util::display::display_name;
use gpui::prelude::*;
use gpui::{
    div, px, App, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, WeakEntity,
    Window,
};
use gpui_component::dock::{Panel, PanelEvent, PanelInfo, PanelState};

pub struct FilePanel {
    /// Weak handle to the orchestrator that owns `AppState`.
    app: WeakEntity<AtlasApp>,
    /// Stable identity of the pane this panel renders (also its layout-tree tile id).
    pub(crate) pane_id: TabId,
    focus_handle: FocusHandle,
}

impl FilePanel {
    pub fn new(app: Entity<AtlasApp>, pane_id: TabId, cx: &mut Context<Self>) -> Self {
        Self {
            app: app.downgrade(),
            pane_id,
            focus_handle: cx.focus_handle(),
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

    /// Hide the trailing `…` toolbar menu (and its grey suffix box) on file panes.
    fn show_toolbar(&self, _cx: &gpui::App) -> bool {
        false
    }

    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .app
            .upgrade()
            .and_then(|a| {
                a.read(cx)
                    .tab_data_for(self.pane_id)
                    .map(|t| display_name(&t.location.display_path()))
            })
            .unwrap_or_else(|| rust_i18n::t!("fallback_tab_title").to_string());
        SharedString::from(label)
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pane_id = self.pane_id;
        match self.app.upgrade() {
            // Transparent padding around each pane so split panes float apart with the
            // blurred backdrop showing through the gaps (Figma "Design" layout).
            Some(app) => div()
                .size_full()
                .p(px(4.))
                .child(app.update(cx, |app, cx| app.render_pane(pane_id, window, cx)))
                .into_any_element(),
            None => div().into_any_element(),
        }
    }
}
