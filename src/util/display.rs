use egui::Color32;
use std::time::SystemTime;

pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

pub fn format_date(t: SystemTime) -> String {
    use std::time::UNIX_EPOCH;
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let (y, mo, d, h, mi) = unix_to_ymdhm(secs);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}")
}

pub fn extension_icon(ext: Option<&str>) -> &'static str {
    match ext {
        Some("rs") => "🦀",
        Some("py") => "🐍",
        Some("js" | "ts" | "jsx" | "tsx") => "📜",
        Some("json" | "yaml" | "yml" | "toml") => "⚙",
        Some("md" | "txt" | "rst") => "📄",
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico") => "🖼",
        Some("mp4" | "mkv" | "avi" | "mov" | "webm") => "🎬",
        Some("mp3" | "flac" | "wav" | "ogg" | "m4a") => "🎵",
        Some("pdf") => "📕",
        Some("zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar") => "📦",
        Some("sh" | "bash" | "zsh" | "fish") => "💻",
        Some("c" | "cpp" | "h" | "hpp") => "⚡",
        Some("html" | "css") => "🌐",
        None => "📁",
        _ => "📄",
    }
}

pub fn extension_color(ext: Option<&str>) -> Color32 {
    match ext {
        Some("rs") => Color32::from_rgb(222, 115, 45),
        Some("py") => Color32::from_rgb(55, 136, 186),
        Some("js" | "ts") => Color32::from_rgb(241, 196, 15),
        Some("json" | "toml" | "yaml" | "yml") => Color32::from_rgb(155, 89, 182),
        Some("md" | "txt") => Color32::from_rgb(149, 165, 166),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp") => Color32::from_rgb(39, 174, 96),
        Some("mp4" | "mkv" | "avi") => Color32::from_rgb(231, 76, 60),
        Some("mp3" | "flac" | "wav") => Color32::from_rgb(52, 152, 219),
        Some("pdf") => Color32::from_rgb(192, 57, 43),
        Some("zip" | "tar" | "gz") => Color32::from_rgb(230, 126, 34),
        _ => Color32::from_rgb(189, 195, 199),
    }
}

fn unix_to_ymdhm(secs: u64) -> (u32, u32, u32, u32, u32) {
    // Simple calendar calculation (no external dep)
    let mut days = (secs / 86400) as u32;
    let time_of_day = (secs % 86400) as u32;
    let h = time_of_day / 3600;
    let mi = (time_of_day % 3600) / 60;

    let mut year = 1970u32;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    let leap = is_leap(year);
    let month_days: [u32; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u32;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }

    (year, month, days + 1, h, mi)
}

fn is_leap(y: u32) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}
