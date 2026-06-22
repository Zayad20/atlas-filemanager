//! Rename-active-profile modal — an app-own overlay (scrim + panel + native
//! input), like `create_tag`/`batch_rename`. We use our own overlay rather than a
//! gpui-component `Dialog` because the Atlas Frost glass theme makes the library
//! dialog's `theme.overlay` backdrop transparent (the modal opened but was
//! invisible). Keyboard focus is granted by the overlay-focus arbiter's claim for
//! `overlay.renaming_profile`.

use crate::app::App;
use crate::input::action::Action;
use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, Context, Entity, FocusHandle, Focusable, KeyDownEvent, MouseButton, Window,
};
use gpui_component::input::InputState;
use gpui_component::{h_flex, v_flex, ActiveTheme};

impl App {
    /// Ensure the rename input exists (seeded with the active profile name) and
    /// return it. Created here so the focus arbiter can claim it the same frame.
    pub(crate) fn rename_profile_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let idx = self
            .renaming_profile_idx
            .unwrap_or(self.state.profiles.active_idx);
        let name = self
            .state
            .profiles
            .profiles
            .get(idx)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| self.state.profiles.active().name.clone());
        self.rename_profile_input
            .get_or_insert_with(|| crate::ui::text_input::single_line(window, cx, "", name))
            .clone()
    }

    /// Focus handle for the arbiter's claim (creates the input if needed).
    pub(crate) fn rename_profile_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> FocusHandle {
        self.rename_profile_input(window, cx)
            .read(cx)
            .focus_handle(cx)
    }

    fn close_rename_profile(&mut self) {
        self.state.overlay.renaming_profile = false;
        self.rename_profile_input = None;
        self.renaming_profile_idx = None;
    }

    fn submit_rename_profile(&mut self, cx: &mut Context<Self>) {
        if let Some(input) = self.rename_profile_input.clone() {
            let value = input.read(cx).value().to_string();
            let action = self
                .renaming_profile_idx
                .map_or(Action::RenameActiveProfile(value.clone()), |index| {
                    Action::RenameProfile { index, name: value }
                });
            self.process_actions(vec![action]);
        }
        self.close_rename_profile();
    }

    pub(crate) fn render_rename_profile(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (popover, fg, muted, border, accent, accent_fg) = {
            let t = cx.theme();
            (
                t.popover,
                t.popover_foreground,
                t.muted_foreground,
                t.border,
                t.accent,
                t.accent_foreground,
            )
        };
        let view = cx.entity();
        let focus = self.overlay_focus(cx);
        let input = self.rename_profile_input(window, cx);

        let cancel_view = view.clone();
        let save_view = view.clone();
        let panel = v_flex()
            .w(px(360.))
            .bg(popover)
            .text_color(fg)
            .border_1()
            .border_color(border)
            .rounded_lg()
            .shadow_lg()
            .occlude()
            .p_4()
            .gap_3()
            .track_focus(&focus)
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| {
                match ev.keystroke.key.as_str() {
                    "escape" => {
                        this.close_rename_profile();
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "enter" => {
                        this.submit_rename_profile(cx);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(rust_i18n::t!("profile_rename_title").to_string()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(rust_i18n::t!("profile_rename_label").to_string()),
                    )
                    .child(crate::ui::text_input::bordered(&input)),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        div()
                            .id("rename-profile-cancel")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(border)
                            .cursor_pointer()
                            .hover(|h| h.bg(border))
                            .child(rust_i18n::t!("profile_rename_cancel").to_string())
                            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                cancel_view.update(cx, |this, cx| {
                                    this.close_rename_profile();
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        div()
                            .id("rename-profile-save")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(accent)
                            .bg(accent)
                            .text_color(accent_fg)
                            .cursor_pointer()
                            .child(rust_i18n::t!("profile_rename_save").to_string())
                            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                save_view.update(cx, |this, cx| {
                                    this.submit_rename_profile(cx);
                                    cx.notify();
                                });
                            }),
                    ),
            );

        self.overlay_scrim_custom(
            "scrim-rename-profile",
            view,
            |this| this.close_rename_profile(),
            focus,
            panel.into_any_element(),
            cx,
        )
    }
}
