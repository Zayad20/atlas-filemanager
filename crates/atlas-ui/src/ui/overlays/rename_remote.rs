//! Rename-cloud-remote modal (same pattern as `rename_profile`).

use crate::app::cloud::CloudAction;
use crate::app::App;
use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, Context, Entity, FocusHandle, Focusable, KeyDownEvent, MouseButton, Window,
};
use gpui_component::input::InputState;
use gpui_component::{h_flex, v_flex, ActiveTheme};

impl App {
    /// Return (or lazily create) the rename input seeded with the remote's current name.
    pub(crate) fn rename_remote_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let seed = self
            .state
            .cloud
            .renaming_remote_id
            .as_deref()
            .and_then(|id| self.state.cloud.remote(id))
            .map(|r| r.name.clone())
            .unwrap_or_default();
        self.rename_remote_input
            .get_or_insert_with(|| crate::ui::text_input::single_line(window, cx, "", seed))
            .clone()
    }

    /// Focus handle for the overlay-focus arbiter.
    pub(crate) fn rename_remote_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> FocusHandle {
        self.rename_remote_input(window, cx)
            .read(cx)
            .focus_handle(cx)
    }

    fn close_rename_remote(&mut self) {
        self.handle_cloud_action(CloudAction::CancelRenameRemote);
    }

    fn submit_rename_remote(&mut self, cx: &mut Context<Self>) {
        let value = self
            .rename_remote_input
            .clone()
            .map(|e| e.read(cx).value().to_string())
            .unwrap_or_default();
        self.handle_cloud_action(CloudAction::SubmitRenameRemote(value));
    }

    pub(crate) fn render_rename_remote(
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
        let input = self.rename_remote_input(window, cx);

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
                        this.close_rename_remote();
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "enter" => {
                        this.submit_rename_remote(cx);
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
                    .child(rust_i18n::t!("cloud_rename_title").to_string()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(rust_i18n::t!("cloud_rename_label").to_string()),
                    )
                    .child(crate::ui::text_input::bordered(&input)),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        div()
                            .id("rename-remote-cancel")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(border)
                            .cursor_pointer()
                            .hover(|h| h.bg(border))
                            .child(rust_i18n::t!("cloud_rename_cancel").to_string())
                            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                cancel_view.update(cx, |this, cx| {
                                    this.close_rename_remote();
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        div()
                            .id("rename-remote-save")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(accent)
                            .bg(accent)
                            .text_color(accent_fg)
                            .cursor_pointer()
                            .child(rust_i18n::t!("cloud_rename_save").to_string())
                            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                save_view.update(cx, |this, cx| {
                                    this.submit_rename_remote(cx);
                                    cx.notify();
                                });
                            }),
                    ),
            );

        self.overlay_scrim_custom(
            "scrim-rename-remote",
            view,
            |this| this.close_rename_remote(),
            focus,
            panel.into_any_element(),
            cx,
        )
    }
}
