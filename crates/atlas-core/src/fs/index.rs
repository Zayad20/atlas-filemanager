#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use crate::search::nucleo_pool::IndexEntry;
use crate::util::display::is_hidden;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn build_index(current_root: PathBuf, injector: nucleo::Injector<IndexEntry>) {
    rayon::spawn(move || {
        let home = dirs::home_dir().unwrap_or_else(|| current_root.clone());
        let mut seen = HashSet::new();

        for path in common_folders()
            .into_iter()
            .chain([home.clone(), current_root.clone()])
        {
            inject_folder(path, &home, &mut seen, &injector);
        }

        index_tree(&home, 5, &home, &mut seen, &injector);
        if !current_root.starts_with(&home) {
            index_tree(&current_root, 3, &home, &mut seen, &injector);
        }
    });
}

/// Scope cap: a drilled-into folder is walked deeper than the global index but
/// still bounded so pathological trees (node_modules…) don't hang the walker.
const SCOPED_MAX_DEPTH: usize = 6;
const SCOPED_MAX_ENTRIES: usize = 50_000;

/// Index *only* `root` — folders **and files** — matching against the path
/// relative to `root`, so the needle filters within the folder instead of
/// trivially matching every descendant's absolute path. Used when the goto
/// query carries an absolute-path prefix (drill-in).
pub fn build_index_scoped(root: PathBuf, injector: nucleo::Injector<IndexEntry>) {
    rayon::spawn(move || {
        let mut injected = 0usize;
        for entry in WalkDir::new(&root)
            .max_depth(SCOPED_MAX_DEPTH)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| entry.depth() == 0 || !is_hidden(entry.path()))
            .flatten()
        {
            if entry.depth() == 0 {
                continue;
            }
            let is_dir = entry.file_type().is_dir();
            let path = entry.into_path();
            let display = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            injector.push(IndexEntry { path, is_dir }, move |_item, cols| {
                cols[0] = display.as_str().into();
            });
            injected += 1;
            if injected >= SCOPED_MAX_ENTRIES {
                break;
            }
        }
    });
}

pub fn prioritize_folders(folders: &mut [PathBuf], current: Option<&Path>) {
    let home = dirs::home_dir();
    let common = common_folders();
    folders.sort_by_key(|path| {
        let priority = if current == Some(path.as_path()) {
            0
        } else if common.iter().any(|folder| folder == path)
            || home.as_ref().is_some_and(|folder| folder == path)
        {
            1
        } else if current.is_some_and(|folder| path.starts_with(folder)) {
            2
        } else if common.iter().any(|folder| path.starts_with(folder)) {
            3
        } else if home.as_ref().is_some_and(|folder| path.starts_with(folder)) {
            4
        } else {
            5
        };
        (priority, path.components().count(), path.clone())
    });
}

fn index_tree(
    root: &Path,
    max_depth: usize,
    home: &Path,
    seen: &mut HashSet<PathBuf>,
    injector: &nucleo::Injector<IndexEntry>,
) {
    for entry in WalkDir::new(root)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !is_hidden(entry.path()))
        .flatten()
        .filter(|entry| entry.file_type().is_dir())
    {
        inject_folder(entry.into_path(), home, seen, injector);
    }
}

fn inject_folder(
    path: PathBuf,
    home: &Path,
    seen: &mut HashSet<PathBuf>,
    injector: &nucleo::Injector<IndexEntry>,
) {
    if !path.is_dir() || !seen.insert(path.clone()) {
        return;
    }
    // Match against the home-relative path when possible: the constant
    // "C:\Users\name\" prefix only adds spurious match material. Home itself
    // would strip to "", so it keeps its absolute form.
    let rel = path.strip_prefix(home).unwrap_or(&path);
    let display = if rel.as_os_str().is_empty() {
        path.to_string_lossy().into_owned()
    } else {
        rel.to_string_lossy().into_owned()
    };
    injector.push(IndexEntry { path, is_dir: true }, move |_item, cols| {
        cols[0] = display.as_str().into();
    });
}

fn common_folders() -> Vec<PathBuf> {
    [
        dirs::desktop_dir(),
        dirs::download_dir(),
        dirs::document_dir(),
        dirs::picture_dir(),
        dirs::video_dir(),
        dirs::audio_dir(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::prioritize_folders;
    use std::path::PathBuf;

    #[test]
    fn prioritizes_current_folder_and_its_nearby_children() {
        let current = PathBuf::from("/tmp/atlas-project");
        let child = current.join("src");
        let nested = child.join("components");
        let mut folders = vec![nested.clone(), child.clone(), current.clone()];

        prioritize_folders(&mut folders, Some(&current));

        assert_eq!(folders, vec![current, child, nested]);
    }
}
