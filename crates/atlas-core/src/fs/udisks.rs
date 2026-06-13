#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
//! Removable-disk enumeration + mounting via udisks2 (`udisksctl`, no root —
//! polkit handles authorization). Linux-only; other platforms return empty.
//!
//! Enumeration shells out to `lsblk --json`; mount/unmount to `udisksctl`. All
//! are cheap, dependency-free CLI calls already present on a standard desktop.

use std::path::PathBuf;
use std::process::Command;

/// One removable block device (partition) worth offering in the sidebar.
#[derive(Debug, Clone)]
pub struct RemovableDevice {
    /// Block device node, e.g. `/dev/sdb1` — the handle for udisksctl.
    pub dev: String,
    /// Display label: filesystem label, else the device node basename.
    pub label: String,
    /// Human-readable size from lsblk (e.g. `14.9G`).
    pub size: String,
    /// Mount point if currently mounted.
    pub mount_point: Option<PathBuf>,
}

#[cfg(target_os = "linux")]
mod cache {
    use super::RemovableDevice;
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    thread_local! {
        static CACHE: RefCell<Option<(Instant, Vec<RemovableDevice>)>> = const { RefCell::new(None) };
    }

    /// Cached for a few seconds so the sidebar doesn't fork `lsblk` every frame.
    pub fn list_removable() -> Vec<RemovableDevice> {
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            if let Some((at, v)) = &*c {
                if at.elapsed() < Duration::from_secs(3) {
                    return v.clone();
                }
            }
            let fresh = super::query_lsblk();
            *c = Some((Instant::now(), fresh.clone()));
            fresh
        })
    }

    /// Drop the cache so a mount/unmount is reflected immediately.
    pub fn invalidate() {
        CACHE.with(|c| *c.borrow_mut() = None);
    }
}

#[cfg(target_os = "linux")]
pub use cache::{invalidate, list_removable};

#[cfg(not(target_os = "linux"))]
pub const fn list_removable() -> Vec<RemovableDevice> {
    Vec::new()
}

#[cfg(not(target_os = "linux"))]
pub const fn invalidate() {}

/// Parse `lsblk --json` into the removable partitions we care about.
#[cfg(target_os = "linux")]
fn query_lsblk() -> Vec<RemovableDevice> {
    let output = match Command::new("lsblk")
        .args([
            "--json",
            "-o",
            "PATH,NAME,LABEL,SIZE,FSTYPE,MOUNTPOINT,RM,HOTPLUG,TYPE",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    let json: serde_json::Value = match serde_json::from_slice(&output) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    if let Some(devs) = json.get("blockdevices").and_then(|v| v.as_array()) {
        for dev in devs {
            collect(dev, false, &mut out);
        }
    }
    out
}

/// Recursively gather removable partitions. `parent_removable` propagates the
/// disk's removable/hotplug flag down to its partitions.
#[cfg(target_os = "linux")]
fn collect(node: &serde_json::Value, parent_removable: bool, out: &mut Vec<RemovableDevice>) {
    let truthy = |v: Option<&serde_json::Value>| match v {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "1" || s == "true",
        Some(serde_json::Value::Number(n)) => n.as_i64() == Some(1),
        _ => false,
    };
    let removable = parent_removable || truthy(node.get("rm")) || truthy(node.get("hotplug"));
    let kind = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let fstype = node.get("fstype").and_then(|v| v.as_str());

    if kind == "part" && removable && fstype.is_some() {
        let dev = node
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if !dev.is_empty() {
            let label = node
                .get("label")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    node.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&dev)
                        .to_string()
                });
            let size = node
                .get("size")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let mount_point = node
                .get("mountpoint")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            out.push(RemovableDevice {
                dev,
                label,
                size,
                mount_point,
            });
        }
    }

    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            collect(child, removable, out);
        }
    }
}

/// Mount a device with udisksctl; returns the resulting mount point.
pub fn mount(dev: &str) -> anyhow::Result<PathBuf> {
    let output = Command::new("udisksctl")
        .args(["mount", "-b", dev])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    // Output: "Mounted /dev/sdb1 at /run/media/user/LABEL"
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mount_point = stdout
        .rsplit_once(" at ")
        .map(|(_, p)| p.trim().trim_end_matches('.').to_string())
        .ok_or_else(|| anyhow::anyhow!("could not parse udisksctl output: {stdout:?}"))?;
    Ok(PathBuf::from(mount_point))
}

/// Unmount a device with udisksctl.
pub fn unmount(dev: &str) -> anyhow::Result<()> {
    let output = Command::new("udisksctl")
        .args(["unmount", "-b", dev])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}
