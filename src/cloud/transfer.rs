//! Transfer Manager data types.
//!
//! The orchestration (queue pumping, concurrency, retry) lives in the cloud
//! worker ([`super::worker`]); this module defines the records and the immutable
//! snapshot pushed to the UI via `BgResult::TransferUpdate`.

use std::path::PathBuf;

pub type TransferId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    Download,
    Upload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferState {
    Queued,
    Running,
    Paused,
    Done,
    Failed(String),
}

impl TransferState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, TransferState::Done | TransferState::Failed(_))
    }
}

#[derive(Debug, Clone)]
pub struct Transfer {
    pub id: TransferId,
    pub kind: TransferKind,
    /// File name for display.
    pub label: String,
    pub remote_id: String,
    pub remote_path: String,
    pub local_path: PathBuf,
    pub state: TransferState,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub retries: u32,
    /// After a download completes, open the file with the OS handler.
    pub open_on_done: bool,
    /// After an upload completes, move the local source to trash (move-semantics).
    pub delete_source: bool,
}

impl Transfer {
    pub fn fraction(&self) -> f32 {
        if self.bytes_total == 0 {
            return 0.0;
        }
        (self.bytes_done as f32 / self.bytes_total as f32).clamp(0.0, 1.0)
    }
}

/// Immutable list snapshot handed to the UI each time the manager changes.
pub type TransferSnapshot = Vec<Transfer>;
