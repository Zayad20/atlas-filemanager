#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use std::path::Path;
use std::time::SystemTime;

/// Consistent color for folder/directory icons across all views, as neutral RGB.
pub const FOLDER_ICON_COLOR: [u8; 3] = [200, 180, 80];

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

const fn is_leap(y: u32) -> bool {
    y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400))
}

/// Check if a path is hidden (name starts with '.').
pub fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

/// Extract a human-readable display name from a path.
pub fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().to_string(),
        |n| n.to_string_lossy().to_string(),
    )
}
