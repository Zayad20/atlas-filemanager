use crate::state::file_entry::{FileEntry, FileKind, SortColumn, SortDir};
use std::path::Path;

pub async fn list_dir(
    path: &Path,
    show_hidden: bool,
    sort_col: SortColumn,
    sort_dir: SortDir,
) -> anyhow::Result<Vec<FileEntry>> {
    let mut rd = tokio::fs::read_dir(path).await?;
    let mut entries = Vec::new();

    while let Ok(Some(entry)) = rd.next_entry().await {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let is_hidden = name.starts_with('.');

        if !show_hidden && is_hidden {
            continue;
        }

        let is_symlink = match entry.file_type().await {
            Ok(ft) => ft.is_symlink(),
            Err(_) => false,
        };

        let kind = if meta.is_dir() {
            FileKind::Dir
        } else if is_symlink {
            FileKind::Symlink
        } else {
            FileKind::File
        };

        let size = if meta.is_file() {
            Some(meta.len())
        } else {
            None
        };
        let modified = meta.modified().ok();
        let extension = Path::new(&name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase());

        entries.push(FileEntry {
            name,
            path: entry.path(),
            kind,
            size,
            modified,
            extension,
            is_hidden,
            is_symlink,
            dir_usage_pct: None,
        });
    }

    sort_entries(&mut entries, &sort_col, &sort_dir);
    Ok(entries)
}

fn sort_entries(entries: &mut [FileEntry], col: &SortColumn, dir: &SortDir) {
    entries.sort_by(|a, b| {
        // Dirs always before files
        let kind_ord = match (&a.kind, &b.kind) {
            (FileKind::Dir, FileKind::Dir) => std::cmp::Ordering::Equal,
            (FileKind::Dir, _) => std::cmp::Ordering::Less,
            (_, FileKind::Dir) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        };
        if kind_ord != std::cmp::Ordering::Equal {
            return kind_ord;
        }

        let ord = match col {
            SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortColumn::Size => a.size.cmp(&b.size),
            SortColumn::Modified => a.modified.cmp(&b.modified),
            SortColumn::Extension => a.extension.cmp(&b.extension),
            SortColumn::Kind => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };

        if *dir == SortDir::Desc {
            ord.reverse()
        } else {
            ord
        }
    });
}
