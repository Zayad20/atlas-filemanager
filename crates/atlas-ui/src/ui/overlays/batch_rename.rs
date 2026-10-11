//! Batch-rename modal: mode tabs, native fields, live preview, apply.
#![allow(clippy::too_many_lines)] // reason: cohesive overlay kept whole

use crate::app::{App, BatchRenameField};
use crate::input::action::Action;
use atlas_model::batch_rename::{RenameIssue, RenameMode};
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, Entity, KeyDownEvent, MouseButton, SharedString, Window};
use gpui_component::{h_flex, input::InputState, v_flex, ActiveTheme};
use std::collections::{HashMap, HashSet};
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
            if batch.mode != RenameMode::MultiCursor {
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
        let sources: HashSet<PathBuf> = previews.iter().map(|(path, _)| path.clone()).collect();
        let mut target_counts: HashMap<PathBuf, usize> = HashMap::new();
        for (path, new_name) in &previews {
            if let Some(parent) = path.parent() {
                *target_counts.entry(parent.join(new_name)).or_insert(0) += 1;
            }
        }

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
                    .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                        let m = button_mode.clone();
                        view.update(cx, |this, cx| {
                            if let Some(batch) = this.state.batch_rename.as_mut() {
                                batch.set_mode(m);
                            }
                            this.batch_rename_field = BatchRenameField::Pattern;
                            let handle = this.batch_focus_handle(window, cx);
                            window.focus(&handle, cx);
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

        let helper_button =
            |id: &'static str,
             label: String,
             btn_view: Entity<Self>,
             on_click: Box<dyn Fn(&mut App, &mut Window, &mut Context<App>)>| {
                div()
                    .id(id)
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(border)
                    .text_xs()
                    .cursor_pointer()
                    .hover(|h| h.bg(border))
                    .child(label)
                    .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                        btn_view.update(cx, |this, cx| {
                            on_click(this, window, cx);
                            cx.notify();
                        });
                    })
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
                let target = path.parent().map(|parent| parent.join(new_name));
                let marker = match issue {
                    RenameIssue::Invalid => rust_i18n::t!("batch_invalid_name").to_string(),
                    RenameIssue::Collision => {
                        if let Some(target) = target {
                            let target_name = target
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| new_name.clone());
                            if target_counts.get(&target).copied().unwrap_or(0) > 1 {
                                rust_i18n::t!("batch_collision_duplicate", name = target_name)
                                    .to_string()
                            } else if target.exists() && !sources.contains(&target) {
                                rust_i18n::t!("batch_collision_with", name = target_name)
                                    .to_string()
                            } else {
                                rust_i18n::t!("batch_collision").to_string()
                            }
                        } else {
                            rust_i18n::t!("batch_collision").to_string()
                        }
                    }
                    RenameIssue::Ok => String::new(),
                };

                if mode == RenameMode::MultiCursor {
                    let line_str = batch
                        .multi_lines
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| new_name.clone());
                    let chars: Vec<char> = line_str.chars().collect();
                    let pos = batch.cursor_pos.min(chars.len());
                    let before: String = chars[..pos].iter().collect();
                    let after: String = chars[pos..].iter().collect();

                    h_flex()
                        .id(SharedString::from(format!("batch-row-{i}")))
                        .gap_2()
                        .items_center()
                        .text_xs()
                        .py_1()
                        .child(
                            div()
                                .w(px(24.))
                                .text_color(muted)
                                .child(format!("{:02}", i + 1)),
                        )
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w(px(0.))
                                .items_center()
                                .overflow_x_hidden()
                                .child(div().child(before))
                                .child(
                                    div()
                                        .text_color(accent)
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .child("|"),
                                )
                                .child(div().child(after)),
                        )
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
                } else {
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
                }
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
                        &rust_i18n::t!("batch_mode_multicursor"),
                        RenameMode::MultiCursor,
                        mode == RenameMode::MultiCursor,
                        view.clone(),
                    ))
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
                RenameMode::MultiCursor => v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(helper_button(
                                "batch-btn-stem-end",
                                rust_i18n::t!("batch_cursor_stem_end").to_string(),
                                view.clone(),
                                Box::new(|this, window, cx| {
                                    if let Some(batch) = this.state.batch_rename.as_mut() {
                                        batch.align_cursor_to_stem_end();
                                    }
                                    let handle = this.batch_focus_handle(window, cx);
                                    window.focus(&handle, cx);
                                }),
                            ))
                            .child(helper_button(
                                "batch-btn-start",
                                rust_i18n::t!("batch_cursor_start").to_string(),
                                view.clone(),
                                Box::new(|this, window, cx| {
                                    if let Some(batch) = this.state.batch_rename.as_mut() {
                                        batch.align_cursor_to_start();
                                    }
                                    let handle = this.batch_focus_handle(window, cx);
                                    window.focus(&handle, cx);
                                }),
                            ))
                            .child(helper_button(
                                "batch-btn-number",
                                rust_i18n::t!("batch_insert_number").to_string(),
                                view.clone(),
                                Box::new(|this, window, cx| {
                                    if let Some(batch) = this.state.batch_rename.as_mut() {
                                        let pad = if batch.files.len() >= 100 { 3 } else { 2 };
                                        batch.insert_counter(pad);
                                    }
                                    let handle = this.batch_focus_handle(window, cx);
                                    window.focus(&handle, cx);
                                }),
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(rust_i18n::t!("batch_multicursor_hint").to_string()),
                    )
                    .into_any_element(),
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
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let is_multicursor = this
                    .state
                    .batch_rename
                    .as_ref()
                    .is_some_and(|b| b.mode == RenameMode::MultiCursor);

                if is_multicursor {
                    if (ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform)
                        && ev.keystroke.key == "v"
                    {
                        if let Some(item) = cx.read_from_clipboard() {
                            if let Some(text) = item.text() {
                                if let Some(batch) = this.state.batch_rename.as_mut() {
                                    batch.insert_str(&text);
                                    cx.stop_propagation();
                                    cx.notify();
                                    return;
                                }
                            }
                        }
                    }

                    match ev.keystroke.key.as_str() {
                        "escape" => {
                            this.close_batch_rename();
                            cx.stop_propagation();
                            cx.notify();
                        }
                        "enter" => {
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
                        "backspace" => {
                            if let Some(batch) = this.state.batch_rename.as_mut() {
                                batch.backspace();
                                cx.stop_propagation();
                                cx.notify();
                            }
                        }
                        "delete" => {
                            if let Some(batch) = this.state.batch_rename.as_mut() {
                                batch.delete_forward();
                                cx.stop_propagation();
                                cx.notify();
                            }
                        }
                        "left" => {
                            if let Some(batch) = this.state.batch_rename.as_mut() {
                                if ev.keystroke.modifiers.control
                                    || ev.keystroke.modifiers.platform
                                {
                                    batch.move_cursor_word_left();
                                } else {
                                    batch.move_cursor_left();
                                }
                                cx.stop_propagation();
                                cx.notify();
                            }
                        }
                        "right" => {
                            if let Some(batch) = this.state.batch_rename.as_mut() {
                                if ev.keystroke.modifiers.control
                                    || ev.keystroke.modifiers.platform
                                {
                                    batch.move_cursor_word_right();
                                } else {
                                    batch.move_cursor_right();
                                }
                                cx.stop_propagation();
                                cx.notify();
                            }
                        }
                        "home" => {
                            if let Some(batch) = this.state.batch_rename.as_mut() {
                                batch.move_cursor_home();
                                cx.stop_propagation();
                                cx.notify();
                            }
                        }
                        "end" => {
                            if let Some(batch) = this.state.batch_rename.as_mut() {
                                batch.move_cursor_end();
                                cx.stop_propagation();
                                cx.notify();
                            }
                        }
                        "tab" => {
                            if let Some(batch) = this.state.batch_rename.as_mut() {
                                batch.set_mode(RenameMode::Find);
                                this.batch_rename_field = BatchRenameField::Pattern;
                                let handle = this.batch_focus_handle(window, cx);
                                window.focus(&handle, cx);
                                cx.stop_propagation();
                                cx.notify();
                            }
                        }
                        _ => {
                            if let Some(c) = get_typed_char(ev) {
                                if let Some(batch) = this.state.batch_rename.as_mut() {
                                    if c == '#' {
                                        let pad = if batch.files.len() >= 100 { 3 } else { 2 };
                                        batch.insert_counter(pad);
                                    } else {
                                        batch.insert_char(c);
                                    }
                                    cx.stop_propagation();
                                    cx.notify();
                                }
                            }
                        }
                    }
                } else {
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

fn get_typed_char(ev: &KeyDownEvent) -> Option<char> {
    if ev.keystroke.modifiers.control
        || ev.keystroke.modifiers.platform
        || ev.keystroke.modifiers.alt
    {
        return None;
    }
    if ev.keystroke.key == "space" {
        return Some(' ');
    }
    if let Some(ref kc) = ev.keystroke.key_char {
        let mut chars = kc.chars();
        if let Some(c) = chars.next() {
            if chars.next().is_none() && !c.is_control() {
                return Some(c);
            }
        }
    }
    let key = ev.keystroke.key.as_str();
    let mut chars = key.chars();
    if let Some(c) = chars.next() {
        if chars.next().is_none() && !c.is_control() {
            return Some(c);
        }
    }
    None
}

