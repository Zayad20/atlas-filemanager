use super::*;
use std::io::Write;

/// Two differently colored pages, with rotation, vector graphics and searchable text.
fn fixture() -> Vec<u8> {
    let streams = [
        "1 0 0 rg 0 0 72 144 re f 0 0 0 rg BT /F1 8 Tf 4 130 Td (First page) Tj ET",
        "0 0 1 rg 0 0 72 144 re f 1 1 1 rg q 0 1 -1 0 60 4 cm BT /F1 8 Tf 0 0 Td (Second page) Tj ET Q",
    ];
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 144] /Resources << /Font << /F1 7 0 R >> >> /Contents 4 0 R >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", streams[0].len(), streams[0]),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 144] /Rotate 90 /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", streams[1].len(), streams[1]),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        write!(pdf, "{} 0 obj\n{}\nendobj\n", index + 1, object).unwrap();
    }
    let xref = pdf.len();
    write!(pdf, "xref\n0 {}\n0000000000 65535 f \n", offsets.len()).unwrap();
    for offset in &offsets[1..] {
        writeln!(pdf, "{offset:010} 00000 n ").unwrap();
    }
    write!(
        pdf,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
        offsets.len()
    )
    .unwrap();
    pdf
}

#[test]
fn windows_pdf_exports_all_pages_at_300_dpi_without_external_engines() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("páginas 日本語.pdf");
    let original = fixture();
    std::fs::write(&source, &original).unwrap();
    for extension in ["png", "jpg"] {
        for edge in [None, Some(120)] {
            let plan = Plan::build(
                std::slice::from_ref(&source),
                extension,
                Settings {
                    max_edge: edge,
                    ..Settings::default()
                },
                Tools::default(),
            )
            .unwrap();
            assert_eq!(plan.commands[0].backend, Backend::PdfPages);
            assert!(plan.tools.0.is_empty());
            let report = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
            assert!(report.errors().is_empty(), "{:?}", report.errors());
            let output = report.outputs()[0].clone();
            assert!(output.is_dir());
            assert_eq!(std::fs::read_dir(&output).unwrap().count(), 2);
            for index in 1..=2 {
                let image = image::open(output.join(format!("page-{index:04}.{extension}")))
                    .unwrap()
                    .to_rgb8();
                let expected = match (index, edge) {
                    (1, None) => (300, 600),
                    (2, None) => (600, 300),
                    (1, Some(_)) => (60, 120),
                    _ => (120, 60),
                };
                assert_eq!(image.dimensions(), expected);
                let color = image.get_pixel(expected.0 / 2, expected.1 / 2).0;
                let channel = if index == 1 { 0 } else { 2 };
                assert!(
                    color[channel] > 245
                        && color[(channel + 1) % 3] < 10
                        && color[(channel + 2) % 3] < 10,
                    "{color:?}"
                );
            }
            if let Some(artifact_dir) = std::env::var_os("ATLAS_PAGE_TEST_ARTIFACTS") {
                let artifact_dir = PathBuf::from(artifact_dir)
                    .join(format!("pdf-{extension}-{}", edge.unwrap_or(0)));
                std::fs::create_dir_all(&artifact_dir).unwrap();
                std::fs::write(artifact_dir.join("original.pdf"), &original).unwrap();
                for entry in std::fs::read_dir(output).unwrap() {
                    let entry = entry.unwrap();
                    std::fs::copy(entry.path(), artifact_dir.join(entry.file_name())).unwrap();
                }
            }
        }
    }
    assert_eq!(std::fs::read(source).unwrap(), original);
}

#[test]
fn page_export_rejects_invalid_and_cancelled_jobs_and_preserves_existing_folders() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.pdf");
    std::fs::write(&source, b"not a PDF").unwrap();
    let plan = Plan::build(
        std::slice::from_ref(&source),
        "png",
        Settings::default(),
        Tools::default(),
    )
    .unwrap();
    let failed = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    assert_eq!(failed.errors().len(), 1);
    assert!(failed.outputs().is_empty());
    let cancelled = execute(&plan, &Arc::new(AtomicBool::new(true)), |_, _, _| {});
    assert_eq!(cancelled.cancelled(), 1);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    std::fs::write(&source, fixture()).unwrap();
    let existing = dir.path().join("source - PNG");
    std::fs::create_dir(&existing).unwrap();
    std::fs::write(existing.join("keep.txt"), b"keep").unwrap();
    let created = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    assert!(created.errors().is_empty(), "{:?}", created.errors());
    assert_eq!(created.outputs()[0], dir.path().join("source - PNG (1)"));
    assert_eq!(std::fs::read(existing.join("keep.txt")).unwrap(), b"keep");
    // A colliding ordinary file also must never be replaced.
    std::fs::write(dir.path().join("source - PNG (2)"), b"keep file").unwrap();
    let created = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    assert!(created.errors().is_empty(), "{:?}", created.errors());
    assert_eq!(created.outputs()[0], dir.path().join("source - PNG (3)"));
}

#[test]
fn windows_ocr_reads_every_pdf_page_without_poppler_and_docx_retains_page_breaks() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("ocr.pdf");
    std::fs::write(&source, fixture()).unwrap();
    for extension in ["txt", "md", "html", "docx"] {
        let tools = Tools::discover().native_and_opus();
        assert!(
            tools.1.ocr,
            "Validation machine needs a Windows OCR language"
        );
        let plan = Plan::build_native(
            std::slice::from_ref(&source),
            extension,
            Settings::default(),
            tools,
        )
        .unwrap();
        assert_eq!(plan.commands[0].backend, Backend::SystemPdfText);
        let result = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
        assert!(result.errors().is_empty(), "{:?}", result.errors());
        let output = result.outputs()[0].clone();
        if ["txt", "md"].contains(&extension) {
            let text = std::fs::read_to_string(&output).unwrap();
            assert!(text.to_lowercase().contains("first"), "{text}");
            assert!(text.to_lowercase().contains("second"), "{text}");
            assert_eq!(text.matches('\u{c}').count(), 1);
        } else if extension == "html" {
            let text = std::fs::read_to_string(&output).unwrap();
            assert_eq!(text.matches("<section").count(), 2);
            assert!(text.contains("First page") && text.contains("Second page"));
            assert_eq!(text.matches("break-before:page").count(), 1);
        } else {
            let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
            let mut xml = String::new();
            std::io::Read::read_to_string(
                &mut archive.by_name("word/document.xml").unwrap(),
                &mut xml,
            )
            .unwrap();
            let doc = roxmltree::Document::parse(&xml).unwrap();
            assert_eq!(
                doc.descendants()
                    .filter(|n| n.has_tag_name((
                        "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
                        "br"
                    )))
                    .count(),
                1
            );
        }
        if let Some(artifacts) = std::env::var_os("ATLAS_PAGE_TEST_ARTIFACTS") {
            std::fs::create_dir_all(&artifacts).unwrap();
            std::fs::copy(
                output,
                PathBuf::from(artifacts).join(format!("ocr.{extension}")),
            )
            .unwrap();
        }
    }
}

#[test]
fn windows_ocr_recognizes_image_only_pdf_without_a_text_layer() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("scan-source.svg");
    std::fs::write(&source, r#"<svg xmlns="http://www.w3.org/2000/svg" width="640" height="320"><rect width="640" height="320" fill="white"/><text x="40" y="120" font-family="Arial" font-size="32" fill="black">First page</text></svg>"#).unwrap();
    let raster =
        Plan::build_native(&[source], "png", Settings::default(), Tools::default()).unwrap();
    let raster = execute(&raster, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    assert!(raster.errors().is_empty(), "{:?}", raster.errors());
    let image_plan = Plan::build_native(
        &raster.outputs(),
        "pdf",
        Settings::default(),
        Tools::default(),
    )
    .unwrap();
    let scan = execute(&image_plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    assert!(scan.errors().is_empty(), "{:?}", scan.errors());
    let scan = scan.outputs()[0].clone();
    let tools = Tools::discover().native_and_opus();
    let plan = Plan::build_native(
        std::slice::from_ref(&scan),
        "txt",
        Settings::default(),
        tools,
    )
    .unwrap();
    let recognized = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    assert!(recognized.errors().is_empty(), "{:?}", recognized.errors());
    let text = std::fs::read_to_string(&recognized.outputs()[0]).unwrap();
    assert!(text.contains("First page"), "{text}");
    if let Some(directory) = std::env::var_os("ATLAS_PAGE_TEST_ARTIFACTS") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::copy(scan, directory.join("scan.pdf")).unwrap();
        std::fs::copy(&recognized.outputs()[0], directory.join("scan.txt")).unwrap();
    }
}
