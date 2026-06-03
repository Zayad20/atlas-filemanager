//! Cloud/network remote definitions.
//!
//! A `CloudRemote` describes one configured backend (SFTP/FTP/…). The actual
//! credentials live in rclone's own (obscured) config — we never persist
//! passwords here. Only the non-secret definition is serialized into the app
//! session so the sidebar can show known remotes across restarts.
//!
//! A few accessors (`label`, `fs`) are consumed by later-phase UI/worker code.
#![allow(dead_code)]

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
    Sftp { host: String, port: u16, user: String },
    Ftp { host: String, port: u16, user: String },
    /// SMB/CIFS share. rclone lists shares as top-level directories.
    Smb { host: String, port: u16, user: String },
    /// Google Drive (OAuth; browser approval, no stored password).
    GoogleDrive,
    /// Microsoft OneDrive (OAuth).
    OneDrive,
}

impl RemoteKind {
    /// rclone backend type string (the `type` parameter of `config/create`).
    pub fn rclone_type(&self) -> &'static str {
        match self {
            RemoteKind::Sftp { .. } => "sftp",
            RemoteKind::Ftp { .. } => "ftp",
            RemoteKind::Smb { .. } => "smb",
            RemoteKind::GoogleDrive => "drive",
            RemoteKind::OneDrive => "onedrive",
        }
    }

    /// Short human label for the UI.
    pub fn label(&self) -> &'static str {
        match self {
            RemoteKind::Sftp { .. } => "SFTP",
            RemoteKind::Ftp { .. } => "FTP",
            RemoteKind::Smb { .. } => "SMB",
            RemoteKind::GoogleDrive => "Google Drive",
            RemoteKind::OneDrive => "OneDrive",
        }
    }

    /// OAuth backends authenticate via the browser instead of a password.
    pub fn is_oauth(&self) -> bool {
        matches!(self, RemoteKind::GoogleDrive | RemoteKind::OneDrive)
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
    pub fn fs(&self) -> String {
        format!("{}:", self.id)
    }
}
