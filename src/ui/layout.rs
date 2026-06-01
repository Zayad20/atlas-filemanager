use egui::{Id, Sense, TextStyle, Vec2};
use egui_tiles::{Behavior, SimplificationOptions, TileId, Tiles, UiResponse};
use std::collections::HashMap;

use crate::fs::preview::PreviewResult;
use crate::input::action::Action;
use crate::state::app_state::AppState;
use crate::state::app_state::DraggedFiles;
use crate::state::app_state::ExternalDropTarget;
use crate::state::app_state::{DragHoverState, DragHoverTarget};
use crate::state::pane::TabData;
use crate::state::settings::{InspectorPlacement, Settings, ViewMode};
use crate::state::tab::TabState;
use crate::ui::breadcrumb::show_breadcrumb;
use crate::ui::file_list::show_file_list;

pub struct AtlasBehavior<'a> {
    pub tab_states: &'a mut HashMap<TileId, TabState>,
    #[allow(dead_code)]
    pub settings: &'a mut Settings,
    pub actions: &'a mut Vec<Action>,
    pub focused_tile: &'a mut TileId,
    pub dragged_files: &'a mut Option<DraggedFiles>,
    pub drag_hover: &'a mut DragHoverState,
    pub external_drop_target: &'a mut Option<ExternalDropTarget>,
    pub preview_results: &'a HashMap<TileId, PreviewResult>,
    pub preview_textures: &'a HashMap<TileId, egui::TextureHandle>,
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
                    if ui
                        .selectable_label(pane.view_mode == ViewMode::Grid, "Grid")
                        .clicked()
                    {
                        self.actions.push(Action::SetViewMode(ViewMode::Grid));
                    }
                    if ui
                        .selectable_label(pane.view_mode == ViewMode::Details, "List")
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
                handle_file_navigation_keyboard(
                    ui,
                    tile_id,
                    &pane.view_mode,
                    tab_state,
                    self.actions,
                );
                update_filter_from_keyboard(ui, tab_state);
            }
            let show_local_inspector = pane.preview_enabled
                && self.settings.inspector_placement == InspectorPlacement::ActiveTab
                && *self.focused_tile == tile_id
                && !tab_state.selected.is_empty();
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
                        self.preview_results.get(&tile_id),
                        self.preview_textures.get(&tile_id),
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
        let name = pane
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());
        egui::RichText::new(name).into()
    }

    fn is_tab_closable(&self, _tiles: &Tiles<TabData>, _tile_id: TileId) -> bool {
        true
    }

    fn on_tab_close(&mut self, _tiles: &mut Tiles<TabData>, tile_id: TileId) -> bool {
        self.actions.push(Action::CloseTab(tile_id));
        false
    }

    fn tab_ui(
        &mut self,
        tiles: &mut Tiles<TabData>,
        ui: &mut egui::Ui,
        id: Id,
        tile_id: TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Response {
        let text = self.tab_title_for_tile(tiles, tile_id);
        let close_btn_size = Vec2::splat(self.close_button_outer_size());
        let close_btn_left_padding = 4.0;
        let font_id = TextStyle::Button.resolve(ui.style());
        let galley = text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, font_id);
        let x_margin = self.tab_title_spacing(ui.visuals());
        let button_width = galley.size().x
            + 2.0 * x_margin
            + f32::from(state.closable) * (close_btn_left_padding + close_btn_size.x);
        let (_, tab_rect) = ui.allocate_space(egui::vec2(button_width, ui.available_height()));
        let tab_response = ui
            .interact(tab_rect, id, Sense::click_and_drag())
            .on_hover_cursor(self.tab_hover_cursor_icon());

        if ui.is_rect_visible(tab_rect) && !state.is_being_dragged {
            let bg_color = self.tab_bg_color(ui.visuals(), tiles, tile_id, state);
            let stroke = self.tab_outline_stroke(ui.visuals(), tiles, tile_id, state);
            ui.painter().rect(
                tab_rect.shrink(0.5),
                0.0,
                bg_color,
                stroke,
                egui::StrokeKind::Inside,
            );
            if state.active {
                ui.painter().hline(
                    tab_rect.x_range(),
                    tab_rect.bottom(),
                    egui::Stroke::new(stroke.width + 1.0, bg_color),
                );
            }

            let text_color = self.tab_text_color(ui.visuals(), tiles, tile_id, state);
            let text_position = egui::Align2::LEFT_CENTER
                .align_size_within_rect(galley.size(), tab_rect.shrink(x_margin))
                .min;
            ui.painter().galley(text_position, galley, text_color);

            if state.closable {
                let close_btn_rect = egui::Align2::RIGHT_CENTER
                    .align_size_within_rect(close_btn_size, tab_rect.shrink(x_margin));
                let close_btn_response = ui
                    .interact(
                        close_btn_rect,
                        ui.auto_id_with("tab_close_btn"),
                        Sense::click_and_drag(),
                    )
                    .on_hover_cursor(self.tab_hover_cursor_icon());
                let visuals = ui.style().interact(&close_btn_response);
                let rect = close_btn_rect
                    .shrink(self.close_button_inner_margin())
                    .expand(visuals.expansion);
                let stroke = visuals.fg_stroke;
                ui.painter()
                    .line_segment([rect.left_top(), rect.right_bottom()], stroke);
                ui.painter()
                    .line_segment([rect.right_top(), rect.left_bottom()], stroke);

                // egui_tiles normally reserves the close button exclusively for clicks.
                // Keep close-on-click while allowing a tab drag to start over the x.
                if close_btn_response.drag_started() {
                    ui.ctx().set_dragged_id(id);
                }
                if close_btn_response.clicked() && self.on_tab_close(tiles, tile_id) {
                    tiles.remove(tile_id);
                }
            }
        }

        self.on_tab_button(tiles, tile_id, tab_response)
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
        if ui.small_button("+").clicked() {
            self.actions.push(Action::NewTab(tile_id));
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

    fn dragged_overlay_color(&self, visuals: &egui::Visuals) -> egui::Color32 {
        visuals.selection.stroke.color.gamma_multiply(0.12)
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }
}

fn paint_drag_hover_confirmation(
    response: &egui::Response,
    drag_hover: &DragHoverState,
    target: &DragHoverTarget,
) {
    let now = response.ctx.input(|input| input.time);
    if drag_hover.confirmation_visible(target, now) {
        response.ctx.debug_painter().rect_stroke(
            response.rect.shrink(1.0),
            4.0,
            egui::Stroke::new(2.0, response.ctx.style().visuals.selection.stroke.color),
            egui::StrokeKind::Inside,
        );
    }
}

pub fn show_layout(ctx: &egui::Context, state: &mut AppState, actions: &mut Vec<Action>) {
    state.drag_hover.observed_target = None;
    if !ctx.input(|input| input.raw.hovered_files.is_empty()) {
        state.external_drop_target = None;
    }
    // Tabs popup — move out to avoid borrow conflict on state.tabs_popup
    {
        let mut popup = std::mem::take(&mut state.tabs_popup);
        crate::ui::tabs_popup::show_tabs_popup(ctx, state, &mut popup, actions);
        state.tabs_popup = popup;
    }

    // Overlays (Foreground — render before panels so they sit on top)
    if state.show_goto_bar {
        crate::ui::goto_bar::show_goto_bar(
            ctx,
            &mut state.goto_state,
            &state.nucleo_results,
            state.focused_tile,
            actions,
        );
    }
    if state.show_palette {
        let mut palette_state = std::mem::take(&mut state.palette_state);
        crate::ui::command_palette::show_command_palette(ctx, state, &mut palette_state, actions);
        state.palette_state = palette_state;
    }

    // Status bar (bottom) — must be added before CentralPanel
    crate::ui::status_bar::show_status_bar(ctx, state);
    show_command_bar(ctx, state, actions);

    // Sidebar (left) — animated slide in/out
    {
        let t = ctx
            .animate_bool_responsive(egui::Id::new("sidebar_slide"), state.settings.show_sidebar);
        let animating = t > 0.001 && t < 0.999;
        if t > 0.001 {
            let w = state.settings.sidebar_width * t;
            let mut panel = egui::SidePanel::left("sidebar");
            if animating {
                panel = panel.exact_width(w).resizable(false);
            } else {
                panel = panel
                    .default_width(state.settings.sidebar_width)
                    .resizable(true);
            }
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
        crate::ui::inspector::show_inspector(ctx, state);
    }

    // Settings window (floating)
    if state.show_settings {
        crate::ui::settings_window::show_settings(
            ctx,
            &mut state.settings,
            &mut state.show_settings,
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

    // Central pane tree
    egui::CentralPanel::default()
        .frame(egui::Frame::central_panel(&ctx.style()).fill(ctx.style().visuals.window_fill))
        .show(ctx, |ui| {
            let mut behavior = AtlasBehavior {
                tab_states: &mut state.tab_states,
                settings: &mut state.settings,
                actions,
                focused_tile: &mut state.focused_tile,
                dragged_files: &mut state.dragged_files,
                drag_hover: &mut state.drag_hover,
                external_drop_target: &mut state.external_drop_target,
                preview_results: &state.preview_results,
                preview_textures: &state.preview_textures,
            };
            state.pane_tree.ui(&mut behavior, ui);
        });

    show_drag_feedback(ctx, state.dragged_files.as_ref());
    show_external_drop_feedback(ctx, state.external_drop_target.as_ref());
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

fn show_command_bar(ctx: &egui::Context, state: &AppState, actions: &mut Vec<Action>) {
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
        let visible: Vec<usize> = tab_state.visible_indices().collect();
        tab_state.selected.retain(|idx| visible.contains(idx));
        if tab_state
            .keyboard_cursor
            .is_some_and(|idx| !visible.contains(&idx))
        {
            tab_state.keyboard_cursor = visible.first().copied();
            tab_state.selection_anchor = tab_state.keyboard_cursor;
            tab_state.selected.clear();
            tab_state.selected.extend(tab_state.keyboard_cursor);
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
        let visible: Vec<usize> = tab_state.visible_indices().collect();
        if key == egui::Key::Enter {
            if let Some(idx) = tab_state
                .keyboard_cursor
                .or_else(|| tab_state.selected.iter().next().copied())
            {
                actions.push(Action::Activate { tile: tile_id, idx });
            }
            continue;
        }
        if visible.is_empty() {
            continue;
        }

        let current_pos = tab_state
            .keyboard_cursor
            .and_then(|idx| visible.iter().position(|visible_idx| *visible_idx == idx))
            .unwrap_or(0);
        let columns = match view_mode {
            ViewMode::Grid => tab_state.grid_columns.max(1),
            ViewMode::Details => 1,
        };
        let next_pos = match key {
            egui::Key::Home => 0,
            egui::Key::End => visible.len() - 1,
            egui::Key::ArrowUp => current_pos.saturating_sub(columns),
            egui::Key::ArrowDown => (current_pos + columns).min(visible.len() - 1),
            egui::Key::ArrowLeft if *view_mode == ViewMode::Grid => current_pos.saturating_sub(1),
            egui::Key::ArrowRight if *view_mode == ViewMode::Grid => {
                (current_pos + 1).min(visible.len() - 1)
            }
            _ => continue,
        };
        let next_idx = visible[next_pos];
        let anchor = tab_state.selection_anchor.unwrap_or(next_idx);
        tab_state.keyboard_cursor = Some(next_idx);
        tab_state.last_clicked_index = Some(next_idx);
        tab_state.scroll_to_selected = true;
        if modifiers.shift {
            let anchor_pos = visible
                .iter()
                .position(|visible_idx| *visible_idx == anchor)
                .unwrap_or(next_pos);
            let lo = anchor_pos.min(next_pos);
            let hi = anchor_pos.max(next_pos);
            tab_state.selected.clear();
            tab_state.selected.extend(visible[lo..=hi].iter().copied());
        } else {
            tab_state.selection_anchor = Some(next_idx);
            tab_state.selected.clear();
            tab_state.selected.insert(next_idx);
        }
    }
}

fn update_view_zoom(ui: &egui::Ui, pane: &mut TabData) {
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
        if pane.view_mode == ViewMode::Details {
            pane.view_mode = ViewMode::Grid;
        } else {
            pane.thumbnail_size = (pane.thumbnail_size + 4.0).min(256.0);
        }
    } else if pane.view_mode == ViewMode::Grid && pane.thumbnail_size > 80.0 {
        pane.thumbnail_size = (pane.thumbnail_size - 4.0).max(80.0);
    } else {
        pane.view_mode = ViewMode::Details;
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
