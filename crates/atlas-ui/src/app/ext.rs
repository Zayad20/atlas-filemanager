#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
//! App-side glue for the extension system: routes `ExtAction`s, runs commands,
//! and applies `BgResult::ExtCommandDone`.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::app::App;
use crate::state::ext_state::PendingCommand;

/// Bundled extensions seeded into the extensions dir on first run.
pub const FILE_TOOLS_MANIFEST: &str =
    include_str!("../../../../extensions/file-tools/extension.toml");

/// Bundled media-thumbnailer extension (video + PSD/raster grid thumbnails).
pub const MEDIA_THUMBS_MANIFEST: &str =
    include_str!("../../../../extensions/media-thumbs/extension.toml");

/// UI-level extension actions.
#[derive(Debug, Clone)]
pub enum ExtAction {
    OpenManager(bool),
    /// Run a contributed command against the current selection.
    Run {
        ext_id: String,
        cmd_id: String,
    },
    /// Run the pending command after its prompts were filled in.
    ConfirmPending,
    CancelPending,
    SetEnabled {
        id: String,
        enabled: bool,
    },
    /// Pick a `.zip`/folder and install it.
    Install,
    Uninstall(String),
}

impl App {
    pub(crate) fn reload_ext_registry(&mut self) {
        self.ext_registry = crate::ext::Registry::load(&self.state.ext.enabled);
    }

    pub(crate) fn handle_ext_action(&mut self, action: ExtAction) {
        match action {
            ExtAction::OpenManager(show) => {
                if show {
                    self.reload_ext_registry();
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
                self.reload_ext_registry();
            }
            ExtAction::Install => self.install_extension(),
            ExtAction::Uninstall(id) => {
                if let Err(e) = crate::ext::install::uninstall(&id) {
                    self.state.ext.last_error = Some(e.to_string());
                } else {
                    self.state.ext.enabled.remove(&id);
                    self.reload_ext_registry();
                }
            }
        }
    }

    /// Resolve a command + selection and either run it or open the prompt modal.
    fn start_ext_command(&mut self, ext_id: &str, cmd_id: &str) {
        tracing::info!("start_ext_command called with ext_id={}, cmd_id={}", ext_id, cmd_id);
        let Some(cmd) = self.ext_registry.command(ext_id, cmd_id) else {
            tracing::error!("Command not found in registry: ext_id={}, cmd_id={}", ext_id, cmd_id);
            return;
        };
        let paths = self.state.selected_paths();
        if paths.is_empty() && !cmd.command.accepts.any {
            tracing::warn!("Paths are empty and accepts.any is false. Returning early.");
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
            self.state.ext.pending = Some(PendingCommand {
                cmd,
                paths,
                answers,
            });
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
                self.state.ext.feedback = Some(rust_i18n::t!("ext_installed", id = id).to_string());
                self.reload_ext_registry();
            }
            Err(e) => self.state.ext.last_error = Some(e.to_string()),
        }
    }

    pub(crate) fn handle_ext_bg(
        &mut self,
        ext_id: String,
        error: Option<String>,
        dirs: Vec<PathBuf>,
    ) {
        if let Some(e) = error {
            self.state.ext.last_error = Some(format!("{ext_id}: {e}"));
            tracing::warn!("extension {ext_id} command failed: {e}");
        }

        // Si la extensión generó un archivo de mapeos (como un Icon Theme), cargarlo en caché.
        let mappings_path = crate::util::config::data_dir()
            .join("extensions")
            .join(&ext_id)
            .join("theme_mappings.json");

        let mut icons_changed = false;
        if mappings_path.exists() {
            #[allow(clippy::disallowed_methods)] // reason: small JSON read on command completion
            let contents = std::fs::read_to_string(&mappings_path);
            if let Ok(contents) = contents {
                if let Ok(map) =
                    serde_json::from_str::<std::collections::HashMap<String, String>>(&contents)
                {
                    // Persist the selection on the active profile (official
                    // per-profile store) and apply it to the live icon cache.
                    self.persist_icon_selection(map);
                    icons_changed = true;
                }
            }
        }
        // Re-list any directory tab whose contents may have changed.
        let targets: Vec<(crate::state::layout_tree::TileId, PathBuf, _, _)> = self
            .state
            .all_pane_tile_ids()
            .into_iter()
            .filter_map(|tile| {
                let td = self.tab_data_for(tile)?;
                td.location.fs_dir()
                    .filter(|p| icons_changed || dirs.iter().any(|d| d.as_path() == *p))
                    .map(|p| p.to_path_buf())
                    .map(|path| (tile, path, td.sort_col.clone(), td.sort_dir.clone()))
            })
            .collect();
        for (tile, path, col, dir) in targets {
            self.spawn_list_dir(tile, path, col, dir);
        }
    }
}
