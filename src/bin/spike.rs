//! Phase 1 viability spike for the egui → gpui migration.
//!
//! Goal (the Windows gate from the gpui migration plan): prove a gpui window opens and
//! receives input on Windows *and* Linux before investing in the full port.
//!
//! Run:  cargo run --features spike --bin spike
//!
//! Pure-gpui counter button (window + click input) plus a `gpui_component::init`
//! call so the widget lib is forced to compile/link against this pinned rev.

use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context, Window, WindowBounds,
    WindowOptions,
};

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
    Application::new().run(|cx: &mut App| {
        // Force the widget lib to link; also seeds its theme/assets.
        gpui_component::init(cx);

        let bounds = Bounds::centered(None, size(px(480.), px(320.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_window, cx| cx.new(|_cx| Spike { count: 0 }),
        )
        .unwrap();
        cx.activate(true);
    });
}
