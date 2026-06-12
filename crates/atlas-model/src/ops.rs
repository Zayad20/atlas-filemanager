use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::state::undo::UndoStack;

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

/// Pending paste that collided with existing names at the destination. While this
/// is `Some`, the conflict dialog is shown and the paste is held until the user
/// chooses a [`Resolution`] (or cancels).
pub struct PasteConflictState {
    pub dest: PathBuf,
    pub is_cut: bool,
    /// Sources whose target name already exists at `dest`.
    pub conflicts: Vec<PathBuf>,
    /// Sources with no collision; pasted as-is once the user confirms.
    pub clean: Vec<PathBuf>,
}

pub struct OpState {
    pub done: u64,
    pub total: u64,
    pub current_file: PathBuf,
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

    /// Register a new background operation and return its ID.
    pub fn start_op(&mut self, count: usize) -> u64 {
        let op_id = self.next_op_id();
        self.active_ops.insert(
            op_id,
            OpState {
                done: 0,
                total: count as u64,
                current_file: PathBuf::new(),
            },
        );
        op_id
    }
}
