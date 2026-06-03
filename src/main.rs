mod app;
#[cfg(feature = "cloud")]
mod cloud;
mod ext;
mod fs;
mod input;
mod search;
mod state;
mod ui;
mod util;

use gpui::{px, size, App, AppContext as _, KeyBinding, WindowBounds, WindowOptions};
use gpui_component::{Root, TitleBar};

gpui::actions!(Atlas, [Quit]);

fn main() {
    use tracing_subscriber::prelude::*;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer().with_filter(tracing_subscriber::filter::filter_fn(
                |meta| *meta.level() <= tracing::Level::INFO,
            )),
        )
        .init();

    // A file manager's async load is filesystem I/O, not CPU-bound parallelism, so a
    // small fixed pool is plenty. The runtime lives on a parked thread for the life of
    // the process; gpui drives the UI on the main thread.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .thread_name("fp-worker")
        .enable_all()
        .build()
        .expect("tokio runtime");
    let handle = rt.handle().clone();
    std::thread::spawn(move || {
        let _rt = rt;
        loop {
            std::thread::park();
        }
    });

    let application = gpui_platform::application().with_assets(gpui_component_assets::Assets);
    application.run(move |cx: &mut App| {
        gpui_component::init(cx);

        cx.bind_keys([
            #[cfg(target_os = "macos")]
            KeyBinding::new("cmd-q", Quit, None),
            #[cfg(not(target_os = "macos"))]
            KeyBinding::new("ctrl-q", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let window_options = WindowOptions {
            titlebar: Some(TitleBar::title_bar_options()),
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                window.set_window_title("Atlas");
                let view = cx.new(|cx| app::App::new(handle, window, cx));
                // The top-level window view must be `Root` (mount point for
                // gpui-component overlays, modals and notifications).
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}
