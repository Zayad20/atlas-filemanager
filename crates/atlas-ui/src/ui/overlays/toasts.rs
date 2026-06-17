//! File-operation error toasts (bottom-right, dismissible).
#![allow(clippy::too_many_lines)] // reason: cohesive overlay kept whole

use crate::app::App;
use gpui::prelude::*;
use gpui::{deferred, div, px, AnyElement, Context, MouseButton, SharedString};
use gpui_component::{h_flex, v_flex, ActiveTheme};

impl App {
    /// Toasts para errores de operaciones de archivo (bottom-right, sin backdrop).
    pub(crate) fn render_op_errors(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (popover, fg, border, destructive) = {
            let t = cx.theme();
            (t.popover, t.popover_foreground, t.border, t.danger)
        };
        let errors: Vec<String> = self.state.overlay.op_errors.iter().cloned().collect();
        let view = cx.entity();

        let mut stack = v_flex().gap_2();
        for (i, msg) in errors.iter().enumerate() {
            let dismiss_view = view.clone();
            let idx = i;
            // Split message on newlines into individual lines.
            let lines: Vec<String> = msg.lines().map(|l| l.to_string()).collect();
            let mut body = v_flex().gap(px(1.)).text_sm();
            for line in lines {
                body = body.child(line);
            }
            let card = h_flex()
                .id(SharedString::from(format!("op-err-{i}")))
                .gap_3()
                .items_start()
                .w(px(360.))
                .bg(popover)
                .text_color(fg)
                .border_1()
                .border_color(border)
                .rounded_lg()
                .shadow_lg()
                .occlude()
                .p_3()
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(4.))
                        .h_full()
                        .rounded_full()
                        .bg(destructive),
                )
                .child(body.flex_1())
                .child(
                    div()
                        .id(SharedString::from(format!("op-err-x-{i}")))
                        .flex_shrink_0()
                        .text_sm()
                        .text_color(fg)
                        .opacity(0.5)
                        .cursor_pointer()
                        .hover(|h| h.opacity(1.0))
                        .child("✕")
                        .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                            dismiss_view.update(cx, |this, cx| {
                                // Remove by index (adjust for previously removed).
                                let mut v: Vec<_> =
                                    this.state.overlay.op_errors.drain(..).collect();
                                if idx < v.len() {
                                    v.remove(idx);
                                }
                                this.state.overlay.op_errors = v.into_iter().collect();
                                cx.notify();
                            });
                        }),
                );
            stack = stack.child(card);
        }

        let dismiss_all_view = view.clone();
        let has_many = errors.len() > 1;

        deferred(
            div().absolute().bottom(px(24.)).right(px(24.)).child(
                v_flex()
                    .gap_2()
                    .items_end()
                    .child(stack)
                    .when(has_many, |s| {
                        s.child(
                            div()
                                .id("op-err-dismiss-all")
                                .text_xs()
                                .text_color(fg)
                                .opacity(0.5)
                                .cursor_pointer()
                                .hover(|h| h.opacity(1.0))
                                .child(rust_i18n::t!("overlay_close_all").to_string())
                                .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                    dismiss_all_view.update(cx, |this, cx| {
                                        this.state.overlay.op_errors.clear();
                                        cx.notify();
                                    });
                                }),
                        )
                    }),
            ),
        )
        .into_any_element()
    }
}
