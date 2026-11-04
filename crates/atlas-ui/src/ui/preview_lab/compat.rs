//! Native format matrix, Miller viewport/scroll QA and Open with keyboard checks.
use crate::{
    app::{ext::ExtAction, App},
    input::action::Action,
};
use anyhow::{ensure, Context as _};
use atlas_model::{preview_cache::PreviewResult, settings::ViewMode};
use gpui::{point, px, size, AnyWindowHandle, AsyncApp, Entity, Keystroke, Window};
use std::{path::PathBuf, time::Duration};

async fn wait_for(
    view: &Entity<App>,
    handle: AnyWindowHandle,
    cx: &mut AsyncApp,
    condition: impl Fn(&App) -> bool,
) -> anyhow::Result<()> {
    for _ in 0..300 {
        cx.background_executor()
            .timer(Duration::from_millis(20))
            .await;
        if handle.update(cx, |_, window, cx| {
            window.simulate_next_frame(cx);
            window.refresh();
            window.draw(cx).clear(cx);
            condition(view.read(cx))
        })? {
            return Ok(());
        }
    }
    anyhow::bail!("Native state timed out")
}

fn kind(result: &PreviewResult) -> &'static str {
    match result {
        PreviewResult::Text { .. } => "text",
        PreviewResult::Markdown { .. } => "markdown",
        PreviewResult::Document { .. } => "document",
        PreviewResult::Table { .. } => "table",
        PreviewResult::Presentation { .. } => "slides",
        PreviewResult::Image { .. } => "image",
        PreviewResult::Dir { .. } => "directory",
        PreviewResult::Error(_) => "error",
        PreviewResult::Binary { .. } => "binary",
        _ => "other",
    }
}

pub(super) fn install(
    view: &Entity<App>,
    window: &mut Window,
    cx: &mut gpui::App,
    directory: PathBuf,
) {
    for name in ["passed.json", "failed.txt"] {
        let _ = std::fs::remove_file(directory.join(name));
    }
    rust_i18n::set_locale("es");
    let fixtures = directory.join("fixtures");
    view.update(cx, |app, cx| {
        app.state.overlay.show_getting_started = false;
        app.state.settings.disable_animations = true;
        app.state.settings.show_inspector = true;
        let tile = app.state.focused_tile;
        app.navigate(tile, fixtures.clone());
        app.process_actions([Action::SetViewMode(ViewMode::Details)]);
        cx.notify();
    });
    let view = view.clone();
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let mut checks = Vec::<String>::new();
        let result: anyhow::Result<()> = async {
            wait_for(&view, handle, cx, |app| app.state.active_tab_state().is_some_and(|s| s.entries.iter().any(|e| e.name == "image.psd"))).await?;
            for enabled in [true, false] {
                handle.update(cx, |_, _, cx| view.update(cx, |app, cx| {
                    app.handle_ext_action(ExtAction::SetEnabled { id: "document-previews".into(), enabled });
                    cx.notify();
                }))?;
                for (name, expected) in [
                    ("unicode.txt", "text"), ("unicode.rs", "text"), ("unicode.md", "markdown"),
                    ("table.csv", "table"), ("utf16.csv", "table"), ("document.docx", "document"),
                    ("workbook.xlsx", "table"), ("slides.pptx", "slides"), ("image.psd", "image"),
                    ("bundle.zip", "directory"), ("bundle.rar", "directory"), ("binary.txt", "binary"),
                    ("broken.docx", "error"), ("broken.xlsx", "error"), ("broken.pptx", "error"),
                    ("broken.zip", "error"), ("broken.rar", "error"), ("broken.psd", "error"), ("oversized.pptx", "error"),
                ] {
                    let path = fixtures.join(name);
                    handle.update(cx, |_, _, cx| -> anyhow::Result<()> {
                        view.update(cx, |app, cx| -> anyhow::Result<()> {
                            let tile = app.state.focused_tile;
                            let idx = app.state.tab_states[&tile].entries.iter().position(|e| e.name == name).context(format!("Missing {name}"))?;
                            app.state.clear_preview(tile);
                            app.process_actions([Action::Select { tile, idx, shift: false, ctrl: false }]);
                            app.load_preview_gpui(tile, path.clone());
                            cx.notify(); Ok(())
                        })
                    })??;
                    wait_for(&view, handle, cx, |app| app.state.preview.result_paths.get(&app.state.focused_tile) == Some(&path)).await.with_context(|| format!("{name}, extension={enabled}"))?;
                    handle.update(cx, |_, window, cx| -> anyhow::Result<()> {
                        let app = view.read(cx);
                        let result = app.state.preview.results.get(&app.state.focused_tile).context("Missing preview")?;
                        ensure!(kind(result) == expected, "{name}, extension={enabled}: expected {expected}, got {}; error={:?}", kind(result), if let PreviewResult::Error(e) = result { Some(e) } else { None });
                        if name == "table.csv" { ensure!(matches!(result, PreviewResult::Table { rows, .. } if rows.len() == 1 && rows[0][1].contains("\n日本語")), "CSV lost multiline field"); }
                        if name == "image.psd" { ensure!(matches!(result, PreviewResult::Image { width: 64, height: 48, .. }), "PSD geometry wrong"); }
                        if name == "bundle.rar" || name == "bundle.zip" {
                            ensure!(matches!(result, PreviewResult::Dir { entries, .. } if entries.len() == 1 && entries[0].is_dir), "{name}: duplicated or mistyped archive folder");
                        }
                        if enabled && ["image.psd", "unicode.md", "unicode.rs", "table.csv", "document.docx"].contains(&name) {
                            window.render_to_image()?.save(directory.join(format!("preview-{name}.png")))?;
                        }
                        Ok(())
                    })??;
                    checks.push(format!("{name}: {expected}, document extension={enabled}"));
                }
            }
            // Exercise virtual files through the same UI navigation as normal archives.
            for archive in ["bundle.zip", "bundle.rar"] {
                handle.update(cx, |_, _, cx| view.update(cx, |app, cx| {
                    let tile = app.state.focused_tile;
                    app.navigate_into_archive(tile, fixtures.join(archive), "archive-files".into()); cx.notify();
                }))?;
                wait_for(&view, handle, cx, |app| app.state.active_tab_state().is_some_and(|s| s.entries.iter().any(|e| e.name == "unicode.md"))).await?;
                handle.update(cx, |_, _, cx| -> anyhow::Result<()> {
                    let tab = view.read(cx).state.active_tab_state().context("Missing archive tab")?;
                    ensure!(tab.entries.iter().any(|e| e.name == "Vacía" && e.kind() == atlas_model::file_entry::FileKind::Dir), "{archive}: empty directory lost its type");
                    ensure!(tab.entries.iter().any(|e| e.name == "niño-日本語.txt"), "{archive}: Unicode filename lost");
                    Ok(())
                })??;
                for (name, expected) in [("unicode.txt", "text"), ("unicode.rs", "text"), ("unicode.md", "markdown"), ("document.docx", "document"), ("table.csv", "table")] {
                    let inner = PathBuf::from("archive-files").join(name);
                    handle.update(cx, |_, _, cx| view.update(cx, |app, cx| {
                        let tile = app.state.focused_tile;
                        let idx = app.state.tab_states[&tile].entries.iter().position(|e| e.name == name).unwrap();
                        app.state.clear_preview(tile);
                        app.process_actions([Action::Select { tile, idx, shift: false, ctrl: false }]); cx.notify();
                    }))?;
                    wait_for(&view, handle, cx, |app| app.state.preview.result_paths.get(&app.state.focused_tile) == Some(&inner)).await?;
                    handle.update(cx, |_, _, cx| -> anyhow::Result<()> {
                        let app = view.read(cx);
                        ensure!(kind(&app.state.preview.results[&app.state.focused_tile]) == expected, "{archive}/{name}: wrong preview"); Ok(())
                    })??;
                    checks.push(format!("{archive}/{name}: {expected}"));
                }
            }
            handle.update(cx, |_, _, cx| view.update(cx, |app, cx| {
                app.state.settings.show_inspector = false;
                let tile = app.state.focused_tile;
                app.navigate(tile, fixtures.clone()); app.process_actions([Action::SetViewMode(ViewMode::Miller)]); cx.notify();
            }))?;
            for level in 0..4 {
                let name = format!("Columna {level} con nombre muy largo y acentos");
                wait_for(&view, handle, cx, |app| app.state.active_tab_state().and_then(|s| s.miller.columns.get(level)).is_some_and(|c| c.entries.iter().any(|e| e.name == name))).await?;
                handle.update(cx, |_, _, cx| view.update(cx, |app, cx| {
                    let tile = app.state.focused_tile;
                    let idx = app.state.tab_states[&tile].miller.columns[level].entries.iter().position(|e| e.name == name).unwrap();
                    app.miller_select(tile, level, idx, false, false);
                    app.state.tab_states.get_mut(&tile).unwrap().miller.focused = level + 1;
                    cx.notify();
                }))?;
            }
            wait_for(&view, handle, cx, |app| app.state.active_tab_state().and_then(|s| s.miller.columns.last()).is_some_and(|c| c.entries.len() >= 120)).await?;
            for (label, width, height, zoom) in [("desktop",1280.,800.,1.), ("laptop-zoom",980.,700.,1.5), ("narrow",860.,500.,0.8), ("short-zoom",980.,240.,2.)] {
                handle.update(cx, |_, window, cx| {
                    window.resize(size(px(width), px(height)));
                    view.update(cx, |app, cx| { app.state.settings.global_zoom = zoom; app.theme_dirty = true; app.notify_all_panes(cx); cx.notify(); });
                })?;
                for _ in 0..20 { cx.background_executor().timer(Duration::from_millis(20)).await; handle.update(cx, |_, window, cx| { window.refresh(); window.draw(cx).clear(cx); })?; }
                handle.update(cx, |_, window, cx| -> anyhow::Result<()> {
                    let tile = view.read(cx).state.focused_tile;
                    let pane = *view.read(cx).pane_view_bounds.get(&tile).context("Miller viewport was not recorded")?;
                    let col = *view.read(cx).miller_column_bounds.get(&(tile, 4)).context("Missing fifth Miller column")?;
                    ensure!(col.right() <= pane.right() + px(2.), "{label}: focused column is outside viewport");
                    let position = point(col.left().max(pane.left()) + px(30.), pane.top() + px(70.));
                    window.simulate_mouse_move(position, cx);
                    window.dispatch_event(gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent { position, delta: gpui::ScrollDelta::Lines(point(0., -12.)), ..Default::default() }), cx);
                    window.refresh(); window.draw(cx).clear(cx);
                    window.render_to_image()?.save(directory.join(format!("miller-{label}.png")))?;
                    Ok(())
                })??;
                checks.push(format!("Miller {label}: five columns, Unicode/long names, vertical clipping and scrollbar"));
            }
            handle.update(cx, |_, window, cx| {
                window.resize(size(px(1280.), px(800.)));
                view.update(cx, |app, cx| { app.state.settings.global_zoom = 1.; app.theme_dirty = true; cx.notify(); });
            })?;
            for _ in 0..3 { cx.background_executor().timer(Duration::from_millis(20)).await; handle.update(cx, |_, window, cx| { window.refresh(); window.draw(cx).clear(cx); })?; }
            let before = handle.update(cx, |_, window, cx| -> anyhow::Result<_> {
                let app = view.read(cx);
                let tile = app.state.focused_tile;
                let pane = *app.pane_view_bounds.get(&tile).context("Missing Miller viewport")?;
                let before = app.miller_column_bounds.get(&(tile, 4)).context("Missing fifth column")?.left();
                let position = point(pane.right() - px(30.), pane.top() + px(70.));
                window.simulate_mouse_move(position, cx);
                window.dispatch_event(gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent { position, delta: gpui::ScrollDelta::Pixels(point(px(120.), px(0.))), ..Default::default() }), cx);
                Ok(before)
            })??;
            for _ in 0..3 { cx.background_executor().timer(Duration::from_millis(20)).await; handle.update(cx, |_, window, cx| { window.refresh(); window.draw(cx).clear(cx); })?; }
            handle.update(cx, |_, _, cx| -> anyhow::Result<()> {
                let app = view.read(cx);
                ensure!(app.miller_column_bounds.get(&(app.state.focused_tile, 4)).context("Missing scrolled column")?.left() > before + px(1.), "Miller swallowed horizontal wheel");
                Ok(())
            })??;
            checks.push("Miller horizontal pixel wheel reaches the outer viewport".into());
            handle.update(cx, |_, window, cx| -> anyhow::Result<()> {
                crate::ui::shell_context_menu::diagnostic::prepare_open_with(&view, fixtures.join("unicode.txt"), window, cx);
                window.refresh(); window.draw(cx).clear(cx);
                crate::ui::shell_context_menu::diagnostic::check_open_with(&view, false, cx)?;
                window.render_to_image()?.save(directory.join("open-with-collapsed.png"))?;
                window.dispatch_keystroke(Keystroke::parse("right")?, cx);
                window.refresh(); window.draw(cx).clear(cx);
                crate::ui::shell_context_menu::diagnostic::check_open_with(&view, true, cx)?;
                window.render_to_image()?.save(directory.join("open-with-expanded.png"))?;
                window.dispatch_keystroke(Keystroke::parse("left")?, cx);
                window.refresh(); window.draw(cx).clear(cx);
                crate::ui::shell_context_menu::diagnostic::check_open_with(&view, false, cx)?;
                view.update(cx, |app, cx| app.close_ctx_menu(cx));
                Ok(())
            })??;
            checks.push("One collapsed Open with group; keyboard expand/collapse, all eight applications, native chooser and search".into());
            Ok(())
        }.await;
        match result {
            Ok(()) => { let _ = std::fs::write(directory.join("passed.json"), serde_json::to_vec_pretty(&checks).unwrap()); }
            Err(error) => { let _ = std::fs::write(directory.join("failed.txt"), format!("{error:#}")); let _ = handle.update(cx, |_, window, _| window.render_to_image().map(|image| image.save(directory.join("failed.png")))); }
        }
        let _ = std::fs::write(directory.join("checks.json"), serde_json::to_vec_pretty(&checks).unwrap());
        drop(view);
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        let _ = cx.update(|cx| cx.quit());
    }).detach();
}
