use std::path::PathBuf;
use std::sync::Arc;
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
    pub display_size: Arc<str>,
    pub display_date: Arc<str>,
    pub display_type: Arc<str>,
    pub _is_hidden: bool,
    pub _is_symlink: bool,
    /// When `Some(archive_path)`, this entry is a virtual item inside that archive
    /// and `path` is its path relative to the archive root.
    pub archive_root: Option<PathBuf>,
    /// Populated only for entries listed from `Location::RecycleBin`.
    pub recycle_info: Option<RecycleBinInfo>,
}

pub fn file_category(kind: &FileKind, ext: Option<&str>) -> String {
    match kind {
        FileKind::Dir => rust_i18n::t!("file_kind_folder").into_owned(),
        FileKind::Symlink => rust_i18n::t!("file_kind_symlink").into_owned(),
        FileKind::File => {
            let Some(e) = ext else {
                return rust_i18n::t!("file_kind_file").into_owned();
            };
            match e.to_ascii_lowercase().as_str() {
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff"
                | "avif" | "heic" | "heif" | "svg" => rust_i18n::t!("file_kind_image").into_owned(),
                "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" => {
                    rust_i18n::t!("file_kind_audio").into_owned()
                }
                "mp4" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "m4v" | "flv" => {
                    rust_i18n::t!("file_kind_video").into_owned()
                }
                "rs" | "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts" | "py" | "pyw"
                | "go" | "java" | "c" | "cc" | "cpp" | "cxx" | "h" | "hpp" | "cs" | "csx" | "php"
                | "rb" | "swift" | "kt" | "kts" | "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat"
                | "cmd" | "lua" | "dart" | "zig" | "nim" | "r" | "scala" | "ex" | "exs" | "clj"
                | "hs" | "ml" | "html" | "htm" | "xhtml" | "xml" | "vue" | "svelte"
                | "astro" | "css" | "scss" | "sass" | "less" | "json" | "jsonc" | "json5"
                | "yaml" | "yml" | "toml" | "sql" | "graphql" | "gql" | "dockerfile" | "makefile" => {
                    rust_i18n::t!("file_kind_code").into_owned()
                }
                "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "tgz" | "zst" => {
                    rust_i18n::t!("file_kind_archive").into_owned()
                }
                "doc" | "docx" | "odt" | "rtf" | "pdf" | "txt" | "md" | "markdown" | "rst"
                | "tex" => rust_i18n::t!("file_kind_document").into_owned(),
                "xls" | "xlsx" | "csv" | "tsv" | "ods" | "numbers" => {
                    rust_i18n::t!("file_kind_spreadsheet").into_owned()
                }
                "ppt" | "pptx" | "odp" | "key" => {
                    rust_i18n::t!("file_kind_presentation").into_owned()
                }
                "exe" | "msi" | "app" | "dmg" | "deb" | "rpm" => {
                    rust_i18n::t!("file_kind_application").into_owned()
                }
                _ => rust_i18n::t!("file_kind_file").into_owned(),
            }
        }
    }
}
