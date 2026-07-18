//! Flat UI surfaces: math layout + measured text + hit slots + theme icons.
//!
//! Custom GPUI Elements use this kit so chrome is ~1 layout node while still
//! matching div-chrome look (Fluent vs Phosphor, xs/sm type scale).

use crate::ui::icons::{windows_icon_glyph, AtlasIcon};
use gpui::{
    point, px, size, transparent_black, Bounds, Corners, Edges, Font, FontFeatures, FontStyle,
    FontWeight, Hsla, Pixels, ShapedLine, SharedString, TextAlign, TextRun, Window,
};

/// Axis-aligned rect in surface-local pixels (origin = surface top-left).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Slot {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Slot {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, lx: f32, ly: f32) -> bool {
        lx >= self.x && lx < self.x + self.w && ly >= self.y && ly < self.y + self.h
    }

    pub fn to_bounds(&self, origin: gpui::Point<Pixels>) -> Bounds<Pixels> {
        Bounds {
            origin: point(origin.x + px(self.x), origin.y + px(self.y)),
            size: size(px(self.w.max(0.0)), px(self.h.max(0.0))),
        }
    }

    pub fn center_y(&self) -> f32 {
        self.y + self.h * 0.5
    }
}

// ── Chrome type scale (matches old .text_xs / .text_sm / .text_base) ─────────

/// Tab label: `text_xs` (non-Windows) / `text_sm` (Windows).
pub(crate) fn chrome_font_size_tabs(window: &Window, windows_theme: bool) -> Pixels {
    let rem = f32::from(window.rem_size());
    px(rem * if windows_theme { 0.875 } else { 0.75 })
}

/// Breadcrumb path labels: `text_sm` / `text_base` (Windows).
pub(crate) fn chrome_font_size_crumbs(window: &Window, windows_theme: bool) -> Pixels {
    let rem = f32::from(window.rem_size());
    px(rem * if windows_theme { 1.0 } else { 0.875 })
}

/// Details column header: always `text_xs`.
pub(crate) fn chrome_font_size_header(window: &Window) -> Pixels {
    let rem = f32::from(window.rem_size());
    px(rem * 0.75)
}

/// UI font at a specific size (keeps family/weight from window style).
pub(crate) fn chrome_ui_font(window: &Window) -> Font {
    window.text_style().font()
}

fn fluent_icons_font() -> Font {
    Font {
        family: SharedString::from("Segoe Fluent Icons"),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: FontWeight::default(),
        style: FontStyle::default(),
    }
}

// ── Text ────────────────────────────────────────────────────────────────────

/// Shape a single-line label (measured width via [`line_width`]).
pub(crate) fn shape_text(
    text: &str,
    font: &Font,
    font_size: Pixels,
    color: Hsla,
    window: &Window,
) -> ShapedLine {
    let run = TextRun {
        len: text.len(),
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window.text_system().shape_line(
        SharedString::from(text.to_string()),
        font_size,
        &[run],
        None,
    )
}

#[inline]
pub(crate) fn line_width(line: &ShapedLine) -> f32 {
    f32::from(line.width())
}

/// Top of the paint box for shaped text inside a slot.
///
/// Pass this as `origin.y` to `ShapedLine::paint` and pass **`slot_h` as
/// `line_height`**. GPUI centers glyphs inside that box — do **not** also
/// subtract font_size/2 (that double-shifts labels up/down).
pub(crate) fn text_origin_y(slot_y: f32, zoom: f32) -> f32 {
    // Tiny optical nudge like Details rows (`ROW_TEXT_Y_NUDGE`).
    slot_y - 1.0 * zoom
}

// ── Hit testing ──────────────────────────────────────────────────────────────

/// First hit among slots (earlier entries win — put close/resize before body).
pub(crate) fn hit_first<T: Copy>(slots: &[(Slot, T)], lx: f32, ly: f32) -> Option<T> {
    for (slot, id) in slots {
        if slot.contains(lx, ly) {
            return Some(*id);
        }
    }
    None
}

// ── Paint primitives ─────────────────────────────────────────────────────────

pub(crate) fn paint_fill(window: &mut Window, bounds: Bounds<Pixels>, color: Hsla) {
    window.paint_quad(gpui::fill(bounds, color));
}

pub(crate) fn paint_rounded(window: &mut Window, bounds: Bounds<Pixels>, radius: f32, color: Hsla) {
    paint_rounded_corners(window, bounds, Corners::all(px(radius.max(0.0))), color);
}

pub(crate) fn paint_rounded_corners(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    corners: Corners<Pixels>,
    color: Hsla,
) {
    window.paint_quad(gpui::quad(
        bounds,
        corners,
        color,
        Edges::all(px(0.0)),
        transparent_black(),
        Default::default(),
    ));
}

pub(crate) fn paint_svg_icon(
    window: &mut Window,
    cx: &mut gpui::App,
    bounds: Bounds<Pixels>,
    path: &'static str,
    color: Hsla,
) {
    let _ = window.paint_svg(
        bounds,
        SharedString::from(path),
        None,
        Default::default(),
        color,
        cx,
    );
}

/// Same policy as `theme_icon`: Fluent glyph on Windows themes, Phosphor SVG otherwise.
pub(crate) fn paint_theme_icon(
    window: &mut Window,
    cx: &mut gpui::App,
    origin: gpui::Point<Pixels>,
    slot: Slot,
    kind: AtlasIcon,
    windows_theme: bool,
    // Icon pixel size already zoom-scaled (e.g. 15.0 * zoom).
    icon_px: f32,
    color: Hsla,
) {
    let sz = icon_px.max(1.0);
    let b = Bounds {
        origin: point(
            origin.x + px(slot.x + (slot.w - sz) * 0.5),
            origin.y + px(slot.y + (slot.h - sz) * 0.5),
        ),
        size: size(px(sz), px(sz)),
    };

    let use_fluent = windows_theme && !matches!(kind, AtlasIcon::TerminalWindow);
    if use_fluent {
        let glyph = windows_icon_glyph(kind);
        let font = fluent_icons_font();
        let line = shape_text(glyph, &font, px(sz), color, window);
        let w = line_width(&line);
        let x = f32::from(b.origin.x) + (sz - w) * 0.5;
        let _ = line.paint(
            point(px(x), b.origin.y),
            px(sz),
            TextAlign::Left,
            None,
            window,
            cx,
        );
    } else {
        paint_svg_icon(window, cx, b, kind.path(), color);
    }
}

/// Legacy helper: SVG-only icon as fraction of slot (prefer [`paint_theme_icon`]).
pub(crate) fn paint_icon_in_slot(
    window: &mut Window,
    cx: &mut gpui::App,
    origin: gpui::Point<Pixels>,
    slot: Slot,
    icon_frac: f32,
    path: &'static str,
    color: Hsla,
) {
    let sz = (slot.w.min(slot.h) * icon_frac).max(1.0);
    let b = Bounds {
        origin: point(
            origin.x + px(slot.x + (slot.w - sz) * 0.5),
            origin.y + px(slot.y + (slot.h - sz) * 0.5),
        ),
        size: size(px(sz), px(sz)),
    };
    paint_svg_icon(window, cx, b, path, color);
}
