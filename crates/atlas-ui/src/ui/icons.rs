use atlas_model::file_entry::FileKind;
use gpui::{div, img, px, svg, AnyElement, Hsla, IntoElement, ParentElement, Styled};

#[inline]
pub(crate) fn is_windows_theme(name: &str) -> bool {
    name == "Atlas Windows" || name == "Atlas Windows Glass"
}

const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tif", "tiff",
];
const AUDIO_EXTS: &[&str] = &["mp3", "wav", "flac", "ogg", "m4a", "aac"];
const VIDEO_EXTS: &[&str] = &["mp4", "mov", "mkv", "webm", "avi"];
const TEXT_EXTS: &[&str] = &[
    "txt", "md", "markdown", "toml", "json", "yaml", "yml", "css", "html", "xml",
];
const CODE_EXTS: &[&str] = &[
    "rs", "js", "jsx", "ts", "tsx", "py", "go", "java", "c", "cc", "cpp", "cxx", "h", "hpp", "cs",
    "php", "rb", "swift", "kt", "kts", "sh", "ps1", "bat",
];
const ARCHIVE_EXTS: &[&str] = &["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "tgz"];
const PRESENTATION_EXTS: &[&str] = &["ppt", "pptx", "odp", "key"];
const DOCUMENT_EXTS: &[&str] = &["doc", "docx", "odt", "rtf"];
const SPREADSHEET_EXTS: &[&str] = &["xls", "xlsx", "csv", "tsv", "ods"];

#[derive(Clone, Copy)]
pub(crate) enum AtlasIcon {
    House,
    Monitor,
    Folder,
    File,
    TextFile,
    Download,
    Image,
    Audio,
    Video,
    Cloud,
    HardDrives,
    Details,
    Grid,
    List,
    Columns,
    CaretDown,
    CaretRight,
    Music,
    Search,
    Copy,
    Scissors,
    Clipboard,
    // Palette engine: section/command icons + per-item buttons.
    Star,
    StarFill,
    Plus,
    Lightning,
    Gear,
    Palette,
    UserSwitch,
    PuzzlePiece,
    ArrowsLeftRight,
    SplitHorizontal,
    SplitVertical,
    SidebarSimple,
    Eye,
    PencilSimple,
    ArrowCounterClockwise,
    ArrowClockwise,
    SelectionAll,
    SelectionInverse,
    SortAscending,
    TreeStructure,
    ClockCounterClockwise,
    Trash,
    Tag,
    BookmarkSimple,
    X,
    Minus,
    Square,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    TrayArrowDown,
}

#[derive(Clone, Copy)]
pub(crate) enum FileTypeIcon {
    Folder,
    File,
    Document,
    Pdf,
    Image,
    Audio,
    Video,
    Code,
    Python,
    Archive,
    Presentation,
}

impl AtlasIcon {
    #[inline]
    pub(crate) const fn path(self) -> &'static str {
        match self {
            Self::House => "icons/ph/house.svg",
            Self::Monitor => "icons/ph/monitor.svg",
            Self::Folder => "icons/ph/folder.svg",
            Self::File => "icons/ph/file.svg",
            Self::TextFile => "icons/ph/file-text.svg",
            Self::Download => "icons/ph/download-simple.svg",
            Self::Image => "icons/ph/image.svg",
            Self::Audio | Self::Music => "icons/ph/music-notes.svg",
            Self::Video => "icons/ph/film-strip.svg",
            Self::Cloud => "icons/ph/cloud.svg",
            Self::HardDrives => "icons/ph/hard-drives.svg",
            Self::Details => "icons/ph/list-dashes.svg",
            Self::Grid => "icons/ph/grid-four.svg",
            Self::List => "icons/ph/list-bullets.svg",
            Self::Columns => "icons/ph/columns.svg",
            Self::CaretDown => "icons/ph/caret-down.svg",
            Self::CaretRight => "icons/ph/caret-right.svg",
            Self::Search => "icons/ph/magnifying-glass.svg",
            Self::Copy => "icons/ph/copy-simple.svg",
            Self::Scissors => "icons/ph/scissors.svg",
            Self::Clipboard => "icons/ph/clipboard.svg",
            Self::Star => "icons/ph/star.svg",
            Self::StarFill => "icons/ph/star-fill.svg",
            Self::Plus => "icons/ph/plus.svg",
            Self::Lightning => "icons/ph/lightning.svg",
            Self::Gear => "icons/ph/gear.svg",
            Self::Palette => "icons/ph/palette.svg",
            Self::UserSwitch => "icons/ph/user-switch.svg",
            Self::PuzzlePiece => "icons/ph/puzzle-piece.svg",
            Self::ArrowsLeftRight => "icons/ph/arrows-left-right.svg",
            Self::SplitHorizontal => "icons/ph/split-horizontal.svg",
            Self::SplitVertical => "icons/ph/split-vertical.svg",
            Self::SidebarSimple => "icons/ph/sidebar-simple.svg",
            Self::Eye => "icons/ph/eye.svg",
            Self::PencilSimple => "icons/ph/pencil-simple.svg",
            Self::ArrowCounterClockwise => "icons/ph/arrow-counter-clockwise.svg",
            Self::ArrowClockwise => "icons/ph/arrow-clockwise.svg",
            Self::SelectionAll => "icons/ph/selection-all.svg",
            Self::SelectionInverse => "icons/ph/selection-inverse.svg",
            Self::SortAscending => "icons/ph/sort-ascending.svg",
            Self::TreeStructure => "icons/ph/tree-structure.svg",
            Self::ClockCounterClockwise => "icons/ph/clock-counter-clockwise.svg",
            Self::Trash => "icons/ph/trash.svg",
            Self::Tag => "icons/ph/tag.svg",
            Self::BookmarkSimple => "icons/ph/bookmark-simple.svg",
            Self::X => "icons/ph/x.svg",
            Self::Minus => "icons/ph/minus.svg",
            Self::Square => "icons/ph/square.svg",
            Self::ArrowLeft => "icons/ph/arrow-left.svg",
            Self::ArrowRight => "icons/ph/arrow-right.svg",
            Self::ArrowUp => "icons/ph/arrow-up.svg",
            Self::ArrowDown => "icons/ph/arrow-down.svg",
            Self::TrayArrowDown => "icons/ph/tray-arrow-down.svg",
        }
    }
}

impl FileTypeIcon {
    #[inline]
    const fn path(self) -> &'static str {
        match self {
            Self::Folder => "icons/filetypes/liquid-glass/folder.png",
            Self::File => "icons/filetypes/liquid-glass/file.png",
            Self::Document => "icons/filetypes/liquid-glass/document.png",
            Self::Pdf => "icons/filetypes/liquid-glass/pdf.png",
            Self::Image => "icons/filetypes/liquid-glass/image.png",
            Self::Audio => "icons/filetypes/liquid-glass/audio.png",
            Self::Video => "icons/filetypes/liquid-glass/video.png",
            Self::Code => "icons/filetypes/liquid-glass/code.png",
            Self::Python => "icons/filetypes/liquid-glass/python.png",
            Self::Archive => "icons/filetypes/liquid-glass/archive.png",
            Self::Presentation => "icons/filetypes/liquid-glass/presentation.png",
        }
    }
}

#[inline]
pub(crate) fn icon(kind: AtlasIcon, size: f32, color: Hsla) -> AnyElement {
    svg()
        .path(kind.path())
        .size(px(size))
        .flex_none()
        .text_color(color)
        .into_any_element()
}

#[inline]
pub(crate) fn windows_icon(kind: AtlasIcon, size: f32, color: Hsla) -> AnyElement {
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .font_family("Segoe Fluent Icons")
        .text_size(px(size))
        .text_color(color)
        .child(windows_icon_glyph(kind))
        .into_any_element()
}

/// Render either a Windows Fluent icon or an SVG icon based on the active theme.
#[inline]
pub(crate) fn theme_icon(
    kind: AtlasIcon,
    size: f32,
    color: Hsla,
    windows_theme: bool,
) -> AnyElement {
    if windows_theme {
        windows_icon(kind, size, color)
    } else {
        icon(kind, size, color)
    }
}

#[inline]
fn windows_icon_glyph(kind: AtlasIcon) -> &'static str {
    match kind {
        AtlasIcon::House => "\u{E80F}",
        AtlasIcon::Monitor => "\u{E7F4}",
        AtlasIcon::Folder => "\u{E8B7}",
        AtlasIcon::File | AtlasIcon::TextFile => "\u{E8A5}",
        AtlasIcon::Download => "\u{E896}",
        AtlasIcon::Image => "\u{E8B9}",
        AtlasIcon::Audio | AtlasIcon::Music => "\u{EC4F}",
        AtlasIcon::Video => "\u{E8B2}",
        AtlasIcon::Cloud => "\u{E753}",
        AtlasIcon::HardDrives => "\u{EDA2}",
        AtlasIcon::Trash => "\u{E74D}",
        AtlasIcon::ArrowLeft => "\u{E72B}",
        AtlasIcon::ArrowRight => "\u{E72A}",
        AtlasIcon::ArrowUp => "\u{E74A}",
        AtlasIcon::ArrowDown => "\u{E74B}",
        AtlasIcon::Plus => "\u{E710}",
        AtlasIcon::X => "\u{E711}",
        AtlasIcon::Search => "\u{E721}",
        AtlasIcon::BookmarkSimple => "\u{E734}",
        AtlasIcon::CaretDown => "\u{E70D}",
        AtlasIcon::CaretRight => "\u{E76C}",
        AtlasIcon::Details => "\u{E8FD}",
        AtlasIcon::Grid => "\u{ECA5}",
        AtlasIcon::List => "\u{EA37}",
        AtlasIcon::Columns => "\u{E9D2}",
        AtlasIcon::Copy => "\u{E8C8}",
        AtlasIcon::Scissors => "\u{E8C6}",
        AtlasIcon::Clipboard => "\u{E77F}",
        AtlasIcon::Star => "\u{E734}",
        AtlasIcon::StarFill => "\u{E735}",
        AtlasIcon::Lightning => "\u{E945}",
        AtlasIcon::Gear => "\u{E713}",
        AtlasIcon::Palette => "\u{E771}",
        AtlasIcon::UserSwitch => "\u{E748}",
        AtlasIcon::PuzzlePiece => "\u{EA86}",
        AtlasIcon::ArrowsLeftRight => "\u{E8AB}",
        AtlasIcon::SplitHorizontal => "\u{E9EF}",
        AtlasIcon::SplitVertical => "\u{E9F0}",
        AtlasIcon::SidebarSimple => "\u{E700}",
        AtlasIcon::Eye => "\u{E890}",
        AtlasIcon::PencilSimple => "\u{E70F}",
        AtlasIcon::ArrowCounterClockwise => "\u{E72C}",
        AtlasIcon::ArrowClockwise => "\u{E72A}",
        AtlasIcon::SelectionAll => "\u{E762}",
        AtlasIcon::SelectionInverse => "\u{E8B5}",
        AtlasIcon::SortAscending => "\u{E8CB}",
        AtlasIcon::TreeStructure => "\u{E8B7}",
        AtlasIcon::ClockCounterClockwise => "\u{E81C}",
        AtlasIcon::Tag => "\u{E8EC}",
        AtlasIcon::Minus => "\u{E738}",
        AtlasIcon::Square => "\u{E739}",
        AtlasIcon::TrayArrowDown => "\u{E896}",
    }
}

#[inline]
pub(crate) fn atlas_logo(size: f32) -> AnyElement {
    img("icons/atlas-logo.png")
        .size(px(size))
        .flex_none()
        .into_any_element()
}

/// Fallback file-type icon (built-in liquid-glass PNG set), used when the active
/// icon theme has no rasterised texture for this entry. Themed icons (VSCode or an
/// extension-provided theme) flow through [`crate::app::App::icon_tex_for_entry`]
/// and the resvg pipeline instead.
#[inline]
pub(crate) fn file_type_icon(kind: &FileKind, extension: Option<&str>, size: f32) -> AnyElement {
    img(file_type_icon_kind(kind, extension).path())
        .w(px(size))
        .h(px(size))
        .flex_none()
        .into_any_element()
}

#[inline]
fn file_type_icon_kind(kind: &FileKind, extension: Option<&str>) -> FileTypeIcon {
    match kind {
        FileKind::Dir => FileTypeIcon::Folder,
        FileKind::Symlink => FileTypeIcon::File,
        FileKind::File => extension.map_or(FileTypeIcon::File, file_type_icon_for_ext),
    }
}

#[inline]
fn file_type_icon_for_ext(ext: &str) -> FileTypeIcon {
    if ext.eq_ignore_ascii_case("pdf") {
        FileTypeIcon::Pdf
    } else if ext.eq_ignore_ascii_case("py") {
        FileTypeIcon::Python
    } else if matches_ext(ext, IMAGE_EXTS) {
        FileTypeIcon::Image
    } else if matches_ext(ext, AUDIO_EXTS) {
        FileTypeIcon::Audio
    } else if matches_ext(ext, VIDEO_EXTS) {
        FileTypeIcon::Video
    } else if matches_ext(ext, CODE_EXTS) {
        FileTypeIcon::Code
    } else if matches_ext(ext, ARCHIVE_EXTS) {
        FileTypeIcon::Archive
    } else if matches_ext(ext, PRESENTATION_EXTS) {
        FileTypeIcon::Presentation
    } else if matches_ext(ext, DOCUMENT_EXTS)
        || matches_ext(ext, SPREADSHEET_EXTS)
        || matches_ext(ext, TEXT_EXTS)
    {
        FileTypeIcon::Document
    } else {
        FileTypeIcon::File
    }
}

#[inline]
fn matches_ext(ext: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| ext.eq_ignore_ascii_case(candidate))
}
