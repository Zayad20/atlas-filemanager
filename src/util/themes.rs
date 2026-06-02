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

struct ThemeColors {
    window_fill: Color32,
    panel_fill: Color32,
    surface0: Color32,
    surface1: Color32,
    overlay1: Color32,
    text: Color32,
    accent: Color32,
    hyperlink: Color32,
}

impl ThemeColors {
    fn build(self) -> Visuals {
        let mut v = Visuals::dark();

        v.window_fill = self.window_fill;
        v.panel_fill = self.panel_fill;
        v.faint_bg_color = self.surface0;
        v.extreme_bg_color = self.panel_fill;
        v.code_bg_color = self.surface0;

        v.widgets.noninteractive.bg_fill = self.panel_fill; // mantle / polar1
        v.widgets.noninteractive.fg_stroke.color = self.overlay1;
        v.widgets.noninteractive.bg_stroke.color = self.surface1;

        v.widgets.inactive.bg_fill = self.surface0;
        v.widgets.inactive.fg_stroke.color = self.text;
        v.widgets.inactive.bg_stroke.color = self.surface1; // polar2

        v.widgets.hovered.bg_fill = self.surface1;
        v.widgets.hovered.fg_stroke.color = self.text;
        v.widgets.hovered.bg_stroke.color = self.accent;

        v.widgets.active.bg_fill = self.accent;
        v.widgets.active.fg_stroke.color = self.window_fill; // base
        v.widgets.active.bg_stroke.color = self.accent;

        v.selection.bg_fill = self.accent.gamma_multiply(0.35);
        v.selection.stroke.color = self.accent;

        v.hyperlink_color = self.hyperlink;
        v.override_text_color = Some(self.text);

        v
    }
}

fn catppuccin_mocha() -> Visuals {
    ThemeColors {
        window_fill: Color32::from_rgb(30, 30, 46), // base
        panel_fill: Color32::from_rgb(24, 24, 37), // mantle
        surface0: Color32::from_rgb(49, 50, 68),
        surface1: Color32::from_rgb(69, 71, 90),
        overlay1: Color32::from_rgb(127, 132, 156),
        text: Color32::from_rgb(205, 214, 244),
        accent: Color32::from_rgb(203, 166, 247), // mauve
        hyperlink: Color32::from_rgb(137, 180, 250), // blue
    }
    .build()
}

fn nord() -> Visuals {
    ThemeColors {
        window_fill: Color32::from_rgb(46, 52, 64), // polar0
        panel_fill: Color32::from_rgb(36, 41, 51),
        surface0: Color32::from_rgb(59, 66, 82), // polar1
        surface1: Color32::from_rgb(67, 76, 94), // polar2
        overlay1: Color32::from_rgb(76, 86, 106), // polar3
        text: Color32::from_rgb(216, 222, 233), // snow0
        accent: Color32::from_rgb(136, 192, 208), // frost1
        hyperlink: Color32::from_rgb(129, 161, 193), // frost2
    }
    .build()
}
