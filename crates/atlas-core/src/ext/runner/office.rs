#![allow(clippy::disallowed_methods)]
// reason: preview functions run on blocking workers and must touch files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use atlas_model::preview_cache::{PreviewResult, SlidePreview};

pub(crate) fn builtin_document_preview(
    path: &Path,
    tools: &HashMap<String, PathBuf>,
) -> PreviewResult {
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
        "psd" | "psb" => {
            if let Some(tool) = tools.get("magick") {
                super::img::run_magick_preview(path, tool)
            } else {
                PreviewResult::Error("Preview unavailable: ImageMagick not found".into())
            }
        }
        _ => PreviewResult::Error(format!("No previewer for .{ext}")),
    }
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

pub(crate) fn xml_text_blocks(xml: &str) -> Vec<String> {
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
