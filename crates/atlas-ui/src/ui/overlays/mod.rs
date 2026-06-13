//! FASE5 — Overlays as gpui layers (port of the egui overlay modules).
//!
//! Returned as extra absolutely-positioned children of the root `v_flex`, painted
//! above everything via `deferred()` (so they sit over the panes/inspector). Each
//! overlay is a full-window backdrop (click-to-close) plus a centered panel.
//!
//! The fuzzy palettes (goto bar, command palette, ext palette) are built on the
//! unified engine in `ui::palette` and hosted in `ui::palette_host`; this module
//! keeps their thin per-frame entry points plus the non-fuzzy modals:
//!   - `paste_conflict` — overwrite/skip/rename/cancel modal.
//!   - settings — general/appearance toggles.
//!   - `batch_rename` — mode, text fields, preview, apply.
//!   - `create_tag`, `op_errors` toasts; context menu lives in `ui/root.rs`.

use crate::app::App;
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Window,
};

impl App {
    /// Build the overlay layers for the current frame. Empty when nothing is open.
    pub(crate) fn render_overlays(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut layers: Vec<AnyElement> = Vec::new();

        // Route keyboard focus: text overlays focus their native `InputState` (so
        // editing + clipboard work), modals without text use the neutral overlay
        // focus handle, and an active inline rename focuses its row input.
        self.sync_overlay_focus(window, cx);

        // Each palette is hosted in its own view so the open animation only
        // re-renders the overlay, not App + every file pane. The goto is kept
        // mounted while `palette_closing` is set so the exit animation can play.
        use crate::ui::palette::PaletteKind;
        let goto_closing = matches!(self.palette_closing, Some((PaletteKind::Goto, _)));
        if self.state.overlay.show_goto_bar || goto_closing {
            layers.push(
                self.ensure_palette_host(PaletteKind::Goto, window, cx)
                    .into_any_element(),
            );
        }
        if self.state.overlay.show_palette {
            layers.push(
                self.ensure_palette_host(PaletteKind::Commands, window, cx)
                    .into_any_element(),
            );
        }
        if self.state.overlay.show_ext_palette {
            layers.push(
                self.ensure_palette_host(PaletteKind::Ext, window, cx)
                    .into_any_element(),
            );
        }
        if self.state.overlay.show_settings {
            layers.push(self.render_settings(cx));
        }
        if self.state.overlay.transfer_conflict.is_some() {
            layers.push(self.render_transfer_conflict(cx));
        }
        if self.state.profile_ui.creating_tag {
            layers.push(self.render_create_tag(cx));
        }
        if self.state.batch_rename.is_some() {
            layers.push(self.render_batch_rename(window, cx));
        }
        if self.state.overlay.renaming_profile {
            layers.push(self.render_rename_profile(window, cx));
        }
        if !self.state.overlay.op_errors.is_empty() {
            layers.push(self.render_op_errors(cx));
        }
        // Right-click menu (Atlas items + native shell items). Painted last so it
        // sits above the other overlays; it owns the keyboard via the branch in
        // `sync_overlay_focus` above.
        if self.ctx_menu.is_some() {
            layers.push(self.render_ctx_menu_layer(window, cx));
        }
        layers
    }

    // Overlay focus arbiter (`sync_overlay_focus`, `batch_focus_handle`,
    // `overlay_focus`) lives in `ui/overlay_focus.rs`.

    // ── palettes (unified engine in `ui::palette`) ───────────────────────────

    /// Dispatch from `PaletteHost` to the right palette layer for this frame.
    pub(crate) fn render_palette_layer(
        &mut self,
        kind: crate::ui::palette::PaletteKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match kind {
            crate::ui::palette::PaletteKind::Goto => self.render_goto_bar(window, cx),
            crate::ui::palette::PaletteKind::Commands => self.render_command_palette(window, cx),
            crate::ui::palette::PaletteKind::Ext => self.render_ext_palette(window, cx),
        }
    }

    fn render_goto_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let kind = crate::ui::palette::PaletteKind::Goto;
        let input = self.palette_input_for(kind, window, cx);
        let changed = self.sync_palette_query(kind, &input, cx);
        // In content mode, re-run the background grep whenever the query changes.
        if changed && self.state.profiles.active().search_in_content {
            self.relaunch_content_search();
        }
        let spec = self.goto_spec(cx);
        self.palette_panel(spec, &input, window, cx)
    }

    // ── command palette ──────────────────────────────────────────────────────

    fn render_command_palette(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kind = crate::ui::palette::PaletteKind::Commands;
        let input = self.palette_input_for(kind, window, cx);
        self.sync_palette_query(kind, &input, cx);
        let spec = self.command_palette_spec(cx);
        self.palette_panel(spec, &input, window, cx)
    }

    fn render_ext_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let kind = crate::ui::palette::PaletteKind::Ext;
        let input = self.palette_input_for(kind, window, cx);
        self.sync_palette_query(kind, &input, cx);
        let spec = self.ext_palette_spec();
        self.palette_panel(spec, &input, window, cx)
    }

    // ── transfer conflict ──────────────────────────────────────────────────────

}

mod transfer_conflict;
mod settings;
mod create_tag;
mod toasts;
mod batch_rename;
mod rename_profile;
mod scrim;
