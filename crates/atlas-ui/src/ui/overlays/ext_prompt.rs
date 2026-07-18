//! Parameter dialog for commands contributed by extensions.

use crate::app::ext::ExtAction;
use crate::app::App;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, KeyDownEvent, MouseButton, Window};
use gpui_component::{h_flex, v_flex, ActiveTheme};

impl App {
    pub(crate) fn ensure_ext_prompt_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.state.ext.pending.as_ref() else {
            self.ext_prompt_inputs.clear();
            return;
        };
        if self.ext_prompt_inputs.len() == pending.answers.len() {
            return;
        }
        self.ext_prompt_inputs = pending
            .answers
            .iter()
            .map(|(_, _, value)| crate::ui::text_input::single_line(window, cx, "", value.clone()))
            .collect();
    }

    fn submit_ext_prompt(&mut self, cx: &mut Context<Self>) {
        let values: Vec<String> = self
            .ext_prompt_inputs
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect();
        if let Some(pending) = self.state.ext.pending.as_mut() {
            for ((_, _, answer), value) in pending.answers.iter_mut().zip(values) {
                *answer = value;
            }
        }
        self.handle_ext_action(ExtAction::ConfirmPending);
    }

    pub(crate) fn render_ext_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.ensure_ext_prompt_inputs(window, cx);
        let Some(pending) = self.state.ext.pending.as_ref() else {
            return div().into_any_element();
        };
        let title = pending.cmd.command.title.clone();
        let fields: Vec<(String, gpui::Entity<gpui_component::input::InputState>)> = pending
            .answers
            .iter()
            .zip(self.ext_prompt_inputs.iter())
            .map(|((_, label, _), input)| (label.clone(), input.clone()))
            .collect();

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
        let cancel_view = view.clone();
        let run_view = view.clone();
        let focus = self.overlay_focus(cx);

        let panel = v_flex()
            .w(px(440.))
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
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, _window, cx| {
                match ev.keystroke.key.as_str() {
                    "escape" => {
                        this.handle_ext_action(ExtAction::CancelPending);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "enter" => {
                        this.submit_ext_prompt(cx);
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
                    .child(title),
            )
            .children(fields.into_iter().map(|(label, input)| {
                v_flex()
                    .gap_1()
                    .child(div().text_xs().text_color(muted).child(label))
                    .child(crate::ui::text_input::bordered(&input))
            }))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        div()
                            .id("ext-prompt-cancel")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(border)
                            .cursor_pointer()
                            .hover(|h| h.bg(border))
                            .child(rust_i18n::t!("overlay_cancel").to_string())
                            .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                                cancel_view.update(cx, |this, cx| {
                                    this.handle_ext_action(ExtAction::CancelPending);
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        div()
                            .id("ext-prompt-run")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(accent)
                            .bg(accent)
                            .text_color(accent_fg)
                            .cursor_pointer()
                            .child(rust_i18n::t!("ext_prompt_run").to_string())
                            .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                                run_view.update(cx, |this, cx| {
                                    this.submit_ext_prompt(cx);
                                    cx.notify();
                                });
                            }),
                    ),
            );

        self.overlay_scrim_custom(
            "scrim-ext-prompt",
            view,
            |this| this.handle_ext_action(ExtAction::CancelPending),
            focus,
            panel.into_any_element(),
            cx,
        )
    }
}
