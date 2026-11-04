//! Hidden, isolated investigation of resize geometry; does not drive desktop input.
//! cargo run -p atlas-ui --example resize_diagnostic --profile release-perf
//!   --features ui-lab,perf-log,rar,sevenz -- <output-directory>
use atlas_ui::{app::App as AtlasApp, assets::AtlasAssets};
use gpui::prelude::*;
use gpui::{px, size, Window, WindowBounds, WindowOptions};
use gpui_component::{Root, ThemeRegistry, TitleBar};
use std::{path::PathBuf, time::Instant};

type Trace = std::rc::Rc<std::cell::RefCell<Vec<serde_json::Value>>>;

struct ObservedApp {
    app: gpui::Entity<AtlasApp>,
    trace: Trace,
    previous_viewport: std::rc::Rc<std::cell::Cell<Option<[f32; 2]>>>,
}

fn trace_frame(window: &Window, viewport: [f32; 2], trace: &Trace) {
    let quads = window.painted_quads();
    let panes = quads
        .iter()
        .filter(|quad| {
            quad.bounds.origin.x.0 > 280.
                && quad.bounds.size.width.0 > 300.
                && quad.bounds.size.height.0 > 200.
        })
        .map(|quad| [quad.bounds.origin.x.0, quad.bounds.size.width.0])
        .collect::<Vec<_>>();
    let dividers = quads
        .iter()
        .filter(|quad| {
            (quad.bounds.origin.y.0 - 76.).abs() < 0.1
                && (quad.bounds.size.width.0 - 1.).abs() < 0.1
                && (quad.bounds.size.height.0 - 18.).abs() < 0.1
        })
        .map(|quad| quad.bounds.origin.x.0)
        .collect::<Vec<_>>();
    let pane_rects = quads
        .iter()
        .filter(|quad| {
            quad.bounds.origin.x.0 > 280.
                && quad.bounds.size.width.0 > 300.
                && quad.bounds.size.height.0 > 200.
        })
        .map(|quad| {
            [
                quad.bounds.origin.x.0,
                quad.bounds.origin.y.0,
                quad.bounds.size.width.0,
                quad.bounds.size.height.0,
            ]
        })
        .collect::<Vec<_>>();
    trace
        .borrow_mut()
        .push(serde_json::json!({"viewport": viewport, "panes": panes, "pane_rects": pane_rects, "dividers": dividers}));
}

impl gpui::Render for ObservedApp {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        let trace = self.trace.clone();
        let previous = self.previous_viewport.clone();
        gpui::div().size_full().child(self.app.clone()).child(
            gpui::canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let viewport = window.viewport_size();
                    // During paint, rendered_frame still holds the last completed
                    // scene. Pair it with that frame's viewport, never this frame's.
                    if let Some(viewport) = previous.replace(Some([
                        f32::from(viewport.width),
                        f32::from(viewport.height),
                    ])) {
                        trace_frame(window, viewport, &trace);
                    }
                },
            )
            .absolute()
            .size_full(),
        )
    }
}

fn capture(
    directory: &std::path::Path,
    label: &str,
    window: &mut Window,
    cx: &mut gpui::App,
) -> anyhow::Result<serde_json::Value> {
    eprintln!("[resize-diagnostic] capture={label}");
    window.refresh();
    let started = Instant::now();
    window.draw(cx).clear(cx);
    let draw_ms = started.elapsed().as_secs_f64() * 1000.;
    let viewport = window.viewport_size();
    let quads = window
        .painted_quads()
        .into_iter()
        .map(|quad| {
            serde_json::json!({
                "rect": [quad.bounds.origin.x.0, quad.bounds.origin.y.0,
                    quad.bounds.size.width.0, quad.bounds.size.height.0],
                "mask": [quad.content_mask.bounds.origin.x.0, quad.content_mask.bounds.origin.y.0,
                    quad.content_mask.bounds.size.width.0, quad.content_mask.bounds.size.height.0],
                "background": format!("{:?}", quad.background),
            })
        })
        .collect::<Vec<_>>();
    std::fs::write(
        directory.join(format!("{label}.quads.json")),
        serde_json::to_vec_pretty(&quads)?,
    )?;
    if let Some(tree) = window.debug_a11y_tree_json() {
        std::fs::write(directory.join(format!("{label}.a11y.json")), tree)?;
    }
    window
        .render_to_image()?
        .save(directory.join(format!("{label}.png")))?;
    Ok(serde_json::json!({
        "label": label,
        "viewport": [f32::from(viewport.width), f32::from(viewport.height)],
        "draw_ms": draw_ms,
        "quad_count": quads.len(),
    }))
}

fn main() -> anyhow::Result<()> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("pass a diagnostic output directory");
    std::fs::create_dir_all(&directory)?;
    let directory = directory.canonicalize()?;
    let fixtures = directory.join("fixtures");
    std::fs::create_dir_all(&fixtures)?;
    for index in 0..24 {
        std::fs::write(
            fixtures.join(format!("resize-fixture-{index:02}.txt")),
            "Resize diagnostic\n",
        )?;
    }
    let config = directory.join("config");
    std::fs::create_dir_all(&config)?;
    std::env::set_var("ATLAS_CONFIG_DIR", &config);
    let mut profiles = atlas_model::profiles::ProfileManager::default();
    profiles.profiles[0].theme = "Atlas Dracula".to_owned();
    std::fs::write(
        config.join("profiles.json"),
        serde_json::to_vec_pretty(&profiles)?,
    )?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .max_blocking_threads(16)
        .enable_all()
        .build()?;
    let rt = runtime.handle().clone();
    let animations = std::env::var_os("RESIZE_PROBE_ANIMATIONS").is_some();
    let split = std::env::var_os("RESIZE_PROBE_SPLIT").is_some();
    let trace: Trace = Default::default();
    let settings = atlas_model::settings::Settings {
        disable_animations: !animations,
        autosave_session: false,
        getting_started_seen: true,
        show_inspector: false,
        sidebar_width: 278.,
        ..Default::default()
    };
    gpui_platform::application()
        .with_assets(AtlasAssets)
        .run(move |cx| {
            gpui_component::init(cx);
            cx.text_system()
                .add_fonts(atlas_ui::assets::bundled_fonts())
                .unwrap();
            ThemeRegistry::global_mut(cx)
                .load_themes_from_str(include_str!("../assets/themes/atlas-themes.json"))
                .unwrap();
            let paint_trace = trace.clone();
            let window = cx
                .open_window(
                    WindowOptions {
                        show: false,
                        focus: false,
                        titlebar: Some(TitleBar::title_bar_options()),
                        window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
                        window_min_size: Some(size(px(900.), px(560.))),
                        ..Default::default()
                    },
                    move |window, cx| {
                        window.set_a11y_forced(true);
                        let app = cx.new(|cx| {
                            AtlasApp::new(rt, settings, Instant::now(), vec![fixtures], window, cx)
                        });
                        let observed = cx.new(|_| ObservedApp {
                            app,
                            trace: paint_trace,
                            previous_viewport: Default::default(),
                        });
                        cx.new(|cx| Root::new(observed, window, cx))
                    },
                )
                .unwrap();
            // A typed handle leases Root during update; drawing would lease it again.
            let window: gpui::AnyWindowHandle = window.into();
            cx.spawn(async move |cx| {
                window
                    .update(cx, |_, window, _| window.resize(size(px(1280.), px(800.))))
                    .unwrap();
                // Drain startup filesystem work with the ordinary foreground executor.
                for tick in 0..30 {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(50))
                        .await;
                    window
                        .update(cx, |_, window, cx| {
                            if split && tick == 10 {
                                window.dispatch_action(Box::new(atlas_ui::SplitHorizontal), cx);
                            }
                            window.simulate_next_frame(cx);
                            window.refresh();
                            window.draw(cx).clear(cx);
                        })
                        .unwrap();
                }
                let result: anyhow::Result<()> = async {
                    let mut samples = vec![window.update(cx, |_, window, cx| {
                        capture(&directory, "baseline", window, cx)
                    })??];
                    for (name, width, height) in [
                        ("grow", 1480., 800.),
                        ("shrink", 1080., 800.),
                        ("tall", 1080., 960.),
                        ("short", 1080., 600.),
                    ] {
                        window.update(cx, |_, window, _| {
                            window.resize(size(px(width), px(height)))
                        })?;
                        // Windows enqueues SetWindowPos on the foreground executor.
                        // Yield to it before recording any of the new-size frames.
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(1))
                            .await;
                        window.update(cx, |_, window, cx| -> anyhow::Result<()> {
                            window.bounds_changed(cx);
                            anyhow::ensure!(
                                (f32::from(window.viewport_size().width) - width).abs() < 1.,
                                "resize did not execute"
                            );
                            anyhow::ensure!(
                                (f32::from(window.viewport_size().height) - height).abs() < 1.,
                                "height resize did not execute"
                            );
                            for frame in 1..=4 {
                                if frame > 1 {
                                    window.simulate_next_frame(cx);
                                }
                                samples.push(capture(
                                    &directory,
                                    &format!("{name}-{frame}"),
                                    window,
                                    cx,
                                )?);
                            }
                            Ok(())
                        })??;
                    }
                    std::fs::write(
                        directory.join("samples.json"),
                        serde_json::to_vec_pretty(&samples)?,
                    )?;
                    eprintln!("[resize-diagnostic] {}", serde_json::to_string(&samples)?);
                    Ok(())
                }
                .await;
                match result {
                    Ok(()) => {}
                    error => {
                        let message = format!("{error:?}");
                        let _ = std::fs::write(directory.join("failed.txt"), &message);
                        eprintln!("[resize-diagnostic] failed: {message}");
                    }
                }
                let _ = std::fs::write(
                    directory.join("frame-trace.json"),
                    serde_json::to_vec_pretty(&*trace.borrow()).unwrap(),
                );
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        });
    Ok(())
}
