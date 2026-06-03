//! Extension manifest (`extension.toml`) — the declarative contract an extension
//! ships. Parsed into these structs; nothing here runs code.

use serde::Deserialize;

/// How an extension's commands are executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtKind {
    /// Commands are shell-command templates run on the selection. No plugin code.
    #[default]
    Declarative,
    /// Commands are dispatched to a long-lived sidecar binary over JSON-RPC.
    Sidecar,
    /// Implemented in-app (compiled in), e.g. the cloud extension.
    Builtin,
}

/// Optional sidecar process for `kind = "sidecar"`. Parsed now; the JSON-RPC
/// sidecar runner that consumes these is a later phase.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Sidecar {
    /// Binary path relative to the extension directory.
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// What a command accepts as input from the current selection.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Accepts {
    /// File extensions (lowercase, no dot) the command applies to.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Applies to directories.
    #[serde(default)]
    pub dirs: bool,
    /// Applies to anything (overrides the filters above).
    #[serde(default)]
    pub any: bool,
}

impl Accepts {
    /// Does this command apply to a path with the given extension / dir-ness?
    pub fn matches(&self, ext: Option<&str>, is_dir: bool) -> bool {
        if self.any {
            return true;
        }
        if is_dir {
            return self.dirs;
        }
        match ext {
            Some(e) => self.extensions.iter().any(|x| x.eq_ignore_ascii_case(e)),
            None => false,
        }
    }
}

/// A single value prompted from the user before running a command, substituted
/// into the template as `{prompt:NAME}`.
#[derive(Debug, Clone, Deserialize)]
pub struct Prompt {
    pub name: String,
    pub label: String,
    #[serde(default)]
    pub default: String,
}

fn default_true() -> bool {
    true
}

/// One contributed command (palette + context-menu entry).
#[derive(Debug, Clone, Deserialize)]
pub struct ExtCommand {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub accepts: Accepts,
    /// Declarative template. Placeholders: `{input}` `{dir}` `{stem}` `{ext}`
    /// `{output}` (= `{dir}/{stem}_out.{ext}`) and `{prompt:NAME}`.
    #[serde(default)]
    pub run: String,
    /// External binaries that must be on `PATH` for this command to be offered.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Run the template once per selected file (true) or once for the whole set.
    #[serde(default = "default_true")]
    pub per_file: bool,
    #[serde(default)]
    pub prompts: Vec<Prompt>,
}

/// A parsed `extension.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub kind: ExtKind,
    /// Consumed by the (later-phase) sidecar runner.
    #[serde(default)]
    #[allow(dead_code)]
    pub sidecar: Option<Sidecar>,
    #[serde(default)]
    pub commands: Vec<ExtCommand>,
}

impl Manifest {
    pub fn parse(toml_str: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(toml_str)?)
    }
}
