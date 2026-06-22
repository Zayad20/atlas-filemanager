//! Extension domain model.
//!
//! These structs describe installed extensions and runnable commands. Discovery,
//! filesystem scans and process execution live in `crate::ext`.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// How an extension's commands are executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtKind {
    /// Commands are shell-command templates run on the selection. No plugin code.
    #[default]
    Declarative,
    /// Commands are dispatched to a long-lived sidecar binary over JSON-RPC.
    Sidecar,
    /// WebAssembly plugins executed via Extism.
    Wasm,
    /// Implemented in-app (compiled in), e.g. the cloud extension.
    Builtin,
    /// Hidden dependency package that contributes portable tools for commands.
    #[serde(rename = "tool_provider")]
    ToolProvider,
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

const fn default_true() -> bool {
    true
}

/// One contributed command (palette + context-menu entry).
#[derive(Debug, Clone, Deserialize)]
pub struct ExtCommand {
    pub id: String,
    #[allow(dead_code)] // reason: extension manager and palette keep this field available
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
    /// When true, this command is a *thumbnailer*: atlas runs it with `{input}` and a
    /// temp `{output}` PNG to generate a grid thumbnail for files it `accepts` (e.g.
    /// video/3D/psd via ffmpeg/ImageMagick). Not shown in the context menu.
    #[serde(default)]
    pub thumbnail: bool,
    /// When true, this command contributes an inspector preview for files it accepts.
    /// Preview commands are background-only and are not shown in command menus.
    #[serde(default)]
    pub preview: bool,
    /// Preview output format. Declarative previews write this to `{output}`; WASM
    /// previews return the same JSON shape directly.
    #[serde(default)]
    pub output: PreviewOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewOutput {
    #[default]
    Json,
    Image,
    Table,
    Text,
    Presentation,
}

/// A parsed `extension.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub id: String,
    #[allow(dead_code)] // reason: extension manager keeps this field available
    pub name: String,
    #[serde(default)]
    #[allow(dead_code)] // reason: extension manager keeps this field available
    pub version: String,
    #[serde(default)]
    #[allow(dead_code)] // reason: extension manager keeps this field available
    pub description: String,
    #[serde(default)]
    pub kind: ExtKind,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    /// Consumed by the (later-phase) sidecar runner.
    #[serde(default)]
    #[allow(dead_code)]
    pub sidecar: Option<Sidecar>,
    /// WebAssembly module filename for `kind = "wasm"` (relative to extension dir).
    #[serde(default)]
    #[allow(dead_code)]
    pub wasm_module: Option<String>,
    #[serde(default)]
    pub commands: Vec<ExtCommand>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub download_url: String,
    #[serde(default)]
    pub archive_path: String,
    #[serde(default)]
    pub archive_format: String,
    #[serde(default)]
    pub signature_url: String,
}

impl Manifest {
    pub fn parse(toml_str: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(toml_str)?)
    }
}

// ── Marketplace / remote registry ────────────────────────────────────────────

/// Metadata for a single extension published in the remote R2 marketplace.
#[derive(Debug, Clone, Deserialize)]
pub struct RemoteExtension {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    pub icon_url: Option<String>,
    pub latest_version: String,
    pub download_url: String,
}

/// The `registry.json` document served from the R2 bucket.
#[derive(Debug, Clone, Deserialize)]
pub struct RemoteRegistry {
    pub last_updated: String,
    pub extensions: HashMap<String, RemoteExtension>,
}

// ── Installed command resolution ──────────────────────────────────────────────

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
    pub tools: HashMap<String, PathBuf>,
}
