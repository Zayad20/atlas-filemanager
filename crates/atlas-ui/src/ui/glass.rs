//! Liquid-glass surface styling, extracted from the Figma "Design" frame
//! (file 4rz5LAf…, node 1:4 "Design"). The design floats rounded, translucent
//! panels — panes, sidebar, chrome — over the window background so they read as
//! one glass material with a soft rim light and drop shadow.
//!
//! gpui has no real backdrop blur, so "glass" here is the *material language*:
//! a translucent fill, a bright 1px rim, a corner radius and a soft shadow. The
//! alpha values come straight from the Figma fills/strokes so the look matches.

use gpui::{
    div, hsla, linear_color_stop, linear_gradient, px, Hsla, IntoElement, ParentElement, Styled,
};

/// Floating-panel corner radius (Figma pane background: radius 20, scaled to a
/// tighter desktop density).
pub const GLASS_RADIUS: f32 = 12.0;

/// Translucent fill for floating panes — Figma pane bg is `#262626 @ 20%` layered
/// under `#ffffff @ 67%`, i.e. a near-white glass. Approximated as white at ~55%.
pub fn glass_fill() -> Hsla {
    hsla(0., 0., 1., 0.55)
}

/// Darker glass for the sidebar — Figma layers a `#262626` base with a `#f5f5f5 @ 67%`
/// veil plus a black glass tint. Kept a touch denser than the panes.
pub fn glass_sidebar_fill() -> Hsla {
    hsla(0., 0., 1., 0.45)
}

/// Translucent chrome (title bar / status bar) — lighter so it sits behind panes.
pub fn glass_chrome_fill() -> Hsla {
    hsla(0., 0., 1., 0.35)
}

/// Expensive soft shadows are opt-in. On Windows they can dominate frame cost while
/// the window is being moved or resized.
pub fn glass_shadows_enabled() -> bool {
    std::env::var("ATLAS_SHADOWS").is_ok_and(|v| v != "0")
}

/// Bright rim light (Figma stroke: white `0.4 → 0.2`). One flat value here.
pub fn glass_border() -> Hsla {
    hsla(0., 0., 1., 0.40)
}

/// Subtle inner separator used for the segmented tab/selector track
/// (Figma "Utility Panel/Tabs" fill: black @ 10%).
pub fn track_fill() -> Hsla {
    hsla(0., 0., 0., 0.10)
}

/// Active segment fill (Figma active tab: light raised `#a8a5a5 @ 20%`).
pub fn track_active_fill() -> Hsla {
    hsla(0., 0., 0., 0.06)
}

/// Full-window soft pastel wallpaper, matching the Figma "Design" background the
/// frosted panels float over. gpui's `linear_gradient` only takes two stops, so we
/// stack a base diagonal gradient with two translucent color "glows" to get the
/// multi-hue lavender/sky/mint/blush wash. Add as the first (bottom) child of the
/// root so panes/sidebar read as frosted glass over it.
pub fn glass_backdrop() -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        // Base: lavender → sky-blue diagonal. Kept translucent so the window's real
        // acrylic blur (WindowBackgroundAppearance::Blurred) shows through, just tinted.
        .bg(linear_gradient(
            125.,
            linear_color_stop(hsla(0.72, 0.55, 0.85, 0.30), 0.0),
            linear_color_stop(hsla(0.60, 0.58, 0.86, 0.30), 1.0),
        ))
        // Mint glow (top-right).
        .child(div().absolute().inset_0().bg(linear_gradient(
            300.,
            linear_color_stop(hsla(0.42, 0.50, 0.84, 0.55), 0.0),
            linear_color_stop(hsla(0.42, 0.50, 0.84, 0.0), 0.55),
        )))
        // Blush glow (bottom-left).
        .child(div().absolute().inset_0().bg(linear_gradient(
            20.,
            linear_color_stop(hsla(0.92, 0.55, 0.88, 0.45), 0.0),
            linear_color_stop(hsla(0.92, 0.55, 0.88, 0.0), 0.5),
        )))
}

/// Style any element as a floating glass panel: rounded, translucent, rim-lit,
/// shadowed. Apply a focus/accent border *after* this to override the rim.
pub trait GlassExt: Styled + Sized {
    /// Full floating panel — rounded corners, glass fill, rim light, soft shadow.
    #[must_use]
    fn glass_panel(self) -> Self {
        let surface = self
            .rounded(px(GLASS_RADIUS))
            .bg(glass_fill())
            .border_1()
            .border_color(glass_border());
        if glass_shadows_enabled() {
            surface.shadow_lg()
        } else {
            surface
        }
    }

    /// Glass with an explicit fill (sidebar/chrome variants): same floating
    /// material as `glass_panel` (rounded corners + rim + soft shadow) but with a
    /// caller-chosen fill so the sidebar/chrome can sit a touch denser than the panes.
    #[must_use]
    fn glass_surface(self, fill: Hsla) -> Self {
        let surface = self
            .rounded(px(GLASS_RADIUS))
            .bg(fill)
            .border_1()
            .border_color(glass_border());
        if glass_shadows_enabled() {
            surface.shadow_lg()
        } else {
            surface
        }
    }
}

impl<T: Styled + Sized> GlassExt for T {}
