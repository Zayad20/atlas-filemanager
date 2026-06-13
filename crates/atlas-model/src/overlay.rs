#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use crate::ops::TransferConflictState;

/// Shared state for the fuzzy palettes (goto bar, command palette, ext palette):
/// query text, the highlighted row, and a scroll-into-view request.
/// Framework-neutral — the open-animation origin is recomputed by the renderer,
/// not stored here.
#[derive(Default)]
pub struct FuzzyOverlayState {
    pub query: String,
    pub selected_idx: usize,
    pub scroll_to_selected: bool,
    /// Wall-clock start of the open tween (set on reset/open).
    pub opening_started_at: Option<std::time::Instant>,
    /// Tab toggle: show every item's description line, not just the selected one's.
    pub expanded: bool,
    /// Section ids the user collapsed via their headers.
    pub collapsed_sections: std::collections::HashSet<String>,
}

impl FuzzyOverlayState {
    pub fn reset(&mut self) {
        self.query.clear();
        self.selected_idx = 0;
        self.scroll_to_selected = false;
        self.opening_started_at = Some(std::time::Instant::now());
        self.expanded = false;
        self.collapsed_sections.clear();
    }
}

#[derive(Default)]
pub struct OverlayState {
    pub show_goto_bar: bool,
    pub show_palette: bool,
    pub show_settings: bool,
    pub goto_state: FuzzyOverlayState,
    pub palette_state: FuzzyOverlayState,
    pub show_ext_palette: bool,
    pub ext_palette_state: FuzzyOverlayState,
    pub ext_palette_items: Vec<String>,
    #[cfg(not(target_arch = "wasm32"))]
    pub ext_palette_reply: Option<std::sync::mpsc::Sender<String>>,
    pub transfer_conflict: Option<TransferConflictState>,
    pub show_analyzer: bool,
    /// The rename-active-profile dialog is open.
    pub renaming_profile: bool,
    /// File-operation errors to show to the user as dismissible toasts.
    pub op_errors: std::collections::VecDeque<String>,
}
