#![windows_subsystem = "windows"]

//! Atlas — thin launcher. The gpui application lives in the `atlas-ui` crate;
//! this binary only sets up logging, the tokio runtime, and hands off to
//! [`atlas_ui::run`].

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    // Package smoke tests must verify the ELF and version without opening a window.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--version")
    {
        println!(
            "Atlas {}",
            option_env!("ATLAS_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
        );
        return;
    }

    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    let app_started = std::time::Instant::now();
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
        .worker_threads(4)
        .max_blocking_threads(32)
        .thread_name("atlas-worker")
        .enable_all()
        .build();
    let Ok(rt) = rt else {
        tracing::error!("failed to create tokio runtime");
        return;
    };
    let handle = rt.handle().clone();
    std::thread::spawn(move || {
        let _rt = rt;
        loop {
            std::thread::park();
        }
    });

    #[cfg(feature = "dhat-heap")]
    let on_quit = Some(Box::new(move || {
        std::mem::drop(_profiler);
    }) as Box<dyn FnOnce() + 'static>);
    #[cfg(not(feature = "dhat-heap"))]
    let on_quit = None;

    let initial_paths = std::env::args_os().skip(1).map(Into::into).collect();
    atlas_ui::run(handle, app_started, on_quit, initial_paths);
}
