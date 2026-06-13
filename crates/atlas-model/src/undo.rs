use std::collections::VecDeque;
use std::path::PathBuf;

pub enum UndoEntry {
    /// Exact (source, destination) pairs as executed — survives conflict
    /// renames, where the destination name differs from the source name.
    Move {
        pairs: Vec<(PathBuf, PathBuf)>,
    },
    /// Exact (source, destination) pairs of a copy; undo trashes the
    /// destinations, redo copies again.
    Copy {
        pairs: Vec<(PathBuf, PathBuf)>,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
}

#[derive(Default)]
pub struct UndoStack {
    pub undo: VecDeque<UndoEntry>,
    pub redo: VecDeque<UndoEntry>,
}

impl UndoStack {
    pub fn push(&mut self, entry: UndoEntry) {
        self.redo.clear();
        self.undo.push_back(entry);
        if self.undo.len() > 50 {
            self.undo.pop_front();
        }
    }
}
