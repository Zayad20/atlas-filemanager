use std::path::Path;

use atlas_model::preview_cache::{DirPreviewEntry, PreviewResult, DIR_PREVIEW_LIMIT};

pub(super) async fn preview_dir(path: &Path) -> PreviewResult {
    let mut rd = match tokio::fs::read_dir(path).await {
        Ok(r) => r,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };

    let mut dirs: Vec<DirPreviewEntry> = Vec::new();
    let mut files: Vec<DirPreviewEntry> = Vec::new();
    let mut total_size = 0u64;
    let mut total_count = 0usize;

    while let Ok(Some(entry)) = rd.next_entry().await {
        total_count += 1;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        let is_dir = meta.is_dir();
        let size = if is_dir { None } else { Some(meta.len()) };
        if let Some(s) = size {
            total_size += s;
        }
        let extension = if is_dir {
            None
        } else {
            std::path::Path::new(&name)
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_lowercase)
        };
        let e = DirPreviewEntry {
            name,
            path,
            is_dir,
            size,
            extension,
        };
        if is_dir {
            dirs.push(e);
        } else {
            files.push(e);
        }
    }

    dirs.sort_by_key(|a| a.name.to_lowercase());
    files.sort_by_key(|a| a.name.to_lowercase());

    let mut entries: Vec<DirPreviewEntry> = dirs;
    entries.extend(files);
    entries.truncate(DIR_PREVIEW_LIMIT);

    PreviewResult::Dir {
        entries,
        total_count,
        total_size,
    }
}
