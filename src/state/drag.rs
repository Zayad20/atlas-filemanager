use std::collections::HashMap;
use std::path::PathBuf;
use crate::state::pane::TabId;
use crate::state::tag_cache::TagId;

#[derive(Clone)]
pub struct DraggedFiles {
    pub source_tab: TabId,
    pub paths: Vec<PathBuf>,
}

/// An item dragged out of the sidebar. Dropping it anywhere over the view opens it in a
/// new tab. Kept separate from `DraggedFiles` (file drags) since the two are mutually
/// exclusive and the existing file-drag plumbing stays untouched.
#[derive(Clone)]
pub enum SidebarDragItem {
    /// A tag — opens a virtual results tab listing every file carrying it.
    Tag(TagId),
    /// A bookmark / recent / drive / place — opens that directory.
    Path(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragHoverTarget {
    Folder { tab: TabId, path: PathBuf },
    Tab(TabId),
    Location { tab: TabId, path: PathBuf },
}

#[derive(Default)]
pub struct DragHoverState {
    pub observed_target: Option<DragHoverTarget>,
    pub candidate: Option<DragHoverTarget>,
    pub candidate_since: f64,
    pub confirmation_target: Option<DragHoverTarget>,
    pub confirmation_started_at: f64,
    pub triggered: bool,
    pub original_paths: HashMap<TabId, PathBuf>,
    pub original_focused_tile: Option<TabId>,
}

impl DragHoverState {
    pub fn confirmation_visible(&self, target: &DragHoverTarget, now: f64) -> bool {
        const BLINK_INTERVAL: f64 = 0.08;
        const BLINK_DURATION: f64 = BLINK_INTERVAL * 4.0;

        if self.confirmation_target.as_ref() != Some(target) {
            return false;
        }
        let elapsed = now - self.confirmation_started_at;
        (0.0..BLINK_DURATION).contains(&elapsed)
            && (elapsed / BLINK_INTERVAL).floor() as u64 % 2 == 0
    }
}

#[derive(Clone)]
pub struct ExternalDropTarget {
    pub tab: TabId,
    pub dest: PathBuf,
}

#[derive(Default)]
pub struct DragDropState {
    pub dragged_files: Option<DraggedFiles>,
    /// A sidebar item currently being dragged (mutually exclusive with `dragged_files`).
    pub sidebar_drag: Option<SidebarDragItem>,
    pub drag_hover: DragHoverState,
    pub external_drop_target: Option<ExternalDropTarget>,
    /// Tabs container the pointer is hovering over the empty tab-strip area of,
    /// while an OS file drag is in progress — a drop there opens folders as new tabs.
    pub tab_strip_drop: Option<TabId>,
    pub drop_shelf_open: bool,
    pub drop_shelf_files: Vec<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::{DragHoverState, DragHoverTarget};
    use egui_tiles::TileId;

    #[test]
    fn drag_hover_confirmation_blinks_twice_then_stops() {
        let target = DragHoverTarget::Tab(TileId(7));
        let state = DragHoverState {
            confirmation_target: Some(target.clone()),
            confirmation_started_at: 10.0,
            ..Default::default()
        };

        assert!(state.confirmation_visible(&target, 10.01));
        assert!(!state.confirmation_visible(&target, 10.09));
        assert!(state.confirmation_visible(&target, 10.17));
        assert!(!state.confirmation_visible(&target, 10.25));
        assert!(!state.confirmation_visible(&target, 10.33));
    }
}
