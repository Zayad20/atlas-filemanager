//! Isolated native pointer, keyboard, marquee and fast-scroll verification.
use crate::{
    app::{App, RubberTarget},
    input::action::Action,
};
use anyhow::{ensure, Context as _};
use atlas_model::{
    file_entry::FileKind,
    pane::TabId,
    settings::ViewMode,
    tab::{GroupBy, SelectionState},
};
use gpui::{
    point, prelude::*, px, size, Bounds, Entity, Modifiers, Pixels, Point, Window, WindowBounds,
    WindowOptions,
};
use gpui_component::{Root, ThemeRegistry, TitleBar};
use std::{
    collections::HashSet,
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

fn pointer(
    position: Point<Pixels>,
    modifiers: Modifiers,
    down: bool,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    pointer_count(position, modifiers, down, 1, window, cx);
}

fn pointer_count(
    position: Point<Pixels>,
    modifiers: Modifiers,
    down: bool,
    click_count: usize,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let event = if down {
        gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
            position,
            modifiers,
            button: gpui::MouseButton::Left,
            click_count,
            first_mouse: false,
        })
    } else {
        gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
            position,
            modifiers,
            button: gpui::MouseButton::Left,
            click_count,
        })
    };
    window.dispatch_event(event, cx);
    draw(window, cx);
}

fn click(position: Point<Pixels>, modifiers: Modifiers, window: &mut Window, cx: &mut gpui::App) {
    pointer(position, modifiers, true, window, cx);
    pointer(position, modifiers, false, window, cx);
}

fn move_drag(position: Point<Pixels>, window: &mut Window, cx: &mut gpui::App) {
    window.dispatch_event(
        gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
            position,
            pressed_button: Some(gpui::MouseButton::Left),
            modifiers: Default::default(),
        }),
        cx,
    );
    draw(window, cx);
}

fn selection(app: &App, pane: TabId) -> &SelectionState {
    let tab = &app.state.tab_states[&pane];
    if app.tab_data_for(pane).unwrap().view_mode == ViewMode::Miller {
        &tab.miller.columns[tab.miller.focused].selection
    } else {
        &tab.selection
    }
}

fn rows(app: &App, pane: TabId) -> Vec<(usize, Bounds<Pixels>)> {
    let tab = app.tab_data_for(pane).unwrap();
    let model = match tab.view_mode {
        ViewMode::Grid => app.grid_hit.get(&pane),
        ViewMode::List => app.list_hit.get(&pane),
        _ => None,
    };
    let mut rows = if let Some(model) = model {
        (model.start_idx..model.end_idx)
            .filter_map(|i| {
                let bounds = model.cell_bounds(i);
                model
                    .bounds
                    .contains(&bounds.center())
                    .then(|| (model.visible[i], bounds))
            })
            .collect::<Vec<_>>()
    } else if tab.view_mode == ViewMode::Details {
        app.details_hit
            .get(&pane)
            .map(|model| {
                (model.start_idx..model.end_idx)
                    .filter_map(|i| {
                        let bounds = model.row_bounds(i);
                        model
                            .bounds
                            .contains(&bounds.center())
                            .then(|| (model.visible[i], bounds))
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else if tab.view_mode == ViewMode::Miller {
        app.miller_row_rects
            .borrow()
            .get(&(pane, 0))
            .cloned()
            .unwrap_or_default()
    } else {
        app.row_rects
            .borrow()
            .get(&pane)
            .cloned()
            .unwrap_or_default()
    };
    // Recorders can publish the same row in several retained frames.
    // Prefer its newest position, then discard overscan outside the viewport.
    rows.reverse();
    rows.sort_by_key(|(i, _)| *i);
    rows.dedup_by_key(|(i, _)| *i);
    rows.retain(|(i, _)| !atlas_model::grouping::is_virtual_cell(*i));
    if tab.group_by != GroupBy::None {
        if let Some(viewport) = app.pane_view_bounds.get(&pane) {
            rows.retain(|(_, bounds)| viewport.contains(&bounds.center()));
        }
    }
    rows
}

fn is_file(app: &App, pane: TabId, index: usize) -> bool {
    let tab = &app.state.tab_states[&pane];
    match app.tab_data_for(pane).unwrap().view_mode {
        ViewMode::Tree => tab
            .visible_tree_rows_snapshot()
            .get(index)
            .is_some_and(|row| row.0.kind() != FileKind::Dir),
        ViewMode::Miller => tab
            .miller
            .columns
            .first()
            .and_then(|col| col.entries.get(index))
            .is_some_and(|e| e.kind() != FileKind::Dir),
        _ => tab
            .entries
            .get(index)
            .is_some_and(|e| e.kind() != FileKind::Dir),
    }
}

fn ordered(app: &mut App, pane: TabId) -> Vec<usize> {
    match app.tab_data_for(pane).unwrap().view_mode {
        ViewMode::Tree => (0..app.state.tab_states[&pane]
            .visible_tree_rows_snapshot()
            .len())
            .collect(),
        ViewMode::Miller => {
            let tab = &app.state.tab_states[&pane];
            tab.miller.columns[0]
                .visible_indices(&tab.filter, tab.show_files, tab.show_folders)
                .to_vec()
        }
        _ => app.flat_visible_indices(pane).to_vec(),
    }
}

fn pointer_policy(
    app: &Entity<App>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> anyhow::Result<()> {
    let pane = app.read(cx).state.focused_tile;
    app.update(cx, |app, cx| {
        app.open_view_selectors.clear();
        let tab = app.state.tab_states.get_mut(&pane).unwrap();
        tab.selection.clear();
        for column in &mut tab.miller.columns {
            column.selection.clear();
        }
        app.notify_pane(pane, cx);
        cx.notify();
    });
    draw(window, cx);
    let targets: Vec<_> = rows(app.read(cx), pane)
        .into_iter()
        .filter(|(i, _)| is_file(app.read(cx), pane, *i))
        .take(5)
        .collect();
    ensure!(targets.len() >= 5, "fewer than five painted file targets");
    let [(a, ab), (b, bb), (_, _), (d, db), ..] = targets.as_slice() else {
        unreachable!()
    };
    click(ab.center(), Default::default(), window, cx);
    ensure!(
        selection(app.read(cx), pane).selected == HashSet::from([*a]),
        "plain click selects exactly one row; target={a} bounds={ab:?} selected={:?} cursor={:?} rubber={} view_menu={}", selection(app.read(cx), pane).selected, selection(app.read(cx), pane).keyboard_cursor, app.read(cx).rubber.is_some(), app.read(cx).open_view_selectors.contains(&pane)
    );
    click(
        bb.center(),
        Modifiers {
            control: true,
            ..Default::default()
        },
        window,
        cx,
    );
    ensure!(
        selection(app.read(cx), pane).selected == HashSet::from([*a, *b]),
        "Ctrl click must add exactly one row"
    );
    pointer(ab.center(), Default::default(), true, window, cx);
    ensure!(
        selection(app.read(cx), pane).selected == HashSet::from([*a, *b]),
        "selected row mousedown must preserve the group for file dragging"
    );
    // Exercise the real shared drag-source gate without starting an OS drag.
    let allowed = app.update(cx, |app, _| app.prepare_selection_file_drag());
    ensure!(allowed, "selected-click marker blocked the group file drag");
    ensure!(
        selection(app.read(cx), pane).selected == HashSet::from([*a, *b])
            && app.read(cx).rubber.is_none(),
        "file drag preparation must consume only the click marker and preserve selection"
    );
    pointer(ab.center(), Default::default(), false, window, cx);
    ensure!(
        selection(app.read(cx), pane).selected == HashSet::from([*a, *b]),
        "release after preparing a file drag collapsed its group"
    );
    // A separate stationary click must still collapse the selected group.
    pointer(ab.center(), Default::default(), true, window, cx);
    pointer(ab.center(), Default::default(), false, window, cx);
    ensure!(
        selection(app.read(cx), pane).selected == HashSet::from([*a]),
        "plain release on a selected row must collapse the group"
    );
    click(
        bb.center(),
        Modifiers {
            control: true,
            ..Default::default()
        },
        window,
        cx,
    );
    pointer(
        db.center(),
        Modifiers {
            shift: true,
            ..Default::default()
        },
        true,
        window,
        cx,
    );
    ensure!(
        app.read(cx).rubber.is_none(),
        "Shift click created a rubber gesture"
    );
    let visible = app.update(cx, |app, _| ordered(app, pane));
    let bi = visible.iter().position(|i| i == b).unwrap();
    let di = visible.iter().position(|i| i == d).unwrap();
    let expected: HashSet<_> = visible[bi.min(di)..=bi.max(di)].iter().copied().collect();
    ensure!(
        selection(app.read(cx), pane).selected == expected,
        "Shift must replace with anchor range"
    );
    pointer(
        db.center(),
        Modifiers {
            shift: true,
            ..Default::default()
        },
        false,
        window,
        cx,
    );
    ensure!(
        selection(app.read(cx), pane).selected == expected,
        "Shift release changed the selection"
    );
    click(
        ab.center(),
        Modifiers {
            shift: true,
            control: true,
            ..Default::default()
        },
        window,
        cx,
    );
    let ai = visible.iter().position(|i| i == a).unwrap();
    let mut expected = expected;
    expected.extend(visible[ai.min(bi)..=ai.max(bi)].iter().copied());
    ensure!(
        selection(app.read(cx), pane).selected == expected,
        "Ctrl+Shift must preserve the previous range"
    );
    let before_location = app.read(cx).tab_data_for(pane).unwrap().location.clone();
    let ctrl = Modifiers {
        control: true,
        ..Default::default()
    };
    pointer_count(db.center(), ctrl, true, 2, window, cx);
    pointer_count(db.center(), ctrl, false, 2, window, cx);
    expected.remove(d);
    ensure!(
        selection(app.read(cx), pane).selected == expected,
        "Ctrl double-click must toggle selection instead of activating"
    );
    ensure!(
        app.read(cx).tab_data_for(pane).unwrap().location == before_location,
        "Ctrl double-click navigated away"
    );
    Ok(())
}

fn blank_and_escape(
    app: &Entity<App>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> anyhow::Result<()> {
    let pane = app.read(cx).state.focused_tile;
    app.update(cx, |app, cx| {
        app.process_actions([Action::ToggleShowFiles(pane)]);
        app.notify_pane(pane, cx);
        cx.notify();
    });
    draw(window, cx);
    let state = app.read(cx);
    let bounds = if state.tab_data_for(pane).unwrap().view_mode == ViewMode::Miller {
        state.miller_column_bounds.get(&(pane, 0))
    } else {
        state.pane_view_bounds.get(&pane)
    }
    .copied()
    .context("missing pane viewport")?;
    // Pane bounds include floating footer controls. Choose empty space below
    // the two fixture directories, safely away from footer, menus and scrollbars.
    let blank = point(
        bounds.left() + bounds.size.width * 0.5,
        bounds.top() + bounds.size.height * 0.5,
    );
    ensure!(
        !rows(app.read(cx), pane)
            .iter()
            .any(|(_, b)| b.contains(&blank)),
        "blank probe overlaps a painted row: bounds={bounds:?} point={blank:?}"
    );
    app.update(cx, |app, cx| {
        let target = if app.tab_data_for(pane).unwrap().view_mode == ViewMode::Miller {
            RubberTarget::MillerColumn(pane, 0)
        } else {
            RubberTarget::Pane(pane)
        };
        app.selection_left_mouse_down(target, Some(0), bounds.center(), false, false, 1, cx);
        app.rubber = None;
    });
    draw(window, cx);
    let original = selection(app.read(cx), pane).clone();
    ensure!(
        !original.selected.is_empty(),
        "blank test needs a selected directory"
    );
    pointer(blank, Default::default(), true, window, cx);
    ensure!(
        selection(app.read(cx), pane).selected.is_empty(),
        "blank mousedown must immediately clear selection; pane={bounds:?} probe={blank:?} selected={:?} cursor={:?} rubber={} view_menu={}", selection(app.read(cx), pane).selected, selection(app.read(cx), pane).keyboard_cursor, app.read(cx).rubber.is_some(), app.read(cx).open_view_selectors.contains(&pane)
    );
    ensure!(
        selection(app.read(cx), pane).keyboard_cursor.is_none(),
        "blank click left a ghost cursor"
    );
    ensure!(
        app.read(cx).rubber.is_some(),
        "blank click must retain a rollback snapshot"
    );
    window.dispatch_event(
        gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
            keystroke: gpui::Keystroke::parse("escape")?,
            is_held: false,
            prefer_character_input: false,
        }),
        cx,
    );
    draw(window, cx);
    ensure!(
        selection(app.read(cx), pane).selected == original.selected,
        "Escape did not restore selection"
    );
    ensure!(app.read(cx).rubber.is_none(), "Escape left rubber active");
    pointer(blank, Default::default(), false, window, cx);
    click(blank, Default::default(), window, cx);
    ensure!(
        selection(app.read(cx), pane).selected.is_empty(),
        "blank release reselected an item"
    );
    app.update(cx, |app, cx| {
        app.process_actions([Action::ToggleShowFiles(pane)]);
        app.notify_pane(pane, cx);
        cx.notify();
    });
    draw(window, cx);
    Ok(())
}

fn list_keyboard(app: &Entity<App>, window: &mut Window, cx: &mut gpui::App) -> anyhow::Result<()> {
    app.update(cx, |app, cx| {
        let pane = app.state.focused_tile;
        let rows = app
            .list_column_step(pane)
            .context("List has no column geometry")?
            .0;
        ensure!(rows > 3, "List viewport too short");
        let visible = app.flat_visible_indices(pane);
        app.state
            .tab_states
            .get_mut(&pane)
            .unwrap()
            .selection
            .select(visible[1], &visible, false, false);
        app.nav_down(true, false);
        app.nav_right(true, false);
        let expected =
            HashSet::from([visible[1], visible[2], visible[rows + 1], visible[rows + 2]]);
        ensure!(
            app.state.tab_states[&pane].selection.selected == expected,
            "List Shift+Down+Right must select a 2x2 rectangle"
        );
        app.notify_pane(pane, cx);
        cx.notify();
        Ok::<_, anyhow::Error>(())
    })?;
    draw(window, cx);
    Ok(())
}

fn scroll_check(
    app: &Entity<App>,
    mode: ViewMode,
    window: &mut Window,
    cx: &mut gpui::App,
) -> anyhow::Result<()> {
    let pane = app.read(cx).state.focused_tile;
    for amount in [12000., 0., 24000., 2000., 0.] {
        // Do not notify App or PaneSignal: exercise the retained element's
        // prepaint against a live native scroll handle, without rebuilding it.
        app.update(cx, |app, _| {
            let handle = app.scroll_handle_for(pane).0.borrow().base_handle.clone();
            handle.set_offset(if mode == ViewMode::List {
                point(px(-amount), px(0.))
            } else {
                point(px(0.), px(-amount))
            });
        });
        draw(window, cx);
        let state = app.read(cx);
        let painted = rows(state, pane);
        ensure!(
            !painted.is_empty(),
            "fast scroll left an empty viewport at {amount}"
        );
        if matches!(mode, ViewMode::Grid | ViewMode::List) {
            let model = if mode == ViewMode::Grid {
                &state.grid_hit[&pane]
            } else {
                &state.list_hit[&pane]
            };
            let handle = &state.scroll_handles[&pane].0.borrow().base_handle;
            let actual = if mode == ViewMode::List {
                -f32::from(handle.offset().x)
            } else {
                -f32::from(handle.offset().y)
            };
            let recorded = if mode == ViewMode::List {
                model.scroll_x
            } else {
                model.scroll_y
            };
            ensure!(
                (recorded - actual).abs() < 1.,
                "stale hit model scroll: {recorded} vs {actual}"
            );
            for (index, bounds) in painted.iter().take(3) {
                ensure!(
                    model.hit_entry(bounds.center()) == Some(*index),
                    "painted row cannot be hit after scroll"
                );
            }
        } else if mode == ViewMode::Details {
            let model = &state.details_hit[&pane];
            let actual = -f32::from(
                state.scroll_handles[&pane]
                    .0
                    .borrow()
                    .base_handle
                    .offset()
                    .y,
            );
            ensure!(
                (model.scroll_y - actual).abs() < 1.,
                "Details hit model scroll is stale"
            );
            for (index, bounds) in painted.iter().take(3) {
                ensure!(
                    model.hit_entry(bounds.center()) == Some(*index),
                    "Details painted row cannot be hit"
                );
            }
        }
    }
    if matches!(mode, ViewMode::Grid | ViewMode::List | ViewMode::Details) {
        let painted = rows(app.read(cx), pane);
        let start = painted[0].1.center();
        let mut end = painted[painted.len().saturating_sub(2)].1.center();
        // A one-column row layout has identical centers; give the marquee area.
        if (f32::from(end.x - start.x)).abs() < 1. {
            end.x += px(20.);
        }
        app.update(cx, |app, _| {
            app.state
                .tab_states
                .get_mut(&pane)
                .unwrap()
                .selection
                .clear();
        });
        pointer(start, Default::default(), true, window, cx);
        move_drag(end, window, cx);
        let count = selection(app.read(cx), pane).selected.len();
        ensure!(count > 1, "marquee did not select multiple items; start={start:?} end={end:?} selected={:?} rubber={} view_menu={}", selection(app.read(cx), pane).selected, app.read(cx).rubber.is_some(), app.read(cx).open_view_selectors.contains(&pane));
        // The pointer stays stationary and pane content remains cached.
        app.update(cx, |app, _| {
            let handle = app.scroll_handle_for(pane).0.borrow().base_handle.clone();
            handle.set_offset(if mode == ViewMode::List {
                point(px(-1800.), px(0.))
            } else {
                point(px(0.), px(-1800.))
            });
        });
        draw(window, cx);
        ensure!(
            selection(app.read(cx), pane).selected.len() > count,
            "stationary marquee did not grow with scroll: before={count} after={} rubber={:?} offset={:?} model_scroll={:?}",
            selection(app.read(cx), pane).selected.len(),
            app.read(cx).rubber.as_ref().map(|r| (r.started, r.origin, r.current, r.content_origin)),
            app.read(cx).scroll_handles[&pane].0.borrow().base_handle.offset(),
            app.read(cx).grid_hit.get(&pane).map(|m| (m.scroll_y, m.bounds, m.cols, m.total_count, m.bands.len()))
        );
        ensure!(
            selection(app.read(cx), pane)
                .selected
                .contains(&painted[0].0),
            "scroll discarded offscreen marquee origin"
        );
        pointer(end, Default::default(), false, window, cx);
    }
    Ok(())
}

pub fn run(directory: PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all(&directory)?;
    let directory = directory.canonicalize()?;
    let directory = PathBuf::from(directory.to_string_lossy().trim_start_matches(r"\\?\"));
    let fixtures = directory.join("fixtures");
    std::fs::create_dir_all(fixtures.join("Directory A"))?;
    std::fs::create_dir_all(fixtures.join("Directory B"))?;
    for index in 0..800 {
        std::fs::write(
            fixtures.join(format!("File {index:04}.txt")),
            format!("Selection fixture {index}\n"),
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
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let rt = runtime.handle().clone();
    let result = std::rc::Rc::new(std::cell::RefCell::new(Ok(())));
    let final_result = result.clone();
    gpui_platform::application().with_assets(crate::assets::AtlasAssets).run(move |cx| {
        gpui_component::init(cx);
        cx.text_system().add_fonts(crate::assets::bundled_fonts()).unwrap();
        ThemeRegistry::global_mut(cx).load_themes_from_str(include_str!("../../../assets/themes/atlas-themes.json")).unwrap();
        let mut app = None;
        let window = cx.open_window(WindowOptions {
            show: false, focus: false, titlebar: Some(TitleBar::title_bar_options()),
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)), ..Default::default()
        }, |window, cx| {
            let view = cx.new(|cx| App::new(rt, atlas_model::settings::Settings {
                autosave_session: false, getting_started_seen: true, show_inspector: false,
                disable_animations: true, ..Default::default()
            }, Instant::now(), vec![fixtures], window, cx));
            app = Some(view.clone());
            cx.new(|cx| Root::new(view, window, cx))
        }).unwrap();
        let app = app.unwrap();
        let window: gpui::AnyWindowHandle = window.into();
        cx.spawn(async move |cx| {
            window.update(cx, |_, window, _| window.resize(size(px(1280.), px(800.)))).unwrap();
            for _ in 0..40 {
                cx.background_executor().timer(Duration::from_millis(40)).await;
                window.update(cx, |_, window, cx| draw(window, cx)).unwrap();
            }
            let checked: anyhow::Result<()> = async {
                let mut checks = Vec::new();
                for (name, mode, group) in [
                    ("details", ViewMode::Details, GroupBy::None), ("grid", ViewMode::Grid, GroupBy::None),
                    ("list", ViewMode::List, GroupBy::None), ("tree", ViewMode::Tree, GroupBy::None),
                    ("miller", ViewMode::Miller, GroupBy::None), ("grouped", ViewMode::Details, GroupBy::Type),
                    ("grouped-list", ViewMode::List, GroupBy::Name), ("grouped-grid", ViewMode::Grid, GroupBy::Name),
                ] {
                    window.update(cx, |_, window, cx| {
                        app.update(cx, |app, cx| {
                            let pane = app.state.focused_tile;
                            app.cancel_rubber();
                            if let Some(tab) = app.state.tab_states.get_mut(&pane) { tab.selection.clear(); }
                            app.process_actions([Action::SetGroupBy(GroupBy::None)]);
                            app.set_pane_view_mode(pane, mode.clone());
                            app.process_actions([Action::SetGroupBy(group)]);
                            app.scroll_handle_for(pane).0.borrow().base_handle.set_offset(point(px(0.), px(0.)));
                            app.notify_pane(pane, cx); cx.notify();
                        });
                        draw(window, cx);
                    })?;
                    for _ in 0..10 {
                        cx.background_executor().timer(Duration::from_millis(30)).await;
                        window.update(cx, |_, window, cx| draw(window, cx))?;
                    }
                    window.update(cx, |_, window, _| { let _ = capture(&directory, &format!("{name}-layout"), window); })?;
                    for case in ["pointer", "blank-escape", "scroll"] {
                        let outcome = window.update(cx, |_, window, cx| {
                            let result = match case {
                                "pointer" => pointer_policy(&app, window, cx),
                                "blank-escape" => blank_and_escape(&app, window, cx),
                                _ => scroll_check(&app, mode.clone(), window, cx),
                            };
                            let _ = capture(&directory, &format!("{name}-{case}"), window);
                            app.update(cx, |app, cx| {
                                let pane = app.state.focused_tile;
                                app.cancel_rubber();
                                app.open_view_selectors.clear();
                                if !app.state.tab_states[&pane].show_files {
                                    app.process_actions([Action::ToggleShowFiles(pane)]);
                                }
                                app.notify_pane(pane, cx); cx.notify();
                            });
                            draw(window, cx);
                            result
                        })?;
                        checks.push(serde_json::json!({"name": format!("{name}/{case}"), "status": if outcome.is_ok() {"passed"} else {"failed"}, "message": outcome.err().map(|e| format!("{e:#}"))}));
                    }
                    if mode == ViewMode::List && group == GroupBy::None {
                        let outcome = window.update(cx, |_, window, cx| list_keyboard(&app, window, cx))?;
                        checks.push(serde_json::json!({"name":"list/keyboard-rectangle", "status": if outcome.is_ok() {"passed"} else {"failed"}, "message": outcome.err().map(|e| format!("{e:#}"))}));
                    }
                    std::fs::write(directory.join("verification.json"), serde_json::to_vec_pretty(&checks)?)?;
                }
                ensure!(checks.iter().all(|check| check["status"] == "passed"), "selection diagnostic failures; see verification.json");
                Ok(())
            }.await;
            if let Err(error) = &checked { let _ = std::fs::write(directory.join("failed.txt"), format!("{error:#}")); }
            if checked.is_ok() { let _ = std::fs::remove_file(directory.join("failed.txt")); }
            *result.borrow_mut() = checked;
            drop(app);
            let _ = window.update(cx, |_, window, _| window.remove_window());
            let _ = cx.update(|cx| cx.quit());
        }).detach();
    });
    let outcome = std::mem::replace(&mut *final_result.borrow_mut(), Ok(()));
    outcome
}
