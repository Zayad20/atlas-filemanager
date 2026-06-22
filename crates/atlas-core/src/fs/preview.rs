#![allow(dead_code)]
// reason: retained API surface not exercised by every GPUI flow yet
// reason: preview generation + temp-file cleanup run on background workers, never
// the gpui render thread; the sync-fs ban targets UI-foreground blocking.
#![allow(clippy::disallowed_methods)]
use std::path::{Path, PathBuf};
use std::process::Command;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

use atlas_model::app_state::BgResult;
use atlas_model::pane::TabId;
use atlas_model::preview_cache::{
    is_audio_ext, is_image_ext, DirPreviewEntry, ImageData, MarkdownBlock, MdSpan, MultiPreviewKey,
    PreviewResult, DIR_PREVIEW_LIMIT,
};

static SYNTAX_SET: std::sync::LazyLock<SyntaxSet> =
    std::sync::LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: std::sync::LazyLock<ThemeSet> = std::sync::LazyLock::new(ThemeSet::load_defaults);

const MAX_TEXT_BYTES: usize = 256 * 1024; // 256 KB
const MAX_IMAGE_PREVIEW_EDGE: u32 = 1600;
const MAX_MULTI_THUMBNAIL_EDGE: u32 = 384;
const MAX_PDF_PREVIEW_EDGE: &str = "1024";

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
        tokio::task::spawn_blocking(move || preview_pdf_public(&p))
            .await
            .unwrap_or_else(|e| PreviewResult::Error(e.to_string()))
    } else if is_audio_ext(&ext) {
        let p = path.clone();
        tokio::task::spawn_blocking(move || preview_audio(&p))
            .await
            .unwrap_or_else(|e| PreviewResult::Error(e.to_string()))
    } else if is_markdown_ext(&ext) {
        preview_markdown(&path).await
    } else {
        preview_text(&path, &ext).await
    };

    let _ = tx.send(BgResult::PreviewReady { tab, path, result });
}

/// Per-position tilt (degrees) for the multi-selection "butterfly" fan. Only the first
/// four thumbnails are tilted; the rotation is baked into the bitmap because gpui cannot
/// rotate a raster element on screen.
const MULTI_BUTTERFLY_ANGLES: [f32; 4] = [-8.0, -3.0, 3.0, 8.0];

pub async fn load_multi_thumbnail(
    tab: TabId,
    key: MultiPreviewKey,
    index: usize,
    path: PathBuf,
    tx: flume::Sender<BgResult>,
) {
    let thumbnail_path = path.clone();
    let thumbnail = tokio::task::spawn_blocking(move || {
        let (rgba, width, height) = preview_thumbnail(&thumbnail_path)?;
        let angle = MULTI_BUTTERFLY_ANGLES.get(index).copied().unwrap_or(0.0);
        Ok::<_, image::ImageError>(if angle == 0.0 {
            (rgba, width, height)
        } else {
            rotate_rgba(&rgba, width, height, angle)
        })
    })
    .await
    .ok()
    .and_then(Result::ok);
    let Some((rgba, width, height)) = thumbnail else {
        return;
    };
    let _ = tx.send(BgResult::MultiPreviewThumbnailReady {
        tab,
        key,
        path,
        rgba,
        width,
        height,
    });
}

/// Rotate an RGBA8 image by `angle_deg` onto a transparent, padded canvas sized to the
/// rotated bounds (bilinear sampling; corners stay transparent). Returns
/// `(rgba, new_w, new_h)`.
// reason: the affine + bilinear math reads far clearer as plain `a*b + c` than as
// chained `mul_add`; this runs once per multi-preview thumbnail, off the render thread.
#[allow(clippy::suboptimal_flops)]
fn rotate_rgba(rgba: &[u8], w: usize, h: usize, angle_deg: f32) -> (Vec<u8>, usize, usize) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return (rgba.to_vec(), w, h);
    }
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    let (wf, hf) = (w as f32, h as f32);
    let new_w = (wf * cos.abs() + hf * sin.abs()).ceil().max(1.0) as usize;
    let new_h = (wf * sin.abs() + hf * cos.abs()).ceil().max(1.0) as usize;
    let mut out = vec![0u8; new_w * new_h * 4];
    let (cx, cy) = (wf / 2.0, hf / 2.0);
    let (ncx, ncy) = (new_w as f32 / 2.0, new_h as f32 / 2.0);
    let idx = |x: usize, y: usize| (y * w + x) * 4;
    for oy in 0..new_h {
        for ox in 0..new_w {
            // Inverse-rotate the output pixel back into the source image.
            let dx = ox as f32 + 0.5 - ncx;
            let dy = oy as f32 + 0.5 - ncy;
            let sx = cos * dx + sin * dy + cx;
            let sy = -sin * dx + cos * dy + cy;
            if sx < 0.0 || sy < 0.0 || sx >= wf || sy >= hf {
                continue; // outside source → transparent
            }
            let x0 = sx.floor() as usize;
            let y0 = sy.floor() as usize;
            let x1 = (x0 + 1).min(w - 1);
            let y1 = (y0 + 1).min(h - 1);
            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;
            let o = (oy * new_w + ox) * 4;
            for c in 0..4 {
                let p00 = f32::from(rgba[idx(x0, y0) + c]);
                let p10 = f32::from(rgba[idx(x1, y0) + c]);
                let p01 = f32::from(rgba[idx(x0, y1) + c]);
                let p11 = f32::from(rgba[idx(x1, y1) + c]);
                let top = p00 + (p10 - p00) * fx;
                let bot = p01 + (p11 - p01) * fx;
                out[o + c] = (top + (bot - top) * fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    (out, new_w, new_h)
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
        Ok(source) => {
            let base_uri = path.parent().map_or_else(
                || "file://".to_string(),
                |parent| format!("file://{}/", parent.display()),
            );
            let blocks = parse_markdown(&source);
            PreviewResult::Markdown {
                blocks,
                source,
                base_uri,
            }
        }
        Err(_) => PreviewResult::Binary {
            size: path.metadata().map_or(0, |m| m.len()),
        },
    }
}

/// Parse markdown into neutral blocks using pulldown-cmark. Inline emphasis/strong/
/// code/links become `MdSpan`s; fenced code is syntax-highlighted with syntect.
fn parse_markdown(source: &str) -> Vec<MarkdownBlock> {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

    enum Ctx {
        None,
        Heading(u8),
        Para,
        Quote,
        Item { ordered: bool, depth: u8 },
        Code { lang: String, text: String },
    }

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(source, opts);

    let mut blocks: Vec<MarkdownBlock> = Vec::new();
    let mut spans: Vec<MdSpan> = Vec::new();
    let mut strong = false;
    let mut emph = false;
    let mut link_url: Option<String> = None;
    let mut link_text = String::new();
    let mut ctx = Ctx::None;
    let mut list_stack: Vec<bool> = Vec::new();

    for ev in parser {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    ctx = Ctx::Heading(level as u8);
                    spans.clear();
                }
                Tag::Paragraph => {
                    ctx = Ctx::Para;
                    spans.clear();
                }
                Tag::BlockQuote(_) => {
                    ctx = Ctx::Quote;
                    spans.clear();
                }
                Tag::CodeBlock(kind) => {
                    let lang = match kind {
                        CodeBlockKind::Fenced(s) => s.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    ctx = Ctx::Code {
                        lang,
                        text: String::new(),
                    };
                }
                Tag::List(start) => list_stack.push(start.is_some()),
                Tag::Item => {
                    let ordered = list_stack.last().copied().unwrap_or(false);
                    let depth = list_stack.len().saturating_sub(1) as u8;
                    ctx = Ctx::Item { ordered, depth };
                    spans.clear();
                }
                Tag::Emphasis => emph = true,
                Tag::Strong => strong = true,
                Tag::Link { dest_url, .. } => {
                    link_url = Some(dest_url.to_string());
                    link_text.clear();
                }
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Heading(_) => {
                    if let Ctx::Heading(level) = &ctx {
                        blocks.push(MarkdownBlock::Heading {
                            level: *level,
                            spans: std::mem::take(&mut spans),
                        });
                    }
                    ctx = Ctx::None;
                }
                TagEnd::Paragraph => {
                    blocks.push(MarkdownBlock::Paragraph {
                        spans: std::mem::take(&mut spans),
                    });
                    ctx = Ctx::None;
                }
                TagEnd::BlockQuote(_) => {
                    blocks.push(MarkdownBlock::Quote {
                        spans: std::mem::take(&mut spans),
                    });
                    ctx = Ctx::None;
                }
                TagEnd::CodeBlock => {
                    if let Ctx::Code { lang, text } = std::mem::replace(&mut ctx, Ctx::None) {
                        let lines = highlight_code(&text, &lang);
                        blocks.push(MarkdownBlock::Code { lang, lines });
                    }
                }
                TagEnd::List(_) => {
                    list_stack.pop();
                }
                TagEnd::Item => {
                    if let Ctx::Item { ordered, depth } = &ctx {
                        blocks.push(MarkdownBlock::ListItem {
                            ordered: *ordered,
                            depth: *depth,
                            spans: std::mem::take(&mut spans),
                        });
                    }
                    ctx = Ctx::None;
                }
                TagEnd::Emphasis => emph = false,
                TagEnd::Strong => strong = false,
                TagEnd::Link => {
                    if let Some(url) = link_url.take() {
                        spans.push(MdSpan::Link {
                            text: std::mem::take(&mut link_text),
                            url,
                        });
                    }
                }
                _ => {}
            },
            Event::Text(t) => {
                if link_url.is_some() {
                    link_text.push_str(&t);
                } else if let Ctx::Code { text, .. } = &mut ctx {
                    text.push_str(&t);
                } else if strong {
                    spans.push(MdSpan::Strong(t.to_string()));
                } else if emph {
                    spans.push(MdSpan::Emphasis(t.to_string()));
                } else {
                    spans.push(MdSpan::Text(t.to_string()));
                }
            }
            Event::Code(t) => {
                if link_url.is_some() {
                    link_text.push_str(&t);
                } else {
                    spans.push(MdSpan::Code(t.to_string()));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if link_url.is_some() {
                    link_text.push(' ');
                } else if let Ctx::Code { text, .. } = &mut ctx {
                    text.push('\n');
                } else {
                    spans.push(MdSpan::Text(" ".to_string()));
                }
            }
            Event::Rule => blocks.push(MarkdownBlock::Rule),
            _ => {}
        }
    }
    blocks
}

/// Like `highlight_text` but resolves the syntax by language *token* (markdown fence
/// info string, e.g. `rust`) rather than file extension.
fn highlight_code(text: &str, lang: &str) -> Vec<([u8; 4], String)> {
    let syntax = SYNTAX_SET
        .find_syntax_by_token(lang)
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
            result.push(([c.r, c.g, c.b, 0xff], token.to_string()));
        }
        result.push(([0, 0, 0, 0], "\n".to_string()));
    }
    result
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
    if let Ok(text) = String::from_utf8(bytes) {
        let ext = ext.to_owned();
        tokio::task::spawn_blocking(move || {
            let lines = highlight_text(&text, &ext);
            PreviewResult::Text { lines, text }
        })
        .await
        .unwrap_or_else(|e| PreviewResult::Error(e.to_string()))
    } else {
        let size = path.metadata().map_or(0, |m| m.len());
        PreviewResult::Binary { size }
    }
}

pub fn preview_pdf_public(path: &Path) -> PreviewResult {
    preview_pdf(path, Path::new("pdftoppm"))
}

pub fn preview_pdf_with_pdftoppm(path: &Path, pdftoppm: &Path) -> PreviewResult {
    preview_pdf(path, pdftoppm)
}

fn preview_pdf(path: &Path, pdftoppm: &Path) -> PreviewResult {
    let output_prefix =
        std::env::temp_dir().join(format!("atlas-pdf-preview-{}", uuid::Uuid::new_v4()));
    let png_path = output_prefix.with_extension("png");
    let output = match Command::new(pdftoppm)
        .args([
            "-f",
            "1",
            "-l",
            "1",
            "-scale-to",
            MAX_PDF_PREVIEW_EDGE,
            "-png",
            "-singlefile",
        ])
        .arg(path)
        .arg(&output_prefix)
        .output()
    {
        Ok(output) => output,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return PreviewResult::Error("PDF preview unavailable: pdftoppm not found".to_string());
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

/// Syntax-highlighted spans as framework-neutral `([r, g, b, a], text)`. An alpha of
/// `0` is the "no color / use default foreground" sentinel (line breaks); the UI
/// layer converts the RGBA into its own color type.
fn highlight_text(text: &str, ext: &str) -> Vec<([u8; 4], String)> {
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
            result.push(([c.r, c.g, c.b, 0xff], token.to_string()));
        }
        result.push(([0, 0, 0, 0], "\n".to_string()));
    }
    result
}

fn preview_image(path: &Path) -> PreviewResult {
    let img = match image::open(path) {
        Ok(i) => i,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };

    let original_width = img.width() as usize;
    let original_height = img.height() as usize;
    let img = img.thumbnail(MAX_IMAGE_PREVIEW_EDGE, MAX_IMAGE_PREVIEW_EDGE);
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();

    let exif = read_exif(path);

    PreviewResult::Image {
        rgba: rgba.into_raw(),
        width: width as usize,
        height: height as usize,
        original_width,
        original_height,
        exif,
    }
}

fn preview_audio(path: &Path) -> PreviewResult {
    use lofty::prelude::{Accessor, AudioFile, TaggedFileExt};

    let tagged = match lofty::read_from_path(path) {
        Ok(t) => t,
        Err(e) => return PreviewResult::Error(format!("No se pudo leer el audio: {e}")),
    };
    let props = tagged.properties();
    let duration_secs = props.duration().as_secs();
    let bitrate = props.audio_bitrate().or_else(|| props.overall_bitrate());

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let (title, artist, album, cover) = match tag {
        Some(tag) => {
            let cover = tag
                .pictures()
                .first()
                .and_then(|pic| decode_cover(pic.data()));
            (
                tag.title().map(|c| c.to_string()),
                tag.artist().map(|c| c.to_string()),
                tag.album().map(|c| c.to_string()),
                cover,
            )
        }
        None => (None, None, None, None),
    };

    PreviewResult::Audio {
        title,
        artist,
        album,
        duration_secs,
        bitrate,
        cover,
    }
}

/// Decode embedded cover art bytes into a small RGBA thumbnail.
fn decode_cover(data: &[u8]) -> Option<ImageData> {
    let img = image::load_from_memory(data)
        .ok()?
        .thumbnail(MAX_MULTI_THUMBNAIL_EDGE, MAX_MULTI_THUMBNAIL_EDGE);
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(ImageData {
        rgba: rgba.into_raw(),
        width: width as usize,
        height: height as usize,
    })
}

fn preview_thumbnail(path: &Path) -> Result<(Vec<u8>, usize, usize), image::ImageError> {
    let img = image::open(path)?.thumbnail(MAX_MULTI_THUMBNAIL_EDGE, MAX_MULTI_THUMBNAIL_EDGE);
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok((rgba.into_raw(), width as usize, height as usize))
}

fn read_exif(path: &Path) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let Ok(file) = std::fs::File::open(path) else {
        return result;
    };
    let mut reader = std::io::BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else {
        return result;
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

fn is_markdown_ext(ext: &str) -> bool {
    matches!(ext, "md" | "markdown" | "mdown" | "mkd")
}

/// Preview a single file inside an archive without extracting to disk.
/// Reads up to `MAX_TEXT_BYTES` (text) or 16 MiB (images) from the entry.
pub async fn load_archive_preview(
    tab: TabId,
    archive: PathBuf,
    inner: PathBuf,
    tx: flume::Sender<BgResult>,
) {
    let ext = inner
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let result = {
        let inner2 = inner.clone();
        tokio::task::spawn_blocking(move || -> PreviewResult {
            let provider = match crate::fs::archive::open_with(&archive, None) {
                Some(p) => p,
                None => return PreviewResult::Error("Formato de archivo no soportado".into()),
            };
            let mut reader = match provider.read_bytes(&inner2) {
                Ok(r) => r,
                Err(e) => return PreviewResult::Error(e.to_string()),
            };
            let cap = if is_image_ext(&ext) {
                16 * 1024 * 1024
            } else {
                MAX_TEXT_BYTES
            };
            let mut buf = Vec::with_capacity(cap.min(1024 * 1024));
            let mut tmp = [0u8; 65536];
            loop {
                use std::io::Read as _;
                match reader.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if buf.len() >= cap {
                            break;
                        }
                    }
                    Err(e) => return PreviewResult::Error(e.to_string()),
                }
            }
            bytes_to_preview(buf, &ext)
        })
        .await
        .unwrap_or_else(|e| PreviewResult::Error(e.to_string()))
    };

    let _ = tx.send(BgResult::PreviewReady {
        tab,
        path: inner,
        result,
    });
}

fn bytes_to_preview(bytes: Vec<u8>, ext: &str) -> PreviewResult {
    if is_image_ext(ext) {
        match image::load_from_memory(&bytes) {
            Ok(img) => {
                let original_width = img.width() as usize;
                let original_height = img.height() as usize;
                let img = img.thumbnail(MAX_IMAGE_PREVIEW_EDGE, MAX_IMAGE_PREVIEW_EDGE);
                let rgba = img.to_rgba8();
                let (width, height) = rgba.dimensions();
                PreviewResult::Image {
                    rgba: rgba.into_raw(),
                    width: width as usize,
                    height: height as usize,
                    original_width,
                    original_height,
                    exif: Vec::new(),
                }
            }
            Err(e) => PreviewResult::Error(e.to_string()),
        }
    } else if is_markdown_ext(ext) {
        if bytes.len() > MAX_TEXT_BYTES {
            return PreviewResult::Binary {
                size: bytes.len() as u64,
            };
        }
        match String::from_utf8(bytes) {
            Ok(source) => {
                let blocks = parse_markdown(&source);
                PreviewResult::Markdown {
                    blocks,
                    source,
                    base_uri: String::new(),
                }
            }
            Err(e) => PreviewResult::Binary {
                size: e.into_bytes().len() as u64,
            },
        }
    } else if ext == "pdf" || is_audio_ext(ext) {
        PreviewResult::Binary {
            size: bytes.len() as u64,
        }
    } else {
        if bytes.len() > MAX_TEXT_BYTES {
            return PreviewResult::Binary {
                size: bytes.len() as u64,
            };
        }
        match String::from_utf8(bytes) {
            Ok(text) => {
                let lines = highlight_text(&text, ext);
                PreviewResult::Text { lines, text }
            }
            Err(e) => PreviewResult::Binary {
                size: e.into_bytes().len() as u64,
            },
        }
    }
}
