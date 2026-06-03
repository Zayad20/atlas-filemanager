//! App-side glue for the extension system: routes `ExtAction`s, runs commands,
//! and applies `BgResult::ExtCommandDone`.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::app::App;
use crate::state::ext_state::PendingCommand;

/// Bundled extensions seeded into the extensions dir on first run.
pub const FILE_TOOLS_MANIFEST: &str = include_str!("../../extensions/file-tools/extension.toml");

/// UI-level extension actions.
#[derive(Debug, Clone)]
pub enum ExtAction {
    OpenManager(bool),
    /// Run a contributed command against the current selection.
    Run { ext_id: String, cmd_id: String },
    /// Run the pending command after its prompts were filled in.
    ConfirmPending,
    CancelPending,
    SetEnabled { id: String, enabled: bool },
    /// Pick a `.zip`/folder and install it.
    Install,
    Uninstall(String),
}

impl App {
    pub(crate) fn handle_ext_action(&mut self, action: ExtAction) {
        match action {
            ExtAction::OpenManager(show) => {
                if show {
                    self.state.ext.reload();
                }
                self.state.ext.show_manager = show;
            }
            ExtAction::Run { ext_id, cmd_id } => self.start_ext_command(&ext_id, &cmd_id),
            ExtAction::ConfirmPending => {
                if let Some(p) = self.state.ext.pending.take() {
                    let answers = p
                        .answers
                        .iter()
                        .map(|(name, _, val)| (name.clone(), val.clone()))
                        .collect();
                    crate::ext::runner::run(&self.rt, self.tx.clone(), p.cmd, p.paths, answers);
                }
            }
            ExtAction::CancelPending => {
                self.state.ext.pending = None;
            }
            ExtAction::SetEnabled { id, enabled } => {
                self.state.ext.enabled.insert(id, enabled);
                self.state.ext.reload();
            }
            ExtAction::Install => self.install_extension(),
            ExtAction::Uninstall(id) => {
                if let Err(e) = crate::ext::install::uninstall(&id) {
                    self.state.ext.last_error = Some(e.to_string());
                } else {
                    self.state.ext.enabled.remove(&id);
                    self.state.ext.reload();
                }
            }
        }
    }

    /// Resolve a command + selection and either run it or open the prompt modal.
    fn start_ext_command(&mut self, ext_id: &str, cmd_id: &str) {
        let Some(cmd) = self.state.ext.registry.command(ext_id, cmd_id) else {
            return;
        };
        let paths = self.state.selected_paths();
        if paths.is_empty() {
            return;
        }
        if cmd.command.prompts.is_empty() {
            crate::ext::runner::run(&self.rt, self.tx.clone(), cmd, paths, HashMap::new());
        } else {
            let answers = cmd
                .command
                .prompts
                .iter()
                .map(|p| (p.name.clone(), p.label.clone(), p.default.clone()))
                .collect();
            self.state.ext.pending = Some(PendingCommand { cmd, paths, answers });
        }
    }

    fn install_extension(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Extension", &["zip"])
            .pick_file()
            .or_else(|| rfd::FileDialog::new().pick_folder())
        else {
            return;
        };
        let result = if path.is_dir() {
            crate::ext::install::install_folder(&path)
        } else {
            crate::ext::install::install_zip(&path)
        };
        match result {
            Ok(id) => {
                self.state.ext.feedback = Some(format!("Installed '{id}'"));
                self.state.ext.reload();
            }
            Err(e) => self.state.ext.last_error = Some(e.to_string()),
        }
    }

    pub(crate) fn handle_ext_bg(&mut self, ext_id: String, error: Option<String>, dirs: Vec<PathBuf>) {
        if let Some(e) = error {
            self.state.ext.last_error = Some(format!("{ext_id}: {e}"));
            tracing::warn!("extension {ext_id} command failed: {e}");
        }
        // Re-list any directory tab whose contents may have changed.
        let targets: Vec<(crate::state::layout_tree::TileId, PathBuf, _, _)> = self
            .state
            .all_pane_tile_ids()
            .into_iter()
            .filter_map(|tile| {
                let td = self.tab_data_for(tile)?;
                (td.source.is_directory() && dirs.contains(&td.path)).then(|| {
                    (tile, td.path.clone(), td.sort_col.clone(), td.sort_dir.clone())
                })
            })
            .collect();
        for (tile, path, col, dir) in targets {
            self.spawn_list_dir(tile, path, col, dir);
        }
    }
}
