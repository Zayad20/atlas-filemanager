//! Root gpui view: the window shell (command bar · sidebar · central · status bar).
//!
//! Fase 2 skeleton — renders the real focused-tab listing to prove the async data
//! flow (tokio → `BgResult` → drain → state → render) and basic click interaction.
//! Real file views, sidebar, inspector and overlays land in later phases.

use crate::app::App;
use crate::input::action::Action;
use crate::state::file_entry::FileKind;
use gpui::prelude::*;
use gpui::{div, px, Context, Window};
use gpui_component::{h_flex, v_flex, ActiveTheme};

impl Render for App {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Copy out the theme colors (Hsla is Copy) so we don't hold a borrow of `cx`
        // while building elements / installing click listeners.
        let (bg, fg, muted, title_bar, sidebar, list_hover) = {
            let t = cx.theme();
            (
                t.background,
                t.foreground,
                t.muted_foreground,
                t.title_bar,
                t.sidebar,
                t.list_active,
            )
        };

        let focused = self.state.focused_tile;
        let path = self
            .state
            .active_tab_data()
            .map(|t| t.path.display().to_string())
            .unwrap_or_default();
        let rows: Vec<(usize, String, bool)> = self
            .state
            .active_tab_state()
            .map(|ts| {
                ts.entries
                    .iter()
                    .enumerate()
                    .map(|(i, e)| (i, e.name.clone(), matches!(e.kind, FileKind::Dir)))
                    .collect()
            })
            .unwrap_or_default();

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .text_sm()
            // Command bar
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .bg(title_bar)
                    .child("Atlas")
                    .child(div().text_color(muted).child(path)),
            )
            // Body: sidebar + central listing
            .child(
                h_flex()
                    .flex_1()
                    .child(
                        div()
                            .w(px(220.))
                            .h_full()
                            .bg(sidebar)
                            .p_2()
                            .child("Sidebar"),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .p_2()
                            .gap_px()
                            .children(rows.into_iter().map(|(i, name, is_dir)| {
                                div()
                                    .id(i)
                                    .w_full()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(list_hover))
                                    .child(format!("{}  {name}", if is_dir { "📁" } else { "📄" }))
                                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                                        this.process_actions(vec![Action::Activate {
                                            tile: focused,
                                            idx: i,
                                        }]);
                                        cx.notify();
                                    }))
                            })),
                    ),
            )
            // Status bar
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1()
                    .bg(title_bar)
                    .text_xs()
                    .text_color(muted)
                    .child("listo"),
            )
    }
}
