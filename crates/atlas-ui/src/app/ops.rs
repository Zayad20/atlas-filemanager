use crate::app::App;
use crate::state::ops::Resolution;
use crate::state::undo::UndoEntry;
use crate::state::layout_tree::{Tile, TileId};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Produce a destination path under `dest` whose name doesn't collide, by
/// inserting " (N)" before the extension: "file.txt" → "file (1).txt".
fn unique_target(dest: &Path, name: &OsStr) -> PathBuf {
    let candidate = dest.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let name = Path::new(name);
    let stem = name
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = name.extension().map(|e| e.to_string_lossy().into_owned());
    for n in 1.. {
        let new_name = match &ext {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = dest.join(new_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

impl App {
    pub(crate) fn affected_tabs_for_path(&self, path: &PathBuf) -> Vec<TileId> {
        self.state
            .pane_tree
            .tiles
            .iter()
            .filter_map(|(id, tile)| {
                if let Tile::Pane(p) = tile {
                    if path.starts_with(&p.path) || p.path.starts_with(path) {
                        return Some(*id);
                    }
                }
                None
            })
            .collect()
    }

    pub(crate) fn delete_files(&mut self, paths: Vec<PathBuf>, to_trash: bool) {
        let affected = paths
            .iter()
            .flat_map(|p| self.affected_tabs_for_path(p))
            .collect();
        let op_id = self.state.start_op(paths.len());

        if to_trash {
            for path in &paths {
                self.state.ops.undo_stack.push(UndoEntry::Delete {
                    original: path.clone(),
                    trash_path: PathBuf::new(), // trash path not easily known beforehand
                });
            }
        }

        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::ops::delete_files(paths, to_trash, op_id, tx, affected).await;
        });
    }

    pub(crate) fn rename_file(&mut self, from: PathBuf, to: PathBuf) {
        self.state.ops.undo_stack.push(UndoEntry::Rename {
            from: from.clone(),
            to: to.clone(),
        });
        // Keep any tags attached to the renamed path (and its subtree).
        let _ = self.tag_db_tx.send(crate::fs::tag_db::TagDbCmd::RenamePath {
            old: from.clone(),
            new: to.clone(),
        });
        let op_id = self.state.next_op_id();
        let affected = self.affected_tabs_for_path(&from);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::ops::rename_file(from, to, op_id, tx, affected).await;
        });
    }

    pub(crate) fn create_dir(&mut self, path: PathBuf) {
        let op_id = self.state.next_op_id();
        let affected = self.affected_tabs_for_path(&path);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::ops::create_dir(path, op_id, tx, affected).await;
        });
    }

    pub(crate) fn paste_files(&mut self, dest: PathBuf) {
        let (paths, is_cut) = match self.state.ops.clipboard.as_ref() {
            Some(c) => (c.paths.clone(), c.is_cut),
            None => return,
        };
        #[cfg(feature = "cloud")]
        if self.try_remote_upload(&paths, &dest, is_cut) {
            return;
        }
        if !dest.is_dir() {
            return;
        }
        // Drop sources that can't be pasted into `dest`: itself, a descendant of a
        // source, and (for cut only) sources already living in `dest` — moving them
        // there is a no-op.
        let paths: Vec<_> = paths
            .into_iter()
            .filter(|path| path != &dest && !dest.starts_with(path))
            .filter(|path| !(is_cut && path.parent() == Some(dest.as_path())))
            .collect();
        if paths.is_empty() {
            return;
        }

        // Split into sources whose target name already exists at `dest` and the rest.
        let (conflicts, clean): (Vec<_>, Vec<_>) = paths
            .into_iter()
            .partition(|path| dest.join(path.file_name().unwrap_or_default()).exists());

        if conflicts.is_empty() {
            self.run_paste(clean, Vec::new(), dest, is_cut, Resolution::Overwrite);
            return;
        }

        // Hold the paste and let the user decide how to resolve the collisions.
        self.state.overlay.paste_conflict = Some(crate::state::ops::PasteConflictState {
            dest,
            is_cut,
            conflicts,
            clean,
        });
    }

    /// Apply the user's choice from the paste-conflict dialog. `None` cancels.
    pub(crate) fn apply_paste_resolution(&mut self, resolution: Option<Resolution>) {
        let Some(state) = self.state.overlay.paste_conflict.take() else {
            return;
        };
        let Some(resolution) = resolution else {
            return; // cancelled
        };
        let crate::state::ops::PasteConflictState {
            dest,
            is_cut,
            conflicts,
            clean,
        } = state;
        // Skip drops the colliding sources entirely; overwrite/rename keep them.
        let conflicts = match resolution {
            Resolution::Skip => Vec::new(),
            _ => conflicts,
        };
        self.run_paste(clean, conflicts, dest, is_cut, resolution);
    }

    /// Build (source, full-destination) pairs and spawn the copy/move. `clean`
    /// sources keep their name; `conflicts` are renamed when `resolution` is
    /// [`Resolution::Rename`], otherwise they overwrite.
    fn run_paste(
        &mut self,
        clean: Vec<PathBuf>,
        conflicts: Vec<PathBuf>,
        dest: PathBuf,
        is_cut: bool,
        resolution: Resolution,
    ) {
        let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
        for src in clean {
            let dst = dest.join(src.file_name().unwrap_or_default());
            pairs.push((src, dst));
        }
        for src in conflicts {
            let name = src.file_name().unwrap_or_default().to_owned();
            let dst = match resolution {
                Resolution::Rename => unique_target(&dest, &name),
                _ => dest.join(&name),
            };
            pairs.push((src, dst));
        }
        if pairs.is_empty() {
            if is_cut {
                self.state.ops.clipboard = None;
            }
            return;
        }
        if is_cut {
            self.state.ops.clipboard = None;
            self.state.ops.undo_stack.push(UndoEntry::Move {
                paths: pairs.iter().map(|(src, _)| src.clone()).collect(),
                dest: dest.clone(),
            });
            self.spawn_move_pairs(pairs, &dest);
        } else {
            self.spawn_copy_pairs(pairs, &dest);
        }
    }

    pub(crate) fn copy_files(&mut self, paths: Vec<PathBuf>, dest: PathBuf) {
        #[cfg(feature = "cloud")]
        if self.try_remote_upload(&paths, &dest, false) {
            return;
        }
        let pairs: Vec<_> = paths
            .into_iter()
            .filter(|path| {
                path.parent() != Some(dest.as_path()) && path != &dest && !dest.starts_with(path)
            })
            .map(|path| {
                let dst = dest.join(path.file_name().unwrap_or_default());
                (path, dst)
            })
            .collect();
        if pairs.is_empty() {
            return;
        }
        self.spawn_copy_pairs(pairs, &dest);
    }

    pub(crate) fn move_files(&mut self, paths: Vec<PathBuf>, dest: PathBuf) {
        // Moving into a remote uploads then trashes the local source on success.
        #[cfg(feature = "cloud")]
        if self.try_remote_upload(&paths, &dest, true) {
            return;
        }
        let paths: Vec<_> = paths
            .into_iter()
            .filter(|path| {
                path.parent() != Some(dest.as_path()) && path != &dest && !dest.starts_with(path)
            })
            .collect();
        if paths.is_empty() {
            return;
        }
        self.state.ops.undo_stack.push(UndoEntry::Move {
            paths: paths.clone(),
            dest: dest.clone(),
        });
        self.spawn_move_files(paths, dest);
    }

    pub(crate) fn spawn_move_files(&mut self, paths: Vec<PathBuf>, dest: PathBuf) {
        let pairs: Vec<_> = paths
            .into_iter()
            .map(|path| {
                let dst = dest.join(path.file_name().unwrap_or_default());
                (path, dst)
            })
            .collect();
        self.spawn_move_pairs(pairs, &dest);
    }

    fn spawn_copy_pairs(&mut self, pairs: Vec<(PathBuf, PathBuf)>, dest: &PathBuf) {
        let affected = self.affected_tabs_for_path(dest);
        let op_id = self.state.start_op(pairs.len());
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::ops::copy_files(pairs, op_id, tx, affected).await;
        });
    }

    fn spawn_move_pairs(&mut self, pairs: Vec<(PathBuf, PathBuf)>, dest: &PathBuf) {
        // Tags follow files moved within the app.
        for (src, dst) in &pairs {
            let _ = self.tag_db_tx.send(crate::fs::tag_db::TagDbCmd::RenamePath {
                old: src.clone(),
                new: dst.clone(),
            });
        }
        let affected: std::collections::HashSet<_> = pairs
            .iter()
            .flat_map(|(src, _)| self.affected_tabs_for_path(src))
            .chain(self.affected_tabs_for_path(dest))
            .collect();
        let op_id = self.state.start_op(pairs.len());
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::ops::move_files(pairs, op_id, tx, affected.into_iter().collect()).await;
        });
    }

    pub(crate) fn undo(&mut self) {
        if let Some(entry) = self.state.ops.undo_stack.undo.pop_back() {
            match &entry {
                UndoEntry::Rename { from, to } => {
                    let from = from.clone();
                    let to = to.clone();
                    let op_id = self.state.next_op_id();
                    let affected = self.affected_tabs_for_path(&to);
                    let tx = self.tx.clone();
                    // Reverse: rename 'to' back to 'from'
                    self.rt.spawn(async move {
                        crate::fs::ops::rename_file(to, from, op_id, tx, affected).await;
                    });
                }
                UndoEntry::Move { paths, dest } => {
                    if let Some(original_parent) = paths.first().and_then(|path| path.parent()) {
                        let moved_paths = paths
                            .iter()
                            .filter_map(|path| path.file_name().map(|name| dest.join(name)))
                            .collect();
                        self.spawn_move_files(moved_paths, original_parent.to_path_buf());
                    }
                }
                _ => {} // Delete undo via trash restore — complex, skip for now
            }
            self.state.ops.undo_stack.redo.push_back(entry);
        }
    }

    pub(crate) fn redo(&mut self) {
        if let Some(entry) = self.state.ops.undo_stack.redo.pop_back() {
            match &entry {
                UndoEntry::Move { paths, dest } => {
                    self.spawn_move_files(paths.clone(), dest.clone());
                }
                UndoEntry::Rename { from, to } => {
                    let op_id = self.state.next_op_id();
                    let affected = self.affected_tabs_for_path(from);
                    let tx = self.tx.clone();
                    let from = from.clone();
                    let to = to.clone();
                    self.rt.spawn(async move {
                        crate::fs::ops::rename_file(from, to, op_id, tx, affected).await;
                    });
                }
                _ => {}
            }
            self.state.ops.undo_stack.undo.push_back(entry);
        }
    }
}
