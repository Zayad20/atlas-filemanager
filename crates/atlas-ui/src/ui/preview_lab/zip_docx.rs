//! Native regression checks for archive activation and optional DOCX providers.
use super::{select_named, stored_zip};
use crate::{
    app::{ext::ExtAction, App},
    input::action::Action,
};
use anyhow::{ensure, Context as _};
use atlas_model::{location::Location, preview_cache::PreviewResult, settings::ViewMode};
use gpui::{Entity, Window};
use std::{path::PathBuf, time::Duration};

pub(super) fn install(
    view: &Entity<App>,
    window: &mut Window,
    cx: &mut gpui::App,
    directory: PathBuf,
) {
    let _ = std::fs::remove_file(directory.join("passed.json"));
    let _ = std::fs::remove_file(directory.join("failed.txt"));
    let fixtures = directory.join("fixtures");
    let document = stored_zip(&[("word/document.xml", br#"<w:document xmlns:w="urn:word"><w:body><w:p><w:r><w:t>Hello </w:t></w:r><w:r><w:t>Word &amp; ZIP</w:t></w:r></w:p></w:body></w:document>"#)]);
    std::fs::write(fixtures.join("sample.docx"), &document).unwrap();
    std::fs::write(
        fixtures.join("nested.zip"),
        stored_zip(&[
            ("docs/sample.docx", &document),
            ("docs/readme.txt", b"Read me"),
        ]),
    )
    .unwrap();
    rust_i18n::set_locale("es");
    view.update(cx, |app, cx| {
        app.state.overlay.show_getting_started = false;
        app.state.settings.disable_animations = true;
        app.state.settings.show_inspector = true;
        cx.notify();
    });
    let handle = window.window_handle();
    let view = view.clone();
    cx.spawn(async move |cx| {
        let result: anyhow::Result<Vec<String>> = async {
            let mut checks = Vec::new();
            for mode in [ViewMode::Details, ViewMode::List, ViewMode::Grid, ViewMode::Tree, ViewMode::Miller] {
                handle.update(cx, |_, _, cx| {
                    view.update(cx, |app, cx| {
                        app.state.overlay.show_getting_started = false;
                        app.state.settings.disable_animations = true;
                        app.state.settings.show_inspector = true;
                        let tile = app.state.focused_tile;
                        app.navigate(tile, fixtures.clone());
                        app.process_actions([Action::SetViewMode(mode.clone())]);
                        cx.notify();
                    });
                })?;
                let mut loaded = false;
                for _ in 0..100 {
                    cx.background_executor().timer(Duration::from_millis(30)).await;
                    loaded = handle.update(cx, |_, window, cx| {
                        window.refresh(); window.draw(cx).clear(cx);
                        let app = view.read(cx);
                        let tile = app.state.focused_tile;
                        app.state.tab_states.get(&tile).is_some_and(|state| {
                            if mode == ViewMode::Miller {
                                state.miller.columns.first().is_some_and(|column| column.entries.iter().any(|entry| entry.name == "nested.zip"))
                            } else { state.entries.iter().any(|entry| entry.name == "nested.zip") }
                        })
                    })?;
                    if loaded { break; }
                }
                ensure!(loaded, "{mode:?}: fixtures never loaded");
                handle.update(cx, |_, _, cx| -> anyhow::Result<()> {
                    view.update(cx, |app, cx| -> anyhow::Result<()> {
                        let tile = app.state.focused_tile;
                        let state = app.state.tab_states.get_mut(&tile).context("No tab")?;
                        if mode == ViewMode::Miller {
                            let column = state.miller.columns.first_mut().context("No Miller column")?;
                            let idx = column.entries.iter().position(|entry| entry.name == "nested.zip").context("No ZIP in Miller")?;
                            column.selection.keyboard_cursor = Some(idx);
                            app.miller_open_cursor();
                        } else if mode == ViewMode::Tree {
                            let rows = state.visible_tree_rows();
                            let idx = rows.iter().position(|row| row.0.name == "nested.zip").context("No ZIP in Tree")?;
                            let path = rows[idx].0.path();
                            // Exercise the shared double-click route used by Tree.
                            app.tree_entry_left_mouse_down(tile, idx, path, false, gpui::point(gpui::px(0.), gpui::px(0.)), false, false, 2, cx);
                        } else {
                            let idx = state.entries.iter().position(|entry| entry.name == "nested.zip").context("No ZIP")?;
                            app.process_actions([Action::Activate { tile, idx }]);
                        }
                        cx.notify(); Ok(())
                    })
                })??;
                let mut opened = false;
                for _ in 0..100 {
                    cx.background_executor().timer(Duration::from_millis(30)).await;
                    opened = handle.update(cx, |_, window, cx| {
                        window.refresh(); window.draw(cx).clear(cx);
                        let app = view.read(cx);
                        let tile = app.state.focused_tile;
                        let state = &app.state.tab_states[&tile];
                        if mode == ViewMode::Miller {
                            state.miller.columns.last().is_some_and(|column| matches!(column.location, Location::Archive { .. }) && column.entries.iter().any(|entry| entry.name == "docs"))
                        } else {
                            app.tab_data_for(tile).is_some_and(|tab| matches!(tab.location, Location::Archive { .. })) && state.entries.iter().any(|entry| entry.name == "docs")
                        }
                    })?;
                    if opened { break; }
                }
                ensure!(opened, "{mode:?}: ZIP did not open internally");
                checks.push(format!("{mode:?}: ZIP opens internally and lists implicit docs directory"));
            }
            // Same selected local DOCX, changing the provider without reselection.
            handle.update(cx, |_, _, cx| {
                view.update(cx, |app, cx| {
                    let tile = app.state.focused_tile;
                    app.process_actions([Action::SetViewMode(ViewMode::Details)]);
                    app.navigate(tile, fixtures.clone());
                    cx.notify();
                });
            })?;
            cx.background_executor().timer(Duration::from_millis(500)).await;
            for enabled in [true, false, true] {
                handle.update(cx, |_, _, cx| -> anyhow::Result<()> {
                    ensure!(select_named(&view, "sample.docx", cx), "No DOCX fixture");
                    view.update(cx, |app, cx| {
                        app.handle_ext_action(ExtAction::SetEnabled { id: "document-previews".into(), enabled });
                        let tile = app.state.focused_tile;
                        assert!(!app.state.preview.paths.contains_key(&tile), "provider change must invalidate selected path");
                        app.state.preview.results.remove(&tile);
                        app.state.preview.result_paths.remove(&tile);
                        app.load_preview_gpui(tile, fixtures.join("sample.docx"));
                        cx.notify();
                    }); Ok(())
                })??;
                let mut ready = false;
                for _ in 0..100 {
                    cx.background_executor().timer(Duration::from_millis(30)).await;
                    ready = handle.update(cx, |_, window, cx| {
                        window.refresh(); window.draw(cx).clear(cx);
                        let app = view.read(cx);
                        let tile = app.state.focused_tile;
                        app.state.preview.paths.get(&tile) == app.state.preview.result_paths.get(&tile)
                            && matches!(app.state.preview.results.get(&tile), Some(PreviewResult::Document { blocks, .. }) if blocks == &["Hello Word & ZIP"])
                    })?;
                    if ready { break; }
                }
                ensure!(ready, "DOCX failed with provider enabled={enabled}");
                checks.push(format!("DOCX preview with Document Previews enabled={enabled}"));
            }
            handle.update(cx, |_, window, cx| -> anyhow::Result<()> {
                view.update(cx, |app, cx| { app.process_actions([Action::OpenGuide(10)]); cx.notify(); });
                window.refresh(); window.draw(cx).clear(cx);
                window.render_to_image()?.save(directory.join("extensions-guide.png"))?;
                view.update(cx, |app, cx| {
                    app.process_actions([Action::CloseGuide, Action::StartGuideCoach(10), Action::OpenExtManager]);
                    cx.notify();
                });
                window.refresh(); window.draw(cx).clear(cx);
                ensure!(view.read(cx).state.overlay.guide_coach.is_some_and(|coach| coach.step == 1), "Extension coach did not observe opening the manager");
                window.render_to_image()?.save(directory.join("extension-manager.png"))?;
                view.update(cx, |app, cx| {
                    app.state.ext.show_manager = false;
                    app.state.overlay.guide_coach = None;
                    app.ext_manager_input = None;
                    app.getting_started_profile_input = None;
                    cx.notify();
                });
                window.refresh(); window.draw(cx).clear(cx);
                Ok(())
            })??;
            checks.push("Extension guide renders live inventory and coach observes manager opening".into());
            // Drain focus/caret tasks before GPUI tears down its entity map.
            for _ in 0..4 {
                cx.background_executor().timer(Duration::from_millis(30)).await;
                handle.update(cx, |_, window, cx| {
                    let focus = view.read(cx).focus_handle.clone();
                    window.focus(&focus, cx);
                    window.refresh(); window.draw(cx).clear(cx);
                })?;
            }
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
