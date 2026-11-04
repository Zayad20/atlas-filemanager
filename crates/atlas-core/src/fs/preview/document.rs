//! Bounded, in-memory Word text preview, also available without extensions.
use std::io::{Read, Seek};
use std::path::Path;

use atlas_model::preview_cache::PreviewResult;

const MAX_DOCUMENT_XML: u64 = 8 * 1024 * 1024;
const MAX_BLOCKS: usize = 200;

pub(crate) fn preview_docx(path: &Path) -> PreviewResult {
    match std::fs::File::open(path) {
        Ok(file) => preview_docx_reader(
            file,
            &path.file_stem().unwrap_or_default().to_string_lossy(),
        ),
        Err(error) => PreviewResult::Error(error.to_string()),
    }
}

pub(super) fn preview_docx_reader(reader: impl Read + Seek, title: &str) -> PreviewResult {
    match read_blocks(reader) {
        Ok(mut blocks) => {
            let truncated = blocks.len() > MAX_BLOCKS;
            blocks.truncate(MAX_BLOCKS);
            PreviewResult::Document {
                title: title.to_owned(),
                blocks,
                truncated,
            }
        }
        Err(error) => PreviewResult::Error(format!("DOCX: {error}")),
    }
}

fn read_blocks(reader: impl Read + Seek) -> anyhow::Result<Vec<String>> {
    let mut archive = zip::ZipArchive::new(reader)?;
    let entry = archive.by_name("word/document.xml")?;
    anyhow::ensure!(
        entry.size() <= MAX_DOCUMENT_XML,
        "document XML exceeds 8 MiB"
    );
    let mut bytes = Vec::new();
    entry.take(MAX_DOCUMENT_XML + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_DOCUMENT_XML,
        "document XML exceeds 8 MiB"
    );
    let xml = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        anyhow::ensure!(bytes.len() % 2 == 0, "invalid UTF-16 XML");
        let little = bytes[0] == 0xff;
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| {
                if little {
                    u16::from_le_bytes([pair[0], pair[1]])
                } else {
                    u16::from_be_bytes([pair[0], pair[1]])
                }
            })
            .collect();
        String::from_utf16(&units)?
    } else {
        String::from_utf8(bytes)?
    };
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}'))?;
    let mut blocks = Vec::new();
    for paragraph in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "p")
    {
        let mut text = String::new();
        for node in paragraph.descendants().filter(roxmltree::Node::is_element) {
            match node.tag_name().name() {
                "t" => text.push_str(node.text().unwrap_or_default()),
                "tab" => text.push('\t'),
                "br" | "cr" => text.push('\n'),
                _ => {}
            }
        }
        if !text.trim().is_empty() {
            blocks.push(text);
            if blocks.len() > MAX_BLOCKS {
                break;
            }
        }
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn docx(xml: &[u8]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(xml).unwrap();
        zip.finish().unwrap().into_inner()
    }

    #[tokio::test]
    async fn docx_loads_without_extension_registry() {
        let file = tempfile::Builder::new().suffix(".DOCX").tempfile().unwrap();
        let xml = br#"<w:document xmlns:w="urn:word"><w:body><w:p><w:r><w:t>Hello </w:t></w:r><w:r><w:t>world &amp; &#xE9;</w:t><w:tab/><w:t>next</w:t></w:r></w:p></w:body></w:document>"#;
        std::fs::write(file.path(), docx(xml)).unwrap();
        let (tx, rx) = flume::unbounded();
        super::super::load_preview(
            atlas_model::layout_tree::TileId(1),
            file.path().to_path_buf(),
            tx,
        )
        .await;
        let atlas_model::app_state::BgResult::PreviewReady { result, .. } = rx.recv().unwrap()
        else {
            panic!("missing preview")
        };
        assert!(
            matches!(result, PreviewResult::Document { blocks, .. } if blocks == ["Hello world & é\tnext"])
        );
    }

    #[test]
    fn rejects_corrupt_and_oversized_documents() {
        assert!(matches!(
            preview_docx_reader(Cursor::new(b"invalid"), "bad"),
            PreviewResult::Error(_)
        ));
        let xml = vec![b' '; MAX_DOCUMENT_XML as usize + 1];
        assert!(matches!(
            preview_docx_reader(Cursor::new(docx(&xml)), "big"),
            PreviewResult::Error(_)
        ));
    }

    #[test]
    fn handles_utf16_and_limits_paragraphs() {
        let xml = format!("<document>{}</document>", "<p><t>Test</t></p>".repeat(201));
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
        assert!(
            matches!(preview_docx_reader(Cursor::new(docx(&bytes)), "test"), PreviewResult::Document { blocks, truncated: true, .. } if blocks.len() == 200)
        );
    }

    #[tokio::test]
    async fn docx_inside_zip_uses_document_preview() {
        let inner = docx(br#"<document><p><t>Inside ZIP</t></p></document>"#);
        let file = tempfile::Builder::new().suffix(".zip").tempfile().unwrap();
        let mut writer = zip::ZipWriter::new(std::fs::File::create(file.path()).unwrap());
        writer
            .start_file("docs/sample.docx", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&inner).unwrap();
        writer.finish().unwrap();
        let (tx, rx) = flume::unbounded();
        super::super::load_archive_preview(
            atlas_model::layout_tree::TileId(2),
            file.path().to_path_buf(),
            "docs/sample.docx".into(),
            tx,
        )
        .await;
        let atlas_model::app_state::BgResult::PreviewReady { result, .. } = rx.recv().unwrap()
        else {
            panic!("missing preview")
        };
        assert!(
            matches!(result, PreviewResult::Document { blocks, .. } if blocks == ["Inside ZIP"])
        );
    }
}
