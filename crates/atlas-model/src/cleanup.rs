//! Framework-neutral contracts for the disk analyzer and reviewed cleanup.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanStatus {
    #[default]
    Scanning,
    Paused,
    Complete,
    Partial,
    Cancelled,
    Failed,
}

impl ScanStatus {
    pub const fn terminal(self) -> bool {
        !matches!(self, Self::Scanning | Self::Paused)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiskEntry {
    pub path: PathBuf,
    pub bytes: u64,
    pub directory: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity {
    pub volume: u64,
    pub id: [u8; 16],
    pub bytes: u64,
    pub modified: u64,
    pub links: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AncestorIdentity {
    pub path: PathBuf,
    pub volume: u64,
    pub id: [u8; 16],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleanupCandidate {
    pub path: PathBuf,
    pub identity: FileIdentity,
    pub ancestors: Vec<AncestorIdentity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleanupPlan {
    pub version: u32,
    pub id: String,
    pub created_unix_seconds: u64,
    pub volume_root: PathBuf,
    pub temp_root: PathBuf,
    pub candidates: Vec<CleanupCandidate>,
}

#[derive(Clone, Debug, Default)]
pub struct DiskSnapshot {
    pub generation: u64,
    pub root: PathBuf,
    pub status: ScanStatus,
    pub entries: Vec<DiskEntry>,
    pub large_files: Vec<DiskEntry>,
    pub candidates: Vec<CleanupCandidate>,
    /// Old files a category scan found. They are only inspected (opened) when the
    /// user confirms, so discovery never triggers on-access antivirus scans.
    pub reclaimable: Vec<DiskEntry>,
    pub bytes: u64,
    pub other_bytes: u64,
    pub entry_count: u64,
    pub skipped: u64,
    pub errors: u64,
    pub limited: bool,
    pub message: Option<String>,
    pub capacity: Option<(u64, u64)>,
    pub backend: ScanBackend,
    pub progress: Option<f32>,
    pub fast_scan_available: bool,
    pub backend_note: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScanBackend {
    #[default]
    NativeDirectory,
    NtfsMft,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupResult {
    pub path: PathBuf,
    pub deleted_bytes: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleanupReport {
    pub plan_id: String,
    pub results: Vec<CleanupResult>,
    pub cancelled: bool,
    pub free_before: Option<u64>,
    pub free_after: Option<u64>,
    pub error: Option<String>,
}
