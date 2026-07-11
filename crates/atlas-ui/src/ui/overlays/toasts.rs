//! File-operation error toasts (bottom-right, dismissible).
#![allow(clippy::too_many_lines)] // reason: cohesive overlay kept whole

use crate::app::App;
use crate::ui::icons::{icon, AtlasIcon};
use atlas_model::overlay::ToastKind;
use gpui::prelude::*;
use gpui::{deferred, div, px, AnyElement, Context, MouseButton, SharedString, Window};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex, ActiveTheme};

const TOAST_MAX_W: f32 = 420.;
const TOAST_SIDE_MARGIN: f32 = 24.;
const TOAST_WRAP_COLS: usize = 58;
const TOAST_MAX_BODY_H: f32 = 140.;

impl App {
    /// File-operation error toasts (bottom-right, no backdrop).
    pub(crate) fn render_op_errors(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (popover, fg, muted, border, destructive, accent) = {
            let t = cx.theme();
            (
                t.popover,
                t.popover_foreground,
                t.muted_foreground,
                t.border,
                t.danger,
                t.accent,
            )
        };
        let errors: Vec<_> = self.state.overlay.op_errors.iter().cloned().collect();
        let view = cx.entity();
        let viewport_w = f32::from(window.bounds().size.width);
        // Chrome constants scaled explicitly: `toast_w` is combined with `viewport_w`
        // (a real measured pixel width) via plain arithmetic, so it can't be
        // resolved through `rem_px()`'s Styled-only Rems path.
        let zoom_scale = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        let toast_w = (TOAST_MAX_W * zoom_scale)
            .min((viewport_w - TOAST_SIDE_MARGIN * zoom_scale * 2.0).max(280. * zoom_scale));

        let mut stack = v_flex().gap_2();
        for (i, toast) in errors.iter().enumerate() {
            let dismiss_view = view.clone();
            let idx = i;
            let tone = match toast.kind {
                ToastKind::Error => destructive,
                ToastKind::Info => accent,
            };
            let glyph = match toast.kind {
                ToastKind::Error => AtlasIcon::X,
                ToastKind::Info => AtlasIcon::Lightning,
            };
            let lines = wrap_toast_message(&toast.message);
            let mut body_lines = v_flex().gap(crate::ui::rem_px(2.));
            for line in lines {
                body_lines = body_lines.child(div().min_w_0().child(line));
            }
            let body = div()
                .flex_1()
                .min_w_0()
                .max_h(crate::ui::rem_px(TOAST_MAX_BODY_H))
                .overflow_y_scrollbar()
                .text_sm()
                .line_height(crate::ui::rem_px(18.))
                .child(body_lines);
            let card = h_flex()
                .id(SharedString::from(format!("op-err-{i}")))
                .gap_2()
                .items_start()
                .w(px(toast_w))
                .bg(popover)
                .text_color(fg)
                .border_1()
                .border_color(tone.opacity(0.35))
                .rounded_lg()
                .shadow_lg()
                .occlude()
                .p_3()
                .child(
                    div()
                        .flex_shrink_0()
                        .mt(crate::ui::rem_px(2.))
                        .size(crate::ui::rem_px(20.))
                        .rounded_full()
                        .bg(tone.opacity(0.12))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(glyph, 10., tone)),
                )
                .child(body)
                .child(
                    div()
                        .id(SharedString::from(format!("op-err-x-{i}")))
                        .flex_shrink_0()
                        .size(crate::ui::rem_px(22.))
                        .rounded(px(crate::ui::RADIUS))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(muted)
                        .cursor_pointer()
                        .hover(move |h| h.bg(border).text_color(fg))
                        .child(icon(AtlasIcon::X, 12., muted))
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
            div()
                .absolute()
                .top(crate::ui::rem_px(24.))
                .right(crate::ui::rem_px(24.))
                .child(
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

fn wrap_toast_message(msg: &str) -> Vec<String> {
    let mut out = Vec::new();

    for raw_line in msg.lines() {
        let mut line = String::new();

        for word in raw_line.split_whitespace() {
            for part in split_long_word(word, TOAST_WRAP_COLS) {
                if !line.is_empty() && line.len() + part.len() + 1 > TOAST_WRAP_COLS {
                    out.push(std::mem::take(&mut line));
                }
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(part);
            }
        }

        if !line.is_empty() {
            out.push(line);
        }
    }

    if out.is_empty() {
        out.push(rust_i18n::t!("toast_op_failed").into_owned());
    }

    out
}

fn split_long_word(word: &str, max_cols: usize) -> Vec<&str> {
    if word.len() <= max_cols {
        return vec![word];
    }

    let mut parts = Vec::new();
    let mut start = 0;
    for (idx, _) in word.char_indices() {
        if idx - start >= max_cols {
            parts.push(&word[start..idx]);
            start = idx;
        }
    }
    if start < word.len() {
        parts.push(&word[start..]);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::{wrap_toast_message, TOAST_WRAP_COLS};

    #[test]
    fn toast_wraps_long_error_words() {
        let msg = "googledrive: rc mount/mount: failed-to-mount-FUSE-fs:abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";
        let lines = wrap_toast_message(msg);

        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| line.len() <= TOAST_WRAP_COLS));
    }
}
