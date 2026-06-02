use crate::state::file_entry::FileEntry;
use std::collections::HashSet;
use std::path::PathBuf;

/// One directory column in the Miller (columns) view.
pub struct MillerColumn {
    pub path: PathBuf,
    pub entries: Vec<FileEntry>,
    pub load_state: LoadState,
    pub op_id: u64,
    /// Index into `entries` of the selected item that opened the next column.
    pub selected: Option<usize>,
}

impl MillerColumn {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            entries: Vec::new(),
            load_state: LoadState::NotLoaded,
            op_id: 0,
            selected: None,
        }
    }
}

/// Cascading column navigation state for the Miller view.
#[derive(Default)]
pub struct MillerState {
    pub columns: Vec<MillerColumn>,
    pub focused: usize,
}

pub struct FileListHitRegions {
    pub list_rect: egui::Rect,
    pub layer_id: egui::LayerId,
    pub item_rects: Vec<(usize, egui::Rect)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoadState {
    NotLoaded,
    Loading,
    Loaded,
    Error(String),
}

#[derive(Default)]
pub struct SelectionState {
    pub selected: HashSet<usize>,
    /// Rubber-band anchor in *content* coordinates (relative to the content
    /// origin, so it survives scrolling). `None` when no rubber-band is active.
    pub drag_select_anchor: Option<egui::Vec2>,
    pub drag_select_base: HashSet<usize>,
    pub last_clicked_index: Option<usize>,
    pub primary_press: Option<(usize, egui::Pos2)>,
    pub last_primary_click: Option<(usize, f64)>,
    pub keyboard_cursor: Option<usize>,
    pub selection_anchor: Option<usize>,
    pub scroll_to_selected: bool,
}

impl SelectionState {
    pub fn clear(&mut self) {
        self.selected.clear();
        self.drag_select_anchor = None;
        self.drag_select_base.clear();
        self.last_clicked_index = None;
        self.primary_press = None;
        self.last_primary_click = None;
        self.keyboard_cursor = None;
        self.selection_anchor = None;
        self.scroll_to_selected = false;
    }
}

#[derive(Default)]
pub struct PathEditState {
    pub editing: bool,
    pub text: String,
    pub focus: bool,
    pub error: Option<String>,
}


pub struct TabState {
    pub entries: Vec<FileEntry>,
    pub filter: String,
    pub load_state: LoadState,
    pub last_nav_op_id: u64,
    pub file_list_hit_regions: Option<FileListHitRegions>,
    pub grid_columns: usize,
    pub selection: SelectionState,
    pub path_edit: PathEditState,
    pub miller: MillerState,
    /// Bumped whenever `entries` is replaced; used to invalidate `visible` cache.
    /// Mutate `entries` only through [`TabState::set_entries`]/[`TabState::begin_navigation`]
    /// so this stays in sync.
    pub entries_version: u64,
    /// Cached result of filtering `entries` by `filter`, rebuilt only when the
    /// filter text or the entry set changes (see [`TabState::visible_indices`]).
    pub visible: Vec<usize>,
    pub visible_filter: String,
    pub visible_version: u64,
    pub visible_valid: bool,
}

impl Default for TabState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            filter: String::new(),
            load_state: LoadState::NotLoaded,
            last_nav_op_id: 0,
            file_list_hit_regions: None,
            grid_columns: 1,
            selection: SelectionState::default(),
            path_edit: PathEditState::default(),
            miller: MillerState::default(),
            entries_version: 0,
            visible: Vec::new(),
            visible_filter: String::new(),
            visible_version: 0,
            visible_valid: false,
        }
    }
}

impl TabState {
    /// Replace the directory listing, invalidating the cached visible set.
    pub fn set_entries(&mut self, entries: Vec<FileEntry>) {
        self.entries = entries;
        self.entries_version = self.entries_version.wrapping_add(1);
        self.visible_valid = false;
    }

    pub fn begin_navigation(&mut self) {
        self.entries.clear();
        self.entries_version = self.entries_version.wrapping_add(1);
        self.visible_valid = false;
        self.selection.clear();
        self.file_list_hit_regions = None;
        self.filter.clear();
        self.path_edit.editing = false;
        self.path_edit.error = None;
        // The Miller view lazily re-initialises its first column from the pane path.
        self.miller = MillerState::default();
    }

    /// Indices into `entries` that pass the current `filter`, as a cached slice.
    ///
    /// The expensive part (lowercasing every entry name) only runs when the filter
    /// text or the entry set actually changes; otherwise this returns the slice
    /// built on a previous frame.
    pub fn visible_indices(&mut self) -> &[usize] {
        if !self.visible_valid
            || self.visible_version != self.entries_version
            || self.visible_filter != self.filter
        {
            let needle = self.filter.to_lowercase();
            self.visible.clear();
            if needle.is_empty() {
                self.visible.extend(0..self.entries.len());
            } else {
                self.visible.extend(
                    self.entries
                        .iter()
                        .enumerate()
                        .filter(|(_, entry)| entry.name_lower.contains(&needle))
                        .map(|(idx, _)| idx),
                );
            }
            self.visible_filter.clear();
            self.visible_filter.push_str(&self.filter);
            self.visible_version = self.entries_version;
            self.visible_valid = true;
        }
        &self.visible
    }
}
