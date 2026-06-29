//! Shared helpers for all archive backends.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use atlas_model::file_entry::FileEntry;

use super::make_entry;

/// Convert a path to an index key: empty string for root, `"dir/"` otherwise.
pub(super) fn path_to_dir_key(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    if s.is_empty() {
        String::new()
    } else {
        format!("{s}/")
    }
}

/// Ensure every ancestor directory of `inner` exists in `index` and has a
/// synthetic `Dir` entry in its parent's listing.
///
/// Required for archives that omit explicit directory entries (many ZIP
/// creators, Python zipfile, 7-Zip, etc.).
pub(super) fn build_synthetic_ancestors(
    archive_path: &Path,
    inner: &Path,
    index: &mut HashMap<String, Vec<FileEntry>>,
) {
    let mut cur = PathBuf::new();
    for comp in inner.parent().into_iter().flat_map(|p| p.components()) {
        let prev = cur.clone();
        cur.push(comp);
        let k = path_to_dir_key(&cur);
        if !index.contains_key(&k) {
            index.entry(k).or_default();
            let dir_name = cur
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !dir_name.is_empty() {
                let prev_key = path_to_dir_key(&prev);
                let synthetic = make_entry(archive_path, cur.clone(), dir_name.clone(), true, None);
                let parent_list = index.entry(prev_key).or_default();
                if !parent_list.iter().any(|e| e.name == dir_name) {
                    parent_list.push(synthetic);
                }
            }
        }
    }
}

/// Create `dest`'s parent directory tree (no-op if it already exists).
pub(super) fn ensure_parent_dir(dest: &Path) -> anyhow::Result<()> {
    if let Some(parent) = dest.parent() {
        #[allow(clippy::disallowed_methods)]
        // reason: trait is sync; runs under spawn_blocking
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

/// Write `data` to `dest`, creating parent directories as needed.
pub(super) fn write_to_dest(dest: &Path, data: &[u8]) -> anyhow::Result<()> {
    ensure_parent_dir(dest)?;
    #[allow(clippy::disallowed_methods)] // reason: trait is sync; runs under spawn_blocking
    std::fs::write(dest, data)?;
    Ok(())
}
