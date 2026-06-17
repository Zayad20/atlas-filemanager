//! Create-tag modal (name + colour) for the active profile or global.
#![allow(clippy::too_many_lines)] // reason: cohesive overlay kept whole

use crate::app::App;
use crate::input::action::Action;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, KeyDownEvent, MouseButton, SharedString};
use gpui_component::{h_flex, v_flex, ActiveTheme};

impl App {
    pub(crate) fn render_create_tag(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
        let global = self.state.profile_ui.new_tag_global;
        let name = self.state.profile_ui.new_tag_name.clone();
        let selected_color = self.state.profile_ui.new_tag_color;
        let can_create = !name.trim().is_empty();
        let title = if global {
            rust_i18n::t!("tag_title_global").to_string()
        } else {
            rust_i18n::t!("tag_title_profile").to_string()
        };
        let scope = if global {
            rust_i18n::t!("tag_scope_global").to_string()
        } else {
            rust_i18n::t!("tag_scope_profile").to_string()
        };

        let color_buttons = crate::state::profiles::PRESET_COLORS
            .iter()
            .enumerate()
            .map(|(idx, color)| {
                let color_value = *color;
                let rgb = gpui::rgb(
                    (u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2]),
                );
                let selected = color_value == selected_color;
                let color_view = view.clone();
                div()
                    .id(SharedString::from(format!("tag-color-{idx}")))
                    .size(px(22.))
                    .rounded_full()
                    .border_2()
                    .border_color(if selected { accent } else { border })
                    .bg(rgb)
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                        color_view.update(cx, |this, cx| {
                            this.state.profile_ui.new_tag_color = color_value;
                            cx.notify();
                        });
                    })
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        let cancel_view = view.clone();
        let create_view = view.clone();
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
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| {
                this.handle_create_tag_key(ev);
                cx.notify();
            }))
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(div().text_sm().text_color(muted).child(scope))
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(rust_i18n::t!("tag_name").to_string()),
                    )
                    .child(
                        div()
                            .id("create-tag-name")
                            .w_full()
                            .min_h(px(34.))
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(accent)
                            .child(if name.is_empty() {
                                div()
                                    .text_color(muted)
                                    .child(rust_i18n::t!("tag_name_placeholder").to_string())
                            } else {
                                div().child(name)
                            }),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(rust_i18n::t!("tag_color").to_string()),
                    )
                    .child(h_flex().gap_2().children(color_buttons)),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        div()
                            .id("create-tag-cancel")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(border)
                            .cursor_pointer()
                            .hover(|h| h.bg(border))
                            .child(rust_i18n::t!("tag_cancel").to_string())
                            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                cancel_view.update(cx, |this, cx| {
                                    this.process_actions(vec![Action::CloseCreateTagOverlay]);
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        div()
                            .id("create-tag-confirm")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(if can_create { accent } else { border })
                            .when(can_create, |s| s.bg(accent).text_color(accent_fg))
                            .when(!can_create, |s| s.text_color(muted))
                            .cursor_pointer()
                            .child(rust_i18n::t!("tag_create").to_string())
                            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                create_view.update(cx, |this, cx| {
                                    this.submit_create_tag();
                                    cx.notify();
                                });
                            }),
                    ),
            );

        self.overlay_scrim_custom(
            "scrim-create-tag",
            view,
            |this| this.process_actions(vec![Action::CloseCreateTagOverlay]),
            focus,
            panel.into_any_element(),
            cx,
        )
    }

    fn handle_create_tag_key(&mut self, ev: &KeyDownEvent) {
        match ev.keystroke.key.as_str() {
            "escape" => self.process_actions(vec![Action::CloseCreateTagOverlay]),
            "enter" => self.submit_create_tag(),
            "backspace" => {
                self.state.profile_ui.new_tag_name.pop();
            }
            _ => {
                if let Some(ch) = ev.keystroke.key_char.as_ref() {
                    if !ch.is_empty()
                        && !ev.keystroke.modifiers.control
                        && !ev.keystroke.modifiers.platform
                    {
                        self.state.profile_ui.new_tag_name.push_str(ch);
                    }
                }
            }
        }
    }

    fn submit_create_tag(&mut self) {
        let name = self.state.profile_ui.new_tag_name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let color = self.state.profile_ui.new_tag_color;
        let profile = if self.state.profile_ui.new_tag_global {
            None
        } else {
            Some(self.state.profiles.active().name.clone())
        };
        self.process_actions(vec![
            Action::CreateTag {
                name,
                color,
                profile,
            },
            Action::CloseCreateTagOverlay,
        ]);
    }
}
