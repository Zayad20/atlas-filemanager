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
    let bytes = match tokio::fs::read(path).await {
        Ok(b) => b,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };
    if bytes.len() > super::MAX_TEXT_BYTES {
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
