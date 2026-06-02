use crate::app::App;
use crate::input::action::Action;
use crate::state::drag::DragHoverTarget;
use std::time::Duration;

impl App {
    pub(crate) fn commit_spring_navigation(&mut self) {
        self.state.drag_drop.drag_hover.original_paths.clear();
        self.state.drag_drop.drag_hover.original_focused_tile = None;
        self.state.drag_drop.drag_hover.candidate = None;
        self.state.drag_drop.drag_hover.observed_target = None;
        self.state.drag_drop.drag_hover.confirmation_target = None;
        self.state.drag_drop.drag_hover.triggered = false;
    }

    pub(crate) fn cancel_spring_navigation(&mut self) {
        let original_paths = std::mem::take(&mut self.state.drag_drop.drag_hover.original_paths);
        for (tab_id, path) in original_paths {
            let Some(egui_tiles::Tile::Pane(pane)) = self.state.pane_tree.tiles.get_mut(tab_id)
            else {
                continue;
            };
            let col = pane.sort_col.clone();
            let dir = pane.sort_dir.clone();
            pane.path = path.clone();
            if let Ok(handle) = crate::fs::watcher::watch_path(&path, self.tx.clone()) {
                self.state.watchers.insert(tab_id, handle);
            }
            if let Some(tab_state) = self.state.tab_states.get_mut(&tab_id) {
                tab_state.begin_navigation();
            }
            self.spawn_list_dir(tab_id, path, col, dir);
        }
        if let Some(tab_id) = self.state.drag_drop.drag_hover.original_focused_tile.take() {
            self.state.focused_tile = tab_id;
        }
        self.state.drag_drop.drag_hover.candidate = None;
        self.state.drag_drop.drag_hover.observed_target = None;
        self.state.drag_drop.drag_hover.confirmation_target = None;
        self.state.drag_drop.drag_hover.triggered = false;
    }

    pub(crate) fn tick_drag_hover(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        const HOVER_DELAY: f64 = 0.65;
        const CONFIRMATION_DURATION: f64 = 0.32;

        let external_drag_active = ctx.input(|input| {
            !input.raw.hovered_files.is_empty() || !input.raw.dropped_files.is_empty()
        });
        if self.state.drag_drop.dragged_files.is_none() && !external_drag_active {
            if !self.state.drag_drop.drag_hover.original_paths.is_empty()
                || self
                    .state
                    .drag_drop
                    .drag_hover
                    .original_focused_tile
                    .is_some()
            {
                self.cancel_spring_navigation();
            }
            self.state.drag_drop.drag_hover.candidate = None;
            self.state.drag_drop.drag_hover.observed_target = None;
            self.state.drag_drop.drag_hover.confirmation_target = None;
            self.state.drag_drop.drag_hover.triggered = false;
            return;
        }
        let observed = self.state.drag_drop.drag_hover.observed_target.take();
        if observed != self.state.drag_drop.drag_hover.candidate {
            self.state.drag_drop.drag_hover.candidate = observed;
            self.state.drag_drop.drag_hover.candidate_since = ctx.input(|input| input.time);
            self.state.drag_drop.drag_hover.confirmation_target = None;
            self.state.drag_drop.drag_hover.triggered = false;
        }
        let Some(target) = self.state.drag_drop.drag_hover.candidate.clone() else {
            return;
        };
        let elapsed =
            ctx.input(|input| input.time) - self.state.drag_drop.drag_hover.candidate_since;
        if elapsed < HOVER_DELAY {
            ctx.request_repaint_after(Duration::from_secs_f64(HOVER_DELAY - elapsed));
            return;
        }
        if self.state.drag_drop.drag_hover.triggered {
            return;
        }
        let now = ctx.input(|input| input.time);
        if self.state.drag_drop.drag_hover.confirmation_target.as_ref() != Some(&target) {
            self.state.drag_drop.drag_hover.confirmation_target = Some(target.clone());
            self.state.drag_drop.drag_hover.confirmation_started_at = now;
        }
        let confirmation_elapsed = now - self.state.drag_drop.drag_hover.confirmation_started_at;
        if confirmation_elapsed < CONFIRMATION_DURATION {
            ctx.request_repaint();
            return;
        }
        self.state.drag_drop.drag_hover.confirmation_target = None;
        self.state.drag_drop.drag_hover.triggered = true;
        match target {
            DragHoverTarget::Folder { tab, path } | DragHoverTarget::Location { tab, path } => {
                actions.push(Action::SpringNavigate { tab, path });
            }
            DragHoverTarget::Tab(tab) => actions.push(Action::SpringFocusTab(tab)),
        }
    }
}
