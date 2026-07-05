#![allow(clippy::disallowed_methods)]
// reason: pdftoppm invocation + temp file cleanup run on blocking workers.

use std::path::Path;
use std::process::Command;

use atlas_model::preview_cache::PreviewResult;

use super::MAX_PDF_PREVIEW_EDGE;

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
