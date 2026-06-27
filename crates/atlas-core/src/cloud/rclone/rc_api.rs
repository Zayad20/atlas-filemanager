//! Thin async client over the rclone remote-control (rc) HTTP API.
//!
//! All calls are `POST http://127.0.0.1:<port>/<command>` with a JSON body and
//! HTTP basic auth. We only wrap the handful of endpoints Atlas needs.
//!
//! Some wrappers (e.g. listmounts, `job_stop`) are used only by later phases.
#![allow(dead_code)]

use anyhow::{anyhow, Context};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct RcClient {
    http: reqwest::Client,
    base: String,
    user: String,
    pass: String,
}

/// A running async job handle (from a `_async: true` call).
pub struct AsyncJob {
    pub jobid: i64,
    /// Stats group name we asked rclone to tag this job's transfers with.
    pub group: String,
}

impl RcClient {
    pub fn new(addr: &str, user: &str, pass: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: format!("http://{addr}"),
            user: user.to_string(),
            pass: pass.to_string(),
        }
    }

    async fn call(&self, command: &str, body: Value) -> anyhow::Result<Value> {
        let resp = self
            .http
            .post(format!("{}/{command}", self.base))
            .basic_auth(&self.user, Some(&self.pass))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("rc call {command} failed"))?;
        let status = resp.status();
        let val: Value = resp
            .json()
            .await
            .with_context(|| format!("rc {command}: bad JSON response"))?;
        if !status.is_success() {
            let msg = val
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown rc error");
            return Err(anyhow!("rc {command}: {msg}"));
        }
        Ok(val)
    }

    /// Health check / readiness probe.
    pub async fn version(&self) -> anyhow::Result<String> {
        let v = self.call("core/version", json!({})).await?;
        Ok(v.get("version")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string())
    }

    /// Create (or overwrite) a remote config. `parameters` are backend-specific.
    pub async fn config_create(
        &self,
        name: &str,
        backend_type: &str,
        parameters: Value,
    ) -> anyhow::Result<()> {
        self.call(
            "config/create",
            json!({
                "name": name,
                "type": backend_type,
                "parameters": parameters,
                // Obscure plaintext passwords so credentials are never stored in clear.
                "opt": { "obscure": true },
            }),
        )
        .await
        .map(|_| ())
    }

    /// One step of the interactive config state machine (used for OAuth backends).
    /// Returns the raw response (`State`, `Option`, `Error`). When `State` comes
    /// back empty the config is complete. With `config_is_local` (default) rclone
    /// runs the browser OAuth + loopback capture itself at the auth step.
    pub async fn config_create_step(
        &self,
        name: &str,
        backend_type: &str,
        parameters: Value,
        state: &str,
        result: &str,
    ) -> anyhow::Result<Value> {
        self.call(
            "config/create",
            json!({
                "name": name,
                "type": backend_type,
                "parameters": parameters,
                "opt": {
                    "nonInteractive": true,
                    "continue": !state.is_empty(),
                    "state": state,
                    "result": result,
                    "obscure": true,
                },
            }),
        )
        .await
    }

    pub async fn config_delete(&self, name: &str) -> anyhow::Result<()> {
        self.call("config/delete", json!({ "name": name }))
            .await
            .map(|_| ())
    }

    pub async fn list_remotes(&self) -> anyhow::Result<Vec<String>> {
        let v = self.call("config/listremotes", json!({})).await?;
        Ok(v.get("remotes")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// List a remote directory. `fs` is e.g. `name:`, `remote` the dir path.
    pub async fn operations_list(&self, fs: &str, remote: &str) -> anyhow::Result<Value> {
        self.call("operations/list", json!({ "fs": fs, "remote": remote }))
            .await
    }

    pub async fn operations_mkdir(&self, fs: &str, remote: &str) -> anyhow::Result<()> {
        self.call("operations/mkdir", json!({ "fs": fs, "remote": remote }))
            .await
            .map(|_| ())
    }

    pub async fn operations_deletefile(&self, fs: &str, remote: &str) -> anyhow::Result<()> {
        self.call(
            "operations/deletefile",
            json!({ "fs": fs, "remote": remote }),
        )
        .await
        .map(|_| ())
    }

    pub async fn operations_purge(&self, fs: &str, remote: &str) -> anyhow::Result<()> {
        self.call("operations/purge", json!({ "fs": fs, "remote": remote }))
            .await
            .map(|_| ())
    }

    pub async fn operations_movefile(
        &self,
        src_fs: &str,
        src_remote: &str,
        dst_fs: &str,
        dst_remote: &str,
    ) -> anyhow::Result<()> {
        self.call(
            "operations/movefile",
            json!({ "srcFs": src_fs, "srcRemote": src_remote, "dstFs": dst_fs, "dstRemote": dst_remote }),
        )
        .await
        .map(|_| ())
    }

    /// Start an async single-file copy. Returns the job handle for progress polling.
    pub async fn copyfile_async(
        &self,
        src_fs: &str,
        src_remote: &str,
        dst_fs: &str,
        dst_remote: &str,
        group: &str,
    ) -> anyhow::Result<AsyncJob> {
        let v = self
            .call(
                "operations/copyfile",
                json!({
                    "srcFs": src_fs, "srcRemote": src_remote,
                    "dstFs": dst_fs, "dstRemote": dst_remote,
                    "_async": true, "_group": group,
                }),
            )
            .await?;
        let jobid = v
            .get("jobid")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("copyfile: no jobid returned"))?;
        Ok(AsyncJob {
            jobid,
            group: group.to_string(),
        })
    }

    /// Poll a job. Returns `Some(Ok(()))` when finished OK, `Some(Err)` on failure,
    /// `None` while still running.
    pub async fn job_status(&self, jobid: i64) -> anyhow::Result<Option<anyhow::Result<()>>> {
        let v = self.call("job/status", json!({ "jobid": jobid })).await?;
        let finished = v.get("finished").and_then(Value::as_bool).unwrap_or(false);
        if !finished {
            return Ok(None);
        }
        let success = v.get("success").and_then(Value::as_bool).unwrap_or(false);
        if success {
            Ok(Some(Ok(())))
        } else {
            let err = v
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("transfer failed")
                .to_string();
            Ok(Some(Err(anyhow!(err))))
        }
    }

    pub async fn job_stop(&self, jobid: i64) -> anyhow::Result<()> {
        self.call("job/stop", json!({ "jobid": jobid }))
            .await
            .map(|_| ())
    }

    /// Bytes transferred so far for a stats group: `(bytes, total_bytes)`.
    pub async fn core_stats(&self, group: &str) -> anyhow::Result<(u64, u64)> {
        let v = self.call("core/stats", json!({ "group": group })).await?;
        let bytes = v.get("bytes").and_then(Value::as_u64).unwrap_or(0);
        let total = v.get("totalBytes").and_then(Value::as_u64).unwrap_or(0);
        Ok((bytes, total))
    }

    /// Mount a remote at `mount_point` with VFS full caching (fast random reads).
    pub async fn mount(&self, fs: &str, mount_point: &str, cache_dir: &str) -> anyhow::Result<()> {
        self.call(
            "mount/mount",
            json!({
                "fs": fs,
                "mountPoint": mount_point,
                "vfsOpt": {
                    "CacheMode": "full",
                    "DirCacheTime": "30s",
                    "CacheMaxSize": "2Gi",
                },
                "_config": { "CacheDir": cache_dir },
            }),
        )
        .await
        .map(|_| ())
    }

    /// Storage usage for a remote fs: `(used_bytes, total_bytes)`. Either may be 0
    /// if the backend doesn't report it.
    pub async fn about(&self, fs: &str) -> anyhow::Result<(u64, u64)> {
        let v = self.call("operations/about", json!({ "fs": fs })).await?;
        let used = v.get("used").and_then(Value::as_u64).unwrap_or(0);
        let total = v.get("total").and_then(Value::as_u64).unwrap_or(0);
        Ok((used, total))
    }

    pub async fn unmount(&self, mount_point: &str) -> anyhow::Result<()> {
        self.call("mount/unmount", json!({ "mountPoint": mount_point }))
            .await
            .map(|_| ())
    }
}
