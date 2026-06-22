#![allow(clippy::disallowed_methods)]
// reason: extension commands/previews run on blocking workers and must touch files.
//! Execute declarative extension commands: substitute placeholders in the `run`
//! template and spawn the process(es). No shell is used — the template is split
//! into program + args and placeholders are substituted per token, so paths with
//! spaces are safe and there is no shell-injection surface.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::registry::CommandRef;
use atlas_model::app_state::BgResult;
use atlas_model::ext_model::{ExtKind, PreviewOutput};
use atlas_model::pane::TabId;
use atlas_model::preview_cache::{PreviewResult, SlidePreview};

#[derive(Serialize)]
struct WasmPayload {
    paths: Vec<String>,
}

/// Substitute placeholders for one input path into a single argument token.
// reason: "{dir}"/"{stem}"/"{ext}" are literal template placeholders for .replace(),
// not format-macro args (they happen to match local variable names).
#[allow(clippy::literal_string_with_formatting_args)]
fn subst_token(token: &str, path: &Path, answers: &HashMap<String, String>) -> String {
    let dir = path
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = token
        .replace("{input}", &path.to_string_lossy())
        .replace("{dir}", &dir)
        .replace("{stem}", &stem)
        .replace("{ext}", &ext)
        .replace("{output}", &format!("{dir}/{stem}_out.{ext}"));
    for (name, value) in answers {
        out = out.replace(&format!("{{prompt:{name}}}"), value);
    }
    out
}

/// Build the concrete argv for one invocation over `inputs` (one path for
/// per-file commands, all paths for batch commands).
fn build_argv(run: &str, inputs: &[PathBuf], answers: &HashMap<String, String>) -> Vec<String> {
    let primary = inputs.first().cloned().unwrap_or_default();
    let mut argv = Vec::new();
    for token in run.split_whitespace() {
        if token == "{input}" && inputs.len() > 1 {
            // Batch: expand a bare {input} to every path as separate args.
            for p in inputs {
                argv.push(p.to_string_lossy().into_owned());
            }
        } else {
            argv.push(subst_token(token, &primary, answers));
        }
    }
    argv
}

/// Run a thumbnailer command for one `input`: substitute `{input}`/`{output}` (a temp
/// PNG) plus `{dir}`/`{stem}`/`{ext}`, spawn the process, then decode the produced PNG
/// into a downscaled RGBA thumbnail. Returns `None` on any failure (no shell is used).
/// Blocking — call from a blocking task.
// reason: the temp-PNG cleanup uses sync fs on the blocking pool, never the gpui render
// thread; the sync-fs ban targets UI-foreground blocking (same as fs/preview.rs).
#[allow(clippy::disallowed_methods)]
pub fn run_thumbnail(
    input: &Path,
    run_tmpl: &str,
    tools: &HashMap<String, PathBuf>,
    max_edge: u32,
) -> Option<(Vec<u8>, usize, usize)> {
    let output = std::env::temp_dir().join(format!("atlas-extthumb-{}.png", uuid::Uuid::new_v4()));
    let argv = build_thumb_argv(run_tmpl, input, &output);
    let (program, args) = argv.split_first()?;
    let program = tools
        .get(program)
        .map_or_else(|| PathBuf::from(program), PathBuf::clone);
    let ran = std::process::Command::new(program).args(args).output();
    let ok = matches!(&ran, Ok(out) if out.status.success());
    let decoded = if ok {
        image::open(&output).ok().map(|img| {
            let thumb = img.thumbnail(max_edge, max_edge).to_rgba8();
            let (w, h) = (thumb.width() as usize, thumb.height() as usize);
            (thumb.into_raw(), w, h)
        })
    } else {
        None
    };
    let _ = std::fs::remove_file(&output);
    decoded
}

// reason: literal template placeholders for .replace(), not format-macro args.
#[allow(clippy::literal_string_with_formatting_args)]
fn build_thumb_argv(run: &str, input: &Path, output: &Path) -> Vec<String> {
    let dir = input
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = input
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    run.split_whitespace()
        .map(|token| {
            token
                .replace("{input}", &input.to_string_lossy())
                .replace("{output}", &output.to_string_lossy())
                .replace("{dir}", &dir)
                .replace("{stem}", &stem)
                .replace("{ext}", &ext)
        })
        .collect()
}

pub fn run_preview(
    rt: &tokio::runtime::Handle,
    bg_tx: flume::Sender<BgResult>,
    cmd: CommandRef,
    tab: TabId,
    path: PathBuf,
) {
    rt.spawn_blocking(move || {
        let result = if cmd.ext_id == "document-previews" {
            builtin_document_preview(&path, &cmd.tools)
        } else if cmd.kind == ExtKind::Wasm {
            run_wasm_preview(&cmd, &path).unwrap_or_else(|e| PreviewResult::Error(e.to_string()))
        } else {
            run_declarative_preview(&cmd, &path)
        };
        let _ = bg_tx.send(BgResult::PreviewReady { tab, path, result });
    });
}

fn run_declarative_preview(cmd: &CommandRef, input: &Path) -> PreviewResult {
    let ext = match cmd.command.output {
        PreviewOutput::Image => "png",
        _ => "json",
    };
    let output =
        std::env::temp_dir().join(format!("atlas-preview-{}.{}", uuid::Uuid::new_v4(), ext));
    let argv = build_thumb_argv(&cmd.command.run, input, &output);
    let Some((program, args)) = argv.split_first() else {
        return PreviewResult::Error("Preview extension has no command".into());
    };
    let program = cmd
        .tools
        .get(program)
        .map_or_else(|| PathBuf::from(program), PathBuf::clone);
    let ran = std::process::Command::new(program).args(args).output();
    let result = match ran {
        Ok(out) if out.status.success() => match cmd.command.output {
            PreviewOutput::Image => image_file_to_preview(&output),
            _ => std::fs::read_to_string(&output)
                .map_err(|e| e.to_string())
                .and_then(|json| ext_preview_from_json(&json))
                .unwrap_or_else(PreviewResult::Error),
        },
        Ok(out) => PreviewResult::Error(String::from_utf8_lossy(&out.stderr).trim().to_string()),
        Err(e) => PreviewResult::Error(e.to_string()),
    };
    let _ = std::fs::remove_file(output);
    result
}

fn run_wasm_preview(cmd: &CommandRef, input: &Path) -> anyhow::Result<PreviewResult> {
    let wasm_path = cmd.ext_dir.join("plugin.wasm");
    let wasm_bytes = std::fs::read(&wasm_path)?;
    let (tx, _rx) = flume::unbounded();
    let manager = crate::ext::wasm_manager::WasmPluginManager::new(
        wasm_bytes,
        cmd.ext_dir.clone(),
        tx,
        cmd.tools.clone(),
    );
    let payload = serde_json::json!({
        "path": input.to_string_lossy(),
        "extension": input.extension().and_then(|e| e.to_str()).unwrap_or(""),
        "max_edge": 1600
    });
    let json = manager.execute_preview(&payload.to_string())?;
    ext_preview_from_json(&json).map_err(anyhow::Error::msg)
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum ExtPreview {
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
        #[serde(default)]
        truncated: bool,
    },
    Document {
        #[serde(default)]
        title: String,
        blocks: Vec<String>,
        #[serde(default)]
        truncated: bool,
    },
    Presentation {
        slides: Vec<ExtSlide>,
        #[serde(default)]
        truncated: bool,
    },
    Error {
        message: String,
    },
}

#[derive(serde::Deserialize)]
struct ExtSlide {
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: Vec<String>,
}

fn ext_preview_from_json(json: &str) -> Result<PreviewResult, String> {
    match serde_json::from_str::<ExtPreview>(json).map_err(|e| e.to_string())? {
        ExtPreview::Table {
            headers,
            rows,
            truncated,
        } => Ok(PreviewResult::Table {
            headers,
            rows,
            truncated,
        }),
        ExtPreview::Document {
            title,
            blocks,
            truncated,
        } => Ok(PreviewResult::Document {
            title,
            blocks,
            truncated,
        }),
        ExtPreview::Presentation { slides, truncated } => Ok(PreviewResult::Presentation {
            slides: slides
                .into_iter()
                .map(|s| SlidePreview {
                    title: s.title,
                    body: s.body,
                })
                .collect(),
            truncated,
        }),
        ExtPreview::Error { message } => Ok(PreviewResult::Error(message)),
    }
}

fn image_file_to_preview(path: &Path) -> PreviewResult {
    match image::open(path) {
        Ok(img) => {
            let original_width = img.width() as usize;
            let original_height = img.height() as usize;
            let img = img.thumbnail(1600, 1600).to_rgba8();
            let (width, height) = img.dimensions();
            PreviewResult::Image {
                rgba: img.into_raw(),
                width: width as usize,
                height: height as usize,
                original_width,
                original_height,
                exif: Vec::new(),
            }
        }
        Err(e) => PreviewResult::Error(e.to_string()),
    }
}

fn builtin_document_preview(path: &Path, tools: &HashMap<String, PathBuf>) -> PreviewResult {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "csv" => delimited_preview(path, b','),
        "tsv" => delimited_preview(path, b'\t'),
        "xls" | "xlsx" | "ods" => spreadsheet_preview(path),
        "docx" => docx_preview(path),
        "pptx" => pptx_preview(path),
        "pdf" => {
            let Some(tool) = tools.get("pdftoppm") else {
                return PreviewResult::Error("Preview unavailable: pdftoppm not found".into());
            };
            crate::fs::preview::preview_pdf_with_pdftoppm(path, tool)
        }
        "psd" | "psb" => magick_preview(path, tools),
        _ => PreviewResult::Error(format!("No previewer for .{ext}")),
    }
}

fn magick_preview(path: &Path, tools: &HashMap<String, PathBuf>) -> PreviewResult {
    if let Some(tool) = tools.get("magick") {
        run_magick_preview(path, &tool)
    } else {
        PreviewResult::Error("Preview unavailable: ImageMagick not found".into())
    }
}

fn run_magick_preview(path: &Path, magick: &Path) -> PreviewResult {
    let output =
        std::env::temp_dir().join(format!("atlas-magick-preview-{}.png", uuid::Uuid::new_v4()));
    let input = format!("{}[0]", path.to_string_lossy());
    let ran = std::process::Command::new(magick)
        .args([
            input.as_str(),
            "-flatten",
            "-resize",
            "1600x1600>",
            output.to_string_lossy().as_ref(),
        ])
        .output();
    let result = match ran {
        Ok(out) if out.status.success() => image_file_to_preview(&output),
        Ok(out) => PreviewResult::Error(String::from_utf8_lossy(&out.stderr).trim().to_string()),
        Err(e) => PreviewResult::Error(e.to_string()),
    };
    let _ = std::fs::remove_file(output);
    result
}

fn delimited_preview(path: &Path, delimiter: u8) -> PreviewResult {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut rows = Vec::new();
    for line in text.lines().take(201) {
        rows.push(
            parse_delimited_line(line, delimiter)
                .into_iter()
                .take(50)
                .collect(),
        );
    }
    let truncated = text.lines().nth(201).is_some();
    let headers = if rows.is_empty() {
        Vec::new()
    } else {
        rows.remove(0)
    };
    PreviewResult::Table {
        headers,
        rows,
        truncated,
    }
}

fn parse_delimited_line(line: &str, delimiter: u8) -> Vec<String> {
    let delimiter = delimiter as char;
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = line.chars().peekable();
    let mut quoted = false;
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    cell.push('"');
                    let _ = chars.next();
                } else {
                    quoted = false;
                }
            } else {
                cell.push(ch);
            }
        } else if ch == '"' && cell.is_empty() {
            quoted = true;
        } else if ch == delimiter {
            cells.push(std::mem::take(&mut cell));
        } else {
            cell.push(ch);
        }
    }
    cells.push(cell);
    cells
}

fn spreadsheet_preview(path: &Path) -> PreviewResult {
    use calamine::{open_workbook_auto, Reader};
    let mut workbook = match open_workbook_auto(path) {
        Ok(w) => w,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };
    let Some(sheet) = workbook.sheet_names().first().cloned() else {
        return PreviewResult::Table {
            headers: Vec::new(),
            rows: Vec::new(),
            truncated: false,
        };
    };
    let range = match workbook.worksheet_range(&sheet) {
        Ok(r) => r,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };
    let mut rows: Vec<Vec<String>> = range
        .rows()
        .take(201)
        .map(|row| row.iter().take(50).map(cell_to_string).collect())
        .collect();
    let truncated = range.height() > 201 || range.width() > 50;
    let headers = if rows.is_empty() {
        Vec::new()
    } else {
        rows.remove(0)
    };
    PreviewResult::Table {
        headers,
        rows,
        truncated,
    }
}

fn cell_to_string(cell: &calamine::Data) -> String {
    match cell {
        calamine::Data::Empty => String::new(),
        calamine::Data::String(s) => s.clone(),
        calamine::Data::Float(n) => n.to_string(),
        calamine::Data::Int(n) => n.to_string(),
        calamine::Data::Bool(v) => v.to_string(),
        calamine::Data::DateTime(v) => v.to_string(),
        calamine::Data::DateTimeIso(s) | calamine::Data::DurationIso(s) => s.clone(),
        calamine::Data::Error(e) => format!("{e:?}"),
    }
}

fn docx_preview(path: &Path) -> PreviewResult {
    let text = read_zip_xml_text(path, "word/document.xml");
    match text {
        Ok(blocks) => PreviewResult::Document {
            title: path
                .file_stem()
                .map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
            truncated: blocks.len() > 200,
            blocks: blocks.into_iter().take(200).collect(),
        },
        Err(e) => PreviewResult::Error(e),
    }
}

fn pptx_preview(path: &Path) -> PreviewResult {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };
    let mut zip = match zip::ZipArchive::new(file) {
        Ok(z) => z,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };
    let mut names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
        .collect();
    names.sort();
    let mut slides = Vec::new();
    for name in names.into_iter().take(50) {
        let Ok(mut entry) = zip.by_name(&name) else {
            continue;
        };
        let mut xml = String::new();
        if std::io::Read::read_to_string(&mut entry, &mut xml).is_ok() {
            let blocks = xml_text_blocks(&xml);
            let title = blocks.first().cloned().unwrap_or_default();
            slides.push(SlidePreview {
                title,
                body: blocks.into_iter().skip(1).take(12).collect(),
            });
        }
    }
    PreviewResult::Presentation {
        truncated: slides.len() >= 50,
        slides,
    }
}

fn read_zip_xml_text(path: &Path, name: &str) -> Result<Vec<String>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut entry = zip.by_name(name).map_err(|e| e.to_string())?;
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut entry, &mut xml).map_err(|e| e.to_string())?;
    Ok(xml_text_blocks(&xml))
}

fn xml_text_blocks(xml: &str) -> Vec<String> {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in xml.chars() {
        match ch {
            '<' => {
                in_tag = true;
                out.push('\n');
            }
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_preview_json_becomes_error() {
        assert!(ext_preview_from_json("{ nope").is_err());
    }

    #[test]
    fn csv_line_parser_handles_quotes() {
        assert_eq!(
            parse_delimited_line(r#"a,"b,c","d""e""#, b','),
            vec!["a", "b,c", "d\"e"]
        );
    }

    #[test]
    fn xml_text_extracts_basic_ooxml_text() {
        assert_eq!(
            xml_text_blocks("<w:p><w:t>Hello</w:t></w:p><w:p><w:t>World</w:t></w:p>"),
            vec!["Hello", "World"]
        );
    }
}

/// Run a declarative command over `paths`, reporting completion via `BgResult`.
/// Sidecar commands are not yet executed (declarative covers the shipped tools).
pub fn run(
    rt: &tokio::runtime::Handle,
    bg_tx: flume::Sender<BgResult>,
    cmd: CommandRef,
    paths: Vec<PathBuf>,
    answers: HashMap<String, String>,
) {
    let ext_id = cmd.ext_id.clone();

    // Si la extensión es WASM, delegamos la ejecución al nuevo manager genérico
    if cmd.kind == atlas_model::ext_model::ExtKind::Wasm {
        rt.spawn_blocking(move || {
            let data_dir = crate::util::config::data_dir()
                .join("extensions")
                .join(&ext_id);

            // TODO: In production we'd load the real .wasm file declared in Manifest.
            // For now, assume it's in the extension directory:
            let wasm_path = cmd.ext_dir.join("plugin.wasm");
            #[allow(clippy::disallowed_methods)] // reason: already inside spawn_blocking
            let wasm_bytes = std::fs::read(&wasm_path).unwrap_or_default();

            let manager = crate::ext::wasm_manager::WasmPluginManager::new(
                wasm_bytes,
                data_dir,
                bg_tx.clone(),
                cmd.tools.clone(),
            );

            let wasm_payload = WasmPayload {
                paths: paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
            };
            let payload_json = serde_json::to_string(&wasm_payload).unwrap_or_default();

            tracing::info!("Delegando comando WASM {} a {}", cmd.command.id, ext_id);
            if let Err(e) = manager.execute_command(&cmd.command.id, &payload_json) {
                tracing::error!("Error executing WASM: {:#}", e);
            }

            let dirs: Vec<PathBuf> = paths
                .iter()
                .filter_map(|p| p.parent().map(Path::to_path_buf))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();

            let _ = bg_tx.send(BgResult::ExtCommandDone {
                ext_id,
                error: None,
                dirs,
            });
        });
        return;
    }

    let run_tmpl = cmd.command.run.clone();
    if run_tmpl.trim().is_empty() {
        return;
    }
    let per_file = cmd.command.per_file;
    let ext_id = cmd.ext_id;

    // Directories to refresh once the command finishes.
    let dirs: HashSet<PathBuf> = paths
        .iter()
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .collect();

    rt.spawn_blocking(move || {
        let batches: Vec<Vec<PathBuf>> = if per_file {
            paths.iter().map(|p| vec![p.clone()]).collect()
        } else {
            vec![paths.clone()]
        };

        let mut errors = Vec::new();
        for batch in batches {
            let argv = build_argv(&run_tmpl, &batch, &answers);
            let Some((program, args)) = argv.split_first() else {
                continue;
            };
            let program = cmd
                .tools
                .get(program)
                .map_or_else(|| PathBuf::from(program), PathBuf::clone);
            match std::process::Command::new(program).args(args).output() {
                Ok(out) if out.status.success() => {}
                Ok(out) => errors.push(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                Err(e) => errors.push(e.to_string()),
            }
        }

        let _ = bg_tx.send(BgResult::ExtCommandDone {
            ext_id,
            error: errors.into_iter().find(|e| !e.is_empty()),
            dirs: dirs.into_iter().collect(),
        });
    });
}
