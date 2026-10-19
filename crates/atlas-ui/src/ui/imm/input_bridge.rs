//! Native text/IME transport for the upstream IMM input. No widget or text mirror.
use crate::app::App;
use gpui::{Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window};
use std::ops::Range;

impl EntityInputHandler for App {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let input = &self.ctx_menu.as_ref()?.input;
        let range = input.byte_range(range);
        *adjusted = Some(input.utf16_offset(range.start)..input.utf16_offset(range.end));
        Some(input.text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if self.command_shortcut_capture.is_some() {
            return None;
        }
        let input = &self.ctx_menu.as_ref()?.input;
        let (start, end) = input.selection_range();
        Some(UTF16Selection {
            range: input.utf16_offset(start)..input.utf16_offset(end),
            reversed: input.cursor < input.anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let input = &self.ctx_menu.as_ref()?.input;
        let range = input.marked.as_ref()?;
        Some(input.utf16_offset(range.start)..input.utf16_offset(range.end))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.ctx_menu.as_mut() {
            menu.input.marked = None;
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.command_shortcut_capture.is_some() {
            return;
        }
        if let Some(menu) = self.ctx_menu.as_mut() {
            menu.input.replace_native(range, text, None, false);
            menu.selected = 0;
            menu.scroll_to_selected = true;
            menu.scroll_top = 0.0;
            menu.scroll_motion =
                crate::ui::imm::motion::RetainedSpring::new(0.0, std::time::Instant::now());
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.command_shortcut_capture.is_some() {
            return;
        }
        if let Some(menu) = self.ctx_menu.as_mut() {
            menu.input.replace_native(range, text, selected, true);
            menu.selected = 0;
            menu.scroll_to_selected = true;
            menu.scroll_top = 0.0;
            menu.scroll_motion =
                crate::ui::imm::motion::RetainedSpring::new(0.0, std::time::Instant::now());
            cx.notify();
        }
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _element: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let menu = self.ctx_menu.as_ref()?;
        let bounds = menu.input_bounds?;
        let line = menu.input_line.as_ref()?;
        let range = menu.input.byte_range(range);
        let start = line.x_for_index(range.start) - gpui::px(menu.input_scroll_x);
        let end = line.x_for_index(range.end) - gpui::px(menu.input_scroll_x);
        Some(Bounds {
            origin: gpui::point(
                (bounds.left() + start).clamp(bounds.left(), bounds.right()),
                bounds.top(),
            ),
            size: gpui::size(
                (end - start).max(gpui::px(1.0)).min(bounds.size.width),
                bounds.size.height,
            ),
        })
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let menu = self.ctx_menu.as_ref()?;
        let bounds = menu.input_bounds?;
        let line = menu.input_line.as_ref()?;
        let byte =
            line.closest_index_for_x(point.x - bounds.left() + gpui::px(menu.input_scroll_x));
        Some(menu.input.utf16_offset(byte))
    }

    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(menu) = self.ctx_menu.as_mut() {
            let range = menu.input.byte_range(range);
            menu.input.anchor = range.start;
            menu.input.cursor = range.end;
            cx.notify();
        }
    }

    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.ctx_menu.as_ref()?.input.text.encode_utf16().count())
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        self.ctx_menu.is_some() && self.command_shortcut_capture.is_none()
    }
}
