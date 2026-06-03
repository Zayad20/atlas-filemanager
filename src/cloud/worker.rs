//! Background cloud worker: owns the rclone daemon + provider and serializes all
//! cloud operations, replying through the shared `BgResult` channel.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use serde_json::json;

use crate::cloud::provider::{CloudProvider, ProgressSink, RemoteEntry};
use crate::cloud::rclone::rc_api::RcClient;
use crate::cloud::rclone::{sidecar, RcloneProvider};
use crate::cloud::remote::{CloudRemote, RemoteId, RemoteKind, RemoteStatus};
use crate::cloud::transfer::{Transfer, TransferId, TransferKind, TransferState};
use crate::state::app_state::BgResult;
use crate::state::file_entry::{FileEntry, FileKind};
use crate::state::pane::TabId;

const MAX_CONCURRENT: usize = 4;

/// Commands the UI sends to the cloud worker.
pub enum CloudCmd {
    /// Create/overwrite a remote's rclone config. `secret` is the password/key
    /// passphrase (obscured by rclone, never stored by us).
    AddRemote {
        remote: CloudRemote,
        secret: String,
    },
    RemoveRemote(RemoteId),
    /// List a remote directory; result is delivered as `BgResult::DirListed` so
    /// it renders through the normal file views.
    ListRemote {
        tab: TabId,
        op_id: u64,
        id: RemoteId,
        path: String,
    },
    Mount {
        id: RemoteId,
    },
    Unmount {
        id: RemoteId,
    },
    EnqueueDownload {
        id: RemoteId,
        remote_path: String,
        local_path: PathBuf,
        label: String,
        open_on_done: bool,
    },
    /// Upload a local file into a remote (drag/paste into an Access-mode tab).
    EnqueueUpload {
        id: RemoteId,
        local_path: PathBuf,
        remote_path: String,
        label: String,
        /// Move-semantics: trash the local source after a successful upload.
        delete_source: bool,
    },
    PauseTransfer(TransferId),
    ResumeTransfer(TransferId),
    CancelTransfer(TransferId),
    RetryTransfer(TransferId),
}

/// Internal progress/result events from spawned transfer tasks.
enum Ev {
    Progress { id: TransferId, done: u64, total: u64 },
    Done(TransferId),
    Failed { id: TransferId, err: String },
}

/// Spawn the cloud worker. Returns the command channel.
pub fn spawn(rt: &tokio::runtime::Handle, bg_tx: flume::Sender<BgResult>) -> flume::Sender<CloudCmd> {
    let (cmd_tx, cmd_rx) = flume::unbounded::<CloudCmd>();
    let (ev_tx, ev_rx) = flume::unbounded::<Ev>();
    let handle = rt.clone();
    rt.spawn(async move {
        let mut worker = Worker {
            rt: handle,
            bg_tx,
            ev_tx,
            provider: None,
            daemon: None,
            transfers: Vec::new(),
            running: HashMap::new(),
            next_id: 1,
        };
        loop {
            tokio::select! {
                cmd = cmd_rx.recv_async() => match cmd {
                    Ok(cmd) => worker.handle_cmd(cmd).await,
                    Err(_) => break, // all senders dropped → app exiting
                },
                ev = ev_rx.recv_async() => {
                    if let Ok(ev) = ev { worker.handle_ev(ev); }
                }
            }
            worker.pump_queue();
        }
    });
    cmd_tx
}

struct Worker {
    rt: tokio::runtime::Handle,
    bg_tx: flume::Sender<BgResult>,
    ev_tx: flume::Sender<Ev>,
    provider: Option<Arc<dyn CloudProvider>>,
    daemon: Option<sidecar::Daemon>,
    transfers: Vec<Transfer>,
    running: HashMap<TransferId, tokio::task::JoinHandle<()>>,
    next_id: TransferId,
}

impl Worker {
    /// Lazily download rclone + start the daemon on first use. Cached afterwards.
    async fn ensure_ready(&mut self) -> anyhow::Result<Arc<dyn CloudProvider>> {
        if let Some(p) = &self.provider {
            return Ok(p.clone());
        }
        let bg = self.bg_tx.clone();
        let progress: sidecar::DownloadProgress = Box::new(move |done, total| {
            let _ = bg.send(BgResult::CloudRcloneProgress { done, total });
        });
        let binary = sidecar::ensure_binary(progress).await?;
        let daemon = sidecar::spawn_daemon(&binary)?;
        let rc = RcClient::new(&daemon.addr, &daemon.user, &daemon.pass);

        // Wait for the daemon to accept connections (poll core/version briefly).
        let mut ready = false;
        for _ in 0..40 {
            if rc.version().await.is_ok() {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !ready {
            return Err(anyhow::anyhow!("rclone daemon did not become ready"));
        }
        // Own the daemon for the worker's (process) lifetime; dropping it (on
        // channel close at app exit) kills the rcd child and its mounts.
        self.daemon = Some(daemon);

        let cache_dir = crate::util::config::data_dir().join("rclone-cache");
        let provider: Arc<dyn CloudProvider> = Arc::new(RcloneProvider::new(rc, cache_dir));
        self.provider = Some(provider.clone());
        let _ = self.bg_tx.send(BgResult::CloudReady);
        Ok(provider)
    }

    async fn handle_cmd(&mut self, cmd: CloudCmd) {
        match cmd {
            CloudCmd::AddRemote { remote, secret } => self.add_remote(remote, secret).await,
            CloudCmd::RemoveRemote(id) => self.remove_remote(id).await,
            CloudCmd::ListRemote { tab, op_id, id, path } => {
                self.list_remote(tab, op_id, id, path).await
            }
            CloudCmd::Mount { id } => self.mount(id).await,
            CloudCmd::Unmount { id } => self.unmount(id).await,
            CloudCmd::EnqueueDownload {
                id,
                remote_path,
                local_path,
                label,
                open_on_done,
            } => {
                let t = Transfer {
                    id: self.alloc_id(),
                    kind: TransferKind::Download,
                    label,
                    remote_id: id,
                    remote_path,
                    local_path,
                    state: TransferState::Queued,
                    bytes_done: 0,
                    bytes_total: 0,
                    retries: 0,
                    open_on_done,
                    delete_source: false,
                };
                self.transfers.push(t);
                self.emit_transfers();
            }
            CloudCmd::EnqueueUpload {
                id,
                local_path,
                remote_path,
                label,
                delete_source,
            } => {
                let t = Transfer {
                    id: self.alloc_id(),
                    kind: TransferKind::Upload,
                    label,
                    remote_id: id,
                    remote_path,
                    local_path,
                    state: TransferState::Queued,
                    bytes_done: 0,
                    bytes_total: 0,
                    retries: 0,
                    open_on_done: false,
                    delete_source,
                };
                self.transfers.push(t);
                self.emit_transfers();
            }
            CloudCmd::PauseTransfer(id) => self.set_state(id, |s| {
                if *s == TransferState::Queued {
                    *s = TransferState::Paused;
                }
            }),
            CloudCmd::ResumeTransfer(id) => self.set_state(id, |s| {
                if *s == TransferState::Paused {
                    *s = TransferState::Queued;
                }
            }),
            CloudCmd::RetryTransfer(id) => {
                if let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) {
                    if matches!(t.state, TransferState::Failed(_)) {
                        t.state = TransferState::Queued;
                        t.bytes_done = 0;
                        t.retries += 1;
                    }
                }
                self.emit_transfers();
            }
            CloudCmd::CancelTransfer(id) => {
                if let Some(h) = self.running.remove(&id) {
                    h.abort();
                }
                self.transfers.retain(|t| t.id != id);
                self.emit_transfers();
            }
        }
    }

    fn alloc_id(&mut self) -> TransferId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn set_state(&mut self, id: TransferId, f: impl Fn(&mut TransferState)) {
        if let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) {
            f(&mut t.state);
        }
        self.emit_transfers();
    }

    async fn add_remote(&mut self, remote: CloudRemote, secret: String) {
        let provider = match self.ensure_ready().await {
            Ok(p) => p,
            Err(e) => {
                let _ = self.bg_tx.send(BgResult::CloudError(e.to_string()));
                return;
            }
        };
        // Build backend params from the remote kind + secret and let the provider
        // persist them (passwords are obscured by the backend, never stored by us).
        let id = remote.id.clone();
        let is_oauth = remote.kind.is_oauth();
        let params = match &remote.kind {
            RemoteKind::Sftp { host, port, user } => json!({
                "host": host, "port": port.to_string(), "user": user, "pass": secret,
            }),
            RemoteKind::Ftp { host, port, user } => json!({
                "host": host, "port": port.to_string(), "user": user, "pass": secret,
            }),
            RemoteKind::Smb { host, port, user } => json!({
                "host": host, "port": port.to_string(), "user": user, "pass": secret,
            }),
            // OAuth backends take no secret; rclone's built-in client id is used.
            RemoteKind::GoogleDrive => json!({ "scope": "drive" }),
            RemoteKind::OneDrive => json!({}),
        };
        let backend = remote.kind.rclone_type().to_string();
        let bg = self.bg_tx.clone();
        let res = if is_oauth {
            provider.as_ref().configure_oauth(&id, &backend, params).await
        } else {
            provider.as_ref().configure(&id, &backend, params).await
        };
        match res {
            Ok(()) => {
                let _ = bg.send(BgResult::RemoteConfigured(remote));
                self.fetch_quota(provider, id);
            }
            Err(e) => {
                let _ = bg.send(BgResult::RemoteConfigFailed {
                    id,
                    error: e.to_string(),
                });
            }
        }
    }

    /// Fetch storage usage in the background and report it (best-effort).
    fn fetch_quota(&self, provider: Arc<dyn CloudProvider>, id: RemoteId) {
        let bg = self.bg_tx.clone();
        self.rt.spawn(async move {
            if let Ok((used, total)) = provider.about(&id).await {
                if total > 0 || used > 0 {
                    let _ = bg.send(BgResult::RemoteQuota { id, used, total });
                }
            }
        });
    }

    async fn remove_remote(&mut self, id: RemoteId) {
        if let Ok(provider) = self.ensure_ready().await {
            let _ = provider.as_ref().deconfigure(&id).await;
        }
        let _ = self.bg_tx.send(BgResult::RemoteRemoved(id));
    }

    async fn list_remote(&mut self, tab: TabId, op_id: u64, id: RemoteId, path: String) {
        let provider = match self.ensure_ready().await {
            Ok(p) => p,
            Err(e) => {
                let _ = self.bg_tx.send(BgResult::DirListError {
                    tab,
                    op_id,
                    error: e.to_string(),
                });
                return;
            }
        };
        let bg = self.bg_tx.clone();
        self.rt.spawn(async move {
            match provider.list(&id, &path).await {
                Ok(entries) => {
                    let mapped = entries.into_iter().map(remote_to_file_entry).collect();
                    let _ = bg.send(BgResult::DirListed {
                        tab,
                        op_id,
                        entries: mapped,
                    });
                }
                Err(e) => {
                    let _ = bg.send(BgResult::DirListError {
                        tab,
                        op_id,
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    async fn mount(&mut self, id: RemoteId) {
        let provider = match self.ensure_ready().await {
            Ok(p) => p,
            Err(e) => {
                let _ = self.bg_tx.send(BgResult::RemoteStatusChanged {
                    id,
                    status: RemoteStatus::Error(e.to_string()),
                });
                return;
            }
        };
        let mount_point = crate::cloud::mount_point_for(&id);
        let _ = self.bg_tx.send(BgResult::RemoteStatusChanged {
            id: id.clone(),
            status: RemoteStatus::Mounting,
        });
        let bg = self.bg_tx.clone();
        self.rt.spawn(async move {
            let status = match provider.mount(&id, &mount_point).await {
                Ok(()) => RemoteStatus::Mounted(mount_point),
                Err(e) => RemoteStatus::Error(e.to_string()),
            };
            let _ = bg.send(BgResult::RemoteStatusChanged { id, status });
        });
    }

    async fn unmount(&mut self, id: RemoteId) {
        let provider = match self.ensure_ready().await {
            Ok(p) => p,
            Err(_) => return,
        };
        let mount_point = crate::cloud::mount_point_for(&id);
        let bg = self.bg_tx.clone();
        self.rt.spawn(async move {
            let _ = provider.unmount(&mount_point).await;
            let _ = bg.send(BgResult::RemoteStatusChanged {
                id,
                status: RemoteStatus::Idle,
            });
        });
    }

    fn handle_ev(&mut self, ev: Ev) {
        match ev {
            Ev::Progress { id, done, total } => {
                if let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) {
                    t.bytes_done = done;
                    if total > 0 {
                        t.bytes_total = total;
                    }
                }
            }
            Ev::Done(id) => {
                self.running.remove(&id);
                if let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) {
                    t.state = TransferState::Done;
                    t.bytes_done = t.bytes_total;
                    if t.open_on_done {
                        let _ = open::that_detached(&t.local_path);
                    }
                    if t.kind == TransferKind::Upload {
                        // Refresh any tab showing the destination dir, and (for a
                        // move) trash the now-uploaded local source.
                        let dir = t
                            .remote_path
                            .rsplit_once('/')
                            .map(|(d, _)| d.to_string())
                            .unwrap_or_default();
                        let _ = self.bg_tx.send(BgResult::RemoteChanged {
                            id: t.remote_id.clone(),
                            path: dir,
                        });
                        if t.delete_source {
                            let local = t.local_path.clone();
                            let bg = self.bg_tx.clone();
                            self.rt.spawn_blocking(move || {
                                let _ = trash::delete(&local);
                                // Refresh the local source dir tab now the file is gone.
                                let _ = bg.send(BgResult::FsEvent(local));
                            });
                        }
                    }
                }
            }
            Ev::Failed { id, err } => {
                self.running.remove(&id);
                if let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) {
                    t.state = TransferState::Failed(err);
                }
            }
        }
        self.emit_transfers();
    }

    /// Start queued transfers up to the concurrency cap.
    fn pump_queue(&mut self) {
        if self.running.len() >= MAX_CONCURRENT {
            return;
        }
        let provider = match &self.provider {
            Some(p) => p.clone(),
            None => return,
        };
        let ids: Vec<TransferId> = self
            .transfers
            .iter()
            .filter(|t| t.state == TransferState::Queued)
            .map(|t| t.id)
            .collect();
        for id in ids {
            if self.running.len() >= MAX_CONCURRENT {
                break;
            }
            let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) else {
                continue;
            };
            t.state = TransferState::Running;
            let (kind, rid, rpath, lpath) = (
                t.kind,
                t.remote_id.clone(),
                t.remote_path.clone(),
                t.local_path.clone(),
            );
            let ev_tx = self.ev_tx.clone();
            let p = provider.clone();
            let handle = self.rt.spawn(async move {
                let sink_tx = ev_tx.clone();
                let sink: ProgressSink = Box::new(move |done, total| {
                    let _ = sink_tx.send(Ev::Progress { id, done, total });
                });
                let res = match kind {
                    TransferKind::Download => p.download(&rid, &rpath, &lpath, sink).await,
                    TransferKind::Upload => p.upload(&rid, &lpath, &rpath, sink).await,
                };
                match res {
                    Ok(()) => {
                        let _ = ev_tx.send(Ev::Done(id));
                    }
                    Err(e) => {
                        let _ = ev_tx.send(Ev::Failed {
                            id,
                            err: e.to_string(),
                        });
                    }
                }
            });
            self.running.insert(id, handle);
        }
        self.emit_transfers();
    }

    fn emit_transfers(&self) {
        let _ = self
            .bg_tx
            .send(BgResult::TransferUpdate(self.transfers.clone()));
    }
}

/// Map a provider entry to a `FileEntry` for the file views. The path is the
/// remote-relative path (a stable key); the owning tab's `TabSource::Remote`
/// marks it as remote so navigation/open route through the worker.
fn remote_to_file_entry(e: RemoteEntry) -> FileEntry {
    let kind = if e.is_dir {
        FileKind::Dir
    } else {
        FileKind::File
    };
    let extension = if e.is_dir {
        None
    } else {
        e.name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase())
    };
    let modified = e
        .modified
        .and_then(|secs| u64::try_from(secs).ok())
        .map(|secs| UNIX_EPOCH + Duration::from_secs(secs));
    FileEntry {
        name_lower: e.name.to_ascii_lowercase(),
        name: e.name,
        path: PathBuf::from(e.path),
        kind,
        size: (!e.is_dir).then_some(e.size),
        modified,
        extension,
        _is_hidden: false,
        _is_symlink: false,
    }
}
