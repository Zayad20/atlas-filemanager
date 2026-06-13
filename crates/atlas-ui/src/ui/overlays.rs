//! FASE5 — Overlays as gpui layers (port of the egui overlay modules).
//!
//! Returned as extra absolutely-positioned children of the root `v_flex`, painted
//! above everything via `deferred()` (so they sit over the panes/inspector). Each
//! overlay is a full-window backdrop (click-to-close) plus a centered panel.
//!
//! The fuzzy palettes (goto bar, command palette, ext palette) are built on the
//! unified engine in `ui::palette` and hosted in `ui::palette_host`; this module
//! keeps their thin per-frame entry points plus the non-fuzzy modals:
//!   - `paste_conflict` — overwrite/skip/rename/cancel modal.
//!   - settings — general/appearance toggles.
//!   - `batch_rename` — mode, text fields, preview, apply.
//!   - `create_tag`, `op_errors` toasts; context menu lives in `ui/root.rs`.

use crate::app::{App, BatchRenameField};
use crate::input::action::Action;
use crate::state::batch_rename::{RenameIssue, RenameMode};
use crate::state::ops::Resolution;
use crate::state::settings::{InspectorPlacement, ViewMode};
use gpui::prelude::*;
use gpui::AnimationExt;
use gpui::{
    deferred, div, px, AnyElement, Context, Entity, FocusHandle, Focusable, Hsla, KeyDownEvent,
    MouseButton, SharedString, Window,
};
use gpui_component::{h_flex, input::InputState, v_flex, ActiveTheme, WindowExt};
use std::path::PathBuf;

impl App {
    /// Build the overlay layers for the current frame. Empty when nothing is open.
    pub(crate) fn render_overlays(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut layers: Vec<AnyElement> = Vec::new();

        // Route keyboard focus: text overlays focus their native `InputState` (so
        // editing + clipboard work), modals without text use the neutral overlay
        // focus handle, and an active inline rename focuses its row input.
        self.sync_overlay_focus(window, cx);

        // Each palette is hosted in its own view so the open animation only
        // re-renders the overlay, not App + every file pane. The goto is kept
        // mounted while `palette_closing` is set so the exit animation can play.
        use crate::ui::palette::PaletteKind;
        let goto_closing = matches!(self.palette_closing, Some((PaletteKind::Goto, _)));
        if self.state.overlay.show_goto_bar || goto_closing {
            layers.push(
                self.ensure_palette_host(PaletteKind::Goto, window, cx)
                    .into_any_element(),
            );
        }
        if self.state.overlay.show_palette {
            layers.push(
                self.ensure_palette_host(PaletteKind::Commands, window, cx)
                    .into_any_element(),
            );
        }
        if self.state.overlay.show_ext_palette {
            layers.push(
                self.ensure_palette_host(PaletteKind::Ext, window, cx)
                    .into_any_element(),
            );
        }
        if self.state.overlay.show_settings {
            layers.push(self.render_settings(cx));
        }
        if self.state.overlay.transfer_conflict.is_some() {
            layers.push(self.render_transfer_conflict(cx));
        }
        if self.state.profile_ui.creating_tag {
            layers.push(self.render_create_tag(cx));
        }
        if self.state.batch_rename.is_some() {
            layers.push(self.render_batch_rename(window, cx));
        }
        if !self.state.overlay.op_errors.is_empty() {
            layers.push(self.render_op_errors(cx));
        }
        // Right-click menu (Atlas items + native shell items). Painted last so it
        // sits above the other overlays; it owns the keyboard via the branch in
        // `sync_overlay_focus` above.
        if self.ctx_menu.is_some() {
            layers.push(self.render_ctx_menu_layer(window, cx));
        }
        layers
    }

    // Overlay focus arbiter (`sync_overlay_focus`, `batch_focus_handle`,
    // `overlay_focus`) lives in `ui/overlay_focus.rs`.

    // ── palettes (unified engine in `ui::palette`) ───────────────────────────

    /// Dispatch from `PaletteHost` to the right palette layer for this frame.
    pub(crate) fn render_palette_layer(
        &mut self,
        kind: crate::ui::palette::PaletteKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match kind {
            crate::ui::palette::PaletteKind::Goto => self.render_goto_bar(window, cx),
            crate::ui::palette::PaletteKind::Commands => self.render_command_palette(window, cx),
            crate::ui::palette::PaletteKind::Ext => self.render_ext_palette(window, cx),
        }
    }

    fn render_goto_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let kind = crate::ui::palette::PaletteKind::Goto;
        let input = self.palette_input_for(kind, window, cx);
        let changed = self.sync_palette_query(kind, &input, cx);
        // In content mode, re-run the background grep whenever the query changes.
        if changed && self.state.profiles.active().search_in_content {
            self.relaunch_content_search();
        }
        let spec = self.goto_spec(cx);
        self.palette_panel(spec, &input, window, cx)
    }

    // ── command palette ──────────────────────────────────────────────────────

    fn render_command_palette(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kind = crate::ui::palette::PaletteKind::Commands;
        let input = self.palette_input_for(kind, window, cx);
        self.sync_palette_query(kind, &input, cx);
        let spec = self.command_palette_spec(cx);
        self.palette_panel(spec, &input, window, cx)
    }

    fn render_ext_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let kind = crate::ui::palette::PaletteKind::Ext;
        let input = self.palette_input_for(kind, window, cx);
        self.sync_palette_query(kind, &input, cx);
        let spec = self.ext_palette_spec();
        self.palette_panel(spec, &input, window, cx)
    }

    // ── transfer conflict ──────────────────────────────────────────────────────

    fn render_transfer_conflict(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
                            r.child(div().text_color(muted).child(rust_i18n::t!("overlay_folder_label").to_string()))
                        }),
                )
                .child(div().text_color(muted).child(rust_i18n::t!(
                    "overlay_existing_new",
                    existing = detail(item.dst_size, item.dst_modified),
                    new = detail(item.src_size, item.src_modified)
                ).to_string()));
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
                .child(policy_btn(&rust_i18n::t!("overlay_apply"), Some(apply), true, view.clone()))
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
                .child(policy_btn(&rust_i18n::t!("overlay_cancel"), None, false, view.clone()))
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
                .child(policy_btn(&rust_i18n::t!("overlay_cancel"), None, false, view.clone()))
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

    fn render_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (popover, fg, muted, border, accent, surface, accent_fg) = {
            let t = cx.theme();
            (
                t.popover,
                t.popover_foreground,
                t.muted_foreground,
                t.border,
                t.accent,
                t.background,
                t.accent_foreground,
            )
        };
        let view = cx.entity();
        let focus = self.overlay_focus(cx);
        // Theme picker data: the registered theme names and the active profile's pick.
        let active_theme = self.state.profiles.active().theme.clone();
        let theme_names: Vec<String> = gpui_component::ThemeRegistry::global(cx)
            .sorted_themes()
            .iter()
            .map(|c| c.name.to_string())
            .collect();
        let s = &self.state.settings;

        // A simple labeled toggle row that flips a settings bool via a closure.
        let toggle = move |label: &str,
                           value: bool,
                           id: &'static str,
                           set: fn(&mut Self),
                           view: Entity<Self>| {
            h_flex()
                .id(SharedString::from(id))
                .w_full()
                .justify_between()
                .items_center()
                .py_1()
                .cursor_pointer()
                .child(label.to_string())
                .child(
                    div()
                        .w(px(36.))
                        .h(px(20.))
                        .rounded_full()
                        .p(px(2.))
                        .when(value, |d| d.bg(accent))
                        .when(!value, |d| d.bg(border))
                        .child(
                            div()
                                .size(px(16.))
                                .rounded_full()
                                .bg(fg)
                                .when(value, |d| d.ml(px(16.))),
                        ),
                )
                .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                    view.update(cx, |this, cx| {
                        set(this);
                        cx.notify();
                    });
                })
        };

        let placement = match s.inspector_placement {
            InspectorPlacement::Global => rust_i18n::t!("settings_inspector_global"),
            InspectorPlacement::ActiveTab => rust_i18n::t!("settings_inspector_tab"),
        };
        let view_mode = match s.view_mode {
            ViewMode::Details => rust_i18n::t!("settings_view_details"),
            ViewMode::Grid => rust_i18n::t!("settings_view_grid"),
            ViewMode::List => rust_i18n::t!("settings_view_list"),
            ViewMode::Miller => rust_i18n::t!("settings_view_miller"),
        };
        let inspector_placement = s.inspector_placement.clone();
        let default_view_mode = s.view_mode.clone();
        let sidebar_width = s.sidebar_width;
        let inspector_width = s.inspector_width;
        let font_size = s.font_size;
        let thumbnail_size = s.thumbnail_size;

        let segment = move |id: &'static str,
                            label: &str,
                            selected: bool,
                            apply: fn(&mut Self),
                            view: Entity<Self>| {
            div()
                .id(id)
                .px_2()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(if selected { accent } else { border })
                .cursor_pointer()
                .when(selected, |s| s.bg(accent))
                .when(!selected, |s| s.hover(|h| h.bg(border)))
                .child(label.to_string())
                .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                    view.update(cx, |this, cx| {
                        apply(this);
                        cx.notify();
                    });
                })
        };

        let stepper = move |id: &'static str,
                            label: &str,
                            value: f32,
                            unit: &str,
                            dec: fn(&mut Self),
                            inc: fn(&mut Self),
                            view: Entity<Self>| {
            h_flex()
                .id(id)
                .w_full()
                .justify_between()
                .items_center()
                .py_1()
                .child(label.to_string())
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .border_1()
                                .border_color(border)
                                .cursor_pointer()
                                .child("−")
                                .on_mouse_down(MouseButton::Left, {
                                    let view = view.clone();
                                    move |_ev, _w, cx| {
                                        view.update(cx, |this, cx| {
                                            dec(this);
                                            cx.notify();
                                        });
                                    }
                                }),
                        )
                        .child(
                            div()
                                .w(px(72.))
                                .text_align(gpui::TextAlign::Center)
                                .child(format!("{value:.0}{unit}")),
                        )
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .border_1()
                                .border_color(border)
                                .cursor_pointer()
                                .child("+")
                                .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                    view.update(cx, |this, cx| {
                                        inc(this);
                                        cx.notify();
                                    });
                                }),
                        ),
                )
        };

        // Theme picker: one wrap-flow button per registered theme (bundled + user).
        let theme_buttons: Vec<AnyElement> = theme_names
            .iter()
            .map(|name| {
                let selected = *name == active_theme;
                let pick = name.clone();
                let v = view.clone();
                div()
                    .id(SharedString::from(format!("set-theme-{name}")))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(if selected { accent } else { border })
                    .cursor_pointer()
                    .when(selected, |d| d.bg(accent))
                    .when(!selected, |d| d.hover(|h| h.bg(border)))
                    .child(name.clone())
                    .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                        let pick = pick.clone();
                        v.update(cx, |this, cx| {
                            this.set_theme(pick);
                            cx.notify();
                        });
                    })
                    .into_any_element()
            })
            .collect();
        let theme_section = h_flex()
            .gap_2()
            .text_xs()
            .flex_wrap()
            .children(theme_buttons);

        let panel = v_flex()
            .w(px(480.))
            .max_h(px(560.))
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
                    .child(rust_i18n::t!("settings_title").to_string()),
            )
            .child(div().text_xs().text_color(muted).child(rust_i18n::t!("settings_general").to_string()))
            .child(
                h_flex().gap_2().items_center().justify_between()
                    .child(div().text_sm().child(rust_i18n::t!("settings_language").to_string()))
                    .child(
                        h_flex().gap_1()
                            .child({
                                let is_selected = s.language.is_none();
                                div().cursor_pointer().px_2().py_1().rounded_md()
                                    .bg(if is_selected { accent } else { surface })
                                    .text_color(if is_selected { accent_fg } else { fg })
                                    .child("Auto")
                                    .on_mouse_down(MouseButton::Left, {
                                        let view = view.clone();
                                        move |_ev, _w, cx| view.update(cx, |app, cx| {
                                            let mut settings = app.state.settings.clone();
                                            settings.language = None;
                                            crate::util::config::save_settings(&settings);
                                            app.state.settings = settings;
                                            rust_i18n::set_locale("en"); // auto fallback for now
                                            cx.notify();
                                        })
                                    })
                            })
                            .child({
                                let is_selected = s.language.as_deref() == Some("en");
                                div().cursor_pointer().px_2().py_1().rounded_md()
                                    .bg(if is_selected { accent } else { surface })
                                    .text_color(if is_selected { accent_fg } else { fg })
                                    .child("EN")
                                    .on_mouse_down(MouseButton::Left, {
                                        let view = view.clone();
                                        move |_ev, _w, cx| view.update(cx, |app, cx| {
                                            let mut settings = app.state.settings.clone();
                                            settings.language = Some("en".to_string());
                                            crate::util::config::save_settings(&settings);
                                            app.state.settings = settings;
                                            rust_i18n::set_locale("en");
                                            cx.notify();
                                        })
                                    })
                            })
                            .child({
                                let is_selected = s.language.as_deref() == Some("es");
                                div().cursor_pointer().px_2().py_1().rounded_md()
                                    .bg(if is_selected { accent } else { surface })
                                    .text_color(if is_selected { accent_fg } else { fg })
                                    .child("ES")
                                    .on_mouse_down(MouseButton::Left, {
                                        let view = view.clone();
                                        move |_ev, _w, cx| view.update(cx, |app, cx| {
                                            let mut settings = app.state.settings.clone();
                                            settings.language = Some("es".to_string());
                                            crate::util::config::save_settings(&settings);
                                            app.state.settings = settings;
                                            rust_i18n::set_locale("es");
                                            cx.notify();
                                        })
                                    })
                            })
                    )
            )
            .child(
                h_flex().gap_2().items_center().justify_between()
                    .child(div().text_sm().child(rust_i18n::t!("settings_zoom").to_string()))
                    .child(
                        h_flex().gap_1().items_center()
                            .child(
                                div().cursor_pointer().px_2().py_1().rounded_md().bg(surface).text_color(fg)
                                    .child("-")
                                    .on_mouse_down(MouseButton::Left, {
                                        let view = view.clone();
                                        move |_ev, window_cx, cx| {
                                            view.update(cx, |app, cx| {
                                                let mut settings = app.state.settings.clone();
                                                settings.global_zoom = (settings.global_zoom / 1.1).max(0.5);
                                                crate::util::config::save_settings(&settings);
                                                app.state.settings = settings;
                                                cx.notify();
                                            });
                                            let zoom = view.read(cx).state.settings.global_zoom;
                                            window_cx.set_rem_size(zoom * 16.0);
                                        }
                                    })
                            )
                            .child(
                                div().cursor_pointer().px_2().py_1().rounded_md().bg(surface).text_color(fg)
                                    .child(format!("{:.0}%", s.global_zoom * 100.0))
                                    .on_mouse_down(MouseButton::Left, {
                                        let view = view.clone();
                                        move |_ev, window_cx, cx| {
                                            view.update(cx, |app, cx| {
                                                let mut settings = app.state.settings.clone();
                                                settings.global_zoom = 1.0;
                                                crate::util::config::save_settings(&settings);
                                                app.state.settings = settings;
                                                cx.notify();
                                            });
                                            let zoom = view.read(cx).state.settings.global_zoom;
                                            window_cx.set_rem_size(zoom * 16.0);
                                        }
                                    })
                            )
                            .child(
                                div().cursor_pointer().px_2().py_1().rounded_md().bg(surface).text_color(fg)
                                    .child("+")
                                    .on_mouse_down(MouseButton::Left, {
                                        let view = view.clone();
                                        move |_ev, window_cx, cx| {
                                            view.update(cx, |app, cx| {
                                                let mut settings = app.state.settings.clone();
                                                settings.global_zoom = (settings.global_zoom * 1.1).min(3.0);
                                                crate::util::config::save_settings(&settings);
                                                app.state.settings = settings;
                                                cx.notify();
                                            });
                                            let zoom = view.read(cx).state.settings.global_zoom;
                                            window_cx.set_rem_size(zoom * 16.0);
                                        }
                                    })
                            )
                    )
            )
            .child(toggle(
                &rust_i18n::t!("settings_show_hidden"),
                s.show_hidden,
                "set-hidden",
                |app| app.process_actions(vec![Action::ToggleHidden]),
                view.clone(),
            ))
            .child(toggle(
                &rust_i18n::t!("settings_show_sidebar"),
                s.show_sidebar,
                "set-sidebar",
                |app| app.process_actions(vec![Action::ToggleSidebar]),
                view.clone(),
            ))
            .child(toggle(
                &rust_i18n::t!("settings_show_inspector"),
                s.show_inspector,
                "set-inspector",
                |app| app.state.settings.show_inspector = !app.state.settings.show_inspector,
                view.clone(),
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("settings_inspector_placement", placement = placement).to_string()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .child(segment(
                        "set-placement-global",
                        &rust_i18n::t!("settings_placement_global"),
                        inspector_placement == InspectorPlacement::Global,
                        |app| app.state.settings.inspector_placement = InspectorPlacement::Global,
                        view.clone(),
                    ))
                    .child(segment(
                        "set-placement-tab",
                        &rust_i18n::t!("settings_placement_tab"),
                        inspector_placement == InspectorPlacement::ActiveTab,
                        |app| {
                            app.state.settings.inspector_placement = InspectorPlacement::ActiveTab;
                        },
                        view.clone(),
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("settings_default_view", view_mode = view_mode).to_string()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .flex_wrap()
                    .child(segment(
                        "set-view-details",
                        &rust_i18n::t!("settings_view_details_btn"),
                        default_view_mode == ViewMode::Details,
                        |app| app.state.settings.view_mode = ViewMode::Details,
                        view.clone(),
                    ))
                    .child(segment(
                        "set-view-grid",
                        &rust_i18n::t!("settings_view_grid_btn"),
                        default_view_mode == ViewMode::Grid,
                        |app| app.state.settings.view_mode = ViewMode::Grid,
                        view.clone(),
                    ))
                    .child(segment(
                        "set-view-list",
                        &rust_i18n::t!("settings_view_list_btn"),
                        default_view_mode == ViewMode::List,
                        |app| app.state.settings.view_mode = ViewMode::List,
                        view.clone(),
                    ))
                    .child(segment(
                        "set-view-miller",
                        &rust_i18n::t!("settings_view_miller_btn"),
                        default_view_mode == ViewMode::Miller,
                        |app| app.state.settings.view_mode = ViewMode::Miller,
                        view.clone(),
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("settings_theme", theme = active_theme).to_string()),
            )
            .child(theme_section)
            .child(div().text_xs().text_color(muted).child(rust_i18n::t!("settings_sizes").to_string()))
            .child(stepper(
                "set-sidebar-width",
                &rust_i18n::t!("settings_sidebar"),
                sidebar_width,
                " px",
                |app| {
                    app.state.settings.sidebar_width =
                        (app.state.settings.sidebar_width - 20.0).clamp(160.0, 420.0);
                },
                |app| {
                    app.state.settings.sidebar_width =
                        (app.state.settings.sidebar_width + 20.0).clamp(160.0, 420.0);
                },
                view.clone(),
            ))
            .child(stepper(
                "set-inspector-width",
                &rust_i18n::t!("settings_inspector"),
                inspector_width,
                " px",
                |app| {
                    app.state.settings.inspector_width =
                        (app.state.settings.inspector_width - 20.0).clamp(180.0, 520.0);
                },
                |app| {
                    app.state.settings.inspector_width =
                        (app.state.settings.inspector_width + 20.0).clamp(180.0, 520.0);
                },
                view.clone(),
            ))
            .child(stepper(
                "set-font-size",
                &rust_i18n::t!("settings_font"),
                font_size,
                " px",
                |app| {
                    app.state.settings.font_size =
                        (app.state.settings.font_size - 1.0).clamp(10.0, 24.0);
                },
                |app| {
                    app.state.settings.font_size =
                        (app.state.settings.font_size + 1.0).clamp(10.0, 24.0);
                },
                view.clone(),
            ))
            .child(stepper(
                "set-thumbnail-size",
                &rust_i18n::t!("settings_thumbnails"),
                thumbnail_size,
                " px",
                |app| {
                    app.state.settings.thumbnail_size =
                        (app.state.settings.thumbnail_size - 16.0).clamp(64.0, 256.0);
                },
                |app| {
                    app.state.settings.thumbnail_size =
                        (app.state.settings.thumbnail_size + 16.0).clamp(64.0, 256.0);
                },
                view,
            ))
            .child(div().text_xs().text_color(muted).child(rust_i18n::t!("settings_about").to_string()))
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("settings_icons_by").to_string())
                    .child(
                        div()
                            .id("about-icons8-link")
                            .text_color(accent)
                            .cursor_pointer()
                            .child("Icons8")
                            .on_mouse_down(MouseButton::Left, move |_ev, _w, _cx| {
                                let _ = open::that("https://icons8.com");
                            }),
                    ),
            );

        // Settings has no dedicated close action; closing sets the flag directly.
        let view_close = cx.entity();
        let scrim_focus = focus;
        self.overlay_scrim_custom(
            "scrim-settings",
            view_close,
            move |this| this.state.overlay.show_settings = false,
            scrim_focus,
            // FASE I: subtle fade-in for the settings panel.
            panel
                .with_animation(
                    "settings-fade",
                    gpui::Animation::new(std::time::Duration::from_millis(140)),
                    gpui::Styled::opacity,
                )
                .into_any_element(),
            cx,
        )
    }

    // ── batch rename ─────────────────────────────────────────────────────────

    fn render_create_tag(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
                    .child(div().text_xs().text_color(muted).child(rust_i18n::t!("tag_name").to_string()))
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
                                div().text_color(muted).child(rust_i18n::t!("tag_name_placeholder").to_string())
                            } else {
                                div().child(name)
                            }),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_xs().text_color(muted).child(rust_i18n::t!("tag_color").to_string()))
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

    /// Toasts para errores de operaciones de archivo (bottom-right, sin backdrop).
    fn render_op_errors(&mut self, cx: &mut Context<Self>) -> AnyElement {
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

    fn render_batch_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
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
                        "Find",
                        RenameMode::Find,
                        mode == RenameMode::Find,
                        view.clone(),
                    ))
                    .child(mode_button(
                        "Prefix",
                        RenameMode::Prefix,
                        mode == RenameMode::Prefix,
                        view.clone(),
                    ))
                    .child(mode_button(
                        "Suffix",
                        RenameMode::Suffix,
                        mode == RenameMode::Suffix,
                        view.clone(),
                    ))
                    .child(mode_button(
                        "Number",
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
                RenameMode::Prefix => field(&rust_i18n::t!("batch_prefix"), &pat_input).into_any_element(),
                RenameMode::Suffix => {
                    field(&rust_i18n::t!("batch_suffix"), &pat_input).into_any_element()
                }
                RenameMode::Number => field(&rust_i18n::t!("batch_separator"), &rep_input).into_any_element(),
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
                                    this.state.batch_rename = None;
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
                                    if !actions.is_empty() {
                                        this.process_actions(actions);
                                    }
                                    this.state.batch_rename = None;
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
                        this.state.batch_rename = None;
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
                            this.state.batch_rename = None;
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
            |this| this.state.batch_rename = None,
            focus,
            panel.into_any_element(),
            cx,
        )
    }

    // ── rename profile ───────────────────────────────────────────────────────

    /// Open the rename-active-profile dialog using gpui-component's `Dialog`
    /// (mounted by the window `Root`). The dialog owns its overlay, focus,
    /// Esc/overlay-close and the OK/Cancel buttons; we only supply a native text
    /// input and the confirm callback. Called once per open (from `render` when
    /// `overlay.renaming_profile` is latched true).
    pub(crate) fn open_rename_profile_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self.state.profiles.active().name.clone();
        // The input must outlive a single frame, so keep it on `App`.
        let input = crate::ui::text_input::single_line(window, cx, "", name);
        self.rename_profile_input = Some(input.clone());
        window.focus(&input.read(cx).focus_handle(cx), cx);

        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input = input.clone();
            let ok_view = view.clone();
            dialog
                .w(px(360.))
                .title(rust_i18n::t!("profile_rename_title").to_string())
                .button_props(
                    gpui_component::dialog::DialogButtonProps::default()
                        .ok_text(rust_i18n::t!("profile_rename_save").to_string())
                        .cancel_text(rust_i18n::t!("profile_rename_cancel").to_string()),
                )
                .content({
                    let input = input.clone();
                    move |content, _w, _cx| {
                        content.child(crate::ui::text_input::bordered(&input))
                    }
                })
                .on_ok(move |_ev, _window, cx| {
                    let value = input.read(cx).value().to_string();
                    ok_view.update(cx, |app, cx| {
                        app.process_actions(vec![Action::RenameActiveProfile(value.clone())]);
                        app.rename_profile_input = None;
                        cx.notify();
                    });
                    true // close the dialog
                })
        });
    }

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

    /// Full-window backdrop + centered panel, closing via a fixed `Action` on
    /// backdrop click. `top` positions the panel near the top (fuzzy bars) vs center.
    #[allow(clippy::unused_self)] // method form for call-site symmetry with other overlay_* builders
    pub(crate) fn overlay_scrim(
        &mut self,
        view: Entity<Self>,
        close_action: Action,
        _focus: FocusHandle,
        panel: AnyElement,
        top: bool,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let id: SharedString = match &close_action {
            Action::ClosePalette => "scrim-palette".into(),
            Action::ResolveTransferConflict(_) => "scrim-paste".into(),
            _ => "scrim-goto".into(),
        };
        let v = view;
        let scrim = scrim_base(id, top)
            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                let close = close_action.clone();
                v.update(cx, |this, cx| {
                    this.process_actions(vec![close]);
                    cx.notify();
                });
            })
            .child(panel);
        deferred(scrim).into_any_element()
    }

    /// Variant whose close runs an arbitrary mutation (for overlays without a
    /// dedicated close `Action`, e.g. settings / batch rename).
    #[allow(clippy::unused_self)] // method form for call-site symmetry with other overlay_* builders
    fn overlay_scrim_custom(
        &mut self,
        id: &'static str,
        view: Entity<Self>,
        close: impl Fn(&mut Self) + 'static,
        _focus: FocusHandle,
        panel: AnyElement,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let scrim = scrim_base(SharedString::from(id), false)
            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                view.update(cx, |this, cx| {
                    close(this);
                    cx.notify();
                });
            })
            .child(panel);
        deferred(scrim).into_any_element()
    }
}

/// The dimmed, click-catching backdrop that fills the window and centers its child.
fn scrim_base(id: SharedString, top: bool) -> gpui::Stateful<gpui::Div> {
    let mut d = v_flex()
        .id(id)
        .absolute()
        .inset_0()
        .size_full()
        .items_center()
        .bg(scrim_color());
    if top {
        d = d.pt(px(48.));
    } else {
        d = d.justify_center();
    }
    d
}

fn scrim_color() -> Hsla {
    gpui::hsla(0., 0., 0., 0.45)
}
