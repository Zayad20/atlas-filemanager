use crate::input::action::Action;

const PANEL_WIDTH: f32 = 620.0;
const RESULTS_HEIGHT: f32 = 320.0;
const RESULT_ROW_HEIGHT: f32 = 30.0;
const OPEN_ANIMATION_SECS: f32 = 0.55;

#[derive(Default)]
pub struct FuzzyOverlayState {
    pub query: String,
    pub selected_idx: usize,
    pub scroll_to_selected: bool,
    /// Search bar rect used as the visual origin for the open tween.
    pub origin_rect: Option<egui::Rect>,
    /// Wall-clock start used because a newly-created `animate_bool` starts at its final value.
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

pub struct FuzzyOverlayOptions<'a> {
    pub placeholder: &'a str,
    pub empty_group: &'a str,
    pub item_kind: &'a str,
    pub item_icon: &'a str,
    pub footer_action: &'a str,
    pub shortcut: &'a str,
}

/// Generic fuzzy overlay panel: shows a search box + scrollable result list.
/// `on_select` converts the selected string item to an Action.
/// `on_close` is the Action pushed when Escape/click-outside closes it.
pub fn show_fuzzy_overlay<T: AsRef<str>>(
    ctx: &egui::Context,
    overlay_id: egui::Id,
    state: &mut FuzzyOverlayState,
    items: &[T],
    options: FuzzyOverlayOptions<'_>,
    close_action: Action,
    on_select: impl Fn(&T) -> Action,
    actions: &mut Vec<Action>,
) {
    let target_pos = egui::pos2(ctx.content_rect().center().x - PANEL_WIDTH * 0.5, 48.0);
    let target_rect =
        egui::Rect::from_min_size(target_pos, egui::vec2(PANEL_WIDTH, RESULT_ROW_HEIGHT));
    let origin_rect = state.origin_rect.unwrap_or(target_rect);
    let elapsed = state
        .opening_started_at
        .get_or_insert_with(std::time::Instant::now)
        .elapsed()
        .as_secs_f32();
    let progress = (elapsed / OPEN_ANIMATION_SECS).clamp(0.0, 1.0);
    if progress < 1.0 {
        ctx.request_repaint();
    }
    let move_t = crate::ui::anim::cubic_out(phase(progress, 0.0, 0.34));
    let expand_t = crate::ui::anim::cubic_out(phase(progress, 0.34, 0.68));
    let reveal = crate::ui::anim::cubic_out(phase(progress, 0.68, 1.0));
    let panel_width = egui::lerp(origin_rect.width()..=PANEL_WIDTH, expand_t);
    let panel_pos = egui::pos2(
        egui::lerp(origin_rect.left()..=target_pos.x, move_t),
        egui::lerp(origin_rect.top()..=target_pos.y, move_t),
    );

    let area = egui::Area::new(overlay_id)
        .fixed_pos(panel_pos)
        .order(egui::Order::Foreground);

    let resp = area.show(ctx, |ui| {
        let panel_fill = if ui.visuals().dark_mode {
            egui::Color32::from_rgba_unmultiplied(25, 18, 22, 244)
        } else {
            egui::Color32::from_rgba_unmultiplied(246, 242, 244, 248)
        };
        let border = ui.visuals().widgets.noninteractive.bg_stroke.color;

        egui::Frame::new()
            .fill(panel_fill)
            .stroke(egui::Stroke::new(1.0, border.gamma_multiply(0.7)))
            .corner_radius(8)
            .shadow(egui::epaint::Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 0,
                color: egui::Color32::from_black_alpha(120),
            })
            .show(ui, |ui| {
                ui.set_min_width(panel_width);
                ui.set_max_width(panel_width);

                // Search input — request focus every frame so it stays active
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(egui_phosphor::regular::MAGNIFYING_GLASS)
                                    .size(16.0)
                                    .family(crate::util::fonts::regular())
                                    .weak(),
                            );
                            let edit = egui::TextEdit::singleline(&mut state.query)
                                .hint_text(options.placeholder)
                                .font(egui::FontId::new(14.0, crate::util::fonts::regular()))
                                .desired_width(ui.available_width() - 78.0)
                                .frame(false);
                            let text_resp = ui.add(edit);
                            text_resp.request_focus();
                            shortcut_badge(ui, options.shortcut);
                        });
                    });

                // Keyboard nav
                ctx.input_mut(|i| {
                    if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                        state.selected_idx =
                            (state.selected_idx + 1).min(items.len().saturating_sub(1));
                        state.scroll_to_selected = true;
                    }
                    if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                        state.selected_idx = state.selected_idx.saturating_sub(1);
                        state.scroll_to_selected = true;
                    }
                    if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                        if let Some(item) = items.get(state.selected_idx) {
                            actions.push(on_select(item));
                            actions.push(close_action.clone());
                        }
                    }
                    if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                        actions.push(close_action.clone());
                    }
                });

                if reveal > 0.0 {
                    ui.set_opacity(reveal);
                    ui.separator();
                    ui.add_space(4.0);
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(10, 0))
                        .show(ui, |ui| {
                            let group = if state.query.is_empty() {
                                options.empty_group
                            } else {
                                "Results"
                            };
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(group)
                                        .size(11.0)
                                        .family(crate::util::fonts::semibold())
                                        .weak(),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new(format!("{} items", items.len()))
                                                .size(11.0)
                                                .family(crate::util::fonts::regular())
                                                .weak(),
                                        );
                                    },
                                );
                            });
                        });

                    // Results list
                    egui::ScrollArea::vertical()
                        .max_height(RESULTS_HEIGHT * reveal)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            for (i, item) in items.iter().enumerate() {
                                let selected = i == state.selected_idx;
                                let row_fill = if selected {
                                    ui.visuals().selection.bg_fill
                                } else {
                                    egui::Color32::TRANSPARENT
                                };
                                let resp = ui.add_sized(
                                    [ui.available_width(), RESULT_ROW_HEIGHT],
                                    egui::Button::new(
                                        egui::RichText::new(format!(
                                            "{}  {}",
                                            options.item_icon,
                                            item.as_ref()
                                        ))
                                        .size(13.0)
                                        .family(crate::util::fonts::medium()),
                                    )
                                    .right_text(
                                        egui::RichText::new(options.item_kind)
                                            .size(11.0)
                                            .family(crate::util::fonts::regular())
                                            .weak(),
                                    )
                                    .fill(row_fill)
                                    .stroke(egui::Stroke::NONE)
                                    .corner_radius(3),
                                );
                                if state.scroll_to_selected && i == state.selected_idx {
                                    resp.scroll_to_me(None);
                                    state.scroll_to_selected = false;
                                }
                                if resp.clicked() {
                                    actions.push(on_select(item));
                                    actions.push(close_action.clone());
                                }
                            }
                        });

                    if items.is_empty() {
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                egui::RichText::new("No results")
                                    .size(12.0)
                                    .family(crate::util::fonts::regular())
                                    .weak(),
                            );
                        });
                    }

                    ui.add_space(4.0);
                    ui.separator();
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(egui_phosphor::regular::PLUS)
                                        .size(14.0)
                                        .family(crate::util::fonts::regular())
                                        .weak(),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        shortcut_badge(ui, "Esc");
                                        ui.label(
                                            egui::RichText::new("Close")
                                                .size(11.0)
                                                .family(crate::util::fonts::regular())
                                                .weak(),
                                        );
                                        shortcut_badge(ui, "Enter");
                                        ui.label(
                                            egui::RichText::new(options.footer_action)
                                                .size(11.0)
                                                .family(crate::util::fonts::regular())
                                                .weak(),
                                        );
                                    },
                                );
                            });
                        });
                }
            });
    });

    // Close on click outside the overlay
    if resp.response.clicked_elsewhere() {
        actions.push(close_action);
    }
}

fn phase(progress: f32, start: f32, end: f32) -> f32 {
    ((progress - start) / (end - start)).clamp(0.0, 1.0)
}

fn shortcut_badge(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
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
        .corner_radius(3)
        .inner_margin(egui::Margin::symmetric(4, 1))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .size(10.0)
                    .family(crate::util::fonts::mono())
                    .weak(),
            );
        });
}
