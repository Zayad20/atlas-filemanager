use crate::state::file_entry::FileEntry;
use std::collections::HashSet;

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

#[allow(dead_code)]
pub struct TabState {
    pub entries: Vec<FileEntry>,
    pub selected: HashSet<usize>,
    pub filter: String,
    pub drag_select_start: Option<egui::Pos2>,
    pub drag_select_base: HashSet<usize>,
    pub load_state: LoadState,
    pub last_nav_op_id: u64,
    pub last_clicked_index: Option<usize>,
    pub primary_press: Option<(usize, egui::Pos2)>,
    pub last_primary_click: Option<(usize, f64)>,
    pub file_list_hit_regions: Option<FileListHitRegions>,
    pub grid_columns: usize,
    pub keyboard_cursor: Option<usize>,
    pub selection_anchor: Option<usize>,
    pub scroll_to_selected: bool,
    pub rename_state: Option<(usize, String)>,
    pub path_editing: bool,
    pub path_edit_text: String,
    pub path_edit_focus: bool,
    pub path_edit_error: Option<String>,
}

impl Default for TabState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            selected: HashSet::new(),
            filter: String::new(),
            drag_select_start: None,
            drag_select_base: HashSet::new(),
            load_state: LoadState::NotLoaded,
            last_nav_op_id: 0,
            last_clicked_index: None,
            primary_press: None,
            last_primary_click: None,
            file_list_hit_regions: None,
            grid_columns: 1,
            keyboard_cursor: None,
            selection_anchor: None,
            scroll_to_selected: false,
            rename_state: None,
            path_editing: false,
            path_edit_text: String::new(),
            path_edit_focus: false,
            path_edit_error: None,
        }
    }
}

impl TabState {
    pub fn begin_navigation(&mut self) {
        self.entries.clear();
        self.selected.clear();
        self.drag_select_start = None;
        self.drag_select_base.clear();
        self.last_clicked_index = None;
        self.primary_press = None;
        self.last_primary_click = None;
        self.file_list_hit_regions = None;
        self.keyboard_cursor = None;
        self.selection_anchor = None;
        self.scroll_to_selected = false;
        self.filter.clear();
        self.path_editing = false;
        self.path_edit_error = None;
    }

    pub fn visible_indices(&self) -> impl Iterator<Item = usize> + '_ {
        let filter = self.filter.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(move |(_, entry)| {
                filter.is_empty() || entry.name.to_lowercase().contains(&filter)
            })
            .map(|(idx, _)| idx)
    }
}
