use crate::state::ops::PasteConflictState;

/// Shared state for the fuzzy overlays (goto bar, command palette): query text,
/// the highlighted row, and a scroll-into-view request. Framework-neutral — the
/// open-animation origin is recomputed by the renderer, not stored here.
#[derive(Default)]
pub struct FuzzyOverlayState {
    pub query: String,
    pub selected_idx: usize,
    pub scroll_to_selected: bool,
    /// Wall-clock start of the open tween (set on reset/open).
    pub opening_started_at: Option<std::time::Instant>,
}

impl FuzzyOverlayState {
    pub fn reset(&mut self) {
        self.query.clear();
        self.selected_idx = 0;
        self.scroll_to_selected = false;
        self.opening_started_at = Some(std::time::Instant::now());
    }
}

/// State for the "open tabs" quick-switcher popup.
#[derive(Default)]
pub struct TabsPopupState {
    pub query: String,
    pub open: bool,
}

#[derive(Default)]
pub struct OverlayState {
    pub show_goto_bar: bool,
    pub show_palette: bool,
    pub show_settings: bool,
    pub goto_state: FuzzyOverlayState,
    pub palette_state: FuzzyOverlayState,
    pub paste_conflict: Option<PasteConflictState>,
    pub show_analyzer: bool,
}
