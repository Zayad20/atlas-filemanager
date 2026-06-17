//! Shared motion language for Atlas UI.
//!
//! RAD Debugger's UI feels consistent because most animated surfaces use the
//! same small set of rates and eased values. Keep those knobs centralized here
//! so panes, overlays, menus, and inputs move as one system instead of each
//! component inventing its own timing.

use gpui::{Animation, Div, Hsla, Pixels, SharedString, Styled};
use std::time::Duration;

pub const MICRO_MS: u64 = 120;
pub const PANEL_MS: u64 = 150;
pub const TAB_MS: u64 = 170;
pub const PANE_MS: u64 = 220;
pub const GLOW_MS: u64 = 280;

pub const PALETTE_GROW_FROM: f32 = 0.80;

pub fn ease_out_quart(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(4)
}

pub fn animation(ms: u64) -> Animation {
    Animation::new(Duration::from_millis(ms)).with_easing(ease_out_quart)
}

pub fn keyed(
    prefix: &str,
    id: impl std::fmt::Display,
    generation: impl std::fmt::Debug,
) -> SharedString {
    SharedString::from(format!("{prefix}-{id}-{generation:?}"))
}

pub fn panel_pop(this: Div, openness: f32, width: f32) -> Div {
    let grow = (1.0 - PALETTE_GROW_FROM).mul_add(openness, PALETTE_GROW_FROM);
    this.opacity(openness).w(gpui::px(width * grow))
}

pub fn pane_intro(this: Div, openness: f32, offset: Pixels, vertical: bool) -> Div {
    let distance = offset * (1.0 - openness);
    let this = this.opacity(openness);
    if vertical {
        this.mt(distance)
    } else {
        this.ml(distance)
    }
}

pub fn glow(this: Div, openness: f32, accent: Hsla) -> Div {
    this.border_color(accent.opacity(0.42 * (1.0 - openness)))
        .bg(accent.opacity(0.08 * (1.0 - openness)))
        .opacity(1.0 - openness)
}
