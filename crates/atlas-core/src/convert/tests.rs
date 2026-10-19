use super::*;
use std::io::{Read, Write};

fn run(path: &Path, target: &str) -> Report {
    let plan = Plan::build(
        &[path.to_path_buf()],
        target,
        Settings::default(),
        Tools::default(),
    )
    .unwrap();
    execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {})
}

#[test]
fn converts_actual_image_bytes_and_preserves_source_and_colliding_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("foto con ñ.png");
    image::RgbaImage::from_pixel(9, 6, image::Rgba([255, 0, 0, 0]))
        .save(&source)
        .unwrap();
    let original = std::fs::read(&source).unwrap();
    let existing = dir.path().join("foto con ñ.jpg");
    std::fs::write(&existing, b"do not overwrite").unwrap();
    let report = run(&source, "jpg");
    assert!(report.errors().is_empty(), "{:?}", report.errors());
    let outputs = report.outputs();
    assert_eq!(outputs, [dir.path().join("foto con ñ (1).jpg")]);
    let img = image::open(&outputs[0]).unwrap().to_rgb8();
    assert_eq!(img.dimensions(), (9, 6));
    assert!(img.get_pixel(0, 0).0.iter().all(|c| *c > 245));
    assert_eq!(std::fs::read(source).unwrap(), original);
    assert_eq!(std::fs::read(existing).unwrap(), b"do not overwrite");
}

#[test]
fn native_image_outputs_are_decodable_and_resize_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("input.png");
    image::RgbaImage::from_pixel(40, 20, image::Rgba([30, 60, 90, 255]))
        .save(&source)
        .unwrap();
    for target in ["jpg", "webp", "bmp", "tiff", "gif", "ico"] {
        let plan = Plan::build(
            &[source.clone()],
            target,
            Settings {
                max_edge: Some(10),
                ..Settings::default()
            },
            Tools::default(),
        )
        .unwrap();
        let report = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
        assert!(
            report.errors().is_empty(),
            "{target}: {:?}",
            report.errors()
        );
        assert_eq!(
            (
                image::open(&report.outputs()[0]).unwrap().width(),
                image::open(&report.outputs()[0]).unwrap().height()
            ),
            (10, 5)
        );
    }
}

#[test]
fn parallel_conversions_claim_distinct_names_in_a_shared_output_directory() {
    let dir = tempfile::tempdir().unwrap();
    let mut sources = Vec::new();
    for folder in ["a", "b"] {
        let parent = dir.path().join(folder);
        std::fs::create_dir(&parent).unwrap();
        let source = parent.join("same.png");
        image::RgbImage::new(2, 2).save(&source).unwrap();
        sources.push(source);
    }
    let plan = Plan::build(
        &sources,
        "bmp",
        Settings {
            output_dir: Some(dir.path().to_path_buf()),
            ..Settings::default()
        },
        Tools::default(),
    )
    .unwrap();
    let report = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    let outputs = report.outputs();
    assert_eq!(outputs.len(), 2);
    assert_ne!(outputs[0], outputs[1]);
    assert!(outputs.iter().all(|p| image::open(p).is_ok()));
}

#[test]
fn malformed_input_leaves_no_partial_output_and_keeps_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("invalid.png");
    std::fs::write(&source, b"not an image").unwrap();
    assert_eq!(run(&source, "jpg").errors().len(), 1);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert_eq!(std::fs::read(source).unwrap(), b"not an image");
}

#[test]
fn pre_cancelled_batch_does_not_create_any_files() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.json");
    std::fs::write(&source, "[1,2]").unwrap();
    let plan = Plan::build(&[source], "jsonl", Settings::default(), Tools::default()).unwrap();
    let report = execute(&plan, &Arc::new(AtomicBool::new(true)), |_, _, _| {});
    assert_eq!(report.cancelled(), 1);
    assert!(report.outputs().is_empty());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn quoted_unicode_tables_roundtrip_with_embedded_newlines_quotes_and_empty_cells() {
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("table.csv");
    std::fs::write(
        &csv,
        "name,note,empty\r\n\"María, Ana\",\"line 1\r\n\"\"quoted\"\"\",\r\n",
    )
    .unwrap();
    let json = run(&csv, "json").outputs().pop().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(value[0]["name"], "María, Ana");
    assert_eq!(value[0]["note"], "line 1\r\n\"quoted\"");
    assert_eq!(value[0]["empty"], "");
    let tsv = run(&json, "tsv").outputs().pop().unwrap();
    let back = run(&tsv, "json").outputs().pop().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(back).unwrap()).unwrap(),
        value
    );
}

#[test]
fn json_lines_and_toml_use_real_serialization() {
    let dir = tempfile::tempdir().unwrap();
    let toml = dir.path().join("config.toml");
    std::fs::write(&toml, "title = 'Atlas'\n[settings]\nquality = 90\n").unwrap();
    let json = run(&toml, "json").outputs().pop().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(value["settings"]["quality"], 90);
    let back = run(&json, "toml").outputs().pop().unwrap();
    assert!(std::fs::read_to_string(back)
        .unwrap()
        .contains("quality = 90"));
    let lines = dir.path().join("rows.jsonl");
    std::fs::write(&lines, "{\"id\":1}\n{\"id\":2}\n").unwrap();
    let json = run(&lines, "json").outputs().pop().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(json).unwrap())
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn invalid_tables_and_non_tabular_json_fail_without_outputs() {
    let dir = tempfile::tempdir().unwrap();
    for (name, body, target) in [
        ("broken.csv", "a,b\n\"unclosed,b", "json"),
        ("duplicates.csv", "a,a\n1,2", "json"),
        ("ragged.csv", "a,b\n1,2,3", "json"),
        ("scalar.json", "42", "csv"),
    ] {
        let source = dir.path().join(name);
        std::fs::write(&source, body).unwrap();
        assert_eq!(run(&source, target).errors().len(), 1, "{name}");
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 4);
}

#[test]
fn markdown_html_contains_real_markup_and_escapes_raw_html() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    std::fs::write(
        &path,
        "# Title\n\nA **bold** paragraph.\n\n<script>alert(1)</script>",
    )
    .unwrap();
    let html = run(&path, "html").outputs().pop().unwrap();
    let html = std::fs::read_to_string(html).unwrap();
    assert!(html.contains("<h1>Title</h1>"));
    assert!(html.contains("<strong>bold</strong>"));
    assert!(!html.contains("<script>"));
}

#[test]
fn native_archive_repack_preserves_contents_without_extracting() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("files.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&source).unwrap());
    zip.add_directory("nested/", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.start_file("nested/note.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"hello atlas").unwrap();
    zip.finish().unwrap();
    let tar = run(&source, "tar.gz");
    assert!(tar.errors().is_empty(), "{:?}", tar.errors());
    let tar = tar.outputs().pop().unwrap();
    let report = run(&tar, "zip");
    assert!(report.errors().is_empty(), "{:?}", report.errors());
    let mut zip = zip::ZipArchive::new(File::open(&report.outputs()[0]).unwrap()).unwrap();
    let mut contents = String::new();
    zip.by_name("nested/note.txt")
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(contents, "hello atlas");
    assert!(!dir.path().join("nested").exists());
}

#[test]
fn archive_traversal_is_rejected_without_publishing_an_output() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("bad.zip");
    let mut zip = zip::ZipWriter::new(File::create(&source).unwrap());
    zip.start_file("../outside.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"bad").unwrap();
    zip.finish().unwrap();
    assert_eq!(run(&source, "tar").errors().len(), 1);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn batch_catalog_requires_every_input_to_be_compatible_and_exposes_missing_engines() {
    let tools = Tools::default();
    let options = options(&[PathBuf::from("a.heic"), PathBuf::from("b.docx")], &tools);
    assert!(options.iter().all(|o| o.target.extension == "pdf"));
    let image = options
        .iter()
        .find(|o| o.target.extension == "pdf")
        .unwrap();
    assert!(!image.available());
    assert!(image.missing.contains(&Tool::Magick));
    assert!(image.missing.contains(&Tool::Office));
    assert!(super::options(&[PathBuf::from("a.unknown")], &tools).is_empty());
    assert!(!super::options(&[PathBuf::from("a.gif")], &tools)
        .iter()
        .find(|o| o.target.extension == "png")
        .unwrap()
        .available());
}

#[test]
fn plan_rejects_directories_missing_engines_and_no_op_batches() {
    let dir = tempfile::tempdir().unwrap();
    assert!(Plan::build(
        &[dir.path().to_path_buf()],
        "zip",
        Settings::default(),
        Tools::default()
    )
    .is_err());
    let source = dir.path().join("audio.mp3");
    std::fs::write(&source, b"placeholder").unwrap();
    assert!(Plan::build(
        &[source.clone()],
        "wav",
        Settings::default(),
        Tools::default()
    )
    .is_err());
    assert!(Plan::build(&[source], "mp3", Settings::default(), Tools::default()).is_err());
}

use std::fs::File;

#[test]
fn native_pdf_has_valid_cross_references_and_a_lossless_rgb_image_stream() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("photo.png");
    image::RgbImage::from_pixel(13, 7, image::Rgb([11, 22, 33]))
        .save(&source)
        .unwrap();
    let report = run(&source, "pdf");
    assert!(report.errors().is_empty(), "{:?}", report.errors());
    let bytes = std::fs::read(&report.outputs()[0]).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.4"));
    let text = String::from_utf8_lossy(&bytes);
    let xref: usize = text
        .split("startxref\n")
        .nth(1)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(&bytes[xref..xref + 4], b"xref");
    assert!(text.contains("/Width 13 /Height 7"));
    assert!(text.contains("/Filter /FlateDecode"));
}

#[test]
fn docx_export_is_an_ooxml_package_with_escaped_text() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("text.docx");
    super::native::extracted_document("A < B & C\nHola, María", "docx", &output).unwrap();
    let mut zip = zip::ZipArchive::new(File::open(output).unwrap()).unwrap();
    assert!(zip.by_name("[Content_Types].xml").is_ok());
    assert!(zip.by_name("_rels/.rels").is_ok());
    let mut document = String::new();
    zip.by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut document)
        .unwrap();
    assert!(document.contains("A &lt; B &amp; C"));
    assert!(document.contains("Hola, María"));
}

#[test]
fn installed_ffmpeg_converts_real_audio_when_available() {
    let tools = Tools::discover();
    if !tools.has(Tool::Ffmpeg) {
        eprintln!("FFmpeg integration not run: engine not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("audio con ñ.wav");
    let samples = 8000_u32;
    let mut wav = File::create(&source).unwrap();
    wav.write_all(b"RIFF").unwrap();
    wav.write_all(&(36 + samples * 2).to_le_bytes()).unwrap();
    wav.write_all(b"WAVEfmt ").unwrap();
    wav.write_all(&16_u32.to_le_bytes()).unwrap();
    wav.write_all(&1_u16.to_le_bytes()).unwrap();
    wav.write_all(&1_u16.to_le_bytes()).unwrap();
    wav.write_all(&8000_u32.to_le_bytes()).unwrap();
    wav.write_all(&16000_u32.to_le_bytes()).unwrap();
    wav.write_all(&2_u16.to_le_bytes()).unwrap();
    wav.write_all(&16_u16.to_le_bytes()).unwrap();
    wav.write_all(b"data").unwrap();
    wav.write_all(&(samples * 2).to_le_bytes()).unwrap();
    wav.write_all(&vec![0; samples as usize * 2]).unwrap();
    drop(wav);
    let plan = Plan::build(&[source], "mp3", Settings::default(), tools).unwrap();
    let report = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    assert!(report.errors().is_empty(), "{:?}", report.errors());
    assert!(std::fs::metadata(&report.outputs()[0]).unwrap().len() > 100);
}

#[test]
fn installed_seven_zip_repacks_an_actual_archive_when_available() {
    let tools = Tools::discover();
    if !tools.has(Tool::SevenZip) {
        eprintln!("7-Zip integration not run: engine not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.zip");
    let mut zip = zip::ZipWriter::new(File::create(&source).unwrap());
    zip.start_file("note.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"7zip roundtrip").unwrap();
    zip.finish().unwrap();
    let plan = Plan::build(&[source], "7z", Settings::default(), tools.clone()).unwrap();
    let report = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    assert!(report.errors().is_empty(), "{:?}", report.errors());
    let plan = Plan::build(&report.outputs(), "zip", Settings::default(), tools).unwrap();
    let back = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    assert!(back.errors().is_empty(), "{:?}", back.errors());
    let mut zip = zip::ZipArchive::new(File::open(&back.outputs()[0]).unwrap()).unwrap();
    let mut text = String::new();
    zip.by_name("note.txt")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert_eq!(text, "7zip roundtrip");
}

#[test]
fn installed_imagemagick_converts_extended_image_format_when_available() {
    let tools = Tools::discover();
    if !tools.has(Tool::Magick) {
        eprintln!("ImageMagick integration not run: engine not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("sample.png");
    image::RgbImage::from_pixel(12, 8, image::Rgb([50, 100, 150]))
        .save(&source)
        .unwrap();
    let plan = Plan::build(&[source], "avif", Settings::default(), tools).unwrap();
    let report = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    assert!(report.errors().is_empty(), "{:?}", report.errors());
    assert!(std::fs::read(&report.outputs()[0])
        .unwrap()
        .windows(4)
        .any(|w| w == b"avif"));
}

#[test]
fn native_text_and_docx_roundtrip_without_an_office_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("note.txt");
    std::fs::write(&source, "Primera línea\nA < B & C\n").unwrap();
    let docx = run(&source, "docx");
    assert!(docx.errors().is_empty(), "{:?}", docx.errors());
    let text = run(&docx.outputs()[0], "txt");
    assert!(text.errors().is_empty(), "{:?}", text.errors());
    assert_eq!(
        std::fs::read_to_string(&text.outputs()[0]).unwrap(),
        "Primera línea\nA < B & C\n"
    );
}
