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
    env_logger::init();

    let rt = tokio::runtime::Builder::new_multi_thread()
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
            .with_transparent(true)
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
