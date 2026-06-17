use crate::app::App;
use crate::fs::transfer::{self, TransferPair};
use crate::state::layout_tree::{Tile, TileId};
use crate::state::ops::{
    ConflictPolicy, Resolution, TransferConflictState, TransferKind, TransferOrigin,
    TransferRequest,
};
use crate::state::undo::UndoEntry;
use std::path::PathBuf;

impl App {
    pub(crate) fn affected_tabs_for_path(&self, path: &PathBuf) -> Vec<TileId> {
        self.state
            .pane_tree
            .tiles
            .iter()
            .filter_map(|(id, tile)| {
                if let Tile::Pane(p) = tile {
                    if let Some(dir) = p.location.fs_dir() {
                        if path.starts_with(dir) || dir.starts_with(path) {
                            return Some(*id);
                        }
                    }
                }
                None
            })
            .collect()
    }

    /// Queue a dismissible error toast (capped to the 5 most recent).
    pub(crate) fn toast_error(&mut self, msg: String) {
        if self.state.overlay.op_errors.len() >= 5 {
            self.state.overlay.op_errors.pop_front();
        }
        self.state.overlay.op_errors.push_back(msg);
    }

    /// Queue an informational toast (non-fatal; reuses the same queue for now).
    pub(crate) fn toast_info(&mut self, msg: String) {
        self.toast_error(msg);
    }

    // -----------------------------------------------------------------------
    // Unified transfer pipeline (paste, drag-and-drop, menu copy/move, undo)
    // -----------------------------------------------------------------------

    /// Single entry point for every copy/move: validates, detects conflicts
    /// (opening the dialog when the policy is `Ask`), then spawns the engine.
    pub(crate) fn request_transfer(&mut self, req: TransferRequest) {
        let outcome = transfer::validate(&req);
        if let Some(fatal) = outcome.fatal {
            self.toast_error(fatal);
            return;
        }
        for err in &outcome.errors {
            self.toast_error(err.to_string());
        }
        if outcome.valid.is_empty() {
            return;
        }
        let (conflicts, clean) = transfer::detect_conflicts(&outcome.valid, &req.dest);
        if !conflicts.is_empty() && req.policy == ConflictPolicy::Ask {
            self.state.overlay.transfer_conflict = Some(TransferConflictState {
                request: TransferRequest {
                    sources: outcome.valid,
                    ..req
                },
                conflicts,
                clean,
                per_item: None,
            });
            return;
        }
        let built = transfer::build_pairs(&clean, &conflicts, &req.policy, &req.dest);
        self.spawn_transfer(built.pairs, req.kind, req.origin);
    }

    /// Apply the user's choice from the conflict dialog. `None` cancels.
    pub(crate) fn apply_transfer_resolution(&mut self, policy: Option<ConflictPolicy>) {
        let Some(state) = self.state.overlay.transfer_conflict.take() else {
            return;
        };
        let Some(policy) = policy else {
            return; // cancelled — sources stay untouched
        };
        let built =
            transfer::build_pairs(&state.clean, &state.conflicts, &policy, &state.request.dest);
        self.spawn_transfer(built.pairs, state.request.kind, state.request.origin);
    }

    /// Spawn the transfer engine over already-resolved pairs.
    pub(crate) fn spawn_transfer(
        &mut self,
        pairs: Vec<TransferPair>,
        kind: TransferKind,
        origin: TransferOrigin,
    ) {
        // A cut is consumed by its paste even if everything else fails later.
        if origin == TransferOrigin::Paste && kind == TransferKind::Move {
            self.state.ops.clipboard = None;
            #[cfg(windows)]
            crate::dnd::windows::ole_clipboard::clear();
        }
        if pairs.is_empty() {
            return;
        }
        let affected: std::collections::HashSet<_> = pairs
            .iter()
            .flat_map(|p| {
                let mut tabs = self.affected_tabs_for_path(&p.src);
                tabs.extend(self.affected_tabs_for_path(&p.dst));
                tabs
            })
            .collect();
        let label = match kind {
            TransferKind::Copy => rust_i18n::t!("op_copying").to_string(),
            TransferKind::Move => rust_i18n::t!("op_moving").to_string(),
        };
        let (op_id, cancel) = self
            .state
            .ops
            .start_op_labeled(pairs.len(), label.to_string());
        let tx = self.tx.clone();
        let affected: Vec<_> = affected.into_iter().collect();
        self.rt.spawn(async move {
            transfer::execute(pairs, kind, origin, op_id, tx, cancel, affected).await;
        });
    }

    /// Re-run exact pairs (undo/redo): no conflict dialog, rename on collision.
    fn spawn_transfer_exact(&mut self, pairs: Vec<(PathBuf, PathBuf)>, kind: TransferKind) {
        let pairs = pairs
            .into_iter()
            .map(|(src, dst)| TransferPair {
                src,
                dst,
                on_conflict: Resolution::Rename,
            })
            .collect();
        self.spawn_transfer(pairs, kind, TransferOrigin::Undo);
    }

    // -----------------------------------------------------------------------
    // Entry-point wrappers
    // -----------------------------------------------------------------------

    /// Set the file clipboard: internal state always; on Windows the system
    /// clipboard too (`CF_HDROP` + preferred effect), so other apps can paste.
    /// `remote_id`: `Some(id)` when paths are remote-relative (not local FS paths).
    pub(crate) fn set_clipboard(&mut self, paths: Vec<PathBuf>, is_cut: bool) {
        self.set_clipboard_with_remote(paths, is_cut, None);
    }

    pub(crate) fn set_clipboard_with_remote(
        &mut self,
        paths: Vec<PathBuf>,
        is_cut: bool,
        remote_id: Option<String>,
    ) {
        // Only put local paths on the system clipboard (CF_HDROP); remote paths
        // are virtual and have no meaning outside Atlas.
        if remote_id.is_none() {
            #[cfg(windows)]
            if let Err(e) = crate::dnd::windows::ole_clipboard::set_clipboard_files(
                &paths,
                is_cut,
                self.tx.clone(),
            ) {
                self.toast_error(rust_i18n::t!("err_clipboard", e = e).to_string());
            }
        }
        self.state.ops.clipboard = Some(crate::state::ops::ClipboardState {
            paths,
            is_cut,
            remote_id,
        });
    }

    pub(crate) fn paste_files(&mut self, dest: PathBuf) {
        // The system clipboard is the source of truth on Windows: it sees files
        // copied in Explorer, and goes stale-aware when another app replaces it.
        #[cfg(windows)]
        let source = {
            match crate::dnd::windows::ole_clipboard::get_clipboard_files() {
                Some(c) if c.from_atlas => {
                    // Our own content: keep the richer internal cut semantics.
                    let is_cut = self.state.ops.clipboard.as_ref().is_some_and(|s| s.is_cut);
                    Some((c.paths, is_cut, TransferOrigin::Paste))
                }
                Some(c) => {
                    let origin = if c.preferred_move {
                        TransferOrigin::PasteExternalCut
                    } else {
                        TransferOrigin::Paste
                    };
                    Some((c.paths, c.preferred_move, origin))
                }
                None => {
                    // No files on the system clipboard (another app replaced
                    // it): our internal cut/copy is stale too.
                    self.state.ops.clipboard = None;
                    None
                }
            }
        };
        #[cfg(not(windows))]
        let source = self
            .state
            .ops
            .clipboard
            .as_ref()
            .map(|c| (c.paths.clone(), c.is_cut, TransferOrigin::Paste));

        let Some((paths, is_cut, origin)) = source else {
            return;
        };
        #[cfg(feature = "cloud")]
        if self.try_remote_upload(&paths, &dest, is_cut) {
            return;
        }
        self.request_transfer(TransferRequest {
            sources: paths,
            dest,
            kind: if is_cut {
                TransferKind::Move
            } else {
                TransferKind::Copy
            },
            policy: ConflictPolicy::Ask,
            origin,
        });
    }

    pub(crate) fn copy_files(
        &mut self,
        paths: Vec<PathBuf>,
        dest: PathBuf,
        origin: TransferOrigin,
    ) {
        #[cfg(feature = "cloud")]
        if self.try_remote_upload(&paths, &dest, false) {
            return;
        }
        self.request_transfer(TransferRequest {
            sources: paths,
            dest,
            kind: TransferKind::Copy,
            policy: ConflictPolicy::Ask,
            origin,
        });
    }

    pub(crate) fn move_files(
        &mut self,
        paths: Vec<PathBuf>,
        dest: PathBuf,
        origin: TransferOrigin,
    ) {
        // Moving into a remote uploads then trashes the local source on success.
        #[cfg(feature = "cloud")]
        if self.try_remote_upload(&paths, &dest, true) {
            return;
        }
        self.request_transfer(TransferRequest {
            sources: paths,
            dest,
            kind: TransferKind::Move,
            policy: ConflictPolicy::Ask,
            origin,
        });
    }

    // -----------------------------------------------------------------------
    // Delete / rename / create
    // -----------------------------------------------------------------------

    pub(crate) fn delete_files(&mut self, paths: Vec<PathBuf>, to_trash: bool) {
        let affected = paths
            .iter()
            .flat_map(|p| self.affected_tabs_for_path(p))
            .collect();
        let (op_id, cancel) = self
            .state
            .ops
            .start_op_labeled(paths.len(), rust_i18n::t!("op_deleting").to_string());
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::ops::delete_files(paths, to_trash, op_id, tx, cancel, affected).await;
        });
    }

    pub(crate) fn rename_file(&mut self, from: PathBuf, to: PathBuf) {
        if let Some(name) = to.file_name().map(|n| n.to_string_lossy().into_owned()) {
            if let Err(e) = transfer::validate_filename(&name) {
                self.toast_error(e);
                return;
            }
        }
        // Renaming onto an existing entry would fail (or worse); only the
        // case-change of the same file is allowed.
        if to.exists() && !transfer::paths_equal_ci(&from, &to) {
            self.toast_error(
                rust_i18n::t!(
                    "err_collision",
                    name = to.file_name().unwrap_or_default().to_string_lossy()
                )
                .to_string(),
            );
            return;
        }
        self.state.ops.undo_stack.push(UndoEntry::Rename {
            from: from.clone(),
            to: to.clone(),
        });
        // Keep any tags attached to the renamed path (and its subtree).
        let _ = self
            .tag_db_tx
            .send(crate::fs::tag_db::TagDbCmd::RenamePath {
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
        if let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) {
            if let Err(e) = transfer::validate_filename(&name) {
                self.toast_error(e);
                return;
            }
        }
        if path.exists() {
            self.toast_error(
                rust_i18n::t!(
                    "err_collision",
                    name = path.file_name().unwrap_or_default().to_string_lossy()
                )
                .to_string(),
            );
            return;
        }
        let op_id = self.state.next_op_id();
        let affected = self.affected_tabs_for_path(&path);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            crate::fs::ops::create_dir(path, op_id, tx, affected).await;
        });
    }

    // -----------------------------------------------------------------------
    // Undo / redo
    // -----------------------------------------------------------------------

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
                UndoEntry::Move { pairs } => {
                    // Reverse each executed pair exactly: dst goes back to src.
                    let rev = pairs.iter().map(|(src, dst)| (dst.clone(), src.clone()));
                    self.spawn_transfer_exact(rev.collect(), TransferKind::Move);
                }
                UndoEntry::Copy { pairs } => {
                    let created: Vec<PathBuf> = pairs.iter().map(|(_, dst)| dst.clone()).collect();
                    self.delete_files(created, true);
                }
            }
            self.state.ops.undo_stack.redo.push_back(entry);
        }
    }

    pub(crate) fn redo(&mut self) {
        if let Some(entry) = self.state.ops.undo_stack.redo.pop_back() {
            match &entry {
                UndoEntry::Move { pairs } => {
                    self.spawn_transfer_exact(pairs.clone(), TransferKind::Move);
                }
                UndoEntry::Copy { pairs } => {
                    self.spawn_transfer_exact(pairs.clone(), TransferKind::Copy);
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
            }
            self.state.ops.undo_stack.undo.push_back(entry);
        }
    }
}
