use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileKind {
    Dir,
    File,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum SortColumn {
    Name,
    Size,
    Modified,
    Extension,
    Kind,
}

impl Default for SortColumn {
    fn default() -> Self {
        Self::Name
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SortDir {
    Asc,
    Desc,
}

impl Default for SortDir {
    fn default() -> Self {
        Self::Asc
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: FileKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub extension: Option<String>,
    pub is_hidden: bool,
    pub is_symlink: bool,
    pub dir_usage_pct: Option<f32>,
}
