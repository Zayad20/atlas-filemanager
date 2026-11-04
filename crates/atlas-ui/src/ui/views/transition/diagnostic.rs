//! Native visual verification without touching desktop input or user settings.
use crate::app::App;
use crate::input::action::Action;
use anyhow::ensure;
use atlas_model::{file_entry::FileKind, settings::ViewMode};
use gpui::prelude::*;
use gpui::{point, px, size, Entity, Window, WindowBounds, WindowOptions};
use gpui_component::{Root, ThemeRegistry, TitleBar};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn draw(window: &mut Window, cx: &mut gpui::App) {
    window.simulate_next_frame(cx);
    window.refresh();
    window.draw(cx).clear(cx);
}

fn capture(directory: &Path, name: &str, window: &mut Window) -> anyhow::Result<()> {
    window
        .render_to_image()?
        .save(directory.join(format!("{name}.png")))?;
    Ok(())
}

fn mode(app: &Entity<App>, value: f32, window: &mut Window, cx: &mut gpui::App) {
    app.update(cx, |app, cx| {
        let pane = app.state.focused_tile;
        app.set_pane_view_from_slider_value(pane, value);
        app.notify_pane(pane, cx);
        cx.notify();
    });
    draw(window, cx);
}

pub fn run(directory: PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all(&directory)?;
    let directory = directory.canonicalize()?;
    let directory = PathBuf::from(directory.to_string_lossy().trim_start_matches(r"\\?\"));
    let fixtures = directory.join("fixtures");
    let child = fixtures.join("Design assets");
    std::fs::create_dir_all(&child)?;
    std::fs::create_dir_all(fixtures.join("References"))?;
    for index in 0..12 {
        std::fs::write(
            fixtures.join(format!("Document {index:02}.txt")),
            vec![b'a'; (index + 1) * 1024],
        )?;
        std::fs::write(
            child.join(format!("Asset {index:02}.svg")),
            "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        )?;
    }
    let config = directory.join("config");
    std::fs::create_dir_all(&config)?;
    std::env::set_var("ATLAS_CONFIG_DIR", &config);
    let mut profiles = atlas_model::profiles::ProfileManager::default();
    profiles.profiles[0].theme = "Atlas Graphite".into();
    std::fs::write(
        config.join("profiles.json"),
        serde_json::to_vec_pretty(&profiles)?,
    )?;
    rust_i18n::set_locale("es");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let rt = runtime.handle().clone();
    let result = std::rc::Rc::new(std::cell::RefCell::new(Ok(())));
    let final_result = result.clone();
    gpui_platform::application()
        .with_assets(crate::assets::AtlasAssets)
        .run(move |cx| {
            gpui_component::init(cx);
            cx.text_system()
                .add_fonts(crate::assets::bundled_fonts())
                .unwrap();
            ThemeRegistry::global_mut(cx)
                .load_themes_from_str(include_str!("../../../../assets/themes/atlas-themes.json"))
                .unwrap();
            let mut app = None;
            let window = cx
                .open_window(
                    WindowOptions {
                        show: false,
                        focus: false,
                        titlebar: Some(TitleBar::title_bar_options()),
                        window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
                        ..Default::default()
                    },
                    |window, cx| {
                        let view = cx.new(|cx| {
                            App::new(
                                rt,
                                atlas_model::settings::Settings {
                                    autosave_session: false,
                                    getting_started_seen: true,
                                    show_inspector: false,
                                    ..Default::default()
                                },
                                Instant::now(),
                                vec![fixtures],
                                window,
                                cx,
                            )
                        });
                        app = Some(view.clone());
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                )
                .unwrap();
            let app = app.unwrap();
            let window: gpui::AnyWindowHandle = window.into();
            cx.spawn(async move |cx| {
                window
                    .update(cx, |_, window, _| window.resize(size(px(1280.), px(800.))))
                    .unwrap();
                for _ in 0..30 {
                    cx.background_executor()
                        .timer(Duration::from_millis(50))
                        .await;
                    window.update(cx, |_, window, cx| draw(window, cx)).unwrap();
                }
                let checked: anyhow::Result<()> = async {
                    window.update(cx, |_, window, cx| {
                        app.update(cx, |app, cx| {
                            let pane = app.state.focused_tile;
                            app.state
                                .tab_states
                                .get_mut(&pane)
                                .unwrap()
                                .selection
                                .selected
                                .clear();
                            app.notify_pane(pane, cx);
                            cx.notify();
                        });
                        draw(window, cx);
                        capture(&directory, "idle", window)
                    })??;
                    window.update(cx, |_, window, cx| {
                        app.update(cx, |app, cx| {
                            let pane = app.state.focused_tile;
                            let tab = app.state.tab_states.get_mut(&pane).unwrap();
                            tab.selection.selected = tab
                                .entries
                                .iter()
                                .enumerate()
                                .filter(|(_, entry)| entry.kind() != FileKind::Dir)
                                .take(3)
                                .map(|(i, _)| i)
                                .collect();
                            app.notify_pane(pane, cx);
                            cx.notify();
                        });
                        draw(window, cx);
                        Ok::<_, anyhow::Error>(())
                    })??;
                    cx.background_executor()
                        .timer(Duration::from_millis(160))
                        .await;
                    window.update(cx, |_, window, cx| {
                        draw(window, cx);
                        capture(&directory, "selection", window)
                    })??;
                    let mut checks = Vec::new();
                    for (value, expected) in [
                        (0., ViewMode::Details),
                        (1., ViewMode::Tree),
                        (2., ViewMode::List),
                        (3., ViewMode::Grid),
                    ] {
                        window.update(cx, |_, window, cx| {
                            mode(&app, value, window, cx);
                            let state = app.read(cx);
                            let pane = state.state.focused_tile;
                            ensure!(
                                state.tab_data_for(pane).unwrap().view_mode == expected,
                                "slider order"
                            );
                            ensure!(
                                state.state.tab_states[&pane]
                                    .selected_entries(&expected)
                                    .len()
                                    == 3,
                                "selection lost during view change"
                            );
                            Ok::<_, anyhow::Error>(())
                        })??;
                        cx.background_executor()
                            .timer(Duration::from_millis(240))
                            .await;
                        window.update(cx, |_, window, cx| {
                            draw(window, cx);
                            capture(&directory, &format!("view-{expected:?}"), window)
                        })??;
                        checks.push(format!("{expected:?}: slider and selection continuity"));
                    }
                    // Exercise actual Ctrl+wheel policy in both directions.
                    window.update(cx, |_, window, cx| {
                        app.update(cx, |app, cx| {
                            let pane = app.state.focused_tile;
                            for expected in [ViewMode::List, ViewMode::Tree, ViewMode::Details] {
                                app.pane_ctrl_scroll(pane, -1.0);
                                ensure!(
                                    app.tab_data_for(pane).unwrap().view_mode == expected,
                                    "Ctrl+wheel reverse order"
                                );
                            }
                            app.notify_pane(pane, cx);
                            cx.notify();
                            Ok::<_, anyhow::Error>(())
                        })?;
                        draw(window, cx);
                        Ok::<_, anyhow::Error>(())
                    })??;
                    for (name, back) in [("folder-forward", false), ("folder-back", true)] {
                        window.update(cx, |_, _, cx| {
                            app.update(cx, |app, cx| {
                                let pane = app.state.focused_tile;
                                if back {
                                    app.navigate_back(pane);
                                } else {
                                    app.navigate(pane, child.clone());
                                }
                                app.flush_dirty_panes(cx);
                            })
                        })?;
                        for _ in 0..20 {
                            cx.background_executor()
                                .timer(Duration::from_millis(20))
                                .await;
                            let ready = window.update(cx, |_, window, cx| {
                                draw(window, cx);
                                let state = app.read(cx);
                                state
                                    .pane_view_transitions
                                    .get(&state.state.focused_tile)
                                    .and_then(|s| s.animation)
                                    .is_some()
                            })?;
                            if ready {
                                break;
                            }
                        }
                        window.update(cx, |_, window, cx| {
                            app.update(cx, |app, _| {
                                let pane = app.state.focused_tile;
                                let animation = app.pane_view_transitions[&pane]
                                    .animation
                                    .ok_or_else(|| anyhow::anyhow!("missing folder entrance"))?;
                                ensure!(
                                    if back {
                                        animation.x < 0.
                                    } else {
                                        animation.x > 0.
                                    },
                                    "folder direction"
                                );
                                Ok::<_, anyhow::Error>(())
                            })?;
                            capture(&directory, name, window)
                        })??;
                        cx.background_executor()
                            .timer(Duration::from_millis(260))
                            .await;
                        window.update(cx, |_, window, cx| draw(window, cx))?;
                        checks.push(format!("{name}: loaded content and direction"));
                    }
                    window.update(cx, |_, window, cx| {
                        app.update(cx, |app, cx| {
                            let pane = app.state.focused_tile;
                            app.state.settings.disable_animations = true;
                            app.set_pane_view_mode(pane, ViewMode::List);
                            app.notify_pane(pane, cx);
                            cx.notify();
                        });
                        draw(window, cx);
                        let state = app.read(cx);
                        ensure!(
                            state.pane_view_transitions[&state.state.focused_tile]
                                .animation
                                .is_none(),
                            "reduced motion"
                        );
                        Ok::<_, anyhow::Error>(())
                    })??;
                    window.update(cx, |_, window, cx| {
                        app.update(cx, |app, cx| {
                            let pane = app.state.focused_tile;
                            app.process_actions([Action::ToggleShowFiles(pane)]);
                            app.flush_dirty_panes(cx);
                        });
                        draw(window, cx);
                        capture(&directory, "filter-active", window)?;
                        app.update(cx, |app, cx| {
                            let pane = app.state.focused_tile;
                            app.process_actions([Action::ToggleShowFiles(pane)]);
                            app.flush_dirty_panes(cx);
                        });
                        draw(window, cx);
                        Ok::<_, anyhow::Error>(())
                    })??;
                    // Open the real footer popover with synthetic input inside the hidden window.
                    window.update(cx, |_, window, cx| {
                        let position = point(px(1248.), px(778.));
                        for down in [true, false] {
                            let input = if down {
                                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                                    position,
                                    button: gpui::MouseButton::Left,
                                    modifiers: Default::default(),
                                    click_count: 1,
                                    first_mouse: false,
                                })
                            } else {
                                gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                                    position,
                                    button: gpui::MouseButton::Left,
                                    modifiers: Default::default(),
                                    click_count: 1,
                                })
                            };
                            window.dispatch_event(input, cx);
                            draw(window, cx);
                        }
                        capture(&directory, "view-menu", window)
                    })??;
                    std::fs::write(
                        directory.join("verification.json"),
                        serde_json::to_vec_pretty(&checks)?,
                    )?;
                    Ok(())
                }
                .await;
                if let Err(error) = &checked {
                    let _ = std::fs::write(directory.join("failed.txt"), format!("{error:#}"));
                }
                *result.borrow_mut() = checked;
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        });
    let outcome = std::mem::replace(&mut *final_result.borrow_mut(), Ok(()));
    outcome
}
