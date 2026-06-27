//! Colored file-type icon loading from VSCode-format icon themes (e.g. Flow Icons).
//!
//! Scans the user's VSCode / Insiders extension directories for a recognised
//! icon-theme package, parses the manifest JSON into an extension→SVG-path
//! index. Rasterisation is done by GPUI's SvgRenderer (see `app::icon_loader`)
//! so icons use the same pipeline GPUI uses internally.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::RenderImage;

// ---------------------------------------------------------------------------
// Icon state (mirrors ThumbState from ui::thumbs)
// ---------------------------------------------------------------------------

pub enum IconState {
    Loading,
    Ready(Arc<RenderImage>),
    Missing,
}

// ---------------------------------------------------------------------------
// Theme index
// ---------------------------------------------------------------------------

/// Parsed index from a VSCode icon theme manifest.
///
/// Maps lowercase extensions (no leading dot) to absolute SVG paths.
/// Does no I/O after construction.
pub struct IconThemeIndex {
    ext_to_svg: HashMap<String, PathBuf>,
    /// Fallback for generic files (no/unrecognised extension).
    pub default_file: Option<PathBuf>,
    /// Fallback for directories.
    pub default_folder: Option<PathBuf>,
}

impl IconThemeIndex {
    /// SVG path for `ext` (lowercase, no dot), or `None` if not in the theme.
    pub fn svg_for_ext(&self, ext: &str) -> Option<&Path> {
        self.ext_to_svg
            .get(&ext.to_ascii_lowercase())
            .map(PathBuf::as_path)
    }

    /// Build an index directly from `extension → absolute SVG path` mappings, e.g.
    /// the `theme_mappings.json` an extension (icon-manager) emits after
    /// downloading a VSCode icon theme. Extensions are lowercased to match
    /// [`svg_for_ext`]. Returns `None` when the map is empty so callers can fall
    /// back to auto-discovery.
    pub fn from_mappings(map: HashMap<String, PathBuf>) -> Option<Self> {
        if map.is_empty() {
            return None;
        }
        let ext_to_svg = map
            .into_iter()
            .map(|(ext, path)| (ext.to_ascii_lowercase(), path))
            .collect();
        Some(Self {
            ext_to_svg,
            default_file: None,
            default_folder: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Priority order for theme JSON file stems within a found extension.
const THEME_STEMS: &[&str] = &["dim", "deep", "dawn", "icons"];

/// Scan VSCode/Insiders extension directories for a supported icon theme.
///
/// Returns the first usable theme found; Flow Icons is preferred over other
/// themes.
pub fn find_vscode_theme() -> Option<IconThemeIndex> {
    let home = dirs::home_dir()?;
    for sub in [".vscode-insiders/extensions", ".vscode/extensions"] {
        if let Some(idx) = scan_ext_dir(&home.join(sub)) {
            return Some(idx);
        }
    }
    None
}

fn scan_ext_dir(base: &Path) -> Option<IconThemeIndex> {
    let mut best: Option<PathBuf> = None;
    for entry in std::fs::read_dir(base).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.contains("flow-icons") {
            // Preferred theme — stop searching.
            best = Some(entry.path());
            break;
        }
        if best.is_none() && (name.contains("fluent-icons") || name.contains("material-icon")) {
            best = Some(entry.path());
        }
    }
    let ext_dir = best?;
    for stem in THEME_STEMS {
        let json = ext_dir.join(format!("{stem}.json"));
        if json.exists() {
            if let Some(idx) = parse_vscode_theme_json(&json, &ext_dir) {
                return Some(idx);
            }
        }
    }
    None
}

fn parse_vscode_theme_json(json_path: &Path, ext_dir: &Path) -> Option<IconThemeIndex> {
    let bytes = std::fs::read(json_path).ok()?;
    let theme: serde_json::Value = serde_json::from_slice(&bytes).ok()?;

    let defs = theme["iconDefinitions"].as_object()?;
    let file_exts = theme["fileExtensions"].as_object()?;

    let mut ext_to_svg: HashMap<String, PathBuf> = HashMap::new();
    for (ext, name_val) in file_exts {
        if let Some(path) = name_val
            .as_str()
            .and_then(|n| resolve_svg_path(defs, n, ext_dir))
        {
            ext_to_svg.insert(ext.to_ascii_lowercase(), path);
        }
    }

    if ext_to_svg.is_empty() {
        return None;
    }

    let default_file = theme["file"]
        .as_str()
        .and_then(|n| resolve_svg_path(defs, n, ext_dir));
    let default_folder = theme["folder"]
        .as_str()
        .and_then(|n| resolve_svg_path(defs, n, ext_dir));

    Some(IconThemeIndex {
        ext_to_svg,
        default_file,
        default_folder,
    })
}

fn resolve_svg_path(
    defs: &serde_json::Map<String, serde_json::Value>,
    icon_name: &str,
    ext_dir: &Path,
) -> Option<PathBuf> {
    let rel = defs.get(icon_name)?["iconPath"].as_str()?;
    let abs = ext_dir.join(rel);
    abs.exists().then_some(abs)
}

// ---------------------------------------------------------------------------
// Key helpers
// ---------------------------------------------------------------------------

/// Cache key for a directory entry: `"/"` for folders, lowercase ext or `""`
/// for files without an extension.
pub fn icon_key(kind: &crate::state::file_entry::FileKind, ext: Option<&str>) -> String {
    use crate::state::file_entry::FileKind;
    match kind {
        FileKind::Dir => "/".to_string(),
        FileKind::File | FileKind::Symlink => ext.unwrap_or("").to_ascii_lowercase(),
    }
}

/// Collect the set of unique icon cache keys present in a file listing.
pub fn collect_keys(entries: &[crate::state::file_entry::FileEntry]) -> Vec<String> {
    use crate::state::file_entry::FileKind;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Always pre-warm the generic file icon.
    seen.insert(String::new());
    for e in entries {
        seen.insert(icon_key(&e.kind, e.extension.as_deref()));
    }
    // Ensure folder icon is requested if any dir is present.
    if entries.iter().any(|e| matches!(e.kind, FileKind::Dir)) {
        seen.insert("/".to_string());
    }
    seen.into_iter().collect()
}
