use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::undo::UndoStack;

pub struct ClipboardState {
    pub paths: Vec<PathBuf>,
    pub is_cut: bool,
}

/// How to handle a destination file that already exists when pasting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Replace the existing file/folder.
    Overwrite,
    /// Leave the existing file untouched; don't paste the source.
    Skip,
    /// Paste under an auto-generated unique name ("name (1)").
    Rename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    Copy,
    Move,
}

/// Where a transfer was initiated from. Drives toast wording and per-origin
/// behavior (e.g. cleaning up temp files extracted for an external drop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferOrigin {
    Paste,
    /// Paste of files *cut in another app* (Explorer): on success,
    /// `Performed DropEffect = MOVE` is written back so the source clears its
    /// cut state.
    PasteExternalCut,
    DndInternal,
    DndExternal,
    Menu,
    Undo,
}

/// Conflict handling for an entire transfer. `Ask` holds the transfer and opens
/// the conflict dialog; the rest execute immediately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictPolicy {
    Ask,
    OverwriteAll,
    SkipAll,
    RenameAll,
    /// One [`Resolution`] per conflict, parallel to
    /// [`TransferConflictState::conflicts`].
    PerItem(Vec<Resolution>),
}

/// A single source→destination transfer of one or more files, with all the
/// context needed to validate, resolve conflicts, and execute it.
#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub sources: Vec<PathBuf>,
    pub dest: PathBuf,
    pub kind: TransferKind,
    pub policy: ConflictPolicy,
    pub origin: TransferOrigin,
}

/// One name collision at the destination, with enough detail for the dialog
/// (sizes/dates are stat'ed once at detection time, not in the render loop).
#[derive(Debug, Clone)]
pub struct ConflictItem {
    pub source: PathBuf,
    /// The pre-existing entry at the destination that `source` collides with.
    pub existing: PathBuf,
    pub is_dir: bool,
    pub src_size: Option<u64>,
    pub src_modified: Option<std::time::SystemTime>,
    pub dst_size: Option<u64>,
    pub dst_modified: Option<std::time::SystemTime>,
}

/// Pending transfer that collided with existing names at the destination. While
/// this is `Some`, the conflict dialog is shown and the transfer is held until
/// the user picks a policy (or cancels).
pub struct TransferConflictState {
    pub request: TransferRequest,
    pub conflicts: Vec<ConflictItem>,
    /// Sources with no collision; transferred as-is once the user confirms.
    pub clean: Vec<PathBuf>,
    /// `Some` while the dialog is in "decide one by one" mode: the pending
    /// choice for each conflict, parallel to `conflicts`.
    pub per_item: Option<Vec<Resolution>>,
}

/// A per-item failure surfaced to the user as its own toast.
#[derive(Debug, Clone)]
pub struct OpError {
    pub path: PathBuf,
    pub message: String,
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        write!(f, "{name}: {}", self.message)
    }
}

/// Outcome of an executed transfer. `completed` holds the *real* src→dst pairs
/// (post conflict-resolution), which is what undo records.
#[derive(Debug, Clone, Default)]
pub struct TransferReport {
    pub completed: Vec<(PathBuf, PathBuf)>,
    pub skipped: Vec<PathBuf>,
    pub errors: Vec<OpError>,
    pub cancelled: bool,
}

pub struct OpState {
    pub done: u64,
    pub total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current_file: PathBuf,
    /// Short verb for the status bar ("Copiando", "Moviendo", "Eliminando"…).
    pub label: String,
    /// Set to request cancellation; the worker checks it per file and per chunk.
    pub cancel: Arc<AtomicBool>,
}

pub struct OperationManager {
    pub undo_stack: UndoStack,
    pub clipboard: Option<ClipboardState>,
    pub active_ops: HashMap<u64, OpState>,
    op_id_counter: Arc<AtomicU64>,
}

impl Default for OperationManager {
    fn default() -> Self {
        Self {
            undo_stack: UndoStack::default(),
            clipboard: None,
            active_ops: HashMap::new(),
            op_id_counter: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl OperationManager {
    pub fn next_op_id(&self) -> u64 {
        self.op_id_counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Register a new background operation with a status-bar label; returns the
    /// op ID and the cancellation flag to hand to the worker.
    pub fn start_op_labeled(&mut self, count: usize, label: String) -> (u64, Arc<AtomicBool>) {
        let op_id = self.next_op_id();
        let cancel = Arc::new(AtomicBool::new(false));
        self.active_ops.insert(
            op_id,
            OpState {
                done: 0,
                total: count as u64,
                bytes_done: 0,
                bytes_total: 0,
                current_file: PathBuf::new(),
                label,
                cancel: cancel.clone(),
            },
        );
        (op_id, cancel)
    }
}
