//! Optional native metadata, independent of the chosen content previewer.
use atlas_model::{app_state::BgResult, pane::TabId};
use std::path::PathBuf;

pub async fn load(tab: TabId, path: PathBuf, tx: flume::Sender<BgResult>) {
    let source = path.clone();
    let properties = tokio::task::spawn_blocking(move || {
        #[cfg(windows)]
        {
            atlas_win_space::metadata::extended_properties(&source)
        }
        #[cfg(not(windows))]
        {
            let _ = source;
            Vec::new()
        }
    })
    .await
    .unwrap_or_default();
    let _ = tx.send(BgResult::PreviewMetadataReady {
        tab,
        path,
        properties,
    });
}
