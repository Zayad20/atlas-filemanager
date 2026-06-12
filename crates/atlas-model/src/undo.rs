use std::collections::VecDeque;
use std::path::PathBuf;


#[allow(dead_code)]
pub enum UndoEntry {
    Move {
        paths: Vec<PathBuf>,
        dest: PathBuf,
    },
    Copy {
        created: PathBuf,
    },
    Delete {
        original: PathBuf,
        trash_path: PathBuf,
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
