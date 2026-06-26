use atlas_model::file_entry::{FileEntry, FileKind, RecycleBinInfo, SortColumn, SortDir};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub async fn list_recycle_bin(sort_col: SortColumn, sort_dir: SortDir) -> Vec<FileEntry> {
    tokio::task::spawn_blocking(move || list_sync(&sort_col, &sort_dir))
        .await
        .unwrap_or_default()
}

fn list_sync(sort_col: &SortColumn, sort_dir: &SortDir) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    for rb_dir in accessible_bin_dirs() {
        let Ok(rd) = std::fs::read_dir(&rb_dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let i_path = entry.path();
            let fname = i_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !fname.starts_with("$I") {
                continue;
            }
            let r_path = rb_dir.join(format!("$R{}", &fname[2..]));
            if !r_path.exists() {
                continue;
            }
            let Ok(bytes) = std::fs::read(&i_path) else {
                continue;
            };
            let Some(meta) = parse_i_file(&bytes) else {
                continue;
            };
            let original_name = meta
                .original_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| fname.clone());
            let kind = if r_path.is_dir() {
                FileKind::Dir
            } else {
                FileKind::File
            };
            let extension = meta
                .original_path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase());
            let name_lower = original_name.to_lowercase();
            entries.push(FileEntry {
                name: original_name,
                name_lower,
                path: r_path,
                kind,
                size: Some(meta.original_size),
                modified: Some(meta.deleted_at),
                extension,
                display_size: String::new(), display_date: String::new(), display_type: String::new(), _is_hidden: false,
                _is_symlink: false,
                archive_root: None,
                recycle_info: Some(RecycleBinInfo {
                    original_path: meta.original_path,
                    deleted_at: meta.deleted_at,
                    i_file: i_path,
                }),
            });
        }
    }
    crate::fs::list::sort_entries(&mut entries, sort_col, sort_dir, true);
    entries
}

struct Meta {
    original_path: PathBuf,
    original_size: u64,
    deleted_at: SystemTime,
}

/// Parse a Windows `$I` recycle bin metadata file.
/// Layout (Vista+, version 1): [8B version][8B original size][8B FILETIME][UTF-16LE path, NUL-term]
/// Version 2 adds a 4-byte path-length prefix before the path (path starts at offset 28).
fn parse_i_file(data: &[u8]) -> Option<Meta> {
    if data.len() < 24 {
        return None;
    }
    let version = u64::from_le_bytes(data[0..8].try_into().ok()?);
    if version != 1 && version != 2 {
        return None;
    }
    let original_size = u64::from_le_bytes(data[8..16].try_into().ok()?);
    let filetime = u64::from_le_bytes(data[16..24].try_into().ok()?);
    let deleted_at = filetime_to_system_time(filetime);

    let path_start: usize = if version == 2 { 28 } else { 24 };
    if data.len() <= path_start {
        return None;
    }
    let utf16: Vec<u16> = data[path_start..]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    let original_path = PathBuf::from(String::from_utf16_lossy(&utf16));

    Some(Meta {
        original_path,
        original_size,
        deleted_at,
    })
}

fn filetime_to_system_time(ft: u64) -> SystemTime {
    if ft == 0 {
        return SystemTime::now();
    }
    // FILETIME: 100-ns ticks since 1601-01-01; UNIX_EPOCH: 1970-01-01
    let unix_secs = (ft / 10_000_000).saturating_sub(11_644_473_600);
    UNIX_EPOCH + Duration::from_secs(unix_secs)
}

/// Restore a file from the recycle bin to its original location.
/// Moves the `$R` file back to `original_path` and deletes the `$I` metadata file.
pub async fn restore(
    r_path: PathBuf,
    i_path: PathBuf,
    original_path: PathBuf,
) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        if let Some(parent) = original_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&r_path, &original_path)?;
        let _ = std::fs::remove_file(&i_path);
        Ok(())
    })
    .await?
}

/// Enumerate all `$Recycle.Bin\<SID>` subdirectories that the current process
/// can read. Works by trying all drive letters rather than resolving the SID
/// via Win32 API, which avoids pulling in extra Windows crate features.
fn accessible_bin_dirs() -> Vec<PathBuf> {
    let mut result = Vec::new();
    #[cfg(windows)]
    {
        for letter in b'A'..=b'Z' {
            let rb = PathBuf::from(format!("{}:\\$Recycle.Bin", letter as char));
            let Ok(rd) = std::fs::read_dir(&rb) else {
                continue;
            };
            for entry in rd.flatten() {
                let dir = entry.path();
                if dir.is_dir() && std::fs::read_dir(&dir).is_ok() {
                    result.push(dir);
                }
            }
        }
    }
    result
}
