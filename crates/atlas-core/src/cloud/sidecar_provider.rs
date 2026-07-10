use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use atlas_model::provider_protocol::{
    EntryRef, EventPayload, Operation, ProviderEntry, RemoteConfig, ResultPayload,
};

use super::provider::{Capabilities, CloudProvider, ProgressSink, RemoteEntry};
use crate::ext::provider_sidecar::ProviderSidecar;

pub struct SidecarProvider {
    sidecar: Arc<ProviderSidecar>,
    progress: Arc<Mutex<HashMap<String, ProgressSink>>>,
}

impl SidecarProvider {
    pub fn spawn(extension_dir: &Path, command: &str, args: &[String]) -> anyhow::Result<Self> {
        let (event_tx, event_rx) = flume::unbounded();
        let sidecar = Arc::new(ProviderSidecar::spawn(
            extension_dir,
            command,
            args,
            event_tx,
        )?);
        let progress: Arc<Mutex<HashMap<String, ProgressSink>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let event_progress = progress.clone();
        std::thread::Builder::new()
            .name("atlas-provider-events".into())
            .spawn(move || {
                while let Ok(event) = event_rx.recv() {
                    if let EventPayload::Progress {
                        transfer_id,
                        done,
                        total,
                    } = event.event
                    {
                        if let Some(sink) = event_progress
                            .lock()
                            .ok()
                            .and_then(|sinks| sinks.get(&transfer_id).cloned())
                        {
                            sink(done, total);
                        }
                    }
                }
            })?;
        let provider = Self { sidecar, progress };
        provider
            .sidecar
            .request(Operation::Initialize {
                data_dir: extension_dir.join("data").to_string_lossy().into_owned(),
            })
            .context("initialize provider sidecar")?;
        Ok(provider)
    }

    async fn call(&self, operation: Operation) -> anyhow::Result<serde_json::Value> {
        let sidecar = self.sidecar.clone();
        let response = tokio::task::spawn_blocking(move || sidecar.request(operation)).await??;
        match response.result {
            ResultPayload::Ok { value } => Ok(value),
            ResultPayload::Error { message } => anyhow::bail!(message),
        }
    }

    async fn transfer(
        &self,
        id: String,
        operation: Operation,
        sink: ProgressSink,
    ) -> anyhow::Result<()> {
        self.progress
            .lock()
            .map_err(|_| anyhow::anyhow!("provider progress map poisoned"))?
            .insert(id.clone(), sink);
        let result = self.call(operation).await.map(|_| ());
        self.progress.lock().ok().map(|mut sinks| sinks.remove(&id));
        result
    }
}

#[async_trait::async_trait]
impl CloudProvider for SidecarProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            can_mount: true,
            can_write: true,
            random_read: false,
        }
    }

    async fn configure(
        &self,
        id: &str,
        backend_type: &str,
        parameters: serde_json::Value,
    ) -> anyhow::Result<()> {
        self.call(Operation::Configure {
            remote: RemoteConfig {
                id: id.into(),
                name: id.into(),
                kind: backend_type.into(),
                options: parameters,
            },
            secret: String::new(),
        })
        .await
        .map(|_| ())
    }

    async fn configure_oauth(
        &self,
        id: &str,
        backend_type: &str,
        parameters: serde_json::Value,
    ) -> anyhow::Result<()> {
        self.configure(id, backend_type, parameters).await
    }

    async fn deconfigure(&self, id: &str) -> anyhow::Result<()> {
        self.call(Operation::RemoveRemote {
            remote_id: id.into(),
        })
        .await
        .map(|_| ())
    }

    async fn list(&self, id: &str, path: &str) -> anyhow::Result<Vec<RemoteEntry>> {
        let value = self
            .call(Operation::List {
                remote_id: id.into(),
                path: path.into(),
            })
            .await?;
        serde_json::from_value::<Vec<ProviderEntry>>(value)?
            .into_iter()
            .map(|entry| {
                Ok(RemoteEntry {
                    name: entry.name,
                    path: entry.path,
                    is_dir: entry.is_dir,
                    size: entry.size.unwrap_or(0),
                    modified: entry.modified_unix_ms.map(|ms| ms / 1000),
                    mime: None,
                })
            })
            .collect()
    }

    async fn download(
        &self,
        id: &str,
        src: &str,
        dst: &Path,
        progress: ProgressSink,
    ) -> anyhow::Result<()> {
        let transfer_id = uuid::Uuid::new_v4().to_string();
        self.transfer(
            transfer_id.clone(),
            Operation::Download {
                transfer_id,
                remote_id: id.into(),
                source: src.into(),
                destination: dst.to_string_lossy().into_owned(),
            },
            progress,
        )
        .await
    }

    async fn upload(
        &self,
        id: &str,
        src: &Path,
        dst: &str,
        progress: ProgressSink,
    ) -> anyhow::Result<()> {
        let transfer_id = uuid::Uuid::new_v4().to_string();
        self.transfer(
            transfer_id.clone(),
            Operation::Upload {
                transfer_id,
                remote_id: id.into(),
                source: src.to_string_lossy().into_owned(),
                destination: dst.into(),
            },
            progress,
        )
        .await
    }

    async fn about(&self, id: &str) -> anyhow::Result<(u64, u64)> {
        let value = self
            .call(Operation::Quota {
                remote_id: id.into(),
            })
            .await?;
        Ok((
            value["used"].as_u64().unwrap_or(0),
            value["total"].as_u64().unwrap_or(0),
        ))
    }

    async fn mkdir(&self, id: &str, path: &str) -> anyhow::Result<()> {
        self.call(Operation::CreateDirectory {
            remote_id: id.into(),
            path: path.into(),
        })
        .await
        .map(|_| ())
    }

    async fn delete(&self, id: &str, path: &str, is_dir: bool) -> anyhow::Result<()> {
        self.call(Operation::Delete {
            remote_id: id.into(),
            entries: vec![EntryRef {
                path: path.into(),
                is_dir,
            }],
        })
        .await
        .map(|_| ())
    }

    async fn rename(&self, id: &str, from: &str, to: &str) -> anyhow::Result<()> {
        self.call(Operation::Rename {
            remote_id: id.into(),
            from: from.into(),
            to: to.into(),
        })
        .await
        .map(|_| ())
    }

    async fn copy(&self, id: &str, from: &str, to: &str) -> anyhow::Result<()> {
        let destination = Path::new(to)
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_string_lossy()
            .into_owned();
        self.call(Operation::Copy {
            remote_id: id.into(),
            entries: vec![EntryRef {
                path: from.into(),
                is_dir: false,
            }],
            destination,
        })
        .await
        .map(|_| ())
    }

    async fn mount(&self, id: &str, mount_point: &Path) -> anyhow::Result<()> {
        self.call(Operation::Mount {
            remote_id: id.into(),
            mount_point: mount_point.to_string_lossy().into_owned(),
        })
        .await
        .map(|_| ())
    }

    async fn unmount(&self, mount_point: &Path) -> anyhow::Result<()> {
        self.call(Operation::Unmount {
            remote_id: String::new(),
            mount_point: mount_point.to_string_lossy().into_owned(),
        })
        .await
        .map(|_| ())
    }

    async fn install_mount_support(&self, progress: ProgressSink) -> anyhow::Result<()> {
        let transfer_id = uuid::Uuid::new_v4().to_string();
        self.progress
            .lock()
            .map_err(|_| anyhow::anyhow!("provider progress map poisoned"))?
            .insert(transfer_id.clone(), progress);
        let result = self
            .call(Operation::InstallMountSupport {
                transfer_id: transfer_id.clone(),
            })
            .await
            .map(|_| ());
        self.progress
            .lock()
            .ok()
            .map(|mut sinks| sinks.remove(&transfer_id));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn real_cloud_sidecar_round_trip_when_enabled() {
        let Some(dir) = std::env::var_os("ATLAS_CLOUD_EXTENSION_DIR").map(PathBuf::from) else {
            return;
        };
        let provider = SidecarProvider::spawn(
            &dir,
            if cfg!(windows) {
                "atlas-cloud-extension.exe"
            } else {
                "atlas-cloud-extension"
            },
            &[],
        )
        .unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let error = runtime
            .block_on(provider.list("atlas-test-missing", ""))
            .unwrap_err();
        assert!(!error.to_string().is_empty());
    }
}
