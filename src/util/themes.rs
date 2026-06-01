use crate::state::settings::Theme;
use egui::{Color32, Visuals};

pub fn apply_theme(ctx: &egui::Context, theme: &Theme) {
    ctx.set_visuals(make_visuals(theme));
}

fn make_visuals(theme: &Theme) -> Visuals {
    match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
        Theme::CatppuccinMocha => catppuccin_mocha(),
        Theme::Nord => nord(),
    }
}

fn catppuccin_mocha() -> Visuals {
    let mut v = Visuals::dark();

    let base = Color32::from_rgb(30, 30, 46); // base
    let mantle = Color32::from_rgb(24, 24, 37); // mantle
    let surface0 = Color32::from_rgb(49, 50, 68); // surface0
    let surface1 = Color32::from_rgb(69, 71, 90); // surface1
    let overlay1 = Color32::from_rgb(127, 132, 156); // overlay1
    let text = Color32::from_rgb(205, 214, 244); // text
    let mauve = Color32::from_rgb(203, 166, 247); // mauve (accent)
    let blue = Color32::from_rgb(137, 180, 250); // blue

    v.window_fill = base;
    v.panel_fill = mantle;
    v.faint_bg_color = surface0;
    v.extreme_bg_color = mantle;
    v.code_bg_color = surface0;

    v.widgets.noninteractive.bg_fill = mantle;
    v.widgets.noninteractive.fg_stroke.color = overlay1;
    v.widgets.noninteractive.bg_stroke.color = surface1;

    v.widgets.inactive.bg_fill = surface0;
    v.widgets.inactive.fg_stroke.color = text;
    v.widgets.inactive.bg_stroke.color = surface1;

    v.widgets.hovered.bg_fill = surface1;
    v.widgets.hovered.fg_stroke.color = text;
    v.widgets.hovered.bg_stroke.color = mauve;

    v.widgets.active.bg_fill = mauve;
    v.widgets.active.fg_stroke.color = base;
    v.widgets.active.bg_stroke.color = mauve;

    v.selection.bg_fill = mauve.gamma_multiply(0.35);
    v.selection.stroke.color = mauve;

    v.hyperlink_color = blue;
    v.override_text_color = Some(text);

    v
}

fn nord() -> Visuals {
    let mut v = Visuals::dark();

    let polar0 = Color32::from_rgb(46, 52, 64); // nord0
    let polar1 = Color32::from_rgb(59, 66, 82); // nord1
    let polar2 = Color32::from_rgb(67, 76, 94); // nord2
    let polar3 = Color32::from_rgb(76, 86, 106); // nord3
    let snow0 = Color32::from_rgb(216, 222, 233); // nord4 (text)
    let frost1 = Color32::from_rgb(136, 192, 208); // nord8 (accent)
    let frost2 = Color32::from_rgb(129, 161, 193); // nord9

    v.window_fill = polar0;
    v.panel_fill = Color32::from_rgb(36, 41, 51);
    v.faint_bg_color = polar1;
    v.extreme_bg_color = Color32::from_rgb(36, 41, 51);
    v.code_bg_color = polar1;

    v.widgets.noninteractive.bg_fill = polar1;
    v.widgets.noninteractive.fg_stroke.color = polar3;
    v.widgets.noninteractive.bg_stroke.color = polar2;

    v.widgets.inactive.bg_fill = polar1;
    v.widgets.inactive.fg_stroke.color = snow0;
    v.widgets.inactive.bg_stroke.color = polar2;

    v.widgets.hovered.bg_fill = polar2;
    v.widgets.hovered.fg_stroke.color = snow0;
    v.widgets.hovered.bg_stroke.color = frost1;

    v.widgets.active.bg_fill = frost1;
    v.widgets.active.fg_stroke.color = polar0;
    v.widgets.active.bg_stroke.color = frost1;

    v.selection.bg_fill = frost1.gamma_multiply(0.3);
    v.selection.stroke.color = frost1;

    v.hyperlink_color = frost2;
    v.override_text_color = Some(snow0);

    v
}
