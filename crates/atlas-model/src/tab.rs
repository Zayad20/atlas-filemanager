#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use crate::file_entry::{FileEntry, FileKind};
use crate::location::Location;
use std::collections::HashSet;
use std::sync::Arc;

/// One directory column in the Miller (columns) view.
#[derive(Clone)]
pub struct MillerColumn {
    pub location: Location,
    pub entries: std::sync::Arc<[FileEntry]>,
    pub load_state: LoadState,
    pub op_id: u64,
    /// Multi-selection state for this column (single-select for navigation,
    /// ctrl/shift for multi, rubber band populates `selected` set).
    pub selection: SelectionState,
}

impl MillerColumn {
    pub fn new(location: Location) -> Self {
        Self {
            location,
            entries: std::sync::Arc::new([]),
            load_state: LoadState::NotLoaded,
            op_id: 0,
            selection: SelectionState::default(),
        }
    }

    /// The primary selected index (first selected item), used for column-open logic.
    pub fn primary_selected(&self) -> Option<usize> {
        self.selection
            .keyboard_cursor
            .or_else(|| self.selection.selected.iter().copied().next())
    }
}

/// Cascading column navigation state for the Miller view.
#[derive(Clone, Default)]
pub struct MillerState {
    pub columns: Vec<MillerColumn>,
    pub focused: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    NotLoaded,
    Loading,
    Loaded,
    Error(String),
}

#[derive(Clone, Default)]
pub struct SelectionState {
    pub selected: HashSet<usize>,
    pub last_clicked_index: Option<usize>,
    pub keyboard_cursor: Option<usize>,
    pub selection_anchor: Option<usize>,
    pub scroll_to_selected: bool,
}

impl SelectionState {
    pub fn clear(&mut self) {
        self.selected.clear();
        self.last_clicked_index = None;
        self.keyboard_cursor = None;
        self.selection_anchor = None;
        self.scroll_to_selected = false;
    }
}

#[derive(Clone, Default)]
pub struct PathEditState {
    pub editing: bool,
    pub text: String,
    pub focus: bool,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchStatus {
    #[default]
    Idle,
    Searching,
    Ready,
    Error,
}

#[derive(Clone, Default)]
pub struct SearchInPlaceState {
    pub active: bool,
    pub query: String,
    pub recursive: bool,
    pub focus: bool,
    pub status: SearchStatus,
    pub generation: u64,
    pub original_entries: Option<std::sync::Arc<[FileEntry]>>,
    pub error: Option<String>,
}

impl SearchInPlaceState {
    pub fn reset(&mut self) {
        let recursive = self.recursive;
        *self = Self {
            recursive,
            ..Self::default()
        };
    }

    pub fn is_filtering(&self) -> bool {
        self.active && !self.query.trim().is_empty()
    }
}

/// Live `shown/total` counts per kind for the bottom bar, cached alongside the
/// `visible` rebuild so reading them every frame is free.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisibleCounts {
    pub folders_shown: usize,
    pub folders_total: usize,
    pub files_shown: usize,
    pub files_total: usize,
}

#[derive(Clone)]
pub struct TabState {
    pub entries: std::sync::Arc<[FileEntry]>,
    pub filter: String,
    /// Per-pane visibility toggles, mirrored from `TabData` so the cached
    /// filtering below doesn't need access to the pane tree.
    pub show_files: bool,
    pub show_folders: bool,
    pub load_state: LoadState,
    pub last_nav_op_id: u64,
    pub grid_columns: usize,
    pub selection: SelectionState,
    pub path_edit: PathEditState,
    pub search: SearchInPlaceState,
    pub miller: MillerState,
    /// Width of the pane as of the last rendered frame, used to detect active
    /// resize and suppress layout thrashing in the grid view.
    pub last_pane_width: f32,
    /// Height of the pane as of the last rendered frame, used by canvas-backed
    /// views to avoid shaping rows outside the visible viewport.
    pub last_pane_height: f32,
    /// Bumped whenever `entries` is replaced; used to invalidate `visible` cache.
    /// Mutate `entries` only through [`TabState::set_entries`]/[`TabState::begin_navigation`]
    /// so this stays in sync.
    pub entries_version: u64,
    /// Cached result of filtering `entries` by `filter`, rebuilt only when the
    /// filter text or the entry set changes (see [`TabState::visible_indices`]).
    pub visible: Arc<Vec<usize>>,
    pub visible_filter: String,
    pub visible_version: u64,
    pub visible_valid: bool,
    visible_show_files: bool,
    visible_show_folders: bool,
    counts: VisibleCounts,
    /// Cached set of unique category labels (e.g. "Carpeta", "Imagen") derived from
    /// `entries`, rebuilt only when `entries_version` advances.
    pub type_labels: Vec<String>,
    type_labels_version: u64,
    /// Bumped once each time a *fresh* listing (tab create / navigation) finishes
    /// loading — NOT on incremental fs-watch refreshes. Views key their item
    /// appear animation on this so rows fade/slide in only when content is new.
    pub appear_seq: u64,
    /// Set by create/navigate paths; consumed on the next completed load to bump
    /// `appear_seq`. Keeps watch refreshes from re-triggering the appear animation.
    pub pending_appear: bool,
}

impl Default for TabState {
    fn default() -> Self {
        Self {
            entries: std::sync::Arc::new([]),
            filter: String::new(),
            show_files: true,
            show_folders: true,
            load_state: LoadState::NotLoaded,
            last_nav_op_id: 0,
            grid_columns: 1,
            selection: SelectionState::default(),
            path_edit: PathEditState::default(),
            search: SearchInPlaceState::default(),
            miller: MillerState::default(),
            last_pane_width: 0.0,
            last_pane_height: 0.0,
            entries_version: 0,
            visible: Arc::new(Vec::new()),
            visible_filter: String::new(),
            visible_version: 0,
            visible_valid: false,
            visible_show_files: true,
            visible_show_folders: true,
            counts: VisibleCounts::default(),
            type_labels: Vec::new(),
            type_labels_version: u64::MAX, // force rebuild on first call
            appear_seq: 0,
            pending_appear: false,
        }
    }
}

impl TabState {
    /// Replace the directory listing, invalidating the cached visible set.
    pub fn set_entries(&mut self, entries: std::sync::Arc<[FileEntry]>) {
        self.entries = entries;
        self.entries_version = self.entries_version.wrapping_add(1);
        self.visible_valid = false;
    }

    /// Returns a deduplicated sorted list of category labels for all entries.
    ///
    /// Rebuilt only when `entries_version` advances (i.e., when the directory listing
    /// changes). The `label_fn` is called only during a rebuild, not on every render.
    pub fn type_labels_cached(&mut self, label_fn: impl Fn(&FileEntry) -> String) -> &[String] {
        if self.type_labels_version != self.entries_version {
            let mut set = std::collections::BTreeSet::new();
            for entry in self.entries.iter() {
                set.insert(label_fn(entry));
            }
            self.type_labels = set.into_iter().collect();
            self.type_labels_version = self.entries_version;
        }
        &self.type_labels
    }

    pub fn begin_navigation(&mut self) {
        // A real navigation (not a watch refresh) — the next completed load should
        // play the item appear animation.
        self.pending_appear = true;
        self.entries = std::sync::Arc::new([]);
        self.entries_version = self.entries_version.wrapping_add(1);
        self.visible_valid = false;
        self.selection.clear();
        self.filter.clear();
        self.show_files = true;
        self.show_folders = true;
        self.search.reset();
        self.path_edit.editing = false;
        self.path_edit.error = None;
        // The Miller view lazily re-initialises its first column from the pane path.
        self.miller = MillerState::default();
    }

    /// Flip the per-kind visibility toggles, invalidating the cached visible set.
    pub fn set_visibility(&mut self, show_files: bool, show_folders: bool) {
        if self.show_files != show_files || self.show_folders != show_folders {
            self.show_files = show_files;
            self.show_folders = show_folders;
            self.visible_valid = false;
        }
    }

    /// `shown/total` per kind as of the last `visible_indices` rebuild.
    pub fn counts(&mut self) -> VisibleCounts {
        self.visible_indices();
        self.counts
    }

    /// Indices into `entries` that pass the current `filter`, as a cached slice.
    ///
    /// The expensive part (lowercasing every entry name) only runs when the filter
    /// text, the visibility toggles or the entry set actually change; otherwise this
    /// returns the slice built on a previous frame.
    pub fn visible_indices(&mut self) -> &[usize] {
        if !self.visible_valid
            || self.visible_version != self.entries_version
            || self.visible_filter != self.filter
            || self.visible_show_files != self.show_files
            || self.visible_show_folders != self.show_folders
        {
            let needle = self.filter.to_lowercase();
            let (show_files, show_folders) = (self.show_files, self.show_folders);
            let entries = &self.entries;
            let visible = Arc::make_mut(&mut self.visible);
            visible.clear();
            let mut counts = VisibleCounts::default();
            for entry in entries.iter() {
                if matches!(entry.kind, FileKind::Dir) {
                    counts.folders_total += 1;
                } else {
                    counts.files_total += 1;
                }
            }
            let kind_visible = |entry: &FileEntry| {
                if matches!(entry.kind, FileKind::Dir) {
                    show_folders
                } else {
                    show_files
                }
            };
            if needle.is_empty() {
                visible.extend(
                    entries
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| kind_visible(e))
                        .map(|(idx, _)| idx),
                );
            } else {
                let mut matches: Vec<(usize, i64)> = entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| kind_visible(e))
                    .filter_map(|(idx, entry)| {
                        fuzzy_score(&needle, &entry.name_lower).map(|score| (idx, score))
                    })
                    .collect();
                matches.sort_by(|(ai, ascore), (bi, bscore)| {
                    bscore.cmp(ascore).then_with(|| ai.cmp(bi))
                });
                visible.extend(matches.into_iter().map(|(idx, _)| idx));
            }
            for &idx in visible.iter() {
                if matches!(entries[idx].kind, FileKind::Dir) {
                    counts.folders_shown += 1;
                } else {
                    counts.files_shown += 1;
                }
            }
            self.counts = counts;
            self.visible_filter.clear();
            self.visible_filter.push_str(&self.filter);
            self.visible_version = self.entries_version;
            self.visible_show_files = self.show_files;
            self.visible_show_folders = self.show_folders;
            self.visible_valid = true;
        }
        &self.visible
    }

    /// Cheap render snapshot of the cached visible indices. Keeping this behind an
    /// `Arc` avoids copying the full directory index every frame in virtualized views.
    pub fn visible_indices_shared(&mut self) -> Arc<Vec<usize>> {
        self.visible_indices();
        self.visible.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: FileKind) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            path: std::path::PathBuf::from(name),
            kind,
            size: None,
            modified: None,
            extension: None,
            display_size: String::new(),
            display_date: String::new(),
            display_type: String::new(),
            _is_hidden: false,
            _is_symlink: false,
            archive_root: None,
            recycle_info: None,
        }
    }

    #[test]
    fn visibility_toggles_filter_by_kind_and_feed_counts() {
        let mut ts = TabState::default();
        ts.set_entries(
            vec![
                entry("docs", FileKind::Dir),
                entry("a.txt", FileKind::File),
                entry("b.txt", FileKind::File),
            ]
            .into(),
        );

        assert_eq!(ts.visible_indices(), &[0, 1, 2]);
        let c = ts.counts();
        assert_eq!((c.folders_shown, c.folders_total), (1, 1));
        assert_eq!((c.files_shown, c.files_total), (2, 2));

        ts.set_visibility(true, false);
        assert_eq!(ts.visible_indices(), &[1, 2]);
        let c = ts.counts();
        assert_eq!((c.folders_shown, c.folders_total), (0, 1));

        ts.set_visibility(false, true);
        assert_eq!(ts.visible_indices(), &[0]);
        let c = ts.counts();
        assert_eq!((c.files_shown, c.files_total), (0, 2));
    }

    #[test]
    fn visibility_combines_with_fuzzy_filter() {
        let mut ts = TabState::default();
        ts.set_entries(
            vec![
                entry("textures", FileKind::Dir),
                entry("notes.txt", FileKind::File),
                entry("image.png", FileKind::File),
            ]
            .into(),
        );
        ts.filter = "txt".to_string();
        // "txt" fuzzy-matches both "textures" (t-x-t) and "notes.txt".
        assert_eq!(ts.visible_indices().len(), 2);

        ts.set_visibility(true, false);
        assert_eq!(ts.visible_indices(), &[1]);
        let c = ts.counts();
        assert_eq!(c.files_shown, 1);
        assert_eq!(c.folders_shown, 0);
    }

    #[test]
    fn navigation_resets_kind_visibility() {
        let mut ts = TabState::default();
        ts.set_visibility(true, false);

        ts.begin_navigation();

        ts.set_entries(vec![entry("docs", FileKind::Dir), entry("a.txt", FileKind::File)].into());
        assert_eq!(ts.visible_indices(), &[0, 1]);
    }
}

fn fuzzy_score(query: &str, candidate: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }

    let mut score = 0_i64;
    let mut last_match: Option<usize> = None;
    let mut search_from = 0;

    for q in query.chars().filter(|c| !c.is_whitespace()) {
        let hay = &candidate[search_from..];
        let offset = hay.find(q)?;
        let pos = search_from + offset;
        if pos == 0 {
            score += 24;
        }
        if candidate[..pos].ends_with(['-', '_', '.', ' ', '/', '\\']) {
            score += 12;
        }
        if let Some(prev) = last_match {
            let gap = pos.saturating_sub(prev + 1);
            score += 20_i64.saturating_sub(i64::try_from(gap).unwrap_or(20));
        }
        score += 4;
        last_match = Some(pos);
        search_from = pos + q.len_utf8();
    }

    if candidate.starts_with(query) {
        score += 60;
    } else if candidate.contains(query) {
        score += 30;
    }

    score -= i64::try_from(candidate.len()).unwrap_or(i64::MAX / 2) / 8;
    Some(score)
}
