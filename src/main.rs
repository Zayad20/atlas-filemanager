//! Atlas — thin launcher. The gpui application lives in the `atlas-ui` crate;
//! this binary only sets up logging, the tokio runtime, and hands off to
//! [`atlas_ui::run`].

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

    atlas_ui::run(handle);
}
