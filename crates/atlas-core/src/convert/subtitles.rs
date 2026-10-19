//! Independent timed-text conversion. The canonical data is a list of UTF-8
//! cues with millisecond intervals; layout/styles remain in the original file.
use super::{native, Command};
use anyhow::{bail, ensure, Context, Result};
use std::fmt::Write as _;
use std::path::Path;

const MAX_CUES: usize = 100_000;
const MAX_TIME: u64 = 36_000_000_000; // 10,000 hours, bounded arithmetic in writers.

#[derive(Debug, PartialEq, Eq)]
struct Cue {
    start: u64,
    end: u64,
    text: String,
}

fn number(text: &str) -> Result<u64> {
    ensure!(
        !text.is_empty() && text.bytes().all(|c| c.is_ascii_digit()),
        "Invalid subtitle number: {text}"
    );
    text.parse().context("Subtitle number is too large")
}

fn clock(text: &str) -> Result<u64> {
    let text = text.trim();
    let (whole, fraction) = text.split_once(['.', ',']).unwrap_or((text, "0"));
    ensure!(
        (1..=3).contains(&fraction.len()),
        "Subtitle fractional seconds need 1–3 digits"
    );
    let fraction = number(fraction)? * 10_u64.pow(3 - fraction.len() as u32);
    let parts = whole.split(':').map(number).collect::<Result<Vec<_>>>()?;
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0, *minutes, *seconds),
        [hours, minutes, seconds] if *minutes < 60 => (*hours, *minutes, *seconds),
        _ => bail!("Expected subtitle time hh:mm:ss.mmm or mm:ss.mmm"),
    };
    ensure!(
        hours < 10_000 && minutes < 600_000 && seconds < 60,
        "Subtitle time is out of range"
    );
    let result = ((hours * 60 + minutes) * 60 + seconds) * 1000 + fraction;
    ensure!(result < MAX_TIME, "Subtitle time exceeds 10,000 hours");
    Ok(result)
}

fn xml_escape(text: &str) -> String {
    text.chars()
        .filter(|c| *c >= ' ' || matches!(c, '\n' | '\t'))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Decode entities after removing markup so `&lt;literal&gt;` remains text.
fn entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        rest = &rest[index..];
        if let Some(end) = rest.find(';').filter(|end| *end <= 16) {
            let entity = &rest[1..end];
            let value = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                "lrm" => Some('\u{200e}'),
                "rlm" => Some('\u{200f}'),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|v| u32::from_str_radix(v, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|v| v.parse().ok()))
                    .and_then(char::from_u32),
            };
            if let Some(value) = value {
                out.push(value);
                rest = &rest[end + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

fn markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('<') {
        out.push_str(&rest[..index]);
        rest = &rest[index..];
        if let Some(end) = rest.find('>') {
            let tag = rest[1..end].trim().to_ascii_lowercase();
            if tag.starts_with(|c: char| c.is_ascii_alphabetic() || matches!(c, '/' | '!'))
                || (tag.contains(':') && tag.starts_with(|c: char| c.is_ascii_digit()))
            {
                if tag == "br" || tag.starts_with("br ") || tag == "br/" || tag == "/p" {
                    out.push('\n');
                }
                rest = &rest[end + 1..];
                continue;
            }
        }
        out.push('<');
        rest = &rest[1..];
    }
    out.push_str(rest);
    entities(&out).trim().to_owned()
}

fn ass_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    let mut override_tag = false;
    while let Some(c) = chars.next() {
        match c {
            '{' => override_tag = true,
            '}' if override_tag => override_tag = false,
            _ if override_tag => {}
            '\\' => match chars.next() {
                Some('N' | 'n') => out.push('\n'),
                Some('h') => out.push(' '),
                Some(c @ ('{' | '}' | '\\')) => out.push(c),
                Some(c) => {
                    out.push('\\');
                    out.push(c);
                }
                None => out.push('\\'),
            },
            c => out.push(c),
        }
    }
    out.trim().to_owned()
}

fn intervals(text: &str, webvtt: bool) -> Result<Vec<Cue>> {
    let mut cues = Vec::new();
    let mut blocks = text.split("\n\n").filter(|b| !b.trim().is_empty());
    if webvtt {
        let header = blocks.next().context("Missing WEBVTT header")?;
        ensure!(
            header.lines().next().is_some_and(|l| l == "WEBVTT"
                || l.starts_with("WEBVTT ")
                || l.starts_with("WEBVTT\t")),
            "Missing WEBVTT header"
        );
        ensure!(
            !header.contains("-->"),
            "WEBVTT header must be separated from cues by a blank line"
        );
    }
    for block in blocks {
        let block = block.trim();
        if webvtt
            && (block == "NOTE"
                || block.starts_with("NOTE ")
                || block.starts_with("NOTE\n")
                || block.starts_with("STYLE\n")
                || block.starts_with("REGION\n"))
        {
            continue;
        }
        let mut lines = block.lines();
        let first = lines.next().context("Empty subtitle block")?;
        let timing = if first.contains("-->") {
            first
        } else {
            lines.next().context("Missing subtitle cue timing")?
        };
        let (start, end) = timing
            .split_once("-->")
            .context("Invalid subtitle cue timing")?;
        let end = end
            .split_whitespace()
            .next()
            .context("Missing subtitle end time")?;
        cues.push(Cue {
            start: clock(start)?,
            end: clock(end)?,
            text: markup(&lines.collect::<Vec<_>>().join("\n")),
        });
        ensure!(
            cues.len() <= MAX_CUES,
            "Subtitle input exceeds 100,000 cues"
        );
    }
    Ok(cues)
}

fn ass(text: &str) -> Result<Vec<Cue>> {
    let mut events = false;
    let mut fields: Vec<String> = Vec::new();
    let mut cues = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            events = line.eq_ignore_ascii_case("[Events]");
            continue;
        }
        if !events {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.eq_ignore_ascii_case("Format") {
            fields = value
                .split(',')
                .map(|f| f.trim().to_ascii_lowercase())
                .collect();
        } else if key.eq_ignore_ascii_case("Dialogue") {
            ensure!(
                !fields.is_empty() && fields.last().is_some_and(|f| f == "text"),
                "ASS/SSA needs an Events Format with Text as the last field"
            );
            let values: Vec<_> = value.trim_start().splitn(fields.len(), ',').collect();
            ensure!(values.len() == fields.len(), "Incomplete ASS/SSA dialogue");
            let field = |name| -> Result<&str> {
                Ok(values[fields
                    .iter()
                    .position(|f| f == name)
                    .context("ASS/SSA is missing a required field")?])
            };
            cues.push(Cue {
                start: clock(field("start")?)?,
                end: clock(field("end")?)?,
                text: ass_text(field("text")?),
            });
            ensure!(
                cues.len() <= MAX_CUES,
                "Subtitle input exceeds 100,000 cues"
            );
        }
    }
    Ok(cues)
}

fn events_to_cues(mut events: Vec<(u64, String)>) -> Result<Vec<Cue>> {
    events.sort_by_key(|event| event.0);
    let mut grouped: Vec<(u64, String)> = Vec::new();
    for (start, text) in events {
        ensure!(start < MAX_TIME, "Subtitle time exceeds 10,000 hours");
        if let Some(last) = grouped.last_mut().filter(|event| event.0 == start) {
            if !text.is_empty() {
                if !last.1.is_empty() {
                    last.1.push('\n');
                }
                last.1.push_str(&text);
            }
        } else {
            grouped.push((start, text));
        }
    }
    let mut cues = Vec::new();
    for (index, (start, text)) in grouped.iter().enumerate() {
        if !text.is_empty() {
            let end = grouped
                .get(index + 1)
                .map_or((*start + 5000).min(MAX_TIME - 1), |e| e.0);
            cues.push(Cue {
                start: *start,
                end,
                text: text.clone(),
            });
        }
    }
    Ok(cues)
}

fn lrc(text: &str) -> Result<Vec<Cue>> {
    let mut offset = 0_i64;
    for line in text.lines() {
        if let Some(value) = line
            .trim()
            .strip_prefix("[offset:")
            .and_then(|s| s.strip_suffix(']'))
        {
            offset = value.parse().context("Invalid LRC offset")?;
            ensure!(offset.unsigned_abs() < MAX_TIME, "LRC offset is too large");
        }
    }
    let mut events = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(after) = rest.strip_prefix('[') {
            let (tag, next) = after.split_once(']').context("Unclosed LRC timestamp")?;
            if tag.starts_with(|c: char| c.is_ascii_digit()) {
                times.push(clock(tag)?);
            }
            rest = next;
        }
        // LRC is plain text. Only enhanced LRC's word timestamps are markup;
        // literal angle brackets and ampersands must survive the conversion.
        let mut text = String::new();
        while let Some(index) = rest.find('<') {
            text.push_str(&rest[..index]);
            rest = &rest[index..];
            if let Some(end) = rest.find('>').filter(|end| clock(&rest[1..*end]).is_ok()) {
                rest = &rest[end + 1..];
            } else {
                text.push('<');
                rest = &rest[1..];
            }
        }
        text.push_str(rest);
        for time in times {
            let time = (time as i64 + offset).max(0) as u64;
            events.push((time, text.clone()));
            ensure!(
                events.len() <= MAX_CUES,
                "Subtitle input exceeds 100,000 cues"
            );
        }
    }
    events_to_cues(events)
}

fn sami(text: &str) -> Result<Vec<Cue>> {
    let sync = regex::Regex::new(r#"(?is)<sync\b[^>]*\bstart\s*=\s*["']?([0-9]+)[^>]*>"#)?;
    let class = regex::Regex::new(r#"(?is)<p\b[^>]*\bclass\s*=\s*["']?([^\s"'>]+)"#)?;
    let classes: std::collections::BTreeSet<_> = class
        .captures_iter(text)
        .map(|c| c[1].to_ascii_lowercase())
        .collect();
    ensure!(
        classes.len() <= 1,
        "Multi-language SAMI needs a track selection; split language tracks before converting"
    );
    let matches: Vec<_> = sync.captures_iter(text).take(MAX_CUES + 1).collect();
    ensure!(
        matches.len() <= MAX_CUES,
        "Subtitle input exceeds 100,000 cues"
    );
    let mut events = Vec::new();
    for (index, capture) in matches.iter().enumerate() {
        let matched = capture.get(0).context("Missing SAMI sync")?;
        let end = matches
            .get(index + 1)
            .and_then(|m| m.get(0))
            .map_or(text.len(), |m| m.start());
        events.push((number(&capture[1])?, markup(&text[matched.end()..end])));
    }
    events_to_cues(events)
}

fn ttml_time(text: &str) -> Result<u64> {
    if text.contains(':') {
        return clock(text);
    }
    for (unit, scale) in [("ms", 1_u64), ("h", 3_600_000), ("m", 60_000), ("s", 1000)] {
        if let Some(value) = text.strip_suffix(unit) {
            let (whole, fraction) = value.split_once('.').unwrap_or((value, "0"));
            ensure!(
                !fraction.is_empty() && fraction.len() <= 3,
                "TTML supports millisecond precision"
            );
            let value = number(whole)?
                .checked_mul(scale)
                .context("TTML time overflow")?
                .checked_add(number(fraction)? * scale / 10_u64.pow(fraction.len() as u32))
                .context("TTML time overflow")?;
            ensure!(value < MAX_TIME, "TTML time exceeds 10,000 hours");
            return Ok(value);
        }
    }
    bail!("TTML supports clock, hour, minute, second and millisecond times; frame/tick timing requires another engine")
}

fn ttml(text: &str) -> Result<Vec<Cue>> {
    let xml = roxmltree::Document::parse(text)?;
    ensure!(
        xml.root_element().tag_name().name() == "tt",
        "Expected a TTML tt document"
    );
    let mut cues = Vec::new();
    for node in xml
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "p")
    {
        ensure!(
            !node.ancestors().skip(1).any(|n| ["begin", "end", "dur"]
                .iter()
                .any(|a| n.attribute(*a).is_some())
                || n.attribute("timeContainer") == Some("seq")),
            "Nested or sequential TTML timing requires another engine"
        );
        ensure!(
            !node.descendants().skip(1).any(|n| n.is_element()
                && ["begin", "end", "dur"]
                    .iter()
                    .any(|a| n.attribute(*a).is_some())),
            "Timed TTML spans require another engine"
        );
        let start = node
            .attribute("begin")
            .map(ttml_time)
            .transpose()?
            .unwrap_or(0);
        let end = if let Some(end) = node.attribute("end") {
            ttml_time(end)?
        } else {
            start
                .checked_add(ttml_time(
                    node.attribute("dur")
                        .context("TTML paragraph needs end or dur")?,
                )?)
                .context("TTML duration overflow")?
        };
        let mut text = String::new();
        for child in node.descendants() {
            if child.is_text() {
                text.push_str(child.text().unwrap_or_default());
            } else if child.is_element() && child.tag_name().name() == "br" {
                text.push('\n');
            }
        }
        cues.push(Cue {
            start,
            end,
            text: text.trim().to_owned(),
        });
        ensure!(
            cues.len() <= MAX_CUES,
            "Subtitle input exceeds 100,000 cues"
        );
    }
    Ok(cues)
}

fn timestamp(time: u64, separator: char, hours: bool) -> String {
    if hours {
        format!(
            "{:02}:{:02}:{:02}{separator}{:03}",
            time / 3_600_000,
            time / 60_000 % 60,
            time / 1000 % 60,
            time % 1000
        )
    } else {
        format!(
            "{:02}:{:02}{separator}{:03}",
            time / 60_000,
            time / 1000 % 60,
            time % 1000
        )
    }
}

fn write(cues: &[Cue], extension: &str) -> Result<String> {
    let mut out = match extension {
        "vtt" => "WEBVTT\n\n".to_owned(),
        "ass" => "[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Arial,48,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,20,20,20,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n".to_owned(),
        "smi" => "<SAMI>\n<HEAD><STYLE TYPE=\"text/css\">P.Atlas { Name:Atlas; lang:und; SAMIType:CC; }</STYLE></HEAD>\n<BODY>\n".to_owned(),
        "ttml" => "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<tt xmlns=\"http://www.w3.org/ns/ttml\"><body><div>\n".to_owned(),
        _ => String::new(),
    };
    if ["lrc", "smi"].contains(&extension) {
        ensure!(
            cues.windows(2).all(|pair| pair[0].end <= pair[1].start),
            "LRC/SAMI cannot represent overlapping cues; choose SRT, WebVTT, ASS or TTML"
        );
    }
    for (index, cue) in cues.iter().enumerate() {
        match extension {
            "srt" | "vtt" => {
                let separator = if extension == "srt" { ',' } else { '.' };
                writeln!(
                    out,
                    "{}\n{} --> {}\n{}\n",
                    index + 1,
                    timestamp(cue.start, separator, true),
                    timestamp(cue.end, separator, true),
                    xml_escape(&cue.text)
                )?;
            }
            "ass" => {
                ensure!(!["\\N", "\\n", "\\h"].iter().any(|token| cue.text.contains(token)), "ASS reserves literal backslash-N/n/h sequences; choose WebVTT, SRT or TTML to preserve this text");
                let time = |t: u64| {
                    format!(
                        "{}:{:02}:{:02}.{:02}",
                        t / 360_000,
                        t / 6000 % 60,
                        t / 100 % 60,
                        t % 100
                    )
                };
                let start = (cue.start + 5) / 10;
                let end = ((cue.end + 5) / 10).max(start + 1);
                let mut text = String::new();
                for c in cue.text.chars() {
                    match c {
                        '\\' => text.push('\\'),
                        '{' => text.push_str("\\{"),
                        '}' => text.push_str("\\}"),
                        '\n' => text.push_str("\\N"),
                        c => text.push(c),
                    }
                }
                writeln!(
                    out,
                    "Dialogue: 0,{},{},Default,,0,0,0,,{text}",
                    time(start),
                    time(end)
                )?;
            }
            "lrc" => {
                writeln!(
                    out,
                    "[{}]{}\n[{}]",
                    timestamp(cue.start, '.', false),
                    cue.text.replace('\n', " "),
                    timestamp(cue.end, '.', false)
                )?;
            }
            "smi" => {
                writeln!(
                    out,
                    "<SYNC Start={}><P Class=Atlas>{}\n<SYNC Start={}><P Class=Atlas>&nbsp;",
                    cue.start,
                    xml_escape(&cue.text).replace('\n', "<BR>"),
                    cue.end
                )?;
            }
            "ttml" => {
                writeln!(
                    out,
                    "<p begin=\"{}\" end=\"{}\" xml:space=\"preserve\">{}</p>",
                    timestamp(cue.start, '.', true),
                    timestamp(cue.end, '.', true),
                    xml_escape(&cue.text).replace('\n', "<br/>")
                )?;
            }
            "txt" => {
                writeln!(out, "{}", cue.text)?;
            }
            _ => bail!("Unsupported subtitle output"),
        }
    }
    if extension == "smi" {
        out.push_str("</BODY>\n</SAMI>\n");
    }
    if extension == "ttml" {
        out.push_str("</div></body></tt>\n");
    }
    Ok(out)
}

pub(super) fn convert(command: &Command, output: &Path) -> Result<()> {
    let text = native::read_text(&command.source)?
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut cues = match super::catalog::extension(&command.source).as_str() {
        "srt" => intervals(&text, false)?,
        "vtt" => intervals(&text, true)?,
        "ass" | "ssa" => ass(&text)?,
        "lrc" => lrc(&text)?,
        "smi" => sami(&text)?,
        "ttml" => ttml(&text)?,
        _ => bail!("Unsupported subtitle input"),
    };
    ensure!(!cues.is_empty(), "No subtitle cues found");
    for cue in &cues {
        ensure!(
            cue.end > cue.start && cue.end < MAX_TIME,
            "Subtitle cue must end after its start and before 10,000 hours"
        );
    }
    cues.retain(|cue| !cue.text.is_empty());
    ensure!(!cues.is_empty(), "No subtitle text found");
    cues.sort_by_key(|cue| cue.start);
    std::fs::write(output, write(&cues, command.target.extension)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_reject_invalid_components_and_overflows() {
        assert_eq!(clock("01:02:03,045").unwrap(), 3_723_045);
        assert_eq!(clock("125:01.25").unwrap(), 7_501_250);
        for time in [
            "-1:00:00",
            "1:60:00",
            "00:99.000",
            "00:01.1234",
            "999999999999999:01.00",
        ] {
            assert!(clock(time).is_err(), "{time}");
        }
    }

    #[test]
    fn vtt_notes_ids_settings_and_entities_preserve_semantic_cues() {
        let cues = intervals("WEBVTT\nKind: captions\n\nNOTE comment\nignored\n\nintro\n00:01.250 --> 00:03.500 align:start\n<v Ana><b>Hola &amp; adiós</b>\n&lt;literal&gt;\n\n", true).unwrap();
        assert_eq!(
            cues,
            [Cue {
                start: 1250,
                end: 3500,
                text: "Hola & adiós\n<literal>".into()
            }]
        );
    }

    #[test]
    fn ass_field_order_and_lrc_offsets_are_read_explicitly() {
        let cues = ass("[Events]\nFormat: End, Layer, Start, Text\nDialogue: 0:00:03.50,0,0:00:01.25,{\\b1}Hola, Ana{\\b0}\\Nmás\n").unwrap();
        assert_eq!(cues[0].text, "Hola, Ana\nmás");
        assert_eq!(cues[0].start, 1250);
        let cues =
            lrc("[offset:-500]\n[ar:Atlas]\n[00:01.000][00:03.000]Hola\n[00:05.000]\n").unwrap();
        assert_eq!(
            (cues[0].start, cues[0].end, cues[1].start, cues[1].end),
            (500, 2500, 2500, 4500)
        );
    }

    #[test]
    fn linear_formats_preserve_clear_intervals_and_refuse_overlaps() {
        let cues = [
            Cue {
                start: 1000,
                end: 2000,
                text: "A & B".into(),
            },
            Cue {
                start: 4000,
                end: 5000,
                text: "C".into(),
            },
        ];
        assert_eq!(lrc(&write(&cues, "lrc").unwrap()).unwrap(), cues);
        assert_eq!(sami(&write(&cues, "smi").unwrap()).unwrap(), cues);
        let overlaps = [
            Cue {
                start: 0,
                end: 2000,
                text: "A".into(),
            },
            Cue {
                start: 1000,
                end: 3000,
                text: "B".into(),
            },
        ];
        assert!(write(&overlaps, "lrc").is_err());
        assert!(write(&overlaps, "smi").is_err());
    }

    #[test]
    fn ttml_handles_durations_breaks_and_refuses_nested_timing() {
        let cues = ttml("<tt xmlns='http://www.w3.org/ns/ttml'><body><div><p begin='1.25s' dur='2s'>Hola<br/>Ana &amp; José</p></div></body></tt>").unwrap();
        assert_eq!(
            cues[0],
            Cue {
                start: 1250,
                end: 3250,
                text: "Hola\nAna & José".into()
            }
        );
        assert_eq!(ttml(&write(&cues, "ttml").unwrap()).unwrap(), cues);
        assert!(ttml(
            "<tt><body><div begin='2s'><p begin='1s' end='3s'>wrong</p></div></body></tt>"
        )
        .is_err());
        assert!(ttml_time("00:00:01:25").is_err());
    }
}
