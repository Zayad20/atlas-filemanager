use crate::app::App;
use atlas_core::util::display::format_size;
use atlas_model::pane::TabId;
use gpui::prelude::*;
use gpui::{div, hsla, px, relative, AnyElement, Context, MouseButton, SharedString, Window};
use gpui_component::{h_flex, v_flex, ActiveTheme};

const MAX_BUBBLES: usize = 72;

impl App {
    pub(crate) fn render_space_lens(
        &mut self,
        pane_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(lens) = self.space_lens.get_mut(&pane_id) else {
            return div().size_full().into_any_element();
        };
        lens.update();
        if lens.loading || !self.state.settings.disable_animations {
            // active-pane RAF for bubble drift; throttle later if idle CPU matters.
            window.request_animation_frame();
        }

        let active_path = lens
            .active_path
            .as_ref()
            .map_or_else(String::new, |p| p.display().to_string());
        let mut items = lens.items.clone();
        items.truncate(MAX_BUBBLES);
        let max_size = items.first().map_or(1, |item| item.size).max(1);
        let total_size = lens.items.iter().map(|item| item.size).sum::<u64>();
        let elapsed = self.startup_started.elapsed().as_secs_f32();

        let fg = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().accent;
        let border = cx.theme().border;
        let bg = cx.theme().secondary.opacity(0.22);
        let app = cx.entity();

        let mut cloud = div()
            .relative()
            .flex_1()
            .min_h(px(0.))
            .overflow_hidden()
            .rounded(px(10.))
            .bg(bg);

        for (ix, item) in items.into_iter().enumerate() {
            let size_ratio = (item.size as f32 / max_size as f32).sqrt();
            let radius = 34.0 + size_ratio * 72.0;
            let ring = (ix as f32 * 2.399963 + elapsed * 0.12) % std::f32::consts::TAU;
            let orbit = 0.18 + ((ix % 9) as f32 / 10.5);
            let wobble = (elapsed * 0.9 + ix as f32).sin() * 1.4;
            let left = 0.50 + ring.cos() * orbit * 0.72;
            let top = 0.50 + ring.sin() * orbit * 0.55;
            let color = bubble_color(ix, accent);
            let path = item.path.clone();
            let label = item.name.clone();
            let size = format_size(item.size);
            let is_dir = item.is_dir;
            let click_app = app.clone();

            cloud = cloud.child(
                div()
                    .absolute()
                    .left(relative(left.clamp(0.05, 0.88)))
                    .top(relative(top.clamp(0.08, 0.82)))
                    .size(px(radius))
                    .ml(px(-(radius / 2.0)))
                    .mt(px(-(radius / 2.0) + wobble))
                    .rounded_full()
                    .border_1()
                    .border_color(color.opacity(0.72))
                    .bg(color.opacity(0.22))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(3.))
                    .text_center()
                    .text_color(fg)
                    .cursor_pointer()
                    .hover(move |s| s.bg(color.opacity(0.34)).border_color(color))
                    .child(
                        div()
                            .max_w(relative(0.78))
                            .text_xs()
                            .truncate()
                            .child(SharedString::from(label)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(SharedString::from(size)),
                    )
                    .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                        if !is_dir {
                            return;
                        }
                        click_app.update(cx, |this, cx| {
                            if let Some(lens) = this.space_lens.get_mut(&pane_id) {
                                lens.open_for_path(path.clone());
                            }
                            cx.notify();
                        });
                    }),
            );
        }

        v_flex()
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .p_3()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_3()
                    .child(
                        v_flex()
                            .min_w(px(0.))
                            .flex_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(SharedString::from(active_path)),
                            )
                            .child(div().text_xs().text_color(muted).child(status_label(
                                lens.loading,
                                lens.scanned_entries,
                                total_size,
                                lens.error_count,
                            ))),
                    )
                    .child(
                        div()
                            .px_2()
                            .py(px(4.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(border)
                            .text_xs()
                            .text_color(muted)
                            .child(format!("top {}", lens.items.len().min(MAX_BUBBLES))),
                    ),
            )
            .child(cloud)
            .into_any_element()
    }
}

fn status_label(loading: bool, entries: u64, total_size: u64, errors: u64) -> String {
    let state = if loading { "Scanning" } else { "Ready" };
    let mut text = format!("{state} - {entries} entries - {}", format_size(total_size));
    if errors > 0 {
        text.push_str(&format!(" - {errors} errors"));
    }
    text
}

fn bubble_color(ix: usize, accent: gpui::Hsla) -> gpui::Hsla {
    let palette = [
        hsla(0.88, 0.86, 0.66, 1.0),
        hsla(0.55, 0.72, 0.58, 1.0),
        hsla(0.43, 0.58, 0.54, 1.0),
        hsla(0.12, 0.78, 0.58, 1.0),
        hsla(0.72, 0.62, 0.64, 1.0),
        accent,
    ];
    palette[ix % palette.len()]
}
