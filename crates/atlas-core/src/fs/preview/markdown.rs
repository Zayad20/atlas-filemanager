use std::path::Path;

use atlas_model::preview_cache::{MarkdownBlock, MdSpan, PreviewResult};

use super::text::highlight_code;

pub(super) fn is_markdown_ext(ext: &str) -> bool {
    matches!(ext, "md" | "markdown" | "mdown" | "mkd")
}

pub(super) async fn preview_markdown(path: &Path) -> PreviewResult {
    let bytes = match tokio::fs::read(path).await {
        Ok(b) => b,
        Err(e) => return PreviewResult::Error(e.to_string()),
    };
    if bytes.len() > super::MAX_TEXT_BYTES {
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

pub(crate) fn parse_markdown(source: &str) -> Vec<MarkdownBlock> {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

    #[allow(dead_code)]
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
