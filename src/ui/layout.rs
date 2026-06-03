use egui_tiles::{Behavior, SimplificationOptions, TileId, Tiles, UiResponse};
use std::collections::HashMap;

use crate::fs::preview::PreviewResult;
use crate::input::action::Action;
use crate::state::app_state::AppState;
use crate::state::drag::DraggedFiles;
use crate::state::drag::ExternalDropTarget;
use crate::state::drag::{DragHoverState, DragHoverTarget};
use crate::state::file_entry::FileKind;
use crate::state::pane::TabData;
use crate::state::preview_cache::MultiPreviewState;
use crate::state::settings::{InspectorPlacement, Settings, ViewMode};
use crate::state::tab::TabState;
use crate::ui::breadcrumb::show_breadcrumb;
use crate::ui::drag_feedback::paint_drag_hover_confirmation;
use crate::ui::file_list::show_file_list;

pub struct AtlasBehavior<'a> {
    pub tab_states: &'a mut HashMap<TileId, TabState>,

    pub settings: &'a mut Settings,
    pub actions: &'a mut Vec<Action>,
    pub focused_tile: &'a mut TileId,
    pub dragged_files: &'a mut Option<DraggedFiles>,
    pub drag_hover: &'a mut DragHoverState,
    pub external_drop_target: &'a mut Option<ExternalDropTarget>,
    pub tab_strip_drop: &'a mut Option<TileId>,
    pub preview_results: &'a HashMap<TileId, PreviewResult>,
    pub preview_textures: &'a HashMap<TileId, egui::TextureHandle>,
    pub multi_previews: &'a HashMap<TileId, MultiPreviewState>,
    pub tag_cache: &'a crate::state::tag_cache::TagCache,
}

impl<'a> Behavior<TabData> for AtlasBehavior<'a> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, tile_id: TileId, pane: &mut TabData) -> UiResponse {
        let has_external_files = ui.input(|input| !input.raw.hovered_files.is_empty());
        if has_external_files && ui.rect_contains_pointer(ui.max_rect()) {
            *self.external_drop_target = Some(ExternalDropTarget {
                tab: tile_id,
                dest: pane.path.clone(),
            });
            ui.painter().rect_stroke(
                ui.max_rect(),
                4.0,
                egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
        }
        // Update focused tile when this pane receives any interaction
        if ui.rect_contains_pointer(ui.max_rect()) && ui.input(|i| i.pointer.any_click()) {
            *self.focused_tile = tile_id;
        }
        if ui.rect_contains_pointer(ui.max_rect()) {
            update_view_zoom(ui, pane);
        }

        ui.vertical(|ui| {
            // Breadcrumb navigation
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Added right-to-left, so these read Details · List · Icons · Columns.
                    if ui
                        .selectable_label(pane.view_mode == ViewMode::Miller, "Columns")
                        .clicked()
                    {
                        self.actions.push(Action::SetViewMode(ViewMode::Miller));
                    }
                    if ui
                        .selectable_label(pane.view_mode == ViewMode::Grid, "Icons")
                        .clicked()
                    {
                        self.actions.push(Action::SetViewMode(ViewMode::Grid));
                    }
                    if ui
                        .selectable_label(pane.view_mode == ViewMode::List, "List")
                        .clicked()
                    {
                        self.actions.push(Action::SetViewMode(ViewMode::List));
                    }
                    if ui
                        .selectable_label(pane.view_mode == ViewMode::Details, "Details")
                        .clicked()
                    {
                        self.actions.push(Action::SetViewMode(ViewMode::Details));
                    }
                    let tab_state = self.tab_states.entry(tile_id).or_default();
                    if let Some(new_path) = show_breadcrumb(
                        ui,
                        tile_id,
                        &pane.path,
                        tab_state,
                        self.dragged_files.is_some()
                            || ui.input(|input| !input.raw.hovered_files.is_empty()),
                        self.drag_hover,
                    ) {
                        self.actions.push(Action::Navigate {
                            tab: tile_id,
                            path: new_path,
                        });
                    }
                });
            });
            ui.separator();

            // File list
            let sort_col = pane.sort_col.clone();
            let sort_dir = pane.sort_dir.clone();
            let tab_state = self.tab_states.entry(tile_id).or_default();
            if *self.focused_tile == tile_id && !ui.ctx().wants_keyboard_input() {
                if pane.view_mode == ViewMode::Miller {
                    handle_miller_keyboard(ui, tile_id, tab_state, self.actions);
                } else {
                    handle_file_navigation_keyboard(
                        ui,
                        tile_id,
                        &pane.view_mode,
                        tab_state,
                        self.actions,
                    );
                    update_filter_from_keyboard(ui, tab_state);
                }
            }
            let preview_summary =
                crate::ui::inspector::SelectionSummary::for_tab(tab_state, &pane.view_mode);
            let show_local_inspector = pane.preview_enabled
                && self.settings.inspector_placement == InspectorPlacement::ActiveTab
                && *self.focused_tile == tile_id
                && preview_summary.count > 0;
            if show_local_inspector {
                ui.columns(2, |columns| {
                    show_file_list(
                        &mut columns[0],
                        tile_id,
                        &pane.path,
                        tab_state,
                        &sort_col,
                        &sort_dir,
                        &pane.view_mode,
                        pane.thumbnail_size,
                        self.dragged_files,
                        self.drag_hover,
                        self.external_drop_target,
                        self.actions,
                    );
                    columns[1].heading("Preview");
                    columns[1].separator();
                    crate::ui::inspector::show_preview(
                        &mut columns[1],
                        &preview_summary,
                        self.preview_results.get(&tile_id),
                        self.preview_textures.get(&tile_id),
                        self.multi_previews.get(&tile_id),
                        self.tag_cache,
                        self.actions,
                    );
                });
            } else {
                show_file_list(
                    ui,
                    tile_id,
                    &pane.path,
                    tab_state,
                    &sort_col,
                    &sort_dir,
                    &pane.view_mode,
                    pane.thumbnail_size,
                    self.dragged_files,
                    self.drag_hover,
                    self.external_drop_target,
                    self.actions,
                );
            }
        });

        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &TabData) -> egui::WidgetText {
        let name = crate::util::display::display_name(&pane.path);
        egui::RichText::new(name).into()
    }

    fn is_tab_closable(&self, _tiles: &Tiles<TabData>, _tile_id: TileId) -> bool {
        true
    }

    fn on_tab_close(&mut self, _tiles: &mut Tiles<TabData>, tile_id: TileId) -> bool {
        self.actions.push(Action::CloseTab(tile_id));
        false
    }

    fn on_tab_button(
        &mut self,
        _tiles: &Tiles<TabData>,
        tile_id: TileId,
        response: egui::Response,
    ) -> egui::Response {
        if response.clicked() {
            self.actions.push(Action::FocusTab(tile_id));
        }
        response.context_menu(|ui| {
            if ui.button("Split Right").clicked() {
                self.actions.push(Action::SplitVertical(tile_id));
                ui.close();
            }
            if ui.button("Split Down").clicked() {
                self.actions.push(Action::SplitHorizontal(tile_id));
                ui.close();
            }
            ui.separator();
            if ui.button("Close Pane").clicked() {
                self.actions.push(Action::ClosePane(tile_id));
                ui.close();
            }
        });
        if response.contains_pointer()
            && (self.dragged_files.is_some()
                || response
                    .ctx
                    .input(|input| !input.raw.hovered_files.is_empty()))
        {
            let target = DragHoverTarget::Tab(tile_id);
            self.drag_hover.observed_target = Some(target.clone());
            paint_drag_hover_confirmation(&response, self.drag_hover, &target);
        }
        response
    }

    fn top_bar_right_ui(
        &mut self,
        _tiles: &Tiles<TabData>,
        ui: &mut egui::Ui,
        tile_id: TileId,
        _tabs: &egui_tiles::Tabs,
        _scroll_offset: &mut f32,
    ) {
        let external_drag = ui.ctx().input(|input| !input.raw.hovered_files.is_empty());
        let internal_drag = self.dragged_files.is_some();

        // The "+" doubles as a drop affordance: dropping folders here opens them
        // as new tabs in this container.
        let resp = ui.add(
            egui::Button::new(egui_phosphor::regular::PLUS).min_size(egui::vec2(28.0, 24.0)),
        );
        if resp.clicked() {
            self.actions.push(Action::NewTab(tile_id));
        }

        if (internal_drag || external_drag) && resp.contains_pointer() {
            ui.painter().rect_stroke(
                resp.rect.expand(1.0),
                6.0,
                egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
                egui::StrokeKind::Outside,
            );
            if internal_drag {
                if ui.input(|input| input.pointer.any_released()) {
                    if let Some(drag) = self.dragged_files.as_ref() {
                        let dirs: Vec<_> =
                            drag.paths.iter().filter(|p| p.is_dir()).cloned().collect();
                        for path in dirs {
                            self.actions.push(Action::NewTabWithPath {
                                pane: tile_id,
                                path,
                            });
                        }
                    }
                }
            } else {
                // OS file drag: remember this strip so app.rs opens dropped folders here.
                *self.tab_strip_drop = Some(tile_id);
            }
        }
    }

    // ── Tab bar aesthetics ────────────────────────────────────────────────────

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        36.0
    }

    fn tab_title_spacing(&self, _visuals: &egui::Visuals) -> f32 {
        16.0
    }

    fn tab_bar_color(&self, visuals: &egui::Visuals) -> egui::Color32 {
        visuals.extreme_bg_color
    }

    fn tab_bg_color(
        &self,
        visuals: &egui::Visuals,
        _tiles: &Tiles<TabData>,
        _tile_id: TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Color32 {
        if state.active {
            visuals.panel_fill
        } else {
            egui::Color32::TRANSPARENT
        }
    }

    fn tab_outline_stroke(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &Tiles<TabData>,
        _tile_id: TileId,
        _state: &egui_tiles::TabState,
    ) -> egui::Stroke {
        egui::Stroke::NONE
    }

    fn tab_bar_hline_stroke(&self, visuals: &egui::Visuals) -> egui::Stroke {
        egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color)
    }

    fn tab_text_color(
        &self,
        visuals: &egui::Visuals,
        _tiles: &Tiles<TabData>,
        _tile_id: TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Color32 {
        if state.active {
            visuals.text_color()
        } else {
            visuals.text_color().gamma_multiply(0.5)
        }
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }

    // ── Drag-and-drop visuals ─────────────────────────────────────────────────

    /// Keep the source pane readable while it is being dragged instead of dimming
    /// it into a near-empty rectangle (egui_tiles' default is `panel_fill * 0.5`).
    fn dragged_overlay_color(&self, visuals: &egui::Visuals) -> egui::Color32 {
        visuals.selection.bg_fill.gamma_multiply(0.08)
    }

    /// Paint a clear, rounded accent indicator for the split/drop target.
    fn paint_drag_preview(
        &self,
        visuals: &egui::Visuals,
        painter: &egui::Painter,
        parent_rect: Option<egui::Rect>,
        preview_rect: egui::Rect,
    ) {
        let accent = visuals.selection.bg_fill;
        let stroke = egui::Stroke::new(2.0, accent);
        if let Some(parent_rect) = parent_rect {
            painter.rect_stroke(
                parent_rect,
                6.0,
                egui::Stroke::new(1.0, accent.gamma_multiply(0.6)),
                egui::StrokeKind::Inside,
            );
        }
        painter.rect(
            preview_rect,
            6.0,
            accent.gamma_multiply(0.18),
            stroke,
            egui::StrokeKind::Inside,
        );
    }

    /// Floating chip that follows the cursor while dragging a tab.
    fn drag_ui(&mut self, tiles: &Tiles<TabData>, ui: &mut egui::Ui, tile_id: TileId) {
        let title = match tiles.get_pane(&tile_id) {
            Some(pane) => crate::util::display::display_name(&pane.path),
            None => self.tab_title_for_tile(tiles, tile_id).text().to_owned(),
        };
        let mut frame = egui::Frame::popup(ui.style());
        frame.fill = ui.visuals().panel_fill;
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(
                    crate::util::display::FOLDER_ICON_COLOR,
                    crate::util::extensions::ExtensionRegistry::icon(None),
                );
                ui.label(egui::RichText::new(title).strong());
            });
        });
    }
}

pub fn show_layout(ctx: &egui::Context, state: &mut AppState, actions: &mut Vec<Action>) {
    state.drag_drop.drag_hover.observed_target = None;
    if !ctx.input(|input| input.raw.hovered_files.is_empty()) {
        state.drag_drop.external_drop_target = None;
        state.drag_drop.tab_strip_drop = None;
    }
    // Tabs popup — move out to avoid borrow conflict on state.tabs_popup
    {
        let mut popup = std::mem::take(&mut state.tabs_popup);
        crate::ui::tabs_popup::show_tabs_popup(ctx, state, &mut popup, actions);
        state.tabs_popup = popup;
    }

    // Overlays (Foreground — render before panels so they sit on top)
    if state.overlay.show_goto_bar {
        crate::ui::goto_bar::show_goto_bar(
            ctx,
            &mut state.overlay.goto_state,
            &state.nucleo_results,
            state.focused_tile,
            actions,
        );
    }
    if state.overlay.show_palette {
        let mut palette_state = std::mem::take(&mut state.overlay.palette_state);
        crate::ui::command_palette::show_command_palette(ctx, state, &mut palette_state, actions);
        state.overlay.palette_state = palette_state;
    }
    if let Some(conflict) = state.overlay.paste_conflict.as_ref() {
        crate::ui::paste_conflict::show_paste_conflict(ctx, conflict, actions);
    }

    // Status bar (bottom) — must be added before CentralPanel
    crate::ui::status_bar::show_status_bar(ctx, state);
    show_command_bar(ctx, state, actions);

    // Sidebar (left) — animated slide in/out
    {
        let t = ctx
            .animate_bool_responsive(egui::Id::new("sidebar_slide"), state.settings.show_sidebar);
        if t > 0.001 {
            let sidebar_width = state.settings.sidebar_width.clamp(160.0, 400.0);
            let w = sidebar_width * t;
            let mut panel = egui::SidePanel::left("sidebar")
                .exact_width(w)
                .resizable(false);
            let panel_fill = ctx.style().visuals.panel_fill;
            panel = panel.frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .fill(egui::Color32::from_rgba_unmultiplied(
                        panel_fill.r(),
                        panel_fill.g(),
                        panel_fill.b(),
                        104,
                    ))
                    .inner_margin(egui::Margin::same(8)),
            );
            panel.show(ctx, |ui| {
                crate::ui::sidebar::show_sidebar_content(ui, state, actions);
            });
        }
    }

    // Inspector (right)
    if state
        .active_tab_data()
        .is_some_and(|tab| tab.preview_enabled)
        && state.settings.inspector_placement == InspectorPlacement::Global
    {
        crate::ui::inspector::show_inspector(ctx, state, actions);
    }

    // Cloud dialogs (floating)
    #[cfg(feature = "cloud")]
    {
        crate::ui::cloud::show_add_remote_window(ctx, state, actions);
        crate::ui::cloud::show_transfers_window(ctx, state, actions);
    }

    // Publish enabled declarative ext commands for the context menu to read.
    crate::ui::context_menu::publish_ext_commands(ctx, state.ext.registry.declarative_commands());

    // Extensions manager + command prompt (floating)
    crate::ui::extensions::show_manager(ctx, state, actions);
    crate::ui::extensions::show_prompt(ctx, state, actions);

    // Settings window (floating)
    if state.overlay.show_settings {
        crate::ui::settings_window::show_settings(
            ctx,
            &mut state.settings,
            &mut state.overlay.show_settings,
        );
    }

    // Batch rename modal
    if let Some(ref mut batch_state) = state.batch_rename {
        let mut open = true;
        crate::ui::batch_rename::show_batch_rename(ctx, batch_state, &mut open, actions);
        if !open {
            state.batch_rename = None;
        }
    }

    // Analyzer Window
    if state.overlay.show_analyzer {
        crate::ui::analyzer::show_analyzer_window(ctx, state, actions);
    }

    // Central pane tree
    egui::CentralPanel::default()
        .frame(egui::Frame::central_panel(&ctx.style()).fill(ctx.style().visuals.window_fill))
        .show(ctx, |ui| {
            let mut behavior = AtlasBehavior {
                tab_states: &mut state.tab_states,
                settings: &mut state.settings,
                actions,
                focused_tile: &mut state.focused_tile,
                dragged_files: &mut state.drag_drop.dragged_files,
                drag_hover: &mut state.drag_drop.drag_hover,
                external_drop_target: &mut state.drag_drop.external_drop_target,
                tab_strip_drop: &mut state.drag_drop.tab_strip_drop,
                preview_results: &state.preview.results,
                preview_textures: &state.preview.textures,
                multi_previews: &state.preview.multi,
                tag_cache: &state.tag_cache,
            };
            state.pane_tree.ui(&mut behavior, ui);
        });

    show_drag_feedback(ctx, state.drag_drop.dragged_files.as_ref());
    show_external_drop_feedback(ctx, state.drag_drop.external_drop_target.as_ref());
}

fn show_external_drop_feedback(ctx: &egui::Context, target: Option<&ExternalDropTarget>) {
    let hovered_count = ctx.input(|input| input.raw.hovered_files.len());
    let (Some(target), Some(pointer)) = (target, ctx.pointer_hover_pos()) else {
        return;
    };
    if hovered_count == 0 {
        return;
    }
    let label = match hovered_count {
        1 => format!("Copy to {}", target.dest.display()),
        count => format!("Copy {count} items to {}", target.dest.display()),
    };
    egui::Area::new(egui::Id::new("external_drop_feedback"))
        .fixed_pos(pointer + egui::vec2(14.0, 14.0))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(label);
            });
        });
}

fn show_command_bar(ctx: &egui::Context, state: &mut AppState, actions: &mut Vec<Action>) {
    egui::TopBottomPanel::top("command_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Profile:");
            egui::ComboBox::from_id_salt("command_bar_profile")
                .selected_text(&state.profiles.active().name)
                .show_ui(ui, |ui| {
                    for (idx, profile) in state.profiles.profiles.iter().enumerate() {
                        let selected = idx == state.profiles.active_idx;
                        if ui.selectable_label(selected, &profile.name).clicked() && !selected {
                            actions.push(Action::SwitchProfile(idx));
                        }
                    }
                });
            ui.separator();
            if ui.button("Drop Shelf").clicked() {
                actions.push(Action::ToggleDropShelf);
            }
            ui.separator();
            if ui.button("Analyzer").clicked() {
                actions.push(Action::ToggleAnalyzer);
            }
            ui.separator();
            if ui
                .button(format!("{}  Extensions", egui_phosphor::regular::PUZZLE_PIECE))
                .clicked()
            {
                actions.push(Action::Ext(crate::app::ext::ExtAction::OpenManager(true)));
            }
            #[cfg(feature = "cloud")]
            if state.ext.registry.is_enabled("cloud") {
                ui.separator();
                if ui
                    .button(format!("{}  Connect", egui_phosphor::regular::CLOUD))
                    .on_hover_text("Mount a cloud / network drive")
                    .clicked()
                {
                    actions.push(Action::Cloud(
                        crate::app::cloud::CloudAction::ShowAddDialog(true),
                    ));
                }
            }

            let search_width = 320.0_f32.min(ui.available_width());
            let left_space = ((ui.available_width() - search_width) * 0.5).max(8.0);
            ui.add_space(left_space);
            let search = ui
                .scope(|ui| {
                    if state.overlay.show_goto_bar {
                        ui.set_opacity(0.0);
                    }
                    ui.add_sized(
                        [search_width, 24.0],
                        egui::Button::new(
                            egui::RichText::new(format!(
                                "{}  Search files and folders...",
                                egui_phosphor::regular::MAGNIFYING_GLASS
                            ))
                            .size(12.0)
                            .family(crate::util::fonts::regular()),
                        )
                        .right_text(
                            egui::RichText::new("Ctrl K")
                                .size(10.0)
                                .family(crate::util::fonts::mono())
                                .weak(),
                        )
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::new(
                            1.0,
                            ui.visuals()
                                .widgets
                                .noninteractive
                                .bg_stroke
                                .color
                                .gamma_multiply(0.8),
                        ))
                        .corner_radius(5),
                    )
                })
                .inner;
            state.overlay.goto_state.origin_rect = Some(search.rect);
            if search.clicked() {
                actions.push(Action::OpenGotoBar);
            }
        });
    });
}

fn update_filter_from_keyboard(ui: &egui::Ui, tab_state: &mut TabState) {
    let mut changed = false;
    ui.input(|input| {
        for event in &input.events {
            if let egui::Event::Text(text) = event {
                if !input.modifiers.ctrl
                    && !input.modifiers.alt
                    && !text.chars().any(char::is_control)
                {
                    tab_state.filter.push_str(text);
                    changed = true;
                }
            }
        }
        if input.key_pressed(egui::Key::Backspace) {
            tab_state.filter.pop();
            changed = true;
        }
        if input.key_pressed(egui::Key::Escape) {
            tab_state.filter.clear();
            changed = true;
        }
    });
    if changed {
        // Keep the selection of items that still pass the filter; only nudge the
        // keyboard cursor back into view if it fell out. Don't wipe the selection.
        let visible: Vec<usize> = tab_state.visible_indices().to_vec();
        tab_state
            .selection
            .selected
            .retain(|idx| visible.contains(idx));
        if tab_state
            .selection
            .keyboard_cursor
            .is_some_and(|idx| !visible.contains(&idx))
        {
            tab_state.selection.keyboard_cursor = visible.first().copied();
            tab_state.selection.selection_anchor = tab_state.selection.keyboard_cursor;
        }
    }
}

fn handle_file_navigation_keyboard(
    ui: &egui::Ui,
    tile_id: TileId,
    view_mode: &ViewMode,
    tab_state: &mut TabState,
    actions: &mut Vec<Action>,
) {
    let events = ui.input(|input| input.events.clone());
    for event in events {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            continue;
        };
        if modifiers.ctrl || modifiers.alt || modifiers.command {
            continue;
        }
        let visible: Vec<usize> = tab_state.visible_indices().to_vec();
        if key == egui::Key::Enter {
            if let Some(idx) = tab_state
                .selection
                .keyboard_cursor
                .or_else(|| tab_state.selection.selected.iter().next().copied())
            {
                actions.push(Action::Activate { tile: tile_id, idx });
            }
            continue;
        }
        if visible.is_empty() {
            continue;
        }

        let current_pos = tab_state
            .selection
            .keyboard_cursor
            .and_then(|idx| visible.iter().position(|visible_idx| *visible_idx == idx))
            .unwrap_or(0);
        let multi_column = matches!(view_mode, ViewMode::Grid | ViewMode::List);
        let columns = if multi_column {
            tab_state.grid_columns.max(1)
        } else {
            1
        };
        let next_pos = match key {
            egui::Key::Home => 0,
            egui::Key::End => visible.len() - 1,
            egui::Key::ArrowUp => current_pos.saturating_sub(columns),
            egui::Key::ArrowDown => (current_pos + columns).min(visible.len() - 1),
            egui::Key::ArrowLeft if multi_column => current_pos.saturating_sub(1),
            egui::Key::ArrowRight if multi_column => (current_pos + 1).min(visible.len() - 1),
            _ => continue,
        };
        let next_idx = visible[next_pos];
        let anchor = tab_state.selection.selection_anchor.unwrap_or(next_idx);
        tab_state.selection.keyboard_cursor = Some(next_idx);
        tab_state.selection.last_clicked_index = Some(next_idx);
        tab_state.selection.scroll_to_selected = true;
        if modifiers.shift {
            let anchor_pos = visible
                .iter()
                .position(|visible_idx| *visible_idx == anchor)
                .unwrap_or(next_pos);
            let lo = anchor_pos.min(next_pos);
            let hi = anchor_pos.max(next_pos);
            tab_state.selection.selected.clear();
            tab_state
                .selection
                .selected
                .extend(visible[lo..=hi].iter().copied());
        } else {
            tab_state.selection.selection_anchor = Some(next_idx);
            tab_state.selection.selected.clear();
            tab_state.selection.selected.insert(next_idx);
        }
    }
}

/// Keyboard navigation for the Miller (columns) view: up/down within the focused
/// column, left/right between columns, Enter to open.
fn handle_miller_keyboard(
    ui: &egui::Ui,
    tile_id: TileId,
    tab_state: &TabState,
    actions: &mut Vec<Action>,
) {
    let focused = tab_state.miller.focused;
    let col = tab_state.miller.columns.get(focused);
    let len = col.map(|c| c.entries.len()).unwrap_or(0);
    let sel = col.and_then(|c| c.selected);
    let sel_is_dir = col
        .zip(sel)
        .and_then(|(c, s)| c.entries.get(s))
        .is_some_and(|e| e.kind == FileKind::Dir);
    let has_child = tab_state.miller.columns.len() > focused + 1;

    let events = ui.input(|input| input.events.clone());
    for event in events {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            continue;
        };
        if modifiers.ctrl || modifiers.alt || modifiers.command {
            continue;
        }
        let select = |idx: usize| Action::MillerSelect {
            tile: tile_id,
            column: focused,
            idx,
        };
        match key {
            egui::Key::ArrowDown if len > 0 => {
                actions.push(select(sel.map(|s| (s + 1).min(len - 1)).unwrap_or(0)));
            }
            egui::Key::ArrowUp if len > 0 => {
                actions.push(select(sel.map(|s| s.saturating_sub(1)).unwrap_or(0)));
            }
            egui::Key::Home if len > 0 => actions.push(select(0)),
            egui::Key::End if len > 0 => actions.push(select(len - 1)),
            egui::Key::ArrowRight if sel_is_dir && has_child => {
                actions.push(Action::MillerFocusColumn {
                    tile: tile_id,
                    column: focused + 1,
                });
            }
            egui::Key::ArrowLeft if focused > 0 => {
                actions.push(Action::MillerFocusColumn {
                    tile: tile_id,
                    column: focused - 1,
                });
            }
            egui::Key::Enter => {
                if let Some(s) = sel {
                    actions.push(Action::MillerActivate {
                        tile: tile_id,
                        column: focused,
                        idx: s,
                    });
                }
            }
            _ => {}
        }
    }
}

/// Ctrl+scroll cycles through the four views (Details ⇄ List ⇄ Icons ⇄ Columns)
/// and, while in Icons, resizes the thumbnails before crossing into the next view.
fn update_view_zoom(ui: &egui::Ui, pane: &mut TabData) {
    const MIN_THUMB: f32 = 80.0;
    const MAX_THUMB: f32 = 256.0;
    let scroll = ui.input(|input| {
        if input.modifiers.ctrl {
            input.raw_scroll_delta.y
        } else {
            0.0
        }
    });
    if scroll.abs() < 0.01 {
        return;
    }
    if scroll > 0.0 {
        match pane.view_mode {
            ViewMode::Details => pane.view_mode = ViewMode::List,
            ViewMode::List => pane.view_mode = ViewMode::Grid,
            ViewMode::Grid if pane.thumbnail_size < MAX_THUMB => {
                pane.thumbnail_size = (pane.thumbnail_size + 4.0).min(MAX_THUMB);
            }
            ViewMode::Grid => pane.view_mode = ViewMode::Miller,
            ViewMode::Miller => {}
        }
    } else {
        match pane.view_mode {
            ViewMode::Miller => pane.view_mode = ViewMode::Grid,
            ViewMode::Grid if pane.thumbnail_size > MIN_THUMB => {
                pane.thumbnail_size = (pane.thumbnail_size - 4.0).max(MIN_THUMB);
            }
            ViewMode::Grid => pane.view_mode = ViewMode::List,
            ViewMode::List => pane.view_mode = ViewMode::Details,
            ViewMode::Details => {}
        }
    }
}

fn show_drag_feedback(ctx: &egui::Context, dragged_files: Option<&DraggedFiles>) {
    let (Some(drag), Some(pointer)) = (dragged_files, ctx.pointer_interact_pos()) else {
        return;
    };
    let label = match drag.paths.len() {
        1 => drag.paths[0]
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "1 item".to_string()),
        count => format!("{count} items"),
    };
    egui::Area::new(egui::Id::new("dragged_files_feedback"))
        .fixed_pos(pointer + egui::vec2(14.0, 14.0))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(label);
            });
        });
}
