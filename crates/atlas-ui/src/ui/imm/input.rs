//! Immediate-mode Text Input Subsystem.
//!
//! Immediate-mode text input.
//! Stateless, procedural, handling:
//! - UTF-8 character boundaries.
//! - Word hopping (Ctrl+Left, Ctrl+Right).
//! - Text selection (Shift+Left/Right, Home, End, Ctrl+A).
//! - Clipboard (Ctrl+C, Ctrl+V, Ctrl+X).
//! - Blinking cursor rendering without waking the entire App.

use crate::ui::surface::{paint_fill, paint_rounded, shape_text, text_origin_y, Slot};
use gpui::ColorExt as _;
use gpui::{point, px, size, Bounds, Font, Hsla, Pixels, Window};
use std::ops::Range;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct ImmTextInputState {
    pub text: String,
    pub cursor: usize,
    pub anchor: usize,
    pub is_active: bool,
    pub blink_epoch: Instant,
    pub marked: Option<Range<usize>>,
    undo: Vec<EditSnapshot>,
    redo: Vec<EditSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EditSnapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}

impl Default for ImmTextInputState {
    fn default() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            anchor: 0,
            is_active: false,
            blink_epoch: Instant::now(),
            marked: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
}

impl ImmTextInputState {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let len = text.len();
        Self {
            text,
            cursor: len,
            anchor: len,
            is_active: true,
            blink_epoch: Instant::now(),
            marked: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn activate(&mut self, text: String, select_all: bool) {
        let len = text.len();
        self.text = text;
        self.marked = None;
        self.undo.clear();
        self.redo.clear();
        self.is_active = true;
        self.blink_epoch = Instant::now();
        if select_all {
            self.anchor = 0;
            self.cursor = len;
        } else {
            self.anchor = len;
            self.cursor = len;
        }
    }

    pub fn deactivate(&mut self) {
        self.is_active = false;
    }

    /// Downstream paint needs current ink only, never the editing history.
    pub fn paint_snapshot(&self) -> Self {
        Self {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
            is_active: self.is_active,
            blink_epoch: self.blink_epoch,
            marked: self.marked.clone(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    fn snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }

    fn checkpoint(&mut self) {
        if self.marked.is_some() {
            return;
        }
        let snapshot = self.snapshot();
        if self.undo.last() != Some(&snapshot) {
            if self.undo.len() >= 64 {
                self.undo.remove(0);
            }
            self.undo.push(snapshot);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self, redo: bool) {
        let prior = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        if let Some(prior) = prior {
            let current = self.snapshot();
            if redo {
                self.undo.push(current);
            } else {
                self.redo.push(current);
            }
            self.text = prior.text;
            self.cursor = prior.cursor;
            self.anchor = prior.anchor;
            self.marked = None;
            self.blink_epoch = Instant::now();
        }
    }

    pub fn select_word(&mut self) {
        let at = self.cursor.min(self.text.len());
        let at = if at == self.text.len() {
            self.prev_char(at)
        } else {
            at
        };
        let class = |ch: char| {
            if ch.is_whitespace() {
                0
            } else if ch.is_alphanumeric() || ch == '_' {
                1
            } else {
                2
            }
        };
        let Some(ch) = self.text[at..].chars().next() else {
            return;
        };
        let kind = class(ch);
        let mut start = at;
        let mut end = self.next_char(at);
        if kind != 2 {
            while start > 0 {
                let prev = self.prev_char(start);
                if self.text[prev..start]
                    .chars()
                    .next()
                    .is_none_or(|ch| class(ch) != kind)
                {
                    break;
                }
                start = prev;
            }
            while end < self.text.len() {
                let next = self.next_char(end);
                if self.text[end..next]
                    .chars()
                    .next()
                    .is_none_or(|ch| class(ch) != kind)
                {
                    break;
                }
                end = next;
            }
        }
        self.anchor = start;
        self.cursor = end;
    }

    #[inline]
    pub fn selection_range(&self) -> (usize, usize) {
        let start = self.cursor.min(self.anchor).min(self.text.len());
        let end = self.cursor.max(self.anchor).min(self.text.len());
        (start, end)
    }

    pub fn byte_offset(&self, utf16: usize) -> usize {
        let mut units = 0;
        for (byte, ch) in self.text.char_indices() {
            if units + ch.len_utf16() > utf16 {
                return byte;
            }
            units += ch.len_utf16();
        }
        self.text.len()
    }

    pub fn utf16_offset(&self, byte: usize) -> usize {
        let mut byte = byte.min(self.text.len());
        while !self.text.is_char_boundary(byte) {
            byte -= 1;
        }
        self.text[..byte].encode_utf16().count()
    }

    pub fn byte_range(&self, range: Range<usize>) -> Range<usize> {
        let start = self.byte_offset(range.start);
        start..self.byte_offset(range.end).max(start)
    }

    pub fn replace_native(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        mark: bool,
    ) {
        self.checkpoint();
        let (start, end) = self.selection_range();
        let range = range
            .map(|r| self.byte_range(r))
            .or_else(|| self.marked.clone())
            .unwrap_or(start..end);
        let text: String = text.chars().filter(|ch| !ch.is_control()).collect();
        self.text.replace_range(range.clone(), &text);
        let end = range.start + text.len();
        self.marked = mark.then_some(range.start..end);
        self.cursor = end;
        self.anchor = end;
        if let Some(selected) = selected {
            let inserted = Self::new(text);
            self.anchor = range.start + inserted.byte_offset(selected.start);
            self.cursor = range.start + inserted.byte_offset(selected.end);
        }
        self.blink_epoch = Instant::now();
    }

    #[inline]
    pub fn has_selection(&self) -> bool {
        let (s, e) = self.selection_range();
        s < e
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    pub fn selected_text(&self) -> &str {
        let (s, e) = self.selection_range();
        &self.text[s..e]
    }

    pub fn delete_selection(&mut self) -> bool {
        let (start, end) = self.selection_range();
        if start < end {
            self.checkpoint();
            self.text.drain(start..end);
            self.marked = None;
            self.cursor = start;
            self.anchor = start;
            self.blink_epoch = Instant::now();
            return true;
        }
        false
    }

    pub fn prev_char(&self, idx: usize) -> usize {
        if idx == 0 {
            return 0;
        }
        let mut i = idx.saturating_sub(1);
        while i > 0 && !self.text.is_char_boundary(i) {
            i -= 1;
        }
        i
    }

    pub fn next_char(&self, idx: usize) -> usize {
        let len = self.text.len();
        if idx >= len {
            return len;
        }
        let mut i = idx + 1;
        while i < len && !self.text.is_char_boundary(i) {
            i += 1;
        }
        i.min(len)
    }

    /// Helper to test if a character is a path or whitespace separator
    #[inline]
    fn is_separator(ch: char) -> bool {
        ch.is_whitespace() || ch == '/' || ch == '\\'
    }

    /// Word boundary jump to the left (word-boundary jump).
    pub fn word_left(&self, mut idx: usize) -> usize {
        // Step 1: Skip leading separators immediately left of cursor
        while idx > 0 {
            let prev = self.prev_char(idx);
            let ch = self.text[prev..idx].chars().next().unwrap_or(' ');
            if !Self::is_separator(ch) {
                break;
            }
            idx = prev;
        }
        // Step 2: Skip token characters until previous character is a separator
        while idx > 0 {
            let prev = self.prev_char(idx);
            let ch = self.text[prev..idx].chars().next().unwrap_or(' ');
            if Self::is_separator(ch) {
                break;
            }
            idx = prev;
        }
        idx
    }

    /// Word boundary jump to the right (word-boundary jump).
    pub fn word_right(&self, mut idx: usize) -> usize {
        let len = self.text.len();
        // Step 1: Skip current token characters to the right
        while idx < len {
            let next = self.next_char(idx);
            let ch = self.text[idx..next].chars().next().unwrap_or(' ');
            if Self::is_separator(ch) {
                break;
            }
            idx = next;
        }
        // Step 2: Skip trailing separators to land at the start of next token
        while idx < len {
            let next = self.next_char(idx);
            let ch = self.text[idx..next].chars().next().unwrap_or(' ');
            if !Self::is_separator(ch) {
                break;
            }
            idx = next;
        }
        idx
    }

    pub fn insert_text(&mut self, s: &str) {
        self.checkpoint();
        self.marked = None;
        self.delete_selection();
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.anchor = self.cursor;
        self.blink_epoch = Instant::now();
    }

    /// Handles a keystroke. Returns `true` if state changed.
    pub fn handle_key(
        &mut self,
        key: &str,
        key_char: Option<&str>,
        ctrl: bool,
        shift: bool,
    ) -> bool {
        self.blink_epoch = Instant::now();
        match key {
            "left" => {
                let target = if self.has_selection() && !shift {
                    self.selection_range().0
                } else if ctrl {
                    self.word_left(self.cursor)
                } else {
                    self.prev_char(self.cursor)
                };
                self.cursor = target;
                if !shift {
                    self.anchor = self.cursor;
                }
                true
            }
            "right" => {
                let target = if self.has_selection() && !shift {
                    self.selection_range().1
                } else if ctrl {
                    self.word_right(self.cursor)
                } else {
                    self.next_char(self.cursor)
                };
                self.cursor = target;
                if !shift {
                    self.anchor = self.cursor;
                }
                true
            }
            "home" => {
                self.cursor = 0;
                if !shift {
                    self.anchor = 0;
                }
                true
            }
            "end" => {
                self.cursor = self.text.len();
                if !shift {
                    self.anchor = self.cursor;
                }
                true
            }
            "backspace" => {
                if self.has_selection() {
                    self.delete_selection();
                } else if self.cursor > 0 {
                    self.checkpoint();
                    let prev = if ctrl {
                        self.word_left(self.cursor)
                    } else {
                        self.prev_char(self.cursor)
                    };
                    self.text.drain(prev..self.cursor);
                    self.cursor = prev;
                    self.anchor = prev;
                }
                true
            }
            "delete" => {
                if self.has_selection() {
                    self.delete_selection();
                } else if self.cursor < self.text.len() {
                    self.checkpoint();
                    let next = if ctrl {
                        self.word_right(self.cursor)
                    } else {
                        self.next_char(self.cursor)
                    };
                    self.text.drain(self.cursor..next);
                }
                true
            }
            "a" if ctrl => {
                self.select_all();
                true
            }
            "z" if ctrl => {
                self.undo(shift);
                true
            }
            "y" if ctrl => {
                self.undo(true);
                true
            }
            _ => {
                if let Some(ch) = key_char {
                    if !ctrl {
                        self.insert_text(ch);
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Measures and paints the input box, text selection, and blinking cursor.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &self,
        window: &mut Window,
        cx: &mut gpui::App,
        slot: Slot,
        origin: gpui::Point<Pixels>,
        font: &Font,
        font_size: Pixels,
        fg: Hsla,
        bg: Hsla,
        accent: Hsla,
        border: Hsla,
    ) {
        let b = slot.to_bounds(origin);
        // Paint background container with subtle border
        paint_rounded(window, b, 6.0, bg);
        window.paint_quad(gpui::outline(b, border.opacity(0.85), Default::default()));

        let pad_x = 8.0;
        let text_x = b.origin.x + px(pad_x);
        let text_y = origin.y + px(text_origin_y(slot.y, 1.0));

        // Shape the full line
        let shaped = shape_text(&self.text, font, font_size, fg, window);

        // Selection highlight
        let (sel_start, sel_end) = self.selection_range();
        if sel_start < sel_end {
            let pre_sel = shape_text(&self.text[..sel_start], font, font_size, fg, window);
            let sel_text = shape_text(&self.text[sel_start..sel_end], font, font_size, fg, window);
            let sel_x = f32::from(pre_sel.width());
            let sel_w = f32::from(sel_text.width());
            let sel_bounds = Bounds {
                origin: point(text_x + px(sel_x), b.origin.y + px(2.0)),
                size: size(px(sel_w), px(slot.h - 4.0)),
            };
            paint_fill(window, sel_bounds, accent.opacity(0.35));
        }

        // Paint the text
        let _ = shaped.paint(
            point(text_x, text_y),
            px(slot.h),
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );

        // Paint the blinking cursor
        let now = Instant::now();
        let blink_phase = (now.duration_since(self.blink_epoch).as_millis() / 530) % 2;
        if blink_phase == 0 {
            let cursor_offset = if self.cursor == 0 {
                0.0
            } else {
                let pre = shape_text(
                    &self.text[..self.cursor.min(self.text.len())],
                    font,
                    font_size,
                    fg,
                    window,
                );
                f32::from(pre.width())
            };
            let cursor_h = (slot.h * 0.65).max(12.0);
            let cursor_y = b.origin.y + px((slot.h - cursor_h) * 0.5);
            let cursor_bounds = Bounds {
                origin: point(text_x + px(cursor_offset), cursor_y),
                size: size(px(1.5), px(cursor_h)),
            };
            paint_fill(window, cursor_bounds, fg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_collapses_and_composition_undo_is_one_edit() {
        let mut state = ImmTextInputState::new("hello café");
        state.cursor = 8;
        state.select_word();
        assert_eq!(state.selected_text(), "café");
        state.handle_key("left", None, false, false);
        assert_eq!(state.cursor, 6);
        assert!(!state.has_selection());
        state.replace_native(None, "に", None, true);
        state.replace_native(None, "日本", None, true);
        state.replace_native(None, "日本語", None, false);
        state.undo(false);
        assert_eq!(state.text, "hello café");
        state.undo(true);
        assert_eq!(state.text, "hello 日本語café");
    }
    #[test]
    fn native_composition_replaces_marked_text_and_preserves_utf16_selection() {
        let mut state = ImmTextInputState::new("a🚀z");
        assert_eq!(state.byte_range(1..3), 1..5);
        state.replace_native(Some(1..3), "に", Some(1..1), true);
        assert_eq!(state.text, "aにz");
        assert_eq!(state.marked, Some(1..4));
        state.replace_native(None, "日本", Some(2..2), true);
        assert_eq!(state.text, "a日本z");
        state.replace_native(None, "日本語", None, false);
        assert_eq!(state.text, "a日本語z");
        assert_eq!(state.marked, None);
        assert_eq!(state.utf16_offset(state.cursor), 4);
        state.select_all();
        state.replace_native(None, "café\r\n🚀", None, false);
        assert_eq!(state.text, "café🚀");
        assert!(state.text.is_char_boundary(state.cursor));
    }

    #[test]
    fn test_input_utf8_char_boundaries() {
        let mut state = ImmTextInputState::new("C:\\Users\\ñoño\\🚀\\file.txt");
        assert_eq!(state.cursor, state.text.len());

        // Left arrow through multibyte characters
        state.handle_key("left", None, false, false); // t
        state.handle_key("left", None, false, false); // x
        state.handle_key("left", None, false, false); // t
        state.handle_key("left", None, false, false); // .
        assert!(state.text.is_char_boundary(state.cursor));

        // Jump across emoji (4-byte UTF-8)
        let rocket_idx = state.text.find('🚀').unwrap();
        state.cursor = rocket_idx + 4;
        let prev = state.prev_char(state.cursor);
        assert_eq!(prev, rocket_idx);
    }

    #[test]
    fn test_input_word_navigation() {
        let mut state = ImmTextInputState::new("C:\\Users\\Atlas\\Documents");
        // Word left should hop to Documents start
        state.cursor = state.text.len();
        state.handle_key("left", None, true, false);
        assert_eq!(&state.text[state.cursor..], "Documents");

        // Word left again hops to Atlas
        state.handle_key("left", None, true, false);
        assert_eq!(&state.text[state.cursor..], "Atlas\\Documents");

        // Word right hops back to Documents
        state.handle_key("right", None, true, false);
        assert_eq!(&state.text[state.cursor..], "Documents");
    }

    #[test]
    fn test_input_selection_and_deletion() {
        let mut state = ImmTextInputState::new("hello world");
        state.select_all();
        assert!(state.has_selection());
        assert_eq!(state.selected_text(), "hello world");

        state.delete_selection();
        assert_eq!(state.text, "");
        assert_eq!(state.cursor, 0);

        state.insert_text("new text");
        assert_eq!(state.text, "new text");
        assert_eq!(state.cursor, 8);
    }

    #[test]
    fn test_input_backspace_and_delete() {
        let mut state = ImmTextInputState::new("abcdef");
        state.cursor = 3; // between c and d
        state.anchor = 3;

        // Backspace deletes 'c'
        state.handle_key("backspace", None, false, false);
        assert_eq!(state.text, "abdef");
        assert_eq!(state.cursor, 2);

        // Delete deletes 'd'
        state.handle_key("delete", None, false, false);
        assert_eq!(state.text, "abef");
        assert_eq!(state.cursor, 2);
    }
}
