use crate::state::app_state::BgResult;
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct WatcherHandle {
    // Holding the debouncer keeps it alive; dropping stops watching
    _debouncer: notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
    pub watched_path: PathBuf,
}

pub fn watch_path(path: &Path, tx: flume::Sender<BgResult>) -> anyhow::Result<WatcherHandle> {
    let (std_tx, std_rx) = std::sync::mpsc::channel();

    let mut debouncer = new_debouncer(Duration::from_millis(300), std_tx)?;
    debouncer
        .watcher()
        .watch(path, RecursiveMode::NonRecursive)?;

    let watched_path = path.to_path_buf();
    std::thread::spawn(move || {
        for event_result in std_rx {
            match event_result {
                Ok(events) => {
                    for ev in events {
                        let _ = tx.send(BgResult::FsEvent(ev.path));
                    }
                }
                Err(_) => break,
            }
        }
    });

    Ok(WatcherHandle {
        _debouncer: debouncer,
        watched_path,
    })
}
