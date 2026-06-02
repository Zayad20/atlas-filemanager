use crate::ui::fuzzy_overlay::FuzzyOverlayState;
use crate::state::ops::PasteConflictState;

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
