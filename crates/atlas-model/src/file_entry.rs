use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct RecycleBinInfo {
    pub original_path: PathBuf,
    pub deleted_at: SystemTime,
    pub i_file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileKind {
    Dir,
    File,
    Symlink,
}

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize, Default,
)]
pub enum SortColumn {
    #[default]
    Name,
    Size,
    Modified,
    Extension,
    Kind,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    /// `name` lowercased once at listing time, so the filter doesn't re-lowercase
    /// every entry on every frame.
    pub name_lower: String,
    /// Real filesystem path, OR the inner path within `archive_root` when browsing
    /// inside an archive.
    pub path: PathBuf,
    pub kind: FileKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub extension: Option<String>,
    pub display_size: String,
    pub display_date: String,
    pub display_type: String,
    pub _is_hidden: bool,
    pub _is_symlink: bool,
    /// When `Some(archive_path)`, this entry is a virtual item inside that archive
    /// and `path` is its path relative to the archive root.
    pub archive_root: Option<PathBuf>,
    /// Populated only for entries listed from `Location::RecycleBin`.
    pub recycle_info: Option<RecycleBinInfo>,
}
