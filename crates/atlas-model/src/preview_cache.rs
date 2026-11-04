use crate::layout_tree::TileId;
use std::collections::HashMap;
use std::path::PathBuf;

pub const MULTI_PREVIEW_LIMIT: usize = 12;
pub const DIR_PREVIEW_LIMIT: usize = 200;

#[derive(Debug, Clone)]
pub struct DirPreviewEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: Option<u64>,
    #[allow(dead_code)] // reason: directory preview UI keeps this variant available
    pub extension: Option<String>,
}

#[derive(Debug, Clone)]
pub enum PreviewResult {
    Text {
        /// Syntax-highlighted spans as `([r, g, b, a], text)`; alpha `0` means
        /// "use the default foreground" (line breaks). UI converts to its color type.
        lines: Vec<([u8; 4], String)>,
        /// Raw file contents (for the "Copiar" button). Same bytes that were highlighted.
        text: String,
    },
    Markdown {
        /// Parsed block structure (headings, paragraphs, lists, code, quotes, rules).
        /// Produced in `atlas-core` via pulldown-cmark; the model stays renderer-neutral.
        blocks: Vec<MarkdownBlock>,
        /// Raw markdown source (for the "Copiar" button).
        source: String,
        #[allow(dead_code)] // reason: markdown asset resolution keeps this data available
        base_uri: String,
    },
    Image {
        rgba: Vec<u8>,
        width: usize,
        height: usize,
        original_width: usize,
        original_height: usize,
        exif: Vec<(String, String)>,
    },
    Pdf {
        rgba: Vec<u8>,
        width: usize,
        height: usize,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
        truncated: bool,
    },
    Document {
        title: String,
        blocks: Vec<String>,
        truncated: bool,
    },
    Presentation {
        slides: Vec<SlidePreview>,
        truncated: bool,
    },
    Audio {
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        duration_secs: u64,
        bitrate: Option<u32>,
        /// Embedded cover art, decoded to a small RGBA thumbnail (if present).
        cover: Option<ImageData>,
    },
    Dir {
        /// First `DIR_PREVIEW_LIMIT` entries, dirs sorted before files.
        entries: Vec<DirPreviewEntry>,
        /// Actual total (may exceed `entries.len()`).
        total_count: usize,
        total_size: u64,
    },
    Binary {
        size: u64,
    },
    Error(String),
}

#[derive(Debug, Clone)]
pub struct SlidePreview {
    pub title: String,
    pub body: Vec<String>,
}

/// One inline run inside a markdown block. Styling is neutral; the UI maps each to a
/// styled element.
#[derive(Debug, Clone)]
pub enum MdSpan {
    Text(String),
    Code(String),
    Strong(String),
    Emphasis(String),
    Link { text: String, url: String },
}

/// One block-level markdown element. `Code.lines` reuses the syntect span format from
/// `PreviewResult::Text` (`([r,g,b,a], token)` with `"\n"` separators).
#[derive(Debug, Clone)]
pub enum MarkdownBlock {
    Heading {
        level: u8,
        spans: Vec<MdSpan>,
    },
    Paragraph {
        spans: Vec<MdSpan>,
    },
    Code {
        lang: String,
        lines: Vec<([u8; 4], String)>,
    },
    ListItem {
        ordered: bool,
        depth: u8,
        spans: Vec<MdSpan>,
    },
    Quote {
        spans: Vec<MdSpan>,
    },
    Rule,
}

pub fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tiff" | "tif" | "svg"
    )
}

/// 3D model formats previewed by the built-in software renderer.
pub fn is_model_ext(ext: &str) -> bool {
    matches!(ext, "glb" | "gltf" | "fbx" | "obj" | "stl")
}

pub fn is_audio_ext(ext: &str) -> bool {
    matches!(
        ext,
        "mp3" | "flac" | "m4a" | "ogg" | "opus" | "wav" | "aac" | "aiff" | "wv" | "ape"
    )
}

pub fn is_video_ext(ext: &str) -> bool {
    matches!(
        ext,
        "mp4"
            | "mkv"
            | "mov"
            | "webm"
            | "avi"
            | "flv"
            | "wmv"
            | "m4v"
            | "mpeg"
            | "mpg"
            | "3gp"
            | "ts"
            | "mts"
            | "m2ts"
            | "vob"
            | "asf"
    )
}

/// Framework-neutral decoded image: tightly-packed RGBA8 plus dimensions. The UI
/// layer uploads this to a GPU texture and caches the handle on its own side, so
/// `state` never holds a renderer texture handle.
#[derive(Clone, Debug)]
pub struct ImageData {
    pub rgba: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MultiPreviewKey {
    pub fingerprint: u64,
    pub selected_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PreviewRequest {
    Single(PathBuf),
    /// Preview for a file entry inside an archive.
    ArchiveSingle {
        archive: PathBuf,
        inner: PathBuf,
    },
    Multiple {
        key: MultiPreviewKey,
        paths: Vec<PathBuf>,
    },
}

pub struct MultiPreviewState {
    pub key: MultiPreviewKey,
    pub paths: Vec<PathBuf>,
    /// Decoded thumbnails, in arrival order. The UI uploads these to textures.
    pub images: Vec<(PathBuf, ImageData)>,
}

#[derive(Default)]
pub struct PreviewState {
    pub paths: HashMap<TileId, PathBuf>,
    pub results: HashMap<TileId, PreviewResult>,
    /// Path each entry in `results` was produced for. It lags behind `paths`
    /// while the next preview loads, so the previous one stays on screen.
    pub result_paths: HashMap<TileId, PathBuf>,
    pub multi: HashMap<TileId, MultiPreviewState>,
    pub metadata: HashMap<TileId, (PathBuf, Vec<(String, String)>)>,
}

impl PreviewState {
    pub fn clear(&mut self, tab: TileId) {
        self.paths.remove(&tab);
        self.results.remove(&tab);
        self.result_paths.remove(&tab);
        self.multi.remove(&tab);
        self.metadata.remove(&tab);
    }

    pub fn clear_all(&mut self) {
        self.paths.clear();
        self.results.clear();
        self.multi.clear();
        self.metadata.clear();
    }
}
