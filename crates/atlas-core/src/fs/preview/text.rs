use std::path::Path;

use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

use atlas_model::preview_cache::PreviewResult;

pub(super) static SYNTAX_SET: std::sync::LazyLock<SyntaxSet> =
    std::sync::LazyLock::new(SyntaxSet::load_defaults_newlines);
pub(super) static THEME_SET: std::sync::LazyLock<ThemeSet> =
    std::sync::LazyLock::new(ThemeSet::load_defaults);

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
        return PreviewResult::Binary {
            size: meta.len(),
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
        PreviewResult::Binary { size: meta.len() }
    }
}

/// Syntax-highlighted spans resolved by file extension.
pub(crate) fn highlight_text(text: &str, ext: &str) -> Vec<([u8; 4], String)> {
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

/// Syntax-highlighted spans resolved by language token (markdown fence info string).
pub(crate) fn highlight_code(text: &str, lang: &str) -> Vec<([u8; 4], String)> {
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

