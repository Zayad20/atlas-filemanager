use egui::Color32;
use egui_phosphor::regular;

/// A registry mapping file extensions to icons and colors.
pub struct ExtensionRegistry;

impl ExtensionRegistry {
    pub fn icon(ext: Option<&str>) -> &'static str {
        match ext {
            Some("rs") => regular::FILE_RS,
            Some("py") => regular::FILE_PY,
            Some("js") => regular::FILE_JS,
            Some("jsx") => regular::FILE_JSX,
            Some("ts") => regular::FILE_TS,
            Some("tsx") => regular::FILE_TSX,
            Some("json" | "yaml" | "yml" | "toml") => regular::FILE_TEXT,
            Some("md") => regular::FILE_MD,
            Some("txt" | "rst") => regular::FILE_TXT,
            Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico") => {
                regular::FILE_IMAGE
            }
            Some("mp4" | "mkv" | "avi" | "mov" | "webm") => regular::FILE_VIDEO,
            Some("mp3" | "flac" | "wav" | "ogg" | "m4a") => regular::FILE_AUDIO,
            Some("pdf") => regular::FILE_PDF,
            Some("zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar") => regular::FILE_ARCHIVE,
            Some("sh" | "bash" | "zsh" | "fish") => regular::TERMINAL,
            Some("c") => regular::FILE_C,
            Some("cpp" | "h" | "hpp") => regular::FILE_CPP,
            Some("html") => regular::FILE_HTML,
            Some("css") => regular::FILE_CSS,
            None => regular::FOLDER,
            _ => regular::FILE,
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
