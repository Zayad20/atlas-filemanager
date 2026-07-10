//! [`CloudProvider`] implementation backed by a running rclone daemon.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context};
use serde_json::{json, Value};

use super::rc_api::RcClient;
use crate::cloud::provider::{Capabilities, CloudProvider, ProgressSink, RemoteEntry};

pub struct RcloneProvider {
    rc: RcClient,
    cache_dir: PathBuf,
}

/// True for `\\server\share` style paths (WSL shares, SMB). Never matches on
/// Unix, where paths can't start with a backslash.
fn is_unc(p: &Path) -> bool {
    p.to_string_lossy().starts_with(r"\\")
}

fn speedup_config() -> Value {
    json!({
        "NoCheckDest": true,
        "MultiThreadCutoff": "64Mi",
        "MultiThreadStreams": 4,
    })
}

impl RcloneProvider {
    pub const fn new(rc: RcClient, cache_dir: PathBuf) -> Self {
        Self { rc, cache_dir }
    }

    /// Split a local file path into the `(dstFs, dstRemote)` pair rclone expects:
    /// the containing directory as the fs, the file name as the remote.
    fn local_split(p: &Path) -> anyhow::Result<(String, String)> {
        let parent = p
            .parent()
            .ok_or_else(|| anyhow!("path has no parent: {}", p.display()))?;
        let name = p
            .file_name()
            .ok_or_else(|| anyhow!("path has no file name: {}", p.display()))?;
        Ok((
            parent.to_string_lossy().into_owned(),
            name.to_string_lossy().into_owned(),
        ))
    }

    /// Unique staging path inside the cache dir for a transfer whose local end
    /// is a UNC path rclone can't be trusted with.
    async fn stage_path(&self, target: &Path) -> anyhow::Result<PathBuf> {
        let dir = self.cache_dir.join("unc-staging");
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("creating staging dir {}", dir.display()))?;
        let name = target
            .file_name()
            .ok_or_else(|| anyhow!("path has no file name: {}", target.display()))?;
        Ok(dir.join(format!(
            "{}-{}",
            uuid::Uuid::new_v4().simple(),
            name.to_string_lossy()
        )))
    }

    /// Drive an async copy job to completion, forwarding byte progress.
    async fn run_copy(
        &self,
        src_fs: &str,
        src_remote: &str,
        dst_fs: &str,
        dst_remote: &str,
        progress: ProgressSink,
    ) -> anyhow::Result<()> {
        let group = format!("fp-{}", uuid::Uuid::new_v4().simple());
        let job = self
            .rc
            .copyfile_async(
                src_fs,
                src_remote,
                dst_fs,
                dst_remote,
                &group,
                Some(speedup_config()),
            )
            .await?;
        loop {
            if let Ok((bytes, total)) = self.rc.core_stats(&job.group).await {
                progress(bytes, total);
            }
            match self.rc.job_status(job.jobid).await? {
                Some(Ok(())) => return Ok(()),
                Some(Err(e)) => return Err(e),
                None => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
    }
}

#[async_trait::async_trait]
impl CloudProvider for RcloneProvider {
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
        parameters: Value,
    ) -> anyhow::Result<()> {
        self.rc.config_create(id, backend_type, parameters).await
    }

    async fn configure_oauth(
        &self,
        id: &str,
        backend_type: &str,
        parameters: Value,
    ) -> anyhow::Result<()> {
        super::oauth::run_config_machine(&self.rc, id, backend_type, parameters).await
    }

    async fn deconfigure(&self, id: &str) -> anyhow::Result<()> {
        self.rc.config_delete(id).await
    }

    async fn list(&self, id: &str, path: &str) -> anyhow::Result<Vec<RemoteEntry>> {
        let v = self.rc.operations_list(&format!("{id}:"), path).await?;
        let list = v
            .get("list")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("operations/list: missing list"))?;
        let entries = list
            .iter()
            .map(|e| RemoteEntry {
                name: e
                    .get("Name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                path: e
                    .get("Path")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                is_dir: e.get("IsDir").and_then(Value::as_bool).unwrap_or(false),
                size: e.get("Size").and_then(Value::as_i64).unwrap_or(0).max(0) as u64,
                modified: e
                    .get("ModTime")
                    .and_then(Value::as_str)
                    .and_then(chrono_secs),
                mime: e
                    .get("MimeType")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
            .collect();
        Ok(entries)
    }

    async fn download(
        &self,
        id: &str,
        src: &str,
        dst: &Path,
        progress: ProgressSink,
    ) -> anyhow::Result<()> {
        if let Some(parent) = dst.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        if crate::fs::paths::wsl_unc_parts(dst).is_some() {
            let (dst_fs, dst_remote) = Self::local_split(dst)?;
            let started = Instant::now();
            match self
                .run_copy(
                    &format!("{id}:"),
                    src,
                    &dst_fs,
                    &dst_remote,
                    progress.clone(),
                )
                .await
            {
                Ok(()) => {
                    tracing::debug!(
                        target = %dst.display(),
                        elapsed_ms = started.elapsed().as_millis(),
                        "cloud download used direct WSL copy"
                    );
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(
                        target = %dst.display(),
                        error = %e,
                        "direct WSL download failed; falling back to staging"
                    );
                }
            }
        }
        // rclone's local backend chokes on some UNC destinations (WSL shares via
        // \\wsl.localhost, SMB): stage the download in our cache dir and finish
        // with a plain filesystem copy, which handles UNC fine.
        if is_unc(dst) {
            let staged = self.stage_path(dst).await?;
            let (dst_fs, dst_remote) = Self::local_split(&staged)?;
            self.run_copy(&format!("{id}:"), src, &dst_fs, &dst_remote, progress)
                .await?;
            tokio::fs::copy(&staged, dst)
                .await
                .with_context(|| format!("copying staged download to {}", dst.display()))?;
            let _ = tokio::fs::remove_file(&staged).await;
            return Ok(());
        }
        let (dst_fs, dst_remote) = Self::local_split(dst)?;
        self.run_copy(&format!("{id}:"), src, &dst_fs, &dst_remote, progress)
            .await
    }

    async fn upload(
        &self,
        id: &str,
        src: &Path,
        dst: &str,
        progress: ProgressSink,
    ) -> anyhow::Result<()> {
        if crate::fs::paths::wsl_unc_parts(src).is_some() {
            let (src_fs, src_remote) = Self::local_split(src)?;
            let started = Instant::now();
            match self
                .run_copy(
                    &src_fs,
                    &src_remote,
                    &format!("{id}:"),
                    dst,
                    progress.clone(),
                )
                .await
            {
                Ok(()) => {
                    tracing::debug!(
                        source = %src.display(),
                        elapsed_ms = started.elapsed().as_millis(),
                        "cloud upload used direct WSL copy"
                    );
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(
                        source = %src.display(),
                        error = %e,
                        "direct WSL upload failed; falling back to staging"
                    );
                }
            }
        }
        // Mirror of the download staging: UNC sources (WSL/SMB) go through the
        // cache dir so rclone only ever sees plain local paths.
        if is_unc(src) {
            let staged = self.stage_path(src).await?;
            tokio::fs::copy(src, &staged)
                .await
                .with_context(|| format!("staging UNC upload source {}", src.display()))?;
            let (src_fs, src_remote) = Self::local_split(&staged)?;
            let result = self
                .run_copy(&src_fs, &src_remote, &format!("{id}:"), dst, progress)
                .await;
            let _ = tokio::fs::remove_file(&staged).await;
            return result;
        }
        let (src_fs, src_remote) = Self::local_split(src)?;
        self.run_copy(&src_fs, &src_remote, &format!("{id}:"), dst, progress)
            .await
    }

    async fn about(&self, id: &str) -> anyhow::Result<(u64, u64)> {
        self.rc.about(&format!("{id}:")).await
    }

    async fn mkdir(&self, id: &str, path: &str) -> anyhow::Result<()> {
        self.rc.operations_mkdir(&format!("{id}:"), path).await
    }

    async fn delete(&self, id: &str, path: &str, is_dir: bool) -> anyhow::Result<()> {
        let fs = format!("{id}:");
        if is_dir {
            self.rc.operations_purge(&fs, path).await
        } else {
            self.rc.operations_deletefile(&fs, path).await
        }
    }

    async fn rename(&self, id: &str, from: &str, to: &str) -> anyhow::Result<()> {
        let fs = format!("{id}:");
        self.rc.operations_movefile(&fs, from, &fs, to).await
    }

    async fn copy(&self, id: &str, from: &str, to: &str) -> anyhow::Result<()> {
        let fs = format!("{id}:");
        let group = format!("copy-{}-{}", id, from.len());
        let job = self
            .rc
            .copyfile_async(&fs, from, &fs, to, &group, Some(speedup_config()))
            .await?;
        // Poll until done (no progress reporting for inline copies).
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            match self.rc.job_status(job.jobid).await? {
                Some(Ok(())) => return Ok(()),
                Some(Err(e)) => return Err(e),
                None => {} // still running
            }
        }
    }

    async fn mount(&self, id: &str, mount_point: &Path) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(mount_point)
            .await
            .context("create mount point")?;
        self.rc
            .mount(
                &format!("{id}:"),
                &mount_point.to_string_lossy(),
                &self.cache_dir.to_string_lossy(),
            )
            .await
    }

    async fn unmount(&self, mount_point: &Path) -> anyhow::Result<()> {
        self.rc.unmount(&mount_point.to_string_lossy()).await
    }

    async fn install_mount_support(&self, progress: ProgressSink) -> anyhow::Result<()> {
        #[cfg(windows)]
        {
            return super::windows_mount::install_winfsp(move |done, total| {
                progress(done, total);
            })
            .await;
        }
        #[cfg(not(windows))]
        anyhow::bail!("mount support is provided by the operating system")
    }
}

/// Parse an RFC3339 timestamp into unix seconds without pulling in a date crate.
/// Falls back to `None` on anything unexpected; only the date/time prefix matters.
fn chrono_secs(s: &str) -> Option<i64> {
    // Format: 2026-06-02T13:45:30.000000000Z — parse the calendar fields directly.
    let bytes = s.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let year = num(0, 4)?;
    let month = num(5, 7)?;
    let day = num(8, 10)?;
    let hour = num(11, 13)?;
    let min = num(14, 16)?;
    let sec = num(17, 19)?;
    Some(days_from_civil(year, month, day) * 86400 + hour * 3600 + min * 60 + sec)
}

/// Days since unix epoch for a civil date (Howard Hinnant's algorithm).
const fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
