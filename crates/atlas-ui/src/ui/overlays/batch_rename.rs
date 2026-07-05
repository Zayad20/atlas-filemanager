//! Batch-rename modal: mode tabs, native fields, live preview, apply.
#![allow(clippy::too_many_lines)] // reason: cohesive overlay kept whole

use crate::app::{App, BatchRenameField};
use crate::input::action::Action;
use atlas_model::batch_rename::{RenameIssue, RenameMode};
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, Entity, KeyDownEvent, MouseButton, SharedString, Window};
use gpui_component::{h_flex, input::InputState, v_flex, ActiveTheme};
use std::path::PathBuf;

impl App {
    pub(crate) fn close_batch_rename(&mut self) {
        self.state.batch_rename = None;
        self.batch_pattern_input = None;
        self.batch_replacement_input = None;
        self.batch_rename_field = BatchRenameField::Pattern;
    }

    pub(crate) fn render_batch_rename(
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

        // Native inputs are the source of truth: ensure they exist (pre-filled), pull
        // their current text into state and refresh the previews when it changed.
        let (pat0, rep0) = self
            .state
            .batch_rename
            .as_ref()
            .map(|b| (b.pattern.clone(), b.replacement.clone()))
            .unwrap_or_default();
        let pat_input = self
            .batch_pattern_input
            .get_or_insert_with(|| crate::ui::text_input::single_line(window, cx, "", pat0))
            .clone();
        let rep_input = self
            .batch_replacement_input
            .get_or_insert_with(|| crate::ui::text_input::single_line(window, cx, "", rep0))
            .clone();
        let pat_val = pat_input.read(cx).value().to_string();
        let rep_val = rep_input.read(cx).value().to_string();
        if let Some(batch) = self.state.batch_rename.as_mut() {
            let pat_changed = batch.pattern != pat_val;
            let rep_changed = batch.replacement != rep_val;
            if pat_changed {
                batch.pattern = pat_val;
            }
            if rep_changed {
                batch.replacement = rep_val;
            }
            if pat_changed || rep_changed {
                batch.update_previews();
            }
        }

        let Some(batch) = self.state.batch_rename.as_ref() else {
            return div().into_any_element();
        };
        let count = batch.files.len();
        let mode = batch.mode.clone();
        let previews: Vec<(PathBuf, String)> = batch
            .files
            .iter()
            .cloned()
            .zip(batch.previews.iter().cloned())
            .collect();
        // Per-row verdicts; a blocking issue (invalid name / collision) disables apply.
        let issues = batch.issues();
        let blocking = batch.has_blocking_issues();
        let danger = gpui::hsla(0.0, 0.75, 0.55, 1.0);

        let mode_button =
            |label: &str, button_mode: RenameMode, selected: bool, view: Entity<Self>| {
                div()
                    .id(SharedString::from(format!("batch-mode-{label}")))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(if selected { accent } else { border })
                    .cursor_pointer()
                    .when(selected, |s| s.bg(accent).text_color(accent_fg))
                    .when(!selected, |s| s.hover(|h| h.bg(border)))
                    .child(label.to_string())
                    .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                        let m = button_mode.clone();
                        view.update(cx, |this, cx| {
                            if let Some(batch) = this.state.batch_rename.as_mut() {
                                batch.mode = m;
                                batch.update_previews();
                            }
                            this.batch_rename_field = BatchRenameField::Pattern;
                            cx.notify();
                        });
                    })
            };

        // A labeled native input. The input owns editing/clipboard; clicking it focuses
        // it (which also updates the active field via the focus, so Find mode's two
        // inputs behave naturally).
        let field = |label: &str, input: &Entity<InputState>| {
            v_flex()
                .flex_1()
                .gap_1()
                .child(div().text_xs().text_color(muted).child(label.to_string()))
                .child(crate::ui::text_input::bordered(input))
        };

        let rows: Vec<AnyElement> = previews
            .iter()
            .take(200)
            .enumerate()
            .map(|(i, (path, new_name))| {
                let old_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let changed = old_name != *new_name;
                let issue = issues.get(i).copied().unwrap_or(RenameIssue::Ok);
                let bad = matches!(issue, RenameIssue::Invalid | RenameIssue::Collision);
                let name_color = if bad {
                    danger
                } else if changed {
                    accent
                } else {
                    muted
                };
                let marker = match issue {
                    RenameIssue::Invalid => rust_i18n::t!("batch_invalid_name").to_string(),
                    RenameIssue::Collision => rust_i18n::t!("batch_collision").to_string(),
                    RenameIssue::Ok => String::new(),
                };
                h_flex()
                    .id(SharedString::from(format!("batch-row-{i}")))
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .py_1()
                    .child(div().flex_1().min_w(px(0.)).truncate().child(old_name))
                    .child(div().text_color(muted).child("→"))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_color(name_color)
                            .child(format!("{new_name}{marker}")),
                    )
                    .into_any_element()
            })
            .collect();
        let hidden = previews.len().saturating_sub(200);

        let apply_actions: Vec<Action> = previews
            .iter()
            .filter_map(|(path, new_name)| {
                let parent = path.parent()?;
                let to = parent.join(new_name);
                (to != *path).then(|| Action::Rename {
                    from: path.clone(),
                    to,
                })
            })
            .collect();
        let can_apply = !apply_actions.is_empty() && !blocking;
        let rename_view = view.clone();
        let cancel_view = view.clone();

        let panel = v_flex()
            .w(px(640.))
            .max_h(px(620.))
            .bg(popover)
            .text_color(fg)
            .border_1()
            .border_color(border)
            .rounded_lg()
            .shadow_lg()
            .occlude()
            .p_4()
            .gap_2()
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(rust_i18n::t!("batch_title").to_string()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(rust_i18n::t!("batch_selected", count = count).to_string()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .child(mode_button(
                        &rust_i18n::t!("batch_mode_find"),
                        RenameMode::Find,
                        mode == RenameMode::Find,
                        view.clone(),
                    ))
                    .child(mode_button(
                        &rust_i18n::t!("batch_mode_prefix"),
                        RenameMode::Prefix,
                        mode == RenameMode::Prefix,
                        view.clone(),
                    ))
                    .child(mode_button(
                        &rust_i18n::t!("batch_mode_suffix"),
                        RenameMode::Suffix,
                        mode == RenameMode::Suffix,
                        view.clone(),
                    ))
                    .child(mode_button(
                        &rust_i18n::t!("batch_mode_number"),
                        RenameMode::Number,
                        mode == RenameMode::Number,
                        view.clone(),
                    )),
            )
            .child(match mode {
                RenameMode::Find => h_flex()
                    .gap_2()
                    .child(field(&rust_i18n::t!("batch_find"), &pat_input))
                    .child(field(&rust_i18n::t!("batch_replace"), &rep_input))
                    .into_any_element(),
                RenameMode::Prefix => {
                    field(&rust_i18n::t!("batch_prefix"), &pat_input).into_any_element()
                }
                RenameMode::Suffix => {
                    field(&rust_i18n::t!("batch_suffix"), &pat_input).into_any_element()
                }
                RenameMode::Number => {
                    field(&rust_i18n::t!("batch_separator"), &rep_input).into_any_element()
                }
            })
            .child(
                div()
                    .id("batch-preview")
                    .flex_1()
                    .min_h(px(120.))
                    .max_h(px(300.))
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(border)
                    .rounded_md()
                    .p_2()
                    .child(v_flex().gap(px(1.)).children(rows).when(hidden > 0, |v| {
                        v.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(rust_i18n::t!("batch_and_more", count = hidden).to_string()),
                        )
                    })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        div()
                            .id("batch-cancel")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(border)
                            .cursor_pointer()
                            .hover(|h| h.bg(border))
                            .child(rust_i18n::t!("batch_cancel").to_string())
                            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                cancel_view.update(cx, |this, cx| {
                                    this.close_batch_rename();
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        div()
                            .id("batch-rename")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(if can_apply { accent } else { border })
                            .when(can_apply, |s| s.bg(accent).text_color(accent_fg))
                            .when(!can_apply, |s| s.text_color(muted))
                            .cursor_pointer()
                            .child(rust_i18n::t!("batch_rename").to_string())
                            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                let actions = apply_actions.clone();
                                rename_view.update(cx, |this, cx| {
                                    if can_apply {
                                        this.process_actions(actions);
                                        this.close_batch_rename();
                                    }
                                    cx.notify();
                                });
                            }),
                    ),
            )
            .track_focus(&focus)
            // Capture phase: Enter applies, Esc cancels, Tab switches the active field;
            // everything else is real text editing handled by the focused inputs.
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "escape" => {
                        this.close_batch_rename();
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "enter" => {
                        // Refuse to apply while any row is invalid or collides.
                        let safe = this
                            .state
                            .batch_rename
                            .as_ref()
                            .is_some_and(|b| !b.has_blocking_issues());
                        if safe {
                            let actions = this.batch_rename_actions();
                            if !actions.is_empty() {
                                this.process_actions(actions);
                            }
                            this.close_batch_rename();
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "tab" => {
                        this.batch_rename_field = match this.batch_rename_field {
                            BatchRenameField::Pattern => BatchRenameField::Replacement,
                            BatchRenameField::Replacement => BatchRenameField::Pattern,
                        };
                        let handle = this.batch_focus_handle(window, cx);
                        window.focus(&handle, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    _ => {}
                }
            }));

        self.overlay_scrim_custom(
            "scrim-batch-rename",
            view,
            |this| this.close_batch_rename(),
            focus,
            panel.into_any_element(),
            cx,
        )
    }

    // ── rename profile ───────────────────────────────────────────────────────

    fn batch_rename_actions(&self) -> Vec<Action> {
        self.state
            .batch_rename
            .as_ref()
            .map(|batch| {
                batch
                    .files
                    .iter()
                    .zip(&batch.previews)
                    .filter_map(|(path, new_name)| {
                        let parent = path.parent()?;
                        let to = parent.join(new_name);
                        (to != *path).then(|| Action::Rename {
                            from: path.clone(),
                            to,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    // ── scrim helpers ──────────────────────────────────────────────────────────
}
