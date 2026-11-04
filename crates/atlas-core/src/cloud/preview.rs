//! Debounced by the UI, bounded and cooperatively cancelled at the provider.
use super::{
    control::{OutcomeUnknown, TransferControl},
    provider::CloudProvider,
};
use atlas_model::{
    app_state::BgResult,
    pane::TabId,
    preview_cache::{DirPreviewEntry, PreviewResult, DIR_PREVIEW_LIMIT},
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

const MAX_BYTES: u64 = 32 * 1024 * 1024;

pub struct PreviewQueue {
    requests: HashMap<TabId, (TransferControl, tokio::task::JoinHandle<()>)>,
    slots: Arc<tokio::sync::Semaphore>,
}
impl Default for PreviewQueue {
    fn default() -> Self {
        Self {
            requests: HashMap::new(),
            slots: Arc::new(tokio::sync::Semaphore::new(2)),
        }
    }
}
impl PreviewQueue {
    pub fn cancel_all(&mut self) {
        for (control, _) in self.requests.drain().map(|(_, request)| request) {
            control.cancel();
        }
    }
    pub fn cancel(&mut self, tab: TabId) {
        if let Some((control, _)) = self.requests.remove(&tab) {
            control.cancel();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn request(
        &mut self,
        rt: &tokio::runtime::Handle,
        provider: Arc<dyn CloudProvider>,
        bg: flume::Sender<BgResult>,
        tab: TabId,
        account: String,
        path: String,
        key: PathBuf,
        size: Option<u64>,
        is_dir: bool,
    ) {
        self.requests.retain(|_, (_, task)| !task.is_finished());
        self.cancel(tab);
        let control = TransferControl::default().with_byte_limit(Some(MAX_BYTES));
        let task_control = control.clone();
        let slots = self.slots.clone();
        let task = rt.spawn(async move {
            let permit = tokio::select! {
                permit = slots.acquire_owned() => permit,
                () = task_control.cancelled() => return,
            };
            let Ok(_permit) = permit else {
                return;
            };
            if task_control.is_cancelled() {
                return;
            }
            let result = load(
                provider,
                tab,
                &account,
                &path,
                size,
                is_dir,
                task_control.clone(),
            )
            .await;
            if !task_control.is_cancelled() {
                let result = result.unwrap_or_else(|error| PreviewResult::Error(error.to_string()));
                let _ = bg.send(BgResult::PreviewReady {
                    tab,
                    path: key,
                    result,
                });
            }
        });
        self.requests.insert(tab, (control, task));
    }
}

async fn load(
    provider: Arc<dyn CloudProvider>,
    tab: TabId,
    account: &str,
    path: &str,
    size: Option<u64>,
    is_dir: bool,
    control: TransferControl,
) -> anyhow::Result<PreviewResult> {
    if is_dir {
        let entries = tokio::select! {
            entries = provider.list(account, path) => entries?,
            () = control.cancelled() => return Err(super::control::TransferCancelled.into()),
        };
        let total_count = entries.len();
        let total_size = entries
            .iter()
            .filter(|entry| !entry.is_dir)
            .fold(0_u64, |sum, entry| sum.saturating_add(entry.size));
        return Ok(PreviewResult::Dir {
            entries: entries
                .into_iter()
                .take(DIR_PREVIEW_LIMIT)
                .map(|entry| DirPreviewEntry {
                    extension: std::path::Path::new(&entry.name)
                        .extension()
                        .map(|value| value.to_string_lossy().into_owned()),
                    name: entry.name,
                    path: entry.path.into(),
                    is_dir: entry.is_dir,
                    size: (!entry.is_dir).then_some(entry.size),
                })
                .collect(),
            total_count,
            total_size,
        });
    }
    anyhow::ensure!(
        size.is_none_or(|size| size <= MAX_BYTES),
        "Download this file to preview it: it exceeds the 32 MiB preview limit"
    );
    let name = atlas_model::remote_path::file_name(path);
    anyhow::ensure!(
        atlas_model::remote_path::safe_local_name(&name),
        "Remote file name is not a safe local name"
    );
    let root = tempfile::Builder::new()
        .prefix("atlas-cloud-preview-")
        .tempdir()?;
    let local = root.path().join(&name);
    let result = provider
        .download_controlled(account, path, &local, Arc::new(|_, _| {}), control.clone())
        .await;
    if result
        .as_ref()
        .is_err_and(|error| error.is::<OutcomeUnknown>())
    {
        let _ = root.keep();
        return result.map(|_| unreachable!());
    }
    result?;
    anyhow::ensure!(
        tokio::fs::metadata(&local).await?.len() <= MAX_BYTES,
        "Remote file grew beyond the preview size limit"
    );
    let (tx, rx) = flume::unbounded();
    crate::fs::preview::load_preview(tab, local, tx).await;
    rx.try_iter()
        .find_map(|result| match result {
            BgResult::PreviewReady { result, .. } => Some(result),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("preview decoder returned no result"))
}
