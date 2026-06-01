use once_cell::sync::Lazy;
use std::path::{Path, PathBuf};
use std::process::Command;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

use crate::state::app_state::BgResult;
use crate::state::pane::TabId;

static SYNTAX_SET: Lazy<SyntaxSet> = Lazy::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: Lazy<ThemeSet> = Lazy::new(ThemeSet::load_defaults);

const MAX_TEXT_BYTES: usize = 256 * 1024; // 256 KB

#[derive(Debug)]
pub struct DirPreviewEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub extension: Option<String>,
}

pub const DIR_PREVIEW_LIMIT: usize = 200;

#[derive(Debug)]
pub enum PreviewResult {
    Text {
        lines: Vec<(egui::Color32, String)>,
    },
    Markdown {
        source: String,
        base_uri: String,
    },
    Image {
        rgba: Vec<u8>,
        width: usize,
        height: usize,
        exif: Vec<(String, String)>,
    },
    Pdf {
        rgba: Vec<u8>,
        width: usize,
        height: usize,
    },
    Dir {
        /// First DIR_PREVIEW_LIMIT entries, dirs sorted before files.
        entries: Vec<DirPreviewEntry>,
        /// Actual total (may exceed entries.len()).
        total_count: usize,
        total_size: u64,
    },
    Binary {
        size: u64,
    },
    Error(String),
}

pub async fn load_preview(tab: TabId, path: PathBuf, tx: flume::Sender<BgResult>) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let result = if path.is_dir() {
        preview_dir(&path).await
    } else if is_image_ext(&ext) {
        let p = path.clone();
        tokio::task::spawn_blocking(move || preview_image(&p))
            .await
            .unwrap_or_else(|e| PreviewResult::Error(e.to_string()))
    } else if ext == "pdf" {
        let p = path.clone();
        tokio::task::spawn_blocking(move || preview_pdf(&p))
            .await
            .unwrap_or_else(|e| PreviewResult::Error(e.to_string()))
    } else if is_markdown_ext(&ext) {
        preview_markdown(&path).await
    } else {
        preview_text(&path, &ext).await
    };

    let _ = tx.send(BgResult::PreviewReady { tab, path, result });
}

async fn preview_dir(path: &Path) -> PreviewResult {
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
        let Ok(meta) = entry.metadata().await else { continue };
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
                .map(|e| e.to_lowercase())
        };
        let e = DirPreviewEntry { name, is_dir, size, extension };
        if is_dir { dirs.push(e) } else { files.push(e) }
    }

    dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    let mut entries: Vec<DirPreviewEntry> = dirs;
    entries.extend(files);
    entries.truncate(DIR_PREVIEW_LIMIT);

    PreviewResult::Dir { entries, total_count, total_size }
}

async fn preview_markdown(path: &Path) -> PreviewResult {
    let bytes = match tokio::fs::read(path).await {
        Ok(b) => b,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };
    if bytes.len() > MAX_TEXT_BYTES {
        return PreviewResult::Binary {
            size: bytes.len() as u64,
        };
    }
    match String::from_utf8(bytes) {
        Ok(source) => PreviewResult::Markdown {
            source,
            base_uri: path
                .parent()
                .map(|parent| format!("file://{}/", parent.display()))
                .unwrap_or_else(|| "file://".to_string()),
        },
        Err(_) => PreviewResult::Binary {
            size: path.metadata().map(|m| m.len()).unwrap_or(0),
        },
    }
}

async fn preview_text(path: &Path, ext: &str) -> PreviewResult {
    let bytes = match tokio::fs::read(path).await {
        Ok(b) => b,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };
    if bytes.len() > MAX_TEXT_BYTES {
        return PreviewResult::Binary {
            size: bytes.len() as u64,
        };
    }
    match String::from_utf8(bytes) {
        Ok(text) => {
            let ext = ext.to_owned();
            tokio::task::spawn_blocking(move || PreviewResult::Text {
                lines: highlight_text(&text, &ext),
            })
            .await
            .unwrap_or_else(|e| PreviewResult::Error(e.to_string()))
        }
        Err(_) => {
            let size = path.metadata().map(|m| m.len()).unwrap_or(0);
            PreviewResult::Binary { size }
        }
    }
}

fn preview_pdf(path: &Path) -> PreviewResult {
    let output_prefix =
        std::env::temp_dir().join(format!("egui-app-pdf-preview-{}", uuid::Uuid::new_v4()));
    let png_path = output_prefix.with_extension("png");
    let output = match Command::new("pdftoppm")
        .args([
            "-f",
            "1",
            "-l",
            "1",
            "-scale-to",
            "1600",
            "-png",
            "-singlefile",
        ])
        .arg(path)
        .arg(&output_prefix)
        .output()
    {
        Ok(output) => output,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return PreviewResult::Error(
                "PDF preview requires pdftoppm (install poppler-utils)".to_string(),
            );
        }
        Err(e) => return PreviewResult::Error(format!("Could not start pdftoppm: {e}")),
    };
    if !output.status.success() {
        let _ = std::fs::remove_file(&png_path);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return PreviewResult::Error(format!("Could not render PDF: {}", stderr.trim()));
    }

    let img = match image::open(&png_path) {
        Ok(img) => img,
        Err(e) => {
            let _ = std::fs::remove_file(&png_path);
            return PreviewResult::Error(format!("Could not decode PDF preview: {e}"));
        }
    };
    let _ = std::fs::remove_file(&png_path);
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    PreviewResult::Pdf {
        rgba: rgba.into_raw(),
        width: width as usize,
        height: height as usize,
    }
}

fn highlight_text(text: &str, ext: &str) -> Vec<(egui::Color32, String)> {
    let syntax = SYNTAX_SET
        .find_syntax_by_extension(ext)
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
    let theme = &THEME_SET.themes["base16-ocean.dark"];
    let mut h = HighlightLines::new(syntax, theme);
    let mut result = Vec::new();

    for line in text.lines().take(2000) {
        let ranges = h.highlight_line(line, &SYNTAX_SET).unwrap_or_default();
        for (style, token) in ranges {
            if token.is_empty() {
                continue;
            }
            let c = style.foreground;
            result.push((egui::Color32::from_rgb(c.r, c.g, c.b), token.to_string()));
        }
        result.push((egui::Color32::TRANSPARENT, "\n".to_string()));
    }
    result
}

fn preview_image(path: &Path) -> PreviewResult {
    let img = match image::open(path) {
        Ok(i) => i,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };

    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();

    let exif = read_exif(path);

    PreviewResult::Image {
        rgba: rgba.into_raw(),
        width: width as usize,
        height: height as usize,
        exif,
    }
}

fn read_exif(path: &Path) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return result,
    };
    let mut reader = std::io::BufReader::new(file);
    let exif = match exif::Reader::new().read_from_container(&mut reader) {
        Ok(e) => e,
        Err(_) => return result,
    };
    for field in exif.fields() {
        let tag = field.tag.to_string();
        let value = field.display_value().to_string();
        if !value.is_empty() && result.len() < 30 {
            result.push((tag, value));
        }
    }
    result
}

fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tiff" | "tif"
    )
}

fn is_markdown_ext(ext: &str) -> bool {
    matches!(ext, "md" | "markdown" | "mdown" | "mkd")
}
