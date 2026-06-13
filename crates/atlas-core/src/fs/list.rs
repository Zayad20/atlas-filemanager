use atlas_model::file_entry::{FileEntry, FileKind, SortColumn, SortDir};
use std::path::{Path, PathBuf};

pub async fn list_dir(
    path: &Path,
    show_hidden: bool,
    sort_col: SortColumn,
    sort_dir: SortDir,
    folders_first: bool,
) -> anyhow::Result<Vec<FileEntry>> {
    let mut rd = tokio::fs::read_dir(path).await?;
    let mut entries = Vec::new();

    while let Ok(Some(entry)) = rd.next_entry().await {
        let path = entry.path();
        if !show_hidden && crate::util::display::is_hidden(&path) {
            continue;
        }
        if let Some(file_entry) = entry_for_path(&path).await {
            entries.push(file_entry);
        }
    }

    sort_entries(&mut entries, &sort_col, &sort_dir, folders_first);
    Ok(entries)
}

/// Build a sorted listing from an explicit set of paths (used by virtual tabs such as
/// tag results, where entries come from many different directories rather than one
/// `read_dir`). Missing/unreadable paths are silently skipped.
pub async fn entries_for_paths(
    paths: &[PathBuf],
    show_hidden: bool,
    sort_col: SortColumn,
    sort_dir: SortDir,
    folders_first: bool,
) -> Vec<FileEntry> {
    let mut entries = Vec::with_capacity(paths.len());
    for path in paths {
        if !show_hidden && crate::util::display::is_hidden(path) {
            continue;
        }
        if let Some(file_entry) = entry_for_path(path).await {
            entries.push(file_entry);
        }
    }
    sort_entries(&mut entries, &sort_col, &sort_dir, folders_first);
    entries
}

/// Stat a single path and build its [`FileEntry`]. Returns `None` if it can't be read.
/// Mirrors the per-entry logic of [`list_dir`]: metadata is read without following
/// symlinks, so a symlink is classified as `Symlink` even when it points at a dir.
pub(crate) async fn entry_for_path(path: &Path) -> Option<FileEntry> {
    let meta = tokio::fs::symlink_metadata(path).await.ok()?;
    let is_symlink = meta.file_type().is_symlink();

    let name = path.file_name().map_or_else(
        || path.to_string_lossy().to_string(),
        |n| n.to_string_lossy().to_string(),
    );

    let kind = if meta.is_dir() {
        FileKind::Dir
    } else if is_symlink {
        FileKind::Symlink
    } else {
        FileKind::File
    };

    let size = meta.is_file().then_some(meta.len());
    let modified = meta.modified().ok();
    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    let name_lower = name.to_lowercase();

    Some(FileEntry {
        name,
        name_lower,
        path: path.to_path_buf(),
        kind,
        size,
        modified,
        extension,
        _is_hidden: crate::util::display::is_hidden(path),
        _is_symlink: is_symlink,
        archive_root: None,
    })
}

pub fn sort_entries(
    entries: &mut [FileEntry],
    col: &SortColumn,
    dir: &SortDir,
    folders_first: bool,
) {
    entries.sort_by(|a, b| {
        if folders_first {
            let kind_ord = match (&a.kind, &b.kind) {
                (FileKind::Dir, FileKind::Dir) => std::cmp::Ordering::Equal,
                (FileKind::Dir, _) => std::cmp::Ordering::Less,
                (_, FileKind::Dir) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            };
            if kind_ord != std::cmp::Ordering::Equal {
                return kind_ord;
            }
        }

        let ord = match col {
            SortColumn::Name | SortColumn::Kind => a.name_lower.cmp(&b.name_lower),
            SortColumn::Size => a.size.cmp(&b.size),
            SortColumn::Modified => a.modified.cmp(&b.modified),
            SortColumn::Extension => a.extension.cmp(&b.extension),
        };

        if *dir == SortDir::Desc {
            ord.reverse()
        } else {
            ord
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: FileKind) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            path: PathBuf::from(name),
            kind,
            size: None,
            modified: None,
            extension: None,
            _is_hidden: false,
            _is_symlink: false,
            archive_root: None,
        }
    }

    #[test]
    fn folders_first_toggle_controls_dir_grouping() {
        let mut entries = vec![
            entry("zeta", FileKind::File),
            entry("alpha", FileKind::File),
            entry("midway", FileKind::Dir),
        ];
        sort_entries(&mut entries, &SortColumn::Name, &SortDir::Asc, true);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["midway", "alpha", "zeta"]);

        sort_entries(&mut entries, &SortColumn::Name, &SortDir::Asc, false);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "midway", "zeta"]);
    }
}
