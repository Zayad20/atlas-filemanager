//! Archive path sanitization: prevents zip-slip and invalid Windows names.

use std::path::{Component, Path, PathBuf};

const RESERVED_WINDOWS: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn is_reserved_windows(name: &str) -> bool {
    // Matches with or without extension, case-insensitive.
    let stem = name.split('.').next().unwrap_or(name);
    let upper = stem.to_ascii_uppercase();
    RESERVED_WINDOWS.contains(&upper.as_str())
}

fn sanitize_component(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '/' | '\\') || (c as u32) < 32
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Trim trailing dots and spaces (Windows rejects them).
    let trimmed = out.trim_end_matches(|c| c == '.' || c == ' ');
    if trimmed.len() < out.len() {
        out.truncate(trimmed.len());
    }
    // Reserved Windows device names → append underscore.
    if is_reserved_windows(&out) {
        out.push('_');
    }
    out
}

/// Sanitize an inner archive path for safe extraction on Windows.
///
/// Rejects or neutralises:
/// - absolute paths and drive prefixes (`C:`)
/// - `..` traversal components
/// - characters invalid on Windows
/// - reserved device names (CON, NUL, COM1–9, LPT1–9)
/// - trailing dots / spaces per path component
///
/// Returns `None` when the entire path is unsafe (empty after sanitisation).
pub fn sanitize_inner_path(path: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();

    for comp in path.components() {
        match comp {
            Component::RootDir | Component::Prefix(_) => return None,
            Component::CurDir => {}
            Component::ParentDir => return None,
            Component::Normal(os) => {
                let s = os.to_string_lossy();
                // Reject drive-letter prefixes like "C:" embedded in a component.
                if s.len() == 2 && s.as_bytes()[1] == b':' {
                    return None;
                }
                let clean = sanitize_component(&s);
                if clean.is_empty() {
                    return None;
                }
                result.push(clean);
            }
        }
    }

    if result.as_os_str().is_empty() {
        None
    } else {
        Some(result)
    }
}
