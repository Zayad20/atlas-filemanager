//! In-memory registry of installed extensions: builtins (compiled-in, e.g. cloud)
//! plus declarative/sidecar extensions discovered under the extensions directory.

use std::collections::HashMap;
use std::path::PathBuf;

use super::manifest::{ExtCommand, ExtKind, Manifest};

/// A loaded extension and its runtime state.
#[derive(Debug, Clone)]
pub struct Extension {
    pub manifest: Manifest,
    /// Directory containing `extension.toml` (synthetic/empty for builtins).
    pub dir: PathBuf,
    pub enabled: bool,
    pub builtin: bool,
}

/// A command resolved for the current selection, ready to run.
#[derive(Debug, Clone)]
pub struct CommandRef {
    pub ext_id: String,
    /// Extension directory + kind — used by the later-phase sidecar runner.
    #[allow(dead_code)]
    pub ext_dir: PathBuf,
    #[allow(dead_code)]
    pub kind: ExtKind,
    pub command: ExtCommand,
}

#[derive(Default)]
pub struct Registry {
    pub extensions: Vec<Extension>,
}

impl Registry {
    /// Build the registry: compiled-in builtins first, then every valid
    /// `extension.toml` under the extensions directory. `enabled` overrides the
    /// default-enabled state per extension id.
    pub fn load(enabled: &HashMap<String, bool>) -> Self {
        let mut extensions = builtins(enabled);
        let dir = super::extensions_dir();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let ext_dir = entry.path();
                if !ext_dir.is_dir() {
                    continue;
                }
                let manifest_path = ext_dir.join("extension.toml");
                let Ok(text) = std::fs::read_to_string(&manifest_path) else {
                    continue;
                };
                let manifest = match Manifest::parse(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!("extension {ext_dir:?}: bad manifest: {e}");
                        continue;
                    }
                };
                // Builtins win on id collision.
                if extensions.iter().any(|e| e.manifest.id == manifest.id) {
                    continue;
                }
                let enabled = enabled.get(&manifest.id).copied().unwrap_or(true);
                extensions.push(Extension {
                    manifest,
                    dir: ext_dir,
                    enabled,
                    builtin: false,
                });
            }
        }
        Registry { extensions }
    }

    pub fn get(&self, id: &str) -> Option<&Extension> {
        self.extensions.iter().find(|e| e.manifest.id == id)
    }

    // Only consumed by cloud-gated UI today; keep available without the feature.
    #[cfg_attr(not(feature = "cloud"), allow(dead_code))]
    pub fn is_enabled(&self, id: &str) -> bool {
        self.get(id).is_some_and(|e| e.enabled)
    }

    /// All enabled declarative commands as `(ext_id, command)`, unfiltered by
    /// selection. The context menu filters these by the right-clicked paths.
    pub fn declarative_commands(&self) -> Vec<(String, ExtCommand)> {
        self.extensions
            .iter()
            .filter(|e| e.enabled && e.manifest.kind == ExtKind::Declarative)
            .flat_map(|e| {
                let id = e.manifest.id.clone();
                e.manifest
                    .commands
                    .iter()
                    .map(move |c| (id.clone(), c.clone()))
            })
            .collect()
    }

    /// Resolve a specific command to a runnable `CommandRef`.
    pub fn command(&self, ext_id: &str, cmd_id: &str) -> Option<CommandRef> {
        let ext = self.get(ext_id)?;
        let command = ext.manifest.commands.iter().find(|c| c.id == cmd_id)?;
        Some(CommandRef {
            ext_id: ext.manifest.id.clone(),
            ext_dir: ext.dir.clone(),
            kind: ext.manifest.kind,
            command: command.clone(),
        })
    }

    /// Commands offered for a selection, described as `(extension, is_dir)` pairs.
    /// Filters by enabled state, `accepts`, and required binaries being present.
    pub fn commands_for(&self, selection: &[(Option<String>, bool)]) -> Vec<CommandRef> {
        let mut out = Vec::new();
        for ext in self.extensions.iter().filter(|e| e.enabled) {
            if ext.manifest.kind == ExtKind::Builtin {
                continue; // builtins wire their own UI
            }
            for cmd in &ext.manifest.commands {
                let applies = !selection.is_empty()
                    && selection
                        .iter()
                        .all(|(e, is_dir)| cmd.accepts.matches(e.as_deref(), *is_dir));
                if !applies {
                    continue;
                }
                if !cmd.requires.iter().all(|b| has_binary(b)) {
                    continue;
                }
                out.push(CommandRef {
                    ext_id: ext.manifest.id.clone(),
                    ext_dir: ext.dir.clone(),
                    kind: ext.manifest.kind,
                    command: cmd.clone(),
                });
            }
        }
        out
    }
}

/// Compiled-in extensions. Cloud is one when its feature is enabled.
fn builtins(enabled: &HashMap<String, bool>) -> Vec<Extension> {
    let _ = enabled;
    #[cfg(feature = "cloud")]
    {
        vec![Extension {
            manifest: Manifest {
                id: "cloud".into(),
                name: "Cloud Drives".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "Mount & browse SFTP, FTP, SMB, Google Drive, OneDrive".into(),
                kind: ExtKind::Builtin,
                sidecar: None,
                commands: Vec::new(),
            },
            dir: PathBuf::new(),
            enabled: enabled.get("cloud").copied().unwrap_or(true),
            builtin: true,
        }]
    }
    #[cfg(not(feature = "cloud"))]
    Vec::new()
}

/// Is `name` an executable on `PATH`? Cheap PATH scan (no process spawn).
pub fn has_binary(name: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let candidate = dir.join(name);
        candidate.is_file()
            || candidate.with_extension("exe").is_file()
    })
}
