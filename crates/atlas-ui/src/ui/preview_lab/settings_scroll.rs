//! Focused native regression run: settings themes/Escape and editor wheel input.
use super::{key, mouse_down, mouse_up, select_named, type_text};
use crate::app::App;
use anyhow::{ensure, Context as _};
use gpui::{point, px, size, Entity, Window};
use gpui_component::{ActiveTheme, ThemeRegistry};
use std::{path::PathBuf, time::Duration};

fn present(window: &mut Window, cx: &mut gpui::App) {
    window.refresh();
    window.draw(cx).clear(cx);
}

pub(super) fn install(
    view: &Entity<App>,
    window: &mut Window,
    cx: &mut gpui::App,
    directory: PathBuf,
) {
    rust_i18n::set_locale("en");
    window.set_a11y_forced(true);
    let handle = window.window_handle();
    let view = view.clone();
    let mut themes: Vec<_> = ThemeRegistry::global(cx)
        .themes()
        .keys()
        .filter(|name| crate::app::theme::theme_is_available(name))
        .map(ToString::to_string)
        .collect();
    themes.sort();
    cx.spawn(async move |cx| {
        let result: anyhow::Result<Vec<serde_json::Value>> = async {
            let mut checks = Vec::new();
            for (index, theme) in themes.iter().enumerate() {
                for section in [0usize, 2, 4] {
                    handle.update(cx, |_, window, cx| {
                        window.resize(size(px(if index % 2 == 0 { 980. } else { 1280. }), px(760.)));
                        view.update(cx, |app, cx| {
                            app.state.overlay.show_getting_started = false;
                            app.state.settings.disable_animations = true;
                            app.state.profiles.active_mut().theme = theme.clone();
                            app.theme_dirty = true;
                            app.settings_ui = crate::ui::settings_palette::SettingsUi::opened_at(
                                section.checked_sub(1).and_then(|i| crate::ui::settings_palette::Category::ALL.get(i).copied()),
                            );
                            app.state.overlay.show_settings = true;
                            cx.notify();
                        });
                        present(window, cx);
                    })?;
                    cx.background_executor().timer(Duration::from_millis(180)).await;
                    handle.update(cx, |_, window, cx| -> anyhow::Result<()> {
                        present(window, cx);
                        let screenshot = format!("settings-{}-{section}.png", theme.replace(' ', "-"));
                        window.render_to_image()?.save(directory.join(&screenshot))?;
                        // Interior clicks must never reach the dismissing scrim.
                        let viewport = window.viewport_size();
                        // The footer hint bar is inert; rows activate on click.
                        let panel_h = (600.0_f32).min(f32::from(viewport.height) * 0.82);
                        let top = ((f32::from(viewport.height) - panel_h) * 0.32).max(12.0);
                        let position = point(viewport.width / 2., px(top + panel_h - 18.));
                        mouse_down(position, window, cx);
                        mouse_up(position, window, cx);
                        ensure!(view.read(cx).state.overlay.show_settings, "Interior click dismissed Settings: {theme}");
                        key("escape", window, cx)?;
                        ensure!(!view.read(cx).state.overlay.show_settings, "Escape failed: {theme}");
                        ensure!(view.read(cx).settings_input.is_none(), "Settings retained its input");
                        ensure!(view.read(cx).focus_handle.is_focused(window), "Escape did not restore workspace focus");
                        checks.push(serde_json::json!({"name": format!("{theme}, section {section}: interior click, Escape, focus"), "status": "passed", "screenshot": screenshot}));
                        Ok(())
                    })??;
                }
            }
            handle.update(cx, |_, window, cx| -> anyhow::Result<()> {
                view.update(cx, |app, cx| {
                    app.state.overlay.show_settings = true;
                    cx.notify();
                });
                present(window, cx);
                type_text("scroll", window, cx)?;
                ensure!(view.read(cx).settings_input.as_ref().unwrap().read(cx).value() == "scroll", "Settings search did not receive text");
                present(window, cx);
                key("escape", window, cx)?;
                ensure!(!view.read(cx).state.overlay.show_settings, "Escape failed with a search query");
                checks.push(serde_json::json!({"name": "Escape dismisses a filtered Settings search in one press", "status": "passed"}));
                ensure!(select_named(&view, "sample.rs", cx), "Missing source fixture");
                view.update(cx, |app, cx| {
                    app.state.settings.show_inspector = true;
                    app.state.settings.scroll_smoothness = 0.6;
                    let tab = app.state.focused_tile;
                    let path = app.state.tab_states[&tab].entries.iter().find(|entry| entry.name == "sample.rs").unwrap().path();
                    app.load_preview_gpui(tab, path);
                    cx.notify();
                });
                present(window, cx);
                Ok(())
            })??;
            cx.background_executor().timer(Duration::from_millis(700)).await;
            let (editor, position, line_height) = handle.update(cx, |_, window, cx| -> anyhow::Result<_> {
                present(window, cx);
                let editor = view.read(cx).preview_editor.as_ref().context("No preview editor")?.state.clone();
                let state = editor.read(cx);
                ensure!(state.value().lines().count() >= 300, "Wrong code fixture");
                let bounds = state.range_to_bounds(&(0..1)).context("Code preview has no text geometry")?;
                let position = point(bounds.left() + px(40.), bounds.top() + px(8.));
                let line_height = state.line_height().context("No line height")?;
                wheel(position, gpui::ScrollDelta::Lines(point(0., -3.)), window, cx);
                let initial = editor.read(cx).scroll_offset().y;
                let target = line_height * 3. * 1.21;
                ensure!(-initial < target * 0.9, "Wheel jumped instantly instead of easing");
                Ok((editor, position, line_height))
            })??;
            // Present actual intermediate frames: the editor must retain its wheel
            // target through text layout, cursor reveal and scrollbar painting.
            let mut offsets = Vec::new();
            for _ in 0..12 {
                cx.background_executor().timer(Duration::from_millis(20)).await;
                offsets.push(handle.update(cx, |_, window, cx| {
                    present(window, cx);
                    f32::from(editor.read(cx).scroll_offset().y)
                })?);
            }
            handle.update(cx, |_, window, cx| -> anyhow::Result<()> {
                let expected = -f32::from(line_height) * 3. * 1.21;
                let settled = f32::from(editor.read(cx).scroll_offset().y);
                ensure!((settled - expected).abs() < 1., "Glide stopped early: {settled}, expected {expected}; frames {offsets:?}");
                ensure!(offsets.windows(2).all(|pair| pair[1] <= pair[0]), "Glide changed direction");
                checks.push(serde_json::json!({"name": "Code preview wheel uses global easing and boost through consecutive frames", "status": "passed", "offsets": offsets}));
                wheel(position, gpui::ScrollDelta::Pixels(point(px(0.), px(12.))), window, cx);
                let precise = f32::from(editor.read(cx).scroll_offset().y);
                ensure!((precise - settled - 12.).abs() < 0.1, "Pixel input was smoothed or boosted");
                view.update(cx, |app, cx| { app.state.settings.scroll_smoothness = 0.; cx.notify(); });
                present(window, cx);
                wheel(position, gpui::ScrollDelta::Lines(point(0., -2.)), window, cx);
                let instant = f32::from(editor.read(cx).scroll_offset().y);
                ensure!((instant - precise + f32::from(line_height) * 2.).abs() < 0.1, "Disabling smooth scroll did not apply immediately");
                wheel(position, gpui::ScrollDelta::Lines(point(0., -10000.)), window, cx);
                let bottom = editor.read(cx).scroll_offset().y;
                present(window, cx);
                wheel(position, gpui::ScrollDelta::Lines(point(0., -10000.)), window, cx);
                ensure!((editor.read(cx).scroll_offset().y - bottom).abs() < px(1.), "Bottom boundary drifts");
                wheel(position, gpui::ScrollDelta::Lines(point(0., 10000.)), window, cx);
                ensure!(editor.read(cx).scroll_offset().y == px(0.), "Top boundary overshoots");
                checks.push(serde_json::json!({"name": "Pixel input, live smoothness-off setting and both code scroll boundaries", "status": "passed"}));
                Ok(())
            })??;
            Ok(checks)
        }.await;
        match result {
            Ok(checks) => {
                let _ = std::fs::write(directory.join("tests.json"), serde_json::to_vec_pretty(&checks).unwrap());
                let _ = std::fs::write(directory.join("passed.txt"), format!("{} checks across {} themes", checks.len(), themes.len()));
            }
            Err(error) => {
                let _ = std::fs::write(directory.join("failed.txt"), format!("{error:#}"));
                let _ = handle.update(cx, |_, window, _| {
                    let _ = window.render_to_image().map(|image| image.save(directory.join("failed.png")));
                });
            }
        }
        let _ = cx.update(|cx| cx.quit());
    }).detach();
}

fn wheel(
    position: gpui::Point<gpui::Pixels>,
    delta: gpui::ScrollDelta,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    window.simulate_mouse_move(position, cx);
    window.dispatch_event(
        gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
            position,
            delta,
            ..Default::default()
        }),
        cx,
    );
}
