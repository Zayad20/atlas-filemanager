use atlas_model::file_entry::{FileEntry, FileKind, SortColumn, SortDir};
use std::path::{Path, PathBuf};

pub async fn list_dir(
    path: &Path,
    show_hidden: bool,
    sort_col: SortColumn,
    sort_dir: SortDir,
    folders_first: bool,
) -> anyhow::Result<Vec<FileEntry>> {
    list_dir_with_preview(path, show_hidden, sort_col, sort_dir, folders_first, |_| {}).await
}

pub async fn list_dir_with_preview(
    path: &Path,
    show_hidden: bool,
    sort_col: SortColumn,
    sort_dir: SortDir,
    folders_first: bool,
    preview: impl FnOnce(Vec<FileEntry>) + Send + 'static,
) -> anyhow::Result<Vec<FileEntry>> {
    let dir = crate::fs::paths::normalize_wsl_unc_path(path.to_path_buf());
    // Single blocking call: `DirEntry::file_type()` and `DirEntry::metadata()` on
    // Windows reuse the WIN32_FIND_DATA already fetched by read_dir, so no extra
    // stat syscall per file (versus the previous N async symlink_metadata round-trips).
    let entries = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<FileEntry>> {
        crate::fs::paths::ensure_wsl_ready(&dir)?;
        let mut entries = Vec::new();
        let mut preview = Some(preview);
        for raw in std::fs::read_dir(&dir)? {
            let Ok(de) = raw else { continue };
            let path = de.path();
            if !show_hidden && crate::util::display::is_hidden(&path) {
                continue;
            }
            let Ok(ft) = de.file_type() else { continue };
            let is_symlink = ft.is_symlink();
            let kind = if ft.is_dir() {
                FileKind::Dir
            } else if is_symlink {
                FileKind::Symlink
            } else {
                FileKind::File
            };
            let meta = de.metadata().ok();
            let size = ft
                .is_file()
                .then(|| meta.as_ref().map(|m| m.len()))
                .flatten();
            let modified = meta.as_ref().and_then(|m| m.modified().ok());
            let name = path.file_name().map_or_else(
                || path.to_string_lossy().to_string(),
                |n| n.to_string_lossy().to_string(),
            );
            let name_lower = name.to_lowercase();
            let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase());
            let is_hidden = crate::util::display::is_hidden(&path);
            entries.push(FileEntry {
                name,
                name_lower,
                path,
                kind,
                size,
                modified,
                extension,
                display_size: String::new(),
                display_date: String::new(),
                // Resolved from kind/extension by the background formatter.
                // SHGetFileInfoW can touch the shell for every distinct type and
                // makes cold listings such as WinSxS unpredictably slow.
                display_type: String::new(),
                _is_hidden: is_hidden,
                _is_symlink: is_symlink,
                archive_root: None,
                recycle_info: None,
            });
            if entries.len() == 256 {
                let mut first = entries.clone();
                sort_entries(&mut first, &sort_col, &sort_dir, folders_first);
                preview.take().unwrap()(first);
            }
        }
        sort_entries(&mut entries, &sort_col, &sort_dir, folders_first);
        Ok(entries)
    })
    .await??;
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
    let display_type = platform_type_name(path).unwrap_or_default();

    Some(FileEntry {
        name,
        name_lower,
        path: path.to_path_buf(),
        kind,
        size,
        modified,
        extension,
        display_size: String::new(),
        display_date: String::new(),
        display_type,
        _is_hidden: crate::util::display::is_hidden(path),
        _is_symlink: is_symlink,
        archive_root: None,
        recycle_info: None,
    })
}

#[cfg(windows)]
fn platform_type_name(path: &Path) -> Option<String> {
    atlas_win_space::type_name_for_path(path)
}

#[cfg(not(windows))]
fn platform_type_name(_path: &Path) -> Option<String> {
    None
}

/// Quick, non-recursive stats for a folder-hover tooltip (Explorer-style): immediate
/// children only, no descent into subfolders. Fast enough to run synchronously from
/// the tooltip closure, on demand, only while actually hovering.
pub struct FolderHoverStats {
    pub created: Option<std::time::SystemTime>,
    pub subfolders: usize,
    pub subfiles: usize,
    pub size: u64,
}

pub fn folder_hover_stats(path: &Path) -> Option<FolderHoverStats> {
    let created = std::fs::metadata(path).ok().and_then(|m| m.created().ok());
    let mut subfolders = 0usize;
    let mut subfiles = 0usize;
    let mut size = 0u64;
    for entry in std::fs::read_dir(path).ok()?.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            subfolders += 1;
        } else if ft.is_file() {
            subfiles += 1;
            size += entry.metadata().map_or(0, |m| m.len());
        }
    }
    Some(FolderHoverStats {
        created,
        subfolders,
        subfiles,
        size,
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
            display_size: String::new(),
            display_date: String::new(),
            display_type: String::new(),
            _is_hidden: false,
            _is_symlink: false,
            archive_root: None,
            recycle_info: None,
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

    #[tokio::test]
    async fn large_directory_emits_preview_before_full_listing() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..300 {
            std::fs::File::create(dir.path().join(format!("{i:03}.txt"))).unwrap();
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let entries = list_dir_with_preview(
            dir.path(),
            true,
            SortColumn::Name,
            SortDir::Asc,
            true,
            move |preview| tx.send(preview.len()).unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(rx.recv().unwrap(), 256);
        assert_eq!(entries.len(), 300);
    }
}
