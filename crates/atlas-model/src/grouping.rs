//! Cached virtual list structure. Headers never masquerade as file indices.
use crate::file_entry::{FileEntry, FileKind};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GroupBy {
    #[default]
    None,
    Date,
    Size,
    Type,
    Name,
}

impl GroupBy {
    pub fn label(self) -> String {
        match self {
            Self::None => rust_i18n::t!("group_none"),
            Self::Date => rust_i18n::t!("group_date"),
            Self::Size => rust_i18n::t!("group_size"),
            Self::Type => rust_i18n::t!("group_type"),
            Self::Name => rust_i18n::t!("group_name"),
        }
        .into_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupRow {
    Header {
        id: String,
        label: String,
        count: usize,
        collapsed: bool,
    },
    Entry(usize),
}

/// Returns both virtual rows and keyboard/shift-selection order. Entry order
/// inside a bucket is inherited from the current sort/filter result.
pub fn build_groups(
    entries: &[FileEntry],
    visible: &[usize],
    mode: GroupBy,
    collapsed: &BTreeSet<String>,
    today: NaiveDate,
) -> (Vec<GroupRow>, Vec<usize>) {
    if mode == GroupBy::None {
        return (
            visible.iter().copied().map(GroupRow::Entry).collect(),
            visible.to_vec(),
        );
    }
    let mut buckets: BTreeMap<(u8, String), (String, Vec<usize>)> = BTreeMap::new();
    for &index in visible {
        let Some(entry) = entries.get(index) else {
            continue;
        };
        let (rank, id, label) = bucket(entry, mode, today);
        buckets
            .entry((rank, id))
            .or_insert_with(|| (label, Vec::new()))
            .1
            .push(index);
    }
    let mut rows = Vec::with_capacity(visible.len() + buckets.len());
    let mut order = Vec::with_capacity(visible.len());
    for ((_, id), (label, indices)) in buckets {
        let is_collapsed = collapsed.contains(&id);
        rows.push(GroupRow::Header {
            id,
            label,
            count: indices.len(),
            collapsed: is_collapsed,
        });
        if !is_collapsed {
            rows.extend(indices.iter().copied().map(GroupRow::Entry));
            order.extend(indices);
        }
    }
    (rows, order)
}

fn bucket(entry: &FileEntry, mode: GroupBy, today: NaiveDate) -> (u8, String, String) {
    let fixed = |rank, key: &str, label: String| (rank, key.to_owned(), label);
    match mode {
        GroupBy::Date => {
            let Some(modified) = entry.modified else {
                return fixed(
                    9,
                    "date:unknown",
                    rust_i18n::t!("group_unknown").into_owned(),
                );
            };
            let date = DateTime::<Local>::from(modified).date_naive();
            let week = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
            let month = today.with_day(1).unwrap_or(today);
            let previous_month = (month - Duration::days(1)).with_day(1).unwrap_or(month);
            let year = month.with_month(1).unwrap_or(month);
            let previous_year = year.with_year(year.year() - 1).unwrap_or(year);
            // Calendar boundaries, with the more immediate bucket taking
            // precedence when a week crosses a month or year boundary. Future
            // timestamps share Today rather than creating an eleventh bucket.
            let (rank, key, label) = if date >= today {
                (0, "date:today", rust_i18n::t!("group_today"))
            } else if date == today - Duration::days(1) {
                (1, "date:yesterday", rust_i18n::t!("group_yesterday"))
            } else if date >= week {
                (2, "date:week", rust_i18n::t!("group_week"))
            } else if date >= week - Duration::days(7) {
                (3, "date:last-week", rust_i18n::t!("group_last_week"))
            } else if date >= month {
                (4, "date:month", rust_i18n::t!("group_month"))
            } else if date >= previous_month {
                (5, "date:last-month", rust_i18n::t!("group_last_month"))
            } else if date >= year {
                (6, "date:year", rust_i18n::t!("group_year"))
            } else if date >= previous_year {
                (7, "date:last-year", rust_i18n::t!("group_last_year"))
            } else {
                (8, "date:older", rust_i18n::t!("group_older"))
            };
            fixed(rank, key, label.into_owned())
        }
        GroupBy::Size => {
            if entry.kind() == FileKind::Dir {
                return fixed(
                    0,
                    "size:folders",
                    rust_i18n::t!("file_kind_folder").into_owned(),
                );
            }
            let (rank, key, label) = match entry.size() {
                Some(0..=16_383) => (1, "size:tiny", rust_i18n::t!("group_tiny")),
                Some(16_384..=1_048_575) => (2, "size:small", rust_i18n::t!("group_small")),
                Some(1_048_576..=134_217_727) => (3, "size:medium", rust_i18n::t!("group_medium")),
                Some(134_217_728..=1_073_741_823) => {
                    (4, "size:large", rust_i18n::t!("group_large"))
                }
                Some(1_073_741_824..=4_294_967_295) => (5, "size:huge", rust_i18n::t!("group_huge")),
                Some(_) => (6, "size:gigantic", rust_i18n::t!("group_gigantic")),
                None => (7, "size:unknown", rust_i18n::t!("group_unknown")),
            };
            fixed(rank, key, label.into_owned())
        }
        GroupBy::Type => {
            let label = crate::file_entry::file_category(&entry.kind(), entry.extension().as_deref());
            let rank = u8::from(entry.kind() != FileKind::Dir);
            (rank, format!("type:{label}"), label)
        }
        GroupBy::Name => {
            let (rank, key, label) = match entry.name.chars().next().map(|ch| ch.to_ascii_uppercase()) {
                Some('0'..='9') => (0, "name:0-9", "0–9".to_string()),
                Some('A'..='H') => (1, "name:A-H", "A–H".to_string()),
                Some('I'..='P') => (2, "name:I-P", "I–P".to_string()),
                Some('Q'..='Z') => (3, "name:Q-Z", "Q–Z".to_string()),
                _ => (4, "name:other", rust_i18n::t!("group_other").into_owned()),
            };
            fixed(rank, key, label)
        }
        GroupBy::None => unreachable!("ungrouped list handled before bucketing"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn entry(name: &str, size: Option<u64>) -> FileEntry {
        FileEntry::from(crate::file_entry::FileEntryData {
            name: name.into(),
            path: name.into(),
            kind: FileKind::File,
            size,
            modified: None,
            _is_hidden: false,
            _is_symlink: false,
            archive_root: None,
            recycle_info: None,
        })
    }

    #[test]
    fn date_buckets_use_local_calendar_days_and_keep_unknown_dates() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let mut entries = Vec::new();
        for days in [0, 1, 2, 7, 30, -1] {
            let mut entry = entry("file", None);
            entry.modified = Some(
                Local
                    .from_local_datetime(
                        &(today - Duration::days(days))
                            .and_hms_opt(12, 0, 0)
                            .unwrap(),
                    )
                    .single()
                    .unwrap()
                    .into(),
            );
            entries.push(entry);
        }
        entries.push(entry("unknown", None));
        let (rows, order) = build_groups(
            &entries,
            &[0, 1, 2, 3, 4, 5, 6],
            GroupBy::Date,
            &BTreeSet::new(),
            today,
        );
        assert_eq!(order, [0, 5, 1, 2, 3, 4, 6]);
        assert_eq!(
            rows.iter()
                .filter(|row| matches!(row, GroupRow::Header { .. }))
                .count(),
            6
        );
    }

    #[test]
    fn size_boundaries_and_sort_within_bucket_are_preserved() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let entries = [
            entry("huge", Some(1_073_741_824)),
            entry("tiny-a", Some(1)),
            entry("tiny-b", Some(16_383)),
            entry("small", Some(16_384)),
        ];
        let (rows, order) = build_groups(
            &entries,
            &[0, 2, 1, 3],
            GroupBy::Size,
            &BTreeSet::new(),
            today,
        );
        assert_eq!(order, [2, 1, 3, 0]);
        assert!(matches!(&rows[0], GroupRow::Header { id, count: 2, .. } if id == "size:tiny"));
        let (rows, order) = build_groups(
            &entries,
            &[0, 2, 1, 3],
            GroupBy::Type,
            &BTreeSet::new(),
            today,
        );
        assert_eq!(order, [0, 2, 1, 3]);
        assert_eq!(rows.len(), 5);
    }

    fn dated(date: NaiveDate) -> FileEntry {
        let mut file = entry("file", None);
        file.modified = Some(Local.from_local_datetime(&date.and_hms_opt(12, 0, 0).unwrap()).single().unwrap().into());
        file
    }

    #[test]
    fn ten_date_buckets_follow_calendar_boundaries_including_year_crossings() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        let dates = [(2026,10,29), (2026,10,28), (2026,10,26), (2026,10,19),
            (2026,10,1), (2026,9,1), (2026,1,1), (2025,1,1), (2024,12,31)];
        let mut files = dates.map(|(y,m,d)| dated(NaiveDate::from_ymd_opt(y,m,d).unwrap())).to_vec();
        files.push(entry("unknown", None));
        let (rows, order) = build_groups(&files, &(0..10).collect::<Vec<_>>(), GroupBy::Date, &BTreeSet::new(), today);
        let headers: Vec<_> = rows.iter().filter_map(|row| match row { GroupRow::Header { id, .. } => Some(id.as_str()), _ => None }).collect();
        assert_eq!(headers, ["date:today", "date:yesterday", "date:week", "date:last-week", "date:month",
            "date:last-month", "date:year", "date:last-year", "date:older", "date:unknown"]);
        assert_eq!(order, (0..10).collect::<Vec<_>>());
        let january = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        assert_eq!(bucket(&dated(NaiveDate::from_ymd_opt(2025,12,31).unwrap()), GroupBy::Date, january).1, "date:week");
        assert_eq!(bucket(&dated(NaiveDate::from_ymd_opt(2025,12,22).unwrap()), GroupBy::Date, january).1, "date:last-week");
        let march = NaiveDate::from_ymd_opt(2024,3,15).unwrap();
        assert_eq!(bucket(&dated(NaiveDate::from_ymd_opt(2024,2,29).unwrap()), GroupBy::Date, march).1, "date:last-month");
        assert_eq!(bucket(&dated(today + Duration::days(1)), GroupBy::Date, today).1, "date:today");
    }

    #[test]
    fn name_groups_have_five_balanced_ranges_with_case_insensitive_edges() {
        let today = NaiveDate::from_ymd_opt(2026,10,15).unwrap();
        for (name, expected) in [("0file", "name:0-9"), ("9file", "name:0-9"), ("alpha", "name:A-H"),
            ("Hotel", "name:A-H"), ("india", "name:I-P"), ("Papa", "name:I-P"), ("queen", "name:Q-Z"),
            ("Zulu", "name:Q-Z"), (".hidden", "name:other"), ("日本語", "name:other")] {
            assert_eq!(bucket(&entry(name, None), GroupBy::Name, today).1, expected, "{name}");
        }
    }

    #[test]
    fn size_ranges_cover_every_boundary_without_gaps() {
        let today = NaiveDate::from_ymd_opt(2026,10,15).unwrap();
        for (size, expected) in [(0, "tiny"), (16_383, "tiny"), (16_384, "small"), (1_048_575, "small"),
            (1_048_576, "medium"), (134_217_727, "medium"), (134_217_728, "large"), (1_073_741_823, "large"),
            (1_073_741_824, "huge"), (4_294_967_295, "huge"), (4_294_967_296, "gigantic"), (u64::MAX, "gigantic")] {
            assert_eq!(bucket(&entry("file", Some(size)), GroupBy::Size, today).1, format!("size:{expected}"));
        }
        assert_eq!(bucket(&entry("unknown", None), GroupBy::Size, today).1, "size:unknown");
    }
}
