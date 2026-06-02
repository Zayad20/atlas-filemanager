mod app;
mod fs;
mod input;
mod search;
mod state;
mod ui;
mod util;

#[cfg(target_os = "linux")]
fn linux_event_loop_builder() -> Option<eframe::EventLoopBuilderHook> {
    if std::env::var_os("DISPLAY").is_none() || std::env::var_os("ATLAS_USE_WAYLAND").is_some()
    {
        return None;
    }
    Some(Box::new(|builder| {
        use winit::platform::x11::EventLoopBuilderExtX11 as _;
        builder.with_x11();
    }))
}

#[cfg(not(target_os = "linux"))]
fn linux_event_loop_builder() -> Option<eframe::EventLoopBuilderHook> {
    None
}

fn main() -> eframe::Result<()> {
    use tracing_subscriber::prelude::*;
    // egui_winit logs an ERROR every time the OS clipboard is read on Ctrl+V and it
    // doesn't hold text (e.g. an image, or empty). Atlas uses Ctrl+C/V for file
    // operations, so this is expected noise — drop that one target. Keep INFO+.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer().with_filter(
                tracing_subscriber::filter::filter_fn(|meta| {
                    *meta.level() <= tracing::Level::INFO
                        && meta.target() != "egui_winit::clipboard"
                }),
            ),
        )
        .init();

    // A file manager's async load is filesystem I/O, not CPU-bound parallelism, so
    // a small fixed pool is plenty. Capping the workers (instead of one-per-core)
    // cuts startup thread-spawn cost and resident memory. Heavy directory walking
    // already runs on rayon, not here. `enable_all` stays — we need the I/O + time
    // drivers for `tokio::fs` and timers.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .thread_name("fp-worker")
        .enable_all()
        .build()
        .expect("tokio runtime");
    let handle = rt.handle().clone();

    // Keep the runtime alive for the duration of the process.
    // eframe::run_native blocks until the window closes, so we park a thread
    // holding the runtime to prevent it from being dropped.
    std::thread::spawn(move || {
        let _rt = rt;
        loop {
            std::thread::park();
        }
    });

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Atlas")
            .with_inner_size([1280.0, 800.0])
            // The transparent window is a Linux/KDE aesthetic paired with compositor blur.
            // On Windows/macOS (no blur) keep it opaque so the background never shows through.
            .with_transparent(cfg!(target_os = "linux"))
            .with_drag_and_drop(true),
        event_loop_builder: linux_event_loop_builder(),
        ..Default::default()
    };

    eframe::run_native(
        "Atlas",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, handle)))),
    )
}
