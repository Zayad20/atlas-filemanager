//! Uses the real App, event dispatch and workers in a hidden native window.
use super::Phase;
use crate::{app::App, input::action::Action};
use anyhow::{ensure, Context as _};
use atlas_model::{preview_cache::PreviewResult, settings::ViewMode};
use gpui::{point, px, Entity, Modifiers, ScrollDelta, Window};
use std::{path::PathBuf, time::Duration};

const TEXT: &str = "café, español y 日本語 🧑🏽‍💻\nUna conversión con Unicode → funciona.\n";

pub(crate) fn install(
    view: &Entity<App>,
    window: &mut Window,
    cx: &mut gpui::App,
    directory: PathBuf,
) {
    let _ = std::fs::remove_file(directory.join("passed.json"));
    let _ = std::fs::remove_file(directory.join("failed.txt"));
    let fixtures = directory.join("fixtures");
    std::fs::write(fixtures.join("poema.txt"), TEXT).unwrap();
    let handle = window.window_handle();
    let view = view.clone();
    view.update(cx, |app, cx| {
        app.state.overlay.show_getting_started = false;
        app.state.settings.disable_animations = true;
        app.state.settings.show_inspector = true;
        let tile = app.state.focused_tile;
        app.navigate(tile, fixtures.clone());
        app.process_actions([Action::SetViewMode(ViewMode::Miller)]);
        cx.notify();
    });
    cx.spawn(async move |cx| {
        let result: anyhow::Result<Vec<String>> = async {
            let mut checks = Vec::new();
            let mut ready = false;
            for _ in 0..150 {
                cx.background_executor().timer(Duration::from_millis(20)).await;
                ready = handle.update(cx, |_, window, cx| {
                    window.refresh(); window.draw(cx).clear(cx);
                    let app = view.read(cx);
                    let tile = app.state.focused_tile;
                    app.miller_column_bounds.contains_key(&(tile, 0)) && app.state.tab_states.get(&tile)
                        .and_then(|tab| tab.miller.columns.first())
                        .is_some_and(|col| col.entries.iter().any(|entry| entry.name == "poema.txt"))
                })?;
                if ready { break; }
            }
            ensure!(ready, "Miller never loaded");
            handle.update(cx, |_, window, cx| -> anyhow::Result<()> {
                let tile = view.read(cx).state.focused_tile;
                let bounds = view.read(cx).miller_column_bounds[&(tile, 0)];
                let position = point(bounds.left() + px(90.), bounds.top() + px(80.));
                window.simulate_mouse_move(position, cx);
                for platform in [false, true] {
                    for delta in [ScrollDelta::Lines(point(0., 1.)), ScrollDelta::Lines(point(0., -1.)),
                        ScrollDelta::Pixels(point(px(0.), px(20.))), ScrollDelta::PreciseLines(point(0., -0.5))] {
                        let dispatched = window.dispatch_event(gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
                            position, delta, modifiers: Modifiers { control: !platform, platform, ..Default::default() },
                            ..Default::default()
                        }), cx);
                        ensure!(dispatched.default_prevented && !dispatched.propagate, "Miller did not consume the modified wheel event");
                        window.refresh(); window.draw(cx).clear(cx);
                        ensure!(view.read(cx).tab_data_for(tile).is_some_and(|tab| tab.view_mode == ViewMode::Miller), "Wheel changed Miller's view");
                    }
                }
                window.render_to_image()?.save(directory.join("miller-ctrl-scroll.png"))?;
                view.update(cx, |app, cx| {
                    let col = &mut app.state.tab_states.get_mut(&tile).unwrap().miller.columns[0];
                    let idx = col.entries.iter().position(|entry| entry.name == "poema.txt").unwrap();
                    col.selection.keyboard_cursor = Some(idx);
                    col.selection.selected = [idx].into_iter().collect();
                    app.load_preview_gpui(tile, fixtures.join("poema.txt"));
                    cx.notify();
                });
                Ok(())
            })??;
            checks.push("Miller: 8 real Ctrl/Platform wheel events, line/pixel/precise deltas, no entity reentry".into());
            ready = false;
            for _ in 0..150 {
                cx.background_executor().timer(Duration::from_millis(20)).await;
                ready = handle.update(cx, |_, window, cx| {
                    window.refresh(); window.draw(cx).clear(cx);
                    let app = view.read(cx);
                    matches!(app.state.preview.results.get(&app.state.focused_tile), Some(PreviewResult::Text { text, .. }) if text == TEXT)
                })?;
                if ready { break; }
            }
            ensure!(ready, "Unicode preview failed");
            checks.push("Native text preview preserves accents, combining marks, CJK and emoji".into());
            let mut source = fixtures.join("poema.txt");
            for (case, target) in [("txt-docx", "docx"), ("docx-txt", "txt"), ("txt-html", "html"), ("png-jpg", "jpg"), ("png-webp", "webp")] {
                if case == "png-jpg" || case == "png-webp" { source = fixtures.join("alpha-preview.png"); }
                let output_dir = directory.join(case);
                std::fs::create_dir_all(&output_dir)?;
                handle.update(cx, |_, _, cx| {
                    view.update(cx, |app, cx| { app.open_converter_for(vec![source.clone()]); cx.notify(); });
                })?;
                ready = false;
                for _ in 0..250 {
                    cx.background_executor().timer(Duration::from_millis(20)).await;
                    ready = handle.update(cx, |_, window, cx| {
                        window.refresh(); window.draw(cx).clear(cx);
                        view.read(cx).converter.as_ref().is_some_and(|session| matches!(session.phase, Phase::Choosing))
                    })?;
                    if ready { break; }
                }
                ensure!(ready, "{case}: format discovery timed out");
                handle.update(cx, |_, window, cx| -> anyhow::Result<()> {
                    view.update(cx, |app, cx| -> anyhow::Result<()> {
                        let session = app.converter.as_mut().context("converter missing")?;
                        session.selected = session.options().iter().position(|option| option.target.extension == target && option.available()).context(format!("{case}: native target missing"))?;
                        session.settings.output_dir = Some(output_dir.clone());
                        cx.notify(); Ok(())
                    })?;
                    window.refresh(); window.draw(cx).clear(cx);
                    view.update(cx, |app, cx| { app.start_conversion(); cx.notify(); });
                    Ok(())
                })??;
                ready = false;
                for _ in 0..250 {
                    cx.background_executor().timer(Duration::from_millis(20)).await;
                    ready = handle.update(cx, |_, window, cx| -> anyhow::Result<bool> {
                        window.refresh(); window.draw(cx).clear(cx);
                        let app = view.read(cx);
                        if let Some(session) = &app.converter {
                            if let Phase::Finished(report) = &session.phase { anyhow::bail!("{case}: {:?}", report.errors()); }
                            Ok(false)
                        } else { Ok(true) }
                    })??;
                    if ready { break; }
                }
                ensure!(ready, "{case}: conversion timed out");
                let output = std::fs::read_dir(&output_dir)?.filter_map(Result::ok).map(|entry| entry.path())
                    .find(|path| path.extension().is_some_and(|extension| extension == target)).context(format!("{case}: no output"))?;
                if target == "txt" { ensure!(std::fs::read_to_string(&output)?.contains("café, español y 日本語"), "Converted text lost Unicode"); }
                checks.push(format!("UI conversion {case}: completed, output {} bytes", std::fs::metadata(&output)?.len()));
                source = output;
            }
            ensure!(std::fs::read_to_string(fixtures.join("poema.txt"))? == TEXT, "Source changed");
            Ok(checks)
        }.await;
        match result {
            Ok(checks) => { let _ = std::fs::write(directory.join("passed.json"), serde_json::to_vec_pretty(&checks).unwrap()); }
            Err(error) => { let _ = std::fs::write(directory.join("failed.txt"), format!("{error:#}")); }
        }
        drop(view);
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        let _ = cx.update(|cx| cx.quit());
    }).detach();
}
