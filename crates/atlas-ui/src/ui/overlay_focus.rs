//! Overlay focus arbiter — the single authority for "who owns the keyboard this
//! frame". Runs once per frame from `render_overlays`.
//!
//! # The recurring bug this solves
//! Secondary panels (modals, popovers, menus) need to capture keyboard input. The
//! arbiter reassigns focus every frame, so historically **any** new panel that
//! wasn't hand-wired here lost focus on the very next frame (its input died). It
//! also fought gpui-component's own overlays (`Dialog`/`Sheet`/`Popover`), which
//! manage focus themselves — the arbiter would yank focus back to the root.
//!
//! # The engine: two layers
//! 1. **Yield to gpui-component.** If a gpui-component `Dialog`/`Sheet` is active,
//!    the arbiter does nothing and lets the library own focus. This is why the
//!    **official way to add a modal/popover that captures input is a
//!    gpui-component `Dialog`/`Popover`** — it Just Works, no wiring here.
//! 2. **Claim resolver for the app's own overlays** ([`App::overlay_focus_target`]):
//!    a single, ordered place that maps app state → the focus handle that should
//!    own the keyboard. Adding a bespoke overlay = one arm here (not a branch
//!    scattered across the codebase). If no claim is active, focus returns to the
//!    root so pane keyboard-nav works.

use crate::app::{App, BatchRenameField};
use crate::state::batch_rename::RenameMode;
use gpui::{Context, Focusable, FocusHandle, Window};
use gpui_component::WindowExt;

impl App {
    /// Decide which focus handle should own the keyboard this frame and move focus
    /// to it (only when not already focused, to avoid fighting the cursor).
    pub(crate) fn sync_overlay_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Layer 1 — yield to gpui-component overlays. A `Dialog`/`Sheet` (and the
        // `AlertDialog` built on `Dialog`) manage their own focus; touching focus
        // here would steal it from their inputs every frame. Bail out entirely.
        if window.has_active_dialog(cx) || window.has_active_sheet(cx) {
            return;
        }

        // Layer 2 — resolve the app's own overlay claims.
        let target = self.overlay_focus_target(window, cx);
        if let Some(handle) = target {
            if !handle.is_focused(window) {
                window.focus(&handle, cx);
            }
        } else {
            // Nothing claims the keyboard: return it to the root so pane nav works.
            let root = self.focus_handle.clone();
            if !root.is_focused(window) {
                window.focus(&root, cx);
            }
        }
    }

    /// Ordered claim resolver for the app's bespoke overlays. First active claim
    /// wins. To add a new bespoke overlay that captures input, add an arm here (or
    /// prefer a gpui-component `Dialog`/`Popover`, which Layer 1 yields to).
    fn overlay_focus_target(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<FocusHandle> {
        use crate::ui::palette::PaletteKind;

        // The right-click menu's search input owns the keyboard while open. Placed
        // first: opening a palette closes the menu, so they never contend.
        if self.ctx_menu.is_some() {
            return self.ctx_menu_focus(cx);
        }
        if self.state.overlay.show_goto_bar {
            let input = self.palette_input_for(PaletteKind::Goto, window, cx);
            return Some(input.read(cx).focus_handle(cx));
        }
        if self.state.overlay.show_palette {
            let input = self.palette_input_for(PaletteKind::Commands, window, cx);
            return Some(input.read(cx).focus_handle(cx));
        }
        if self.state.overlay.show_ext_palette {
            let input = self.palette_input_for(PaletteKind::Ext, window, cx);
            return Some(input.read(cx).focus_handle(cx));
        }
        if self.state.batch_rename.is_some() {
            return Some(self.batch_focus_handle(window, cx));
        }
        // Text-less modals: a neutral handle so Esc/buttons work but no input steals.
        if self.state.overlay.show_settings
            || self.state.overlay.transfer_conflict.is_some()
            || self.state.profile_ui.creating_tag
        {
            return Some(self.overlay_focus(cx));
        }
        // Inline (in-row) rename owns the keyboard while active.
        if let Some(edit) = self.rename_edit.as_ref() {
            return Some(edit.input.read(cx).focus_handle(cx));
        }
        // The focused pane's local filter owns the keyboard while active; arrows/
        // enter/escape still work via the filter pill's key capture.
        let tile = self.state.focused_tile;
        if self
            .state
            .tab_states
            .get(&tile)
            .is_some_and(|ts| ts.search.active)
        {
            return self
                .search_inputs
                .get(&tile)
                .map(|input| input.read(cx).focus_handle(cx));
        }
        None
    }

    /// Ensure both batch-rename inputs exist (pre-filled from state) and return the
    /// focus handle of whichever field is currently active for the selected mode.
    pub(crate) fn batch_focus_handle(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> FocusHandle {
        let (pat0, rep0) = self
            .state
            .batch_rename
            .as_ref()
            .map(|b| (b.pattern.clone(), b.replacement.clone()))
            .unwrap_or_default();
        if self.batch_pattern_input.is_none() {
            self.batch_pattern_input =
                Some(crate::ui::text_input::single_line(window, cx, "", pat0));
        }
        if self.batch_replacement_input.is_none() {
            self.batch_replacement_input =
                Some(crate::ui::text_input::single_line(window, cx, "", rep0));
        }
        let mode = self.state.batch_rename.as_ref().map(|b| b.mode.clone());
        let use_replacement = matches!(mode, Some(RenameMode::Number))
            || (matches!(mode, Some(RenameMode::Find))
                && self.batch_rename_field == BatchRenameField::Replacement);
        let input = if use_replacement {
            self.batch_replacement_input.clone()
        } else {
            self.batch_pattern_input.clone()
        };
        // Both were just ensured above; fall back to the neutral handle defensively.
        if let Some(i) = input {
            i.read(cx).focus_handle(cx)
        } else {
            self.overlay_focus(cx)
        }
    }

    /// A neutral focus handle for text-less modals (so they own the keyboard for
    /// Esc/button handling without an input field stealing it).
    pub(crate) fn overlay_focus(&mut self, cx: &mut Context<Self>) -> FocusHandle {
        self.overlay_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone()
    }
}
