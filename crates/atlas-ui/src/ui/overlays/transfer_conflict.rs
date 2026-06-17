//! Transfer (copy/move) conflict modal: overwrite/skip/rename/per-item.
#![allow(clippy::too_many_lines)] // reason: cohesive overlay kept whole

use crate::app::App;
use crate::input::action::Action;
use crate::state::ops::Resolution;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, Entity, MouseButton, SharedString};
use gpui_component::{h_flex, v_flex, ActiveTheme};

impl App {
    pub(crate) fn render_transfer_conflict(&mut self, cx: &mut Context<Self>) -> AnyElement {
        use crate::state::ops::{ConflictPolicy, TransferKind};
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
        let Some(conflict) = self.state.overlay.transfer_conflict.as_ref() else {
            return div().into_any_element();
        };
        let verb = match conflict.request.kind {
            TransferKind::Move => rust_i18n::t!("overlay_verb_move"),
            TransferKind::Copy => rust_i18n::t!("overlay_verb_copy"),
        };
        let count = conflict.conflicts.len();
        let per_item = conflict.per_item.clone();
        let has_dirs = conflict.conflicts.iter().any(|c| c.is_dir);

        // One side of the "existing vs incoming" detail line.
        let detail = |size: Option<u64>, modified: Option<std::time::SystemTime>| {
            let mut parts: Vec<String> = Vec::new();
            if let Some(s) = size {
                parts.push(crate::util::display::format_size(s));
            }
            if let Some(m) = modified {
                parts.push(crate::util::display::format_date(m));
            }
            if parts.is_empty() {
                "—".to_string()
            } else {
                parts.join(" · ")
            }
        };

        let policy_btn =
            |label: &str, policy: Option<ConflictPolicy>, primary: bool, view: Entity<Self>| {
                let l = label.to_string();
                div()
                    .id(SharedString::from(format!("tc-{label}")))
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .border_1()
                    .border_color(border)
                    .when(primary, |s| s.bg(accent).text_color(accent_fg))
                    .when(!primary, |s| s.hover(|h| h.bg(border)))
                    .child(l)
                    .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                        let policy = policy.clone();
                        view.update(cx, |this, cx| {
                            this.process_actions(vec![Action::ResolveTransferConflict(policy)]);
                            cx.notify();
                        });
                    })
            };

        let mut rows = v_flex().gap_1().text_xs();
        for (i, item) in conflict.conflicts.iter().take(100).enumerate() {
            let name = item
                .source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let mut row = v_flex()
                .gap(px(1.))
                .py_1()
                .px_2()
                .rounded_md()
                .border_1()
                .border_color(border)
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_color(fg)
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(name),
                        )
                        .when(item.is_dir, |r| {
                            r.child(
                                div()
                                    .text_color(muted)
                                    .child(rust_i18n::t!("overlay_folder_label").to_string()),
                            )
                        }),
                )
                .child(
                    div().text_color(muted).child(
                        rust_i18n::t!(
                            "overlay_existing_new",
                            existing = detail(item.dst_size, item.dst_modified),
                            new = detail(item.src_size, item.src_modified)
                        )
                        .to_string(),
                    ),
                );
            if let Some(choices) = &per_item {
                let chosen = choices.get(i).copied().unwrap_or(Resolution::Skip);
                let mut choice_row = h_flex().gap_1().pt_1();
                for (label_id, res) in [
                    ("Omitir", Resolution::Skip),
                    ("Sobrescribir", Resolution::Overwrite),
                    ("Renombrar", Resolution::Rename),
                ] {
                    let view = view.clone();
                    let label = match res {
                        Resolution::Skip => rust_i18n::t!("overlay_skip"),
                        Resolution::Overwrite => rust_i18n::t!("overlay_overwrite"),
                        Resolution::Rename => rust_i18n::t!("overlay_rename"),
                    };
                    choice_row = choice_row.child(
                        div()
                            .id(SharedString::from(format!("tc-item-{i}-{label_id}")))
                            .px_2()
                            .py(px(2.))
                            .rounded_md()
                            .cursor_pointer()
                            .border_1()
                            .border_color(border)
                            .when(chosen == res, |s| s.bg(accent).text_color(accent_fg))
                            .when(chosen != res, |s| s.hover(|h| h.bg(border)))
                            .child(label.to_string())
                            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                view.update(cx, |this, cx| {
                                    if let Some(choices) = this
                                        .state
                                        .overlay
                                        .transfer_conflict
                                        .as_mut()
                                        .and_then(|tc| tc.per_item.as_mut())
                                    {
                                        if let Some(slot) = choices.get_mut(i) {
                                            *slot = res;
                                        }
                                    }
                                    cx.notify();
                                });
                            }),
                    );
                }
                row = row.child(choice_row);
            }
            rows = rows.child(row);
        }
        if count > 100 {
            rows = rows.child(
                div()
                    .text_color(muted)
                    .child(rust_i18n::t!("overlay_and_more", count = count - 100).to_string()),
            );
        }

        let footer = if let Some(choices) = &per_item {
            let apply = ConflictPolicy::PerItem(choices.clone());
            let back_view = view.clone();
            h_flex()
                .gap_2()
                .child(policy_btn(
                    &rust_i18n::t!("overlay_apply"),
                    Some(apply),
                    true,
                    view.clone(),
                ))
                .child(
                    div()
                        .id("tc-back")
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .border_1()
                        .border_color(border)
                        .hover(|h| h.bg(border))
                        .child(rust_i18n::t!("overlay_back").to_string())
                        .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                            back_view.update(cx, |this, cx| {
                                if let Some(tc) = this.state.overlay.transfer_conflict.as_mut() {
                                    tc.per_item = None;
                                }
                                cx.notify();
                            });
                        }),
                )
                .child(policy_btn(
                    &rust_i18n::t!("overlay_cancel"),
                    None,
                    false,
                    view.clone(),
                ))
        } else {
            let n = count;
            let per_item_view = view.clone();
            let overwrite_label = if has_dirs {
                rust_i18n::t!("overlay_overwrite_merge")
            } else {
                rust_i18n::t!("overlay_overwrite_all")
            };
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(policy_btn(
                    &rust_i18n::t!("overlay_rename_all"),
                    Some(ConflictPolicy::RenameAll),
                    true,
                    view.clone(),
                ))
                .child(policy_btn(
                    &overwrite_label,
                    Some(ConflictPolicy::OverwriteAll),
                    false,
                    view.clone(),
                ))
                .child(policy_btn(
                    &rust_i18n::t!("overlay_skip_all"),
                    Some(ConflictPolicy::SkipAll),
                    false,
                    view.clone(),
                ))
                .child(
                    div()
                        .id("tc-per-item")
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .border_1()
                        .border_color(border)
                        .hover(|h| h.bg(border))
                        .child(rust_i18n::t!("overlay_decide_one_by_one").to_string())
                        .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                            per_item_view.update(cx, |this, cx| {
                                if let Some(tc) = this.state.overlay.transfer_conflict.as_mut() {
                                    tc.per_item = Some(vec![Resolution::Skip; n]);
                                }
                                cx.notify();
                            });
                        }),
                )
                .child(policy_btn(
                    &rust_i18n::t!("overlay_cancel"),
                    None,
                    false,
                    view.clone(),
                ))
        };

        let subtitle = if has_dirs {
            rust_i18n::t!("overlay_conflict_dirs_subtitle", verb = verb).to_string()
        } else {
            rust_i18n::t!("overlay_conflict_files_subtitle", verb = verb).to_string()
        };

        let panel = v_flex()
            .w(px(520.))
            .bg(popover)
            .text_color(fg)
            .border_1()
            .border_color(border)
            .rounded_lg()
            .shadow_lg()
            .occlude()
            .p_4()
            .gap_3()
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(rust_i18n::t!("overlay_elements_exist", count = count).to_string()),
            )
            .child(div().text_sm().text_color(muted).child(subtitle))
            .child(
                div()
                    .id("transfer-conflict-names")
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .child(rows),
            )
            .child(div().h(px(1.)).w_full().bg(border))
            .child(footer);

        // Esc cancels (handled by the modal's own close action on backdrop click).
        let close = Action::ResolveTransferConflict(None);
        self.overlay_scrim(view, close, focus, panel.into_any_element(), false, cx)
    }

    // ── settings ─────────────────────────────────────────────────────────────
}
