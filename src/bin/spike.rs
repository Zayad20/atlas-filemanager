//! Phase 1 viability spike for the egui → gpui migration.
//!
//! Goal (the Windows gate from the gpui migration plan): prove a gpui window opens and
//! receives input on Windows *and* Linux, using the **official gpui-component
//! bootstrap** for the pinned rev so the real shell (Fase 2) builds on the same
//! pattern:
//!   * `gpui_platform::application()` (not `Application::new()` — platform
//!     construction moved into the `gpui_platform` crate in this rev),
//!   * `with_assets(gpui_component_assets::Assets)` for the default icon pack,
//!   * `gpui_component::init(cx)` to seed the theme/components,
//!   * the top-level window view wrapped in `gpui_component::Root` (required by
//!     gpui-component so overlays/modals/notifications have a mount point).
//!
//! Run:  cargo run --features spike --bin spike

use gpui::*;
use gpui_component::Root;

struct Spike {
    count: usize,
}

impl Render for Spike {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .text_color(rgb(0xffffff))
            .justify_center()
            .items_center()
            .child(format!("gpui spike — clicks: {}", self.count))
            .child(
                div()
                    .id("btn")
                    .px_4()
                    .py_2()
                    .bg(rgb(0x4444ff))
                    .rounded_md()
                    .cursor_pointer()
                    .child("Click me")
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        this.count += 1;
                        cx.notify();
                    })),
            )
    }
}

fn main() {
    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);

    app.run(move |cx: &mut App| {
        gpui_component::init(cx);

        let bounds = Bounds::centered(None, size(px(480.), px(320.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|_| Spike { count: 0 });
                // The first level on the window must be `Root`.
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}
