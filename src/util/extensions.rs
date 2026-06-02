use egui::Color32;

/// A registry mapping file extensions to icons and colors.
pub struct ExtensionRegistry;

impl ExtensionRegistry {
    pub fn icon(ext: Option<&str>) -> &'static str {
        match ext {
            Some("rs") => "🦀",
            Some("py") => "🐍",
            Some("js" | "ts" | "jsx" | "tsx") => "📜",
            Some("json" | "yaml" | "yml" | "toml") => "⚙",
            Some("md" | "txt" | "rst") => "📄",
            Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico") => "🖼",
            Some("mp4" | "mkv" | "avi" | "mov" | "webm") => "🎬",
            Some("mp3" | "flac" | "wav" | "ogg" | "m4a") => "🎵",
            Some("pdf") => "📕",
            Some("zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar") => "📦",
            Some("sh" | "bash" | "zsh" | "fish") => "💻",
            Some("c" | "cpp" | "h" | "hpp") => "⚡",
            Some("html" | "css") => "🌐",
            None => "📁",
            _ => "📄",
        }
    }

    pub fn color(ext: Option<&str>) -> Color32 {
        match ext {
            Some("rs") => Color32::from_rgb(222, 115, 45),
            Some("py") => Color32::from_rgb(55, 136, 186),
            Some("js" | "ts") => Color32::from_rgb(241, 196, 15),
            Some("json" | "toml" | "yaml" | "yml") => Color32::from_rgb(155, 89, 182),
            Some("md" | "txt") => Color32::from_rgb(149, 165, 166),
            Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp") => Color32::from_rgb(39, 174, 96),
            Some("mp4" | "mkv" | "avi") => Color32::from_rgb(231, 76, 60),
            Some("mp3" | "flac" | "wav") => Color32::from_rgb(52, 152, 219),
            Some("pdf") => Color32::from_rgb(192, 57, 43),
            Some("zip" | "tar" | "gz") => Color32::from_rgb(230, 126, 34),
            _ => Color32::from_rgb(189, 195, 199),
        }
    }
}
