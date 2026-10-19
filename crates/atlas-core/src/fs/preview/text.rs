use std::path::Path;

use atlas_model::preview_cache::PreviewResult;

pub(super) async fn preview_text(path: &Path, ext: &str) -> PreviewResult {
    let meta = match tokio::fs::metadata(path).await {
        Ok(m) => m,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        let ft = meta.file_type();
        if ft.is_fifo() || ft.is_socket() || ft.is_char_device() || ft.is_block_device() {
            return PreviewResult::Error(
                "Archivos especiales (FIFO, socket, dispositivo) no están soportados para previsualización"
                    .to_string(),
            );
        }
    }

    if !meta.is_file() {
        return PreviewResult::Error("Not a regular file".to_string());
    }

    let file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };

    let mut bytes = Vec::new();
    use tokio::io::AsyncReadExt;
    if let Err(e) = file
        .take(super::MAX_TEXT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .await
    {
        return PreviewResult::Error(e.to_string());
    }

    if bytes.len() > super::MAX_TEXT_BYTES {
        return PreviewResult::Binary { size: meta.len() };
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
        PreviewResult::Binary { size: meta.len() }
    }
}

/// Syntax-highlighted spans resolved by file extension using the zero-dependency syntax lexer.
pub(crate) fn highlight_text(text: &str, ext: &str) -> Vec<([u8; 4], String)> {
    let mut result = Vec::new();
    let lines = super::syntax::tokenize_document(text, ext);
    for line in lines.into_iter().take(2000) {
        for (kind, tok) in line {
            if tok.is_empty() {
                continue;
            }
            let color = match kind {
                super::syntax::TokenKind::Keyword => [0x56, 0x9c, 0xd6, 0xff], // Blue/Cyan
                super::syntax::TokenKind::Type => [0x4e, 0xc9, 0xb0, 0xff],    // Teal
                super::syntax::TokenKind::String => [0xce, 0x91, 0x78, 0xff],  // Green/Tan
                super::syntax::TokenKind::Number => [0xb5, 0xce, 0xa8, 0xff],  // Light Green/Gold
                super::syntax::TokenKind::Comment => [0x6a, 0x99, 0x55, 0xff], // Green/Muted
                super::syntax::TokenKind::Preprocessor => [0xc5, 0x86, 0xc0, 0xff], // Magenta
                super::syntax::TokenKind::Symbol => [0xd4, 0xd4, 0xd4, 0xff],  // Subtle
                super::syntax::TokenKind::DiffAdd => [0x34, 0xc7, 0x59, 0xff],
                super::syntax::TokenKind::DiffRemove => [0xff, 0x3b, 0x30, 0xff],
                super::syntax::TokenKind::DiffHeader => [0x00, 0x7a, 0xff, 0xff],
                super::syntax::TokenKind::LogError => [0xff, 0x3b, 0x30, 0xff],
                super::syntax::TokenKind::LogWarn => [0xff, 0x9f, 0x0a, 0xff],
                super::syntax::TokenKind::LogInfo => [0x62, 0x62, 0x68, 0xff],
                _ => [0xd4, 0xd4, 0xd4, 0xff],
            };
            result.push((color, tok.to_string()));
        }
        result.push(([0, 0, 0, 0], "\n".to_string()));
    }
    result
}

/// Syntax-highlighted spans resolved by language token.
pub(crate) fn highlight_code(text: &str, lang: &str) -> Vec<([u8; 4], String)> {
    highlight_text(text, lang)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_preview_text_regular_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.txt");
        tokio::fs::write(&path, b"hello world").await.unwrap();

        let res = preview_text(&path, "txt").await;
        match res {
            PreviewResult::Text { text, .. } => assert_eq!(text, "hello world"),
            other => panic!("Expected text preview, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_preview_fifo_does_not_hang() {
        let tmp = tempfile::tempdir().unwrap();
        let fifo_path = tmp.path().join("test_fifo");

        let status = std::process::Command::new("mkfifo")
            .arg(&fifo_path)
            .status();
        if let Ok(st) = status {
            if st.success() {
                let preview_result = tokio::time::timeout(
                    std::time::Duration::from_millis(500),
                    preview_text(&fifo_path, "txt"),
                )
                .await;

                assert!(
                    preview_result.is_ok(),
                    "preview_text se bloqueó indefinidamente al leer un FIFO"
                );
                assert!(
                    matches!(preview_result.unwrap(), PreviewResult::Error(_)),
                    "preview_text no debe aceptar FIFOs"
                );
            }
        }
    }
}
