//! Host for virtual-file-provider sidecars using the versioned JSON-lines API.

use anyhow::{bail, Context, Result};
use atlas_model::provider_protocol::{Event, Request, Response, ResultPayload, VERSION};
use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

pub struct ProviderSidecar {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, flume::Sender<Response>>>>,
    next_id: AtomicU64,
}

impl ProviderSidecar {
    pub fn spawn(
        extension_dir: &Path,
        command: &str,
        args: &[String],
        event_tx: flume::Sender<Event>,
    ) -> Result<Self> {
        let program = confined_program(extension_dir, command)?;
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(extension_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let mut child = cmd.spawn().context("start provider sidecar")?;
        let stdin = Arc::new(Mutex::new(
            child.stdin.take().context("provider stdin unavailable")?,
        ));
        let stdout = child.stdout.take().context("provider stdout unavailable")?;
        let pending: Arc<Mutex<HashMap<u64, flume::Sender<Response>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = pending.clone();

        std::thread::Builder::new()
            .name("atlas-provider-sidecar".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Ok(response) = serde_json::from_str::<Response>(&line) {
                        if let Some(tx) = reader_pending
                            .lock()
                            .ok()
                            .and_then(|mut pending| pending.remove(&response.id))
                        {
                            let _ = tx.send(response);
                        }
                    } else if let Ok(event) = serde_json::from_str::<Event>(&line) {
                        let _ = event_tx.send(event);
                    } else {
                        tracing::warn!("provider emitted invalid JSON: {line}");
                    }
                }
                if let Ok(mut pending) = reader_pending.lock() {
                    for (id, tx) in pending.drain() {
                        let _ = tx.send(Response {
                            version: VERSION,
                            id,
                            result: ResultPayload::Error {
                                message: "provider sidecar exited".into(),
                            },
                        });
                    }
                }
            })
            .context("start provider response reader")?;

        Ok(Self {
            child,
            stdin,
            pending,
            next_id: AtomicU64::new(1),
        })
    }

    pub fn request(
        &self,
        operation: atlas_model::provider_protocol::Operation,
    ) -> Result<Response> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = Request::new(id, operation);
        let (tx, rx) = flume::bounded(1);
        self.pending
            .lock()
            .map_err(|_| anyhow::anyhow!("provider request map poisoned"))?
            .insert(id, tx);

        let write_result = self.stdin.lock().map_or_else(
            |_| Err(anyhow::anyhow!("provider stdin poisoned")),
            |mut stdin| {
                serde_json::to_writer(&mut *stdin, &request)?;
                stdin.write_all(b"\n")?;
                stdin.flush()?;
                Ok(())
            },
        );
        if let Err(error) = write_result {
            self.pending.lock().ok().map(|mut p| p.remove(&id));
            return Err(error);
        }

        rx.recv_timeout(REQUEST_TIMEOUT)
            .with_context(|| format!("provider request {id} timed out"))
    }
}

impl Drop for ProviderSidecar {
    fn drop(&mut self) {
        if let Ok(mut stdin) = self.stdin.lock() {
            let request = Request::new(0, atlas_model::provider_protocol::Operation::Shutdown);
            let _ = serde_json::to_writer(&mut *stdin, &request);
            let _ = stdin.write_all(b"\n");
            let _ = stdin.flush();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn confined_program(extension_dir: &Path, command: &str) -> Result<PathBuf> {
    let root = extension_dir
        .canonicalize()
        .context("canonicalize extension directory")?;
    let candidates = if cfg!(windows) {
        if command.ends_with(".exe") {
            vec![command.to_string()]
        } else {
            vec![format!("{command}.exe"), command.to_string()]
        }
    } else if command.ends_with(".exe") {
        vec![
            command.trim_end_matches(".exe").to_string(),
            command.to_string(),
        ]
    } else {
        vec![command.to_string()]
    };
    let program = candidates
        .into_iter()
        .find_map(|cmd| root.join(cmd).canonicalize().ok())
        .ok_or_else(|| anyhow::anyhow!("resolve sidecar in {:?}", root))?;
    if !program.starts_with(&root) || !program.is_file() {
        bail!("provider sidecar must be a file inside its extension directory");
    }
    Ok(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_cannot_escape_extension_directory() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        assert!(confined_program(root.path(), outside.path().to_str().unwrap()).is_err());
    }
}
