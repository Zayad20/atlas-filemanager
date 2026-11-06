//! Frosted surface geometry and optional soft depth. Theme application owns the
//! palette and material weights; glass surfaces never add a decorative rim.

use atlas_model::clamp::ClampLenient;
use gpui::ColorExt as _;
use gpui::{
    div, hsla, linear_color_stop, linear_gradient, point, px, BoxShadow, Hsla, InteractiveElement,
    IntoElement, ParentElement, Styled,
};

/// Floating-panel corner radius (Figma pane background: radius 20, scaled to a
/// tighter desktop density).
pub const GLASS_RADIUS: f32 = 12.0;

/// Figma glass fill: #262626 at 20%, then #fff at 67%, nudged to the middle
/// between the previous airy panes and dense sidebar.
pub fn glass_fill() -> Hsla {
    hsla(0., 0., 0.93, 0.80)
}

pub fn glass_fill_with_opacity(opacity: f32) -> Hsla {
    hsla(0., 0., 0.93, opacity.clamp(0.0, 1.0))
}

/// Same material as panes so the shell reads as one glass sheet.
pub fn glass_sidebar_fill() -> Hsla {
    glass_fill()
}

/// Internal glass strips: headers/columns need separation without becoming gray bars.
pub fn glass_header_fill() -> Hsla {
    hsla(0., 0., 1., 0.24)
}

/// Raised chip/tab fill over a glass strip.
pub fn glass_chip_fill() -> Hsla {
    hsla(0., 0., 1., 0.58)
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

pub fn backdrop_tint(color: Hsla, alpha: f32) -> Hsla {
    color.opacity(alpha)
}

pub fn layer_color(base: Hsla, delta_l: f32, alpha: f32) -> Hsla {
    hsla(
        (base.hue.into_positive_degrees() / 360.0),
        base.saturation,
        (base.lightness + delta_l).clamp(0.0, 1.0),
        // Hover and floating controls inherit the theme's material density.
        // Replacing alpha here creates opaque islands on otherwise frosted panes.
        base.alpha * alpha,
    )
}

/// Floating controls need their own readable material above the pane. Their
/// state overlays must not compound the pane's alpha or fade their icons.
#[derive(Clone, Copy)]
pub(crate) struct ControlColors {
    pub surface: Hsla,
    pub hover: Hsla,
    pub pressed: Hsla,
    pub border: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
}

pub(crate) fn control_hover(foreground: Hsla) -> Hsla {
    foreground.opacity(0.08)
}

pub(crate) fn control_colors(theme: &gpui_component::Theme, glass: bool) -> ControlColors {
    let mut surface = theme.secondary;
    surface.alpha = if glass {
        // The floating toolbar and magnifier share a translucent backing;
        // only the fill fades, so icons retain their full contrast.
        let (min, max) = if theme.is_dark() {
            (0.54, 0.72)
        } else {
            (0.62, 0.78)
        };
        // Follow the Glass opacity setting through the canvas density, but only
        // within this range: capsules adapt slightly and always keep a backing.
        let (lo, hi) = crate::app::theme::glass_canvas_range(theme.is_dark());
        let t = ((theme.colors.background.alpha - lo) / (hi - lo).max(f32::EPSILON))
            .clamp_lenient(0.0, 1.0);
        min + (max - min) * t
    } else {
        1.0
    };
    ControlColors {
        surface,
        hover: control_hover(theme.foreground),
        pressed: theme.foreground.opacity(0.14),
        border: theme
            .foreground
            .opacity(if theme.is_dark() { 0.14 } else { 0.12 }),
        foreground: theme.foreground,
        muted: theme.muted_foreground,
    }
}

pub fn raised_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(0., 0., 0., 0.18).into(),
        offset: point(px(0.0), px(8.0)),
        blur_radius: px(18.0),
        spread_radius: px(-8.0),
        inset: false,
    }]
}

pub fn sunken_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(0., 0., 0., 0.18).into(),
        offset: point(px(0.0), px(1.0)),
        blur_radius: px(2.0),
        spread_radius: px(0.0),
        inset: true,
    }]
}

/// Transparent border for seamless glass styling.
pub fn glass_border() -> Hsla {
    gpui::transparent_black()
}

/// Subtle inner separator used for the segmented tab/selector track
/// (Figma "Utility Panel/Tabs" fill: black @ 10%).
pub fn track_fill() -> Hsla {
    hsla(0., 0., 0., 0.10)
}

pub fn glass_separator() -> Hsla {
    track_fill()
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

/// Style a floating frosted panel with rounded corners and optional soft depth.
/// Functional focus/accent borders belong to the caller.
pub trait GlassExt: Styled + Sized {
    /// Full floating panel — rounded corners, glass fill, soft shadow.
    #[must_use]
    fn glass_panel(self) -> Self {
        let surface = self
            .rounded(px(GLASS_RADIUS))
            .rounded_smoothing_ios()
            .backdrop_blur(px(12.0))
            .bg(glass_fill());
        if glass_shadows_enabled() {
            surface.shadow(raised_shadow())
        } else {
            surface
        }
    }

    /// Glass with an explicit fill (sidebar/chrome variants): same floating
    /// material as `glass_panel` (rounded corners + soft shadow) but with a
    /// caller-chosen fill so the sidebar/chrome can sit a touch denser than the panes.
    ///
    /// No in-app backdrop blur: these surfaces sit directly on the translucent
    /// window canvas, which the OS already blurs. The DX11 blur composite adds
    /// the backdrop's alpha on top of itself, so blurring here roughly doubled
    /// the canvas density under every pane and the Glass opacity setting
    /// barely changed them (near-black panes at minimum opacity).
    #[must_use]
    fn glass_surface(self, fill: Hsla) -> Self {
        let surface = self
            .rounded(px(GLASS_RADIUS))
            .rounded_smoothing_ios()
            .bg(fill);
        if glass_shadows_enabled() {
            surface.shadow(raised_shadow())
        } else {
            surface
        }
    }

    #[must_use]
    fn raised_surface(self) -> Self {
        if glass_shadows_enabled() {
            self.shadow(raised_shadow())
        } else {
            self
        }
    }

    #[must_use]
    fn sunken_surface(self) -> Self {
        self.shadow(sunken_shadow())
    }
}

impl<T: Styled + Sized> GlassExt for T {}

pub trait GlassHoverExt: Styled + InteractiveElement + Sized {
    #[must_use]
    fn hover_lift(self) -> Self {
        if glass_shadows_enabled() {
            self.hover(|s| s.shadow(raised_shadow()))
        } else {
            self
        }
    }
}

impl<T: Styled + InteractiveElement + Sized> GlassHoverExt for T {}
