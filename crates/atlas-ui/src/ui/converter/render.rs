use super::{wheel, Phase};
use crate::app::App;
use crate::ui::icons::{icon, AtlasIcon};
use gpui::prelude::*;
use gpui::ColorExt as _;
use gpui::{
    deferred, div, hsla, px, AnyElement, Context, KeyDownEvent, MouseButton, SharedString, Window,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex, ActiveTheme};

fn text(key: &str) -> String {
    rust_i18n::t!(key).into_owned()
}

fn button(
    id: &'static str,
    label: String,
    cx: &mut Context<App>,
    action: impl Fn(&mut App) + 'static,
) -> AnyElement {
    let fg = cx.theme().foreground;
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .text_color(fg)
        .text_sm()
        .hover(move |s| s.bg(fg.opacity(0.08)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |app, _, _, cx| {
                action(app);
                cx.stop_propagation();
                cx.notify();
            }),
        )
        .child(label)
        .into_any_element()
}

impl App {
    pub(crate) fn handle_converter_key(&mut self, event: &KeyDownEvent) -> bool {
        let Some(session) = self.converter.as_ref().filter(|s| s.visible()) else {
            return false;
        };
        let choosing = matches!(session.phase, Phase::Choosing);
        let count = session.options().len();
        match event.keystroke.key.as_str() {
            "escape" => self.dismiss_converter(),
            "enter" if choosing => self.start_conversion(),
            "left" | "up" | "right" | "down" | "tab" | "pageup" | "pagedown"
                if choosing && count > 0 =>
            {
                let session = self.converter.as_mut().unwrap();
                let delta: isize = match event.keystroke.key.as_str() {
                    "left" | "up" => -1,
                    "pageup" => -(wheel::PAGE_SIZE as isize),
                    "pagedown" => wheel::PAGE_SIZE as isize,
                    "tab" if event.keystroke.modifiers.shift => -1,
                    _ => 1,
                };
                session.selected =
                    (session.selected as isize + delta).rem_euclid(count as isize) as usize;
            }
            "1" | "2" | "3" | "4" if choosing => {
                let index = event.keystroke.key.parse::<usize>().unwrap_or(1) - 1;
                if let Some(session) = self.converter.as_mut() {
                    let target = session.selected / wheel::PAGE_SIZE * wheel::PAGE_SIZE + index;
                    if target < count {
                        session.selected = target;
                    }
                }
            }
            _ => {}
        }
        true
    }

    pub(crate) fn render_converter(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(session) = self.converter.as_ref() else {
            return div().into_any_element();
        };
        let fg = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let surface = cx.theme().background;
        let dark = cx.theme().is_dark();
        let running = session.running();
        let done = match &session.phase {
            Phase::Running { done, total, .. } => format!("{done} / {total}"),
            Phase::Finished(report) => {
                text("convert_done_count").replace("%{count}", &report.outputs().len().to_string())
            }
            _ => text("convert_title"),
        };
        if session.minimized {
            return deferred(
                h_flex()
                    .absolute()
                    .right(px(24.))
                    .bottom(px(22.))
                    .gap_3()
                    .items_center()
                    .p_2()
                    .rounded_lg()
                    .bg(cx.theme().popover)
                    .shadow_lg()
                    .occlude()
                    .child(icon(AtlasIcon::ArrowClockwise, 16., muted))
                    .child(div().text_sm().child(done))
                    .child(button("convert-reopen", text("convert_show"), cx, |app| {
                        if let Some(s) = &mut app.converter {
                            s.minimized = false;
                        }
                    })),
            )
            .into_any_element();
        }
        let expanded = session.expanded;
        let viewport = window.viewport_size();
        let available_height = f32::from(viewport.height) - 265.;
        let diameter = available_height
            .min(f32::from(viewport.width) - 64.)
            .clamp(190., 360.);
        let filename = session
            .sources
            .first()
            .and_then(|p| p.file_name())
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let filename = if session.sources.len() > 1 {
            format!("{filename}  +{}", session.sources.len() - 1)
        } else {
            filename
        };
        let mut panel = v_flex()
            .w(px((diameter + 64.).max(350.)))
            .max_w_full()
            .gap_3()
            .items_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        panel = panel.child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_lg()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(text("convert_title")),
                )
                .child(button(
                    "convert-dismiss",
                    text(if running {
                        "convert_minimize"
                    } else {
                        "convert_close"
                    }),
                    cx,
                    App::dismiss_converter,
                )),
        );
        panel = panel.child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .text_color(muted)
                .child(icon(AtlasIcon::File, 15., muted))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .child(SharedString::from(filename)),
                ),
        );

        match &self.converter.as_ref().unwrap().phase {
            Phase::Discovering => {
                panel = panel.child(
                    v_flex()
                        .h(px(diameter))
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .child(crate::ui::loading_spinner::loading_spinner(
                            muted,
                            !self.state.settings.disable_animations,
                        ))
                        .child(text("convert_discovering")),
                );
            }
            Phase::Choosing => {
                panel = panel.child(self.render_format_wheel(diameter, dark, cx));
                let session = self.converter.as_ref().unwrap();
                let count = session.options().len();
                if count > 0 {
                    let page = session.selected / wheel::PAGE_SIZE;
                    let pages = count.div_ceil(wheel::PAGE_SIZE);
                    panel = panel.child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(button(
                                "convert-prev",
                                text("convert_previous"),
                                cx,
                                |app| app.converter_page(-1),
                            ))
                            .child(div().text_sm().text_color(muted).child(format!(
                                "{} / {}",
                                page + 1,
                                pages
                            )))
                            .child(button("convert-next", text("convert_next"), cx, |app| {
                                app.converter_page(1)
                            })),
                    );
                    panel = panel.child(self.render_conversion_detail(cx));
                    panel = panel.child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(text("convert_keep_originals")),
                            )
                            .child(button(
                                "convert-options",
                                text("convert_options"),
                                cx,
                                |app| {
                                    if let Some(s) = &mut app.converter {
                                        s.expanded = !s.expanded;
                                    }
                                },
                            )),
                    );
                    panel = panel.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(text("convert_keys")),
                    );
                }
            }
            Phase::Running {
                done,
                total,
                current,
                cancel,
            } => {
                let fraction = *done as f32 / (*total).max(1) as f32;
                let cancelling = cancel.load(std::sync::atomic::Ordering::Relaxed);
                panel = panel.child(
                    v_flex()
                        .w_full()
                        .h(px(diameter))
                        .items_center()
                        .justify_center()
                        .gap_4()
                        .child(crate::ui::loading_spinner::loading_spinner(
                            muted,
                            !self.state.settings.disable_animations,
                        ))
                        .child(div().text_lg().child(text(if cancelling {
                            "convert_cancelling"
                        } else {
                            "convert_running"
                        })))
                        .child(
                            div()
                                .text_3xl()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(format!("{done} / {total}")),
                        )
                        .child(
                            div()
                                .w_full()
                                .h(px(3.))
                                .rounded_full()
                                .bg(fg.opacity(0.10))
                                .child(
                                    div()
                                        .h_full()
                                        .w(px((diameter + 64.) * fraction))
                                        .rounded_full()
                                        .bg(cx.theme().accent),
                                ),
                        )
                        .child(
                            div().w_full().truncate().text_sm().text_color(muted).child(
                                current
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                            ),
                        )
                        .child(button(
                            "convert-cancel",
                            text("convert_cancel"),
                            cx,
                            App::cancel_conversion,
                        )),
                );
                panel = panel.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(text("convert_background")),
                );
            }
            Phase::Finished(report) => {
                let outputs = report.outputs();
                let errors = report.errors();
                let cancelled = report.cancelled();
                let summary = rust_i18n::t!(
                    "convert_summary",
                    created = outputs.len(),
                    failed = errors.len(),
                    cancelled = cancelled
                )
                .into_owned();
                let results = report.outcomes.clone();
                let mut rows = v_flex().gap_2().w_full();
                for (index, outcome) in results.iter().enumerate() {
                    let (label, error) = match outcome {
                        atlas_core::convert::Outcome::Created { output, .. } => {
                            (output.display().to_string(), false)
                        }
                        atlas_core::convert::Outcome::Failed { source, error } => (
                            format!(
                                "{}: {error}",
                                source.file_name().unwrap_or_default().to_string_lossy()
                            ),
                            true,
                        ),
                        atlas_core::convert::Outcome::Cancelled { source } => (
                            format!(
                                "{} · {}",
                                source.file_name().unwrap_or_default().to_string_lossy(),
                                text("convert_cancelled")
                            ),
                            false,
                        ),
                    };
                    rows = rows.child(
                        div()
                            .id(SharedString::from(format!("convert-result-{index}")))
                            .text_sm()
                            .text_color(if error { cx.theme().danger } else { muted })
                            .child(label),
                    );
                }
                panel = panel.child(
                    v_flex()
                        .w_full()
                        .min_h(px(diameter * 0.65))
                        .gap_4()
                        .justify_center()
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(text(if errors.is_empty() && cancelled == 0 {
                                    "convert_finished"
                                } else {
                                    "convert_review"
                                })),
                        )
                        .child(div().text_sm().text_color(muted).child(summary))
                        .child(
                            div()
                                .id("convert-results")
                                .w_full()
                                .max_h(px(170.))
                                .overflow_y_scrollbar()
                                .child(rows),
                        ),
                );
                if let Some(folder) = outputs
                    .first()
                    .and_then(|p| p.parent())
                    .map(std::path::Path::to_path_buf)
                {
                    panel = panel.child(button(
                        "convert-reveal",
                        text("convert_reveal"),
                        cx,
                        move |app| {
                            let folder = folder.clone();
                            app.rt.spawn_blocking(move || {
                                let _ = open::that(folder);
                            });
                        },
                    ));
                }
                panel = panel.child(button("convert-again", text("convert_again"), cx, |app| {
                    if let Some(s) = &mut app.converter {
                        s.phase = Phase::Choosing;
                    }
                }));
            }
        }
        let content = if expanded
            && self
                .converter
                .as_ref()
                .is_some_and(|s| matches!(s.phase, Phase::Choosing))
        {
            h_flex()
                .gap_6()
                .items_center()
                .child(panel)
                .child(
                    v_flex()
                        .w(px(260.))
                        .gap_3()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(text("convert_options")),
                        )
                        .child(self.render_conversion_settings(cx)),
                )
                .into_any_element()
        } else {
            panel.into_any_element()
        };
        let focus = self.overlay_focus(cx);
        let scrim = div()
            .id("convert-layer")
            .absolute()
            .inset_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(gpui::Hsla {
                alpha: 0.96,
                ..surface
            })
            .text_color(fg)
            .occlude()
            .track_focus(&focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|app, _, _, cx| {
                    app.dismiss_converter();
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .capture_key_down(cx.listener(|app, event, _, cx| {
                if app.handle_converter_key(event) {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(content);
        deferred(scrim).into_any_element()
    }

    fn converter_page(&mut self, delta: isize) {
        let Some(session) = &mut self.converter else {
            return;
        };
        let count = session.options().len();
        let pages = count.div_ceil(wheel::PAGE_SIZE).max(1);
        let page = (session.selected as isize / wheel::PAGE_SIZE as isize + delta)
            .rem_euclid(pages as isize) as usize;
        session.selected = (page * wheel::PAGE_SIZE).min(count.saturating_sub(1));
    }

    fn render_format_wheel(
        &mut self,
        diameter: f32,
        dark: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session = self.converter.as_ref().unwrap();
        let options = session.options();
        if options.is_empty() {
            return v_flex()
                .h(px(diameter * 0.7))
                .justify_center()
                .gap_3()
                .child(div().text_lg().child(text("convert_no_formats")))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(text("convert_no_formats_hint")),
                )
                .child(button(
                    "convert-zip-selection",
                    text("convert_pack_zip"),
                    cx,
                    |app| {
                        let paths = app
                            .converter
                            .as_ref()
                            .map(|s| s.sources.clone())
                            .unwrap_or_default();
                        app.dismiss_converter();
                        app.process_actions(vec![crate::input::action::Action::CompressToZip(
                            paths,
                        )]);
                    },
                ))
                .into_any_element();
        }
        let selected = session.selected.min(options.len() - 1);
        let page = selected / wheel::PAGE_SIZE;
        let visible = options
            .iter()
            .skip(page * wheel::PAGE_SIZE)
            .take(wheel::PAGE_SIZE)
            .collect::<Vec<_>>();
        let count = visible.len();
        let accent = hsla(0.055, 0.84, if dark { 0.47 } else { 0.45 }, 1.0);
        let fill = hsla(
            0.075,
            if dark { 0.24 } else { 0.86 },
            if dark { 0.20 } else { 0.88 },
            1.0,
        );
        let disabled = hsla(
            0.075,
            if dark { 0.12 } else { 0.36 },
            if dark { 0.15 } else { 0.92 },
            1.0,
        );
        let ink = if dark {
            hsla(0.08, 0.35, 0.91, 1.)
        } else {
            hsla(0.045, 0.54, 0.14, 1.)
        };
        let mut ring = div()
            .relative()
            .size(px(diameter))
            .rounded_full()
            .bg(hsla(0.075, 0.38, if dark { 0.12 } else { 0.78 }, 1.))
            .child(wheel::paint_wheel(
                diameter,
                selected,
                visible.iter().map(|o| o.available()).collect(),
                (fill, accent, disabled),
                cx,
            ));
        for (index, option) in visible.iter().enumerate() {
            let angle = wheel::angle(index, count);
            let radius = diameter * 0.345;
            let x = diameter * 0.5 + radius * angle.cos();
            let y = diameter * 0.5 + radius * angle.sin();
            let chosen = selected == page * wheel::PAGE_SIZE + index;
            ring = ring.child(
                div()
                    .absolute()
                    .left(px(x - diameter * 0.155))
                    .top(px(y - 13.))
                    .w(px(diameter * 0.31))
                    .text_center()
                    .text_size(px(if option.target.label.len() > 6 {
                        15.
                    } else {
                        20.
                    }))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(if chosen {
                        gpui::white()
                    } else {
                        ink.opacity(if option.available() { 1. } else { 0.55 })
                    })
                    .child(option.target.label),
            );
        }
        let label = options[selected].target.label;
        let enabled = options[selected].available();
        ring.child(
            v_flex()
                .absolute()
                .left(px(diameter * 0.30))
                .top(px(diameter * 0.39))
                .w(px(diameter * 0.40))
                .h(px(diameter * 0.22))
                .items_center()
                .justify_center()
                .gap_1()
                .child(
                    div()
                        .id("convert-apply")
                        .w_full()
                        .py_3()
                        .rounded_full()
                        .bg(fill)
                        .text_center()
                        .text_color(ink)
                        .text_lg()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .when(enabled, |s| {
                            s.cursor_pointer()
                                .hover(move |s| s.bg(accent).text_color(gpui::white()))
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |app, _, _, cx| {
                                if enabled {
                                    app.start_conversion();
                                }
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        )
                        .child(label),
                )
                .child(div().text_xs().text_color(ink).child(text(if enabled {
                    "convert_apply"
                } else {
                    "convert_engine_needed"
                }))),
        )
        .into_any_element()
    }

    fn render_conversion_detail(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.converter.as_ref().unwrap();
        let options = session.options();
        let Some(option) = options.get(session.selected) else {
            return div().into_any_element();
        };
        let mut detail = v_flex()
            .w_full()
            .gap_1()
            .min_h(px(40.))
            .items_center()
            .text_sm()
            .text_color(cx.theme().muted_foreground);
        if option.available() {
            detail = detail.child(format!(
                "{} · {}",
                option.target.label,
                option.engines.join(" + ")
            ));
            let extension = option.target.extension;
            let has_source = |extensions: &[&str]| {
                session.sources.iter().any(|p| {
                    p.extension()
                        .is_some_and(|e| extensions.iter().any(|s| e.eq_ignore_ascii_case(s)))
                })
            };
            let note = if option.engines.contains(&"Windows") {
                Some("convert_windows_audio_note")
            } else if has_source(&["srt", "vtt", "ass", "ssa", "lrc", "smi", "ttml", "dfxp"]) {
                Some(if extension == "lrc" || extension == "smi" {
                    "convert_subtitle_linear_note"
                } else {
                    "convert_subtitle_note"
                })
            } else if extension == "vcf" || has_source(&["vcf"]) {
                Some("convert_contacts_note")
            } else if option.engines.contains(&"Atlas")
                && (["epub", "fb2"].contains(&extension) || has_source(&["epub", "fb2"]))
            {
                Some("convert_book_note")
            } else if has_source(&["wav", "wave", "aiff", "aif", "aifc", "au", "snd"])
                && option.engines.contains(&"Atlas")
            {
                Some("convert_pcm_note")
            } else if has_source(&["svg", "svgz"]) && extension == "pdf" {
                Some("convert_svg_pdf_note")
            } else if ["gif", "tiff", "pdf"].iter().any(|ext| {
                session
                    .sources
                    .iter()
                    .any(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
            }) && ["png", "jpg", "webp", "bmp", "ico", "avif", "heic", "jxl"]
                .contains(&extension)
            {
                Some("convert_first_frame")
            } else if ["csv", "tsv"].contains(&extension)
                && session.sources.iter().any(|p| {
                    p.extension().is_some_and(|e| {
                        ["xlsx", "xlsm", "xls", "xlsb", "ods"]
                            .iter()
                            .any(|s| e.eq_ignore_ascii_case(s))
                    })
                })
            {
                Some("convert_first_sheet")
            } else if extension == "jpg" {
                Some("convert_jpeg_note")
            } else if extension == "webp" && option.engines.contains(&"Atlas") {
                Some("convert_webp_note")
            } else if ["docx", "txt", "md", "html"].contains(&extension)
                && session
                    .sources
                    .iter()
                    .any(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")))
            {
                Some("convert_pdf_text_note")
            } else if option.engines.contains(&"Atlas")
                && session.sources.iter().any(|p| {
                    p.extension().is_some_and(|e| {
                        ["docx", "docm", "dotx"]
                            .iter()
                            .any(|s| e.eq_ignore_ascii_case(s))
                    })
                })
            {
                Some("convert_doc_text_note")
            } else if ["csv", "tsv", "toml"].contains(&extension) {
                Some("convert_data_note")
            } else {
                None
            };
            if let Some(note) = note {
                detail = detail.child(div().text_xs().text_center().child(text(note)));
            }
        } else {
            let names = option
                .missing
                .iter()
                .map(|t| t.name())
                .collect::<Vec<_>>()
                .join(" + ");
            detail = detail.child(rust_i18n::t!("convert_needs", engine = names).into_owned());
            for tool in &option.missing {
                let url = tool.url();
                detail = detail.child(button(
                    tool.executable(),
                    rust_i18n::t!("convert_get_engine", engine = tool.name()).into_owned(),
                    cx,
                    move |app| {
                        app.rt.spawn_blocking(move || {
                            let _ = open::that(url);
                        });
                    },
                ));
            }
            detail = detail.child(button(
                "convert-rescan",
                text("convert_rescan"),
                cx,
                App::open_converter,
            ));
        }
        detail.into_any_element()
    }

    fn render_conversion_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.converter.as_ref().unwrap();
        let quality = session.settings.quality;
        let edge = session.settings.max_edge;
        let destination = session
            .settings
            .output_dir
            .as_ref()
            .map_or_else(|| text("convert_beside"), |p| p.display().to_string());
        v_flex()
            .w_full()
            .gap_1()
            .text_sm()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(text("convert_quality")),
                    )
                    .child(button(
                        "convert-quality",
                        format!("{quality}%"),
                        cx,
                        |app| {
                            if let Some(s) = &mut app.converter {
                                s.settings.quality = match s.settings.quality {
                                    90 => 75,
                                    75 => 100,
                                    _ => 90,
                                };
                            }
                        },
                    )),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(text("convert_dimensions")),
                    )
                    .child(button(
                        "convert-size",
                        edge.map_or_else(|| text("convert_original"), |n| format!("{n} px")),
                        cx,
                        |app| {
                            if let Some(s) = &mut app.converter {
                                s.settings.max_edge = match s.settings.max_edge {
                                    None => Some(1920),
                                    Some(1920) => Some(1280),
                                    _ => None,
                                };
                            }
                        },
                    )),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(cx.theme().muted_foreground)
                            .child(destination),
                    )
                    .child(button(
                        "convert-folder",
                        text("convert_folder"),
                        cx,
                        App::choose_conversion_directory,
                    ))
                    .child(button(
                        "convert-reset-folder",
                        text("convert_reset"),
                        cx,
                        |app| {
                            if let Some(s) = &mut app.converter {
                                s.settings.output_dir = None;
                            }
                        },
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(text("convert_options_note")),
            )
            .into_any_element()
    }
}
