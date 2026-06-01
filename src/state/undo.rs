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

#[allow(dead_code)]
#[derive(Default)]
pub struct UndoStack {
    pub undo: Vec<UndoEntry>,
    pub redo: Vec<UndoEntry>,
}

impl UndoStack {
    pub fn push(&mut self, entry: UndoEntry) {
        self.redo.clear();
        self.undo.push(entry);
        if self.undo.len() > 50 {
            self.undo.remove(0);
        }
    }
}
