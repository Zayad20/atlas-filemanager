//! Cloud domain model.
//!
//! These are inert IDs, configuration records and transfer snapshots. Backend
//! behavior lives in `crate::cloud`; keeping the data here lets state/model stay
//! independent from rclone and filesystem implementation details.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// rclone remote name. Must be unique and free of spaces/`:`; used as the `name:`
/// fs prefix in every rc call.
pub type RemoteId = String;

/// Which protocol a remote speaks. Each variant carries the non-secret connection
/// fields needed to (re)build the rclone config; the password/key is supplied
/// once at creation time and handed straight to rclone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteKind {
    Sftp {
        host: String,
        port: u16,
        user: String,
    },
    Ftp {
        host: String,
        port: u16,
        user: String,
    },
    WebDav {
        url: String,
        vendor: String,
        user: String,
    },
    /// SMB/CIFS share. rclone lists shares as top-level directories.
    Smb {
        host: String,
        port: u16,
        user: String,
    },
    /// Google Drive (OAuth; browser approval, no stored password).
    GoogleDrive,
    /// Microsoft `OneDrive` (OAuth).
    OneDrive,
}

impl RemoteKind {
    /// rclone backend type string (the `type` parameter of `config/create`).
    pub const fn rclone_type(&self) -> &'static str {
        match self {
            Self::Sftp { .. } => "sftp",
            Self::Ftp { .. } => "ftp",
            Self::WebDav { .. } => "webdav",
            Self::Smb { .. } => "smb",
            Self::GoogleDrive => "drive",
            Self::OneDrive => "onedrive",
        }
    }

    /// Short human label for the UI.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Sftp { .. } => "SFTP",
            Self::Ftp { .. } => "FTP",
            Self::WebDav { .. } => "WebDAV",
            Self::Smb { .. } => "SMB",
            Self::GoogleDrive => "Google Drive",
            Self::OneDrive => "OneDrive",
        }
    }

    /// OAuth backends authenticate via the browser instead of a password.
    pub const fn is_oauth(&self) -> bool {
        matches!(self, Self::GoogleDrive | Self::OneDrive)
    }
}

/// How the user wants to reach a remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AccessKind {
    /// Browse/upload/download through the provider; nothing is mounted.
    #[default]
    Access,
    /// FUSE-mounted via rclone so it appears as a real filesystem path.
    Mount,
}

/// Runtime mount/connection status (not persisted).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RemoteStatus {
    #[default]
    Idle,
    Connecting,
    Mounting,
    Mounted(PathBuf),
    /// Drive mounting not available (e.g. WinFsp absent); falling back to
    /// browse-via-API (Access mode). Not an error — click still browses.
    BrowseFallback(String),
    Error(String),
}

/// A configured remote as the app knows it. `status` is runtime-only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudRemote {
    pub id: RemoteId,
    pub name: String,
    pub kind: RemoteKind,
    pub access: AccessKind,
    #[serde(default)]
    pub auto_mount: bool,
    #[serde(skip)]
    pub status: RemoteStatus,
    /// Storage usage `(used, total)` in bytes, fetched after connecting. Runtime only.
    #[serde(skip)]
    pub quota: Option<(u64, u64)>,
}

impl CloudRemote {
    /// The rclone fs prefix for this remote, e.g. `myserver:`.
    #[allow(dead_code)] // reason: mount/access UI uses this when cloud flows are enabled
    pub fn fs(&self) -> String {
        format!("{}:", self.id)
    }
}

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
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed(_))
    }
}

#[derive(Debug, Clone)]
pub struct Transfer {
    pub id: TransferId,
    pub kind: TransferKind,
    /// File name for display.
    #[allow(dead_code)] // reason: transfer manager UI uses this when cloud flows are enabled
    pub label: String,
    pub remote_id: RemoteId,
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
    #[allow(dead_code)] // reason: transfer progress UI uses this when cloud flows are enabled
    pub fn fraction(&self) -> f32 {
        if self.bytes_total == 0 {
            return 0.0;
        }
        (self.bytes_done as f32 / self.bytes_total as f32).clamp(0.0, 1.0)
    }
}

/// Immutable list snapshot handed to the UI each time the manager changes.
pub type TransferSnapshot = Vec<Transfer>;
