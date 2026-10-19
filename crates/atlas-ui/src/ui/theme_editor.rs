//! Custom theme copies are persisted independently of bundled presets.
use crate::app::App;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, Entity, Hsla, SharedString, Subscription, Window};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_component::{
    h_flex, v_flex, ActiveTheme, Colorize, Disableable, Sizable, ThemeConfig, ThemeRegistry,
    ThemeSet,
};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy)]
enum Role {
    Accent,
    Background,
    Foreground,
}

impl Role {
    fn label(self) -> String {
        rust_i18n::t!(match self {
            Self::Accent => "theme_color_accent",
            Self::Background => "theme_color_background",
            Self::Foreground => "theme_color_foreground",
        })
        .to_string()
    }
    fn get(self, config: &ThemeConfig) -> Option<&SharedString> {
        match self {
            Self::Accent => config.colors.accent.as_ref(),
            Self::Background => config.colors.background.as_ref(),
            Self::Foreground => config.colors.foreground.as_ref(),
        }
    }
    fn set(self, config: &mut ThemeConfig, color: Hsla) {
        let value = Some(color.to_hex().into());
        match self {
            Self::Accent => config.colors.accent = value,
            Self::Background => config.colors.background = value,
            Self::Foreground => config.colors.foreground = value,
        }
    }
}

pub(crate) struct ThemeEditor {
    config: ThemeConfig,
    file_name: String,
    controls: Vec<(Role, Entity<ColorPickerState>)>,
    dirty: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

fn fork_config(base: &ThemeConfig, existing: impl Fn(&str) -> bool) -> ThemeConfig {
    let (stem, suffix) = base
        .name
        .strip_suffix(" Glass")
        .map_or((base.name.as_ref(), ""), |stem| (stem, " Glass"));
    let mut number = 1u64;
    let name = loop {
        let candidate = if number == 1 {
            format!("{stem} Copy{suffix}")
        } else {
            format!("{stem} Copy {number}{suffix}")
        };
        if !existing(&candidate) {
            break candidate;
        }
        number += 1;
    };
    let mut config = base.clone();
    config.name = name.into();
    config.is_default = false;
    config
}

fn theme_set(config: &ThemeConfig) -> ThemeSet {
    ThemeSet {
        name: config.name.clone(),
        themes: vec![config.clone()],
        ..ThemeSet::default()
    }
}

impl App {
    fn fork_active_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let registry = ThemeRegistry::global(cx);
        let Some(base) = registry
            .themes()
            .get(self.state.profiles.active().theme.as_str())
        else {
            self.theme_fork_error = Some(rust_i18n::t!("theme_fork_unavailable").to_string());
            cx.notify();
            return;
        };
        let config = fork_config(base, |name| registry.themes().contains_key(name));
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let file_name = format!(
            "custom-{timestamp:x}-{}-{}.json",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        if let Err(error) =
            atlas_core::util::config::save_custom_theme(&file_name, &theme_set(&config))
        {
            self.theme_fork_error =
                Some(rust_i18n::t!("theme_save_error", error = error.to_string()).to_string());
            cx.notify();
            return;
        }
        let mut editor = ThemeEditor {
            config: config.clone(),
            file_name,
            controls: Vec::new(),
            dirty: false,
            error: None,
            _subscriptions: Vec::new(),
        };
        for role in [Role::Accent, Role::Background, Role::Foreground] {
            let fallback = match role {
                Role::Accent => cx.theme().accent,
                Role::Background => cx.theme().background,
                Role::Foreground => cx.theme().foreground,
            };
            let color = role
                .get(&config)
                .and_then(|value| Hsla::parse_hex(value).ok())
                .unwrap_or(fallback);
            let animate = !self.state.settings.disable_animations;
            let control = cx.new(|cx| {
                ColorPickerState::new(window, cx)
                    .default_value(color)
                    .default_angular()
                    .angular_animation(animate)
            });
            editor._subscriptions.push(cx.subscribe(
                &control,
                move |this, _, event: &ColorPickerEvent, cx| {
                    if let ColorPickerEvent::Change(Some(color)) = event {
                        if let Some(editor) = &mut this.theme_editor {
                            role.set(&mut editor.config, *color);
                            editor.dirty = true;
                            editor.error = None;
                        }
                        cx.notify();
                    }
                },
            ));
            editor.controls.push((role, control));
        }
        ThemeRegistry::global_mut(cx).register_custom_theme(config.clone());
        self.set_theme(config.name.to_string());
        self.theme_editor = Some(editor);
        self.theme_fork_error = None;
        cx.notify();
    }

    fn save_theme_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.theme_editor else {
            return;
        };
        if !editor.dirty {
            return;
        }
        if let Err(error) = atlas_core::util::config::save_custom_theme(
            &editor.file_name,
            &theme_set(&editor.config),
        ) {
            editor.error =
                Some(rust_i18n::t!("theme_save_error", error = error.to_string()).to_string());
        } else {
            ThemeRegistry::global_mut(cx).register_custom_theme(editor.config.clone());
            editor.dirty = false;
            editor.error = None;
            let name = editor.config.name.to_string();
            self.set_theme(name);
        }
        cx.notify();
    }

    pub(crate) fn render_theme_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let fg = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let mut content = v_flex().w_full().gap_2().py_2().child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            rust_i18n::t!(
                                "theme_active",
                                name = self.state.profiles.active().theme.clone()
                            )
                            .to_string(),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(rust_i18n::t!("theme_fork_description").to_string()),
                        ),
                )
                .child(
                    Button::new("fork-active-theme")
                        .small()
                        .label(rust_i18n::t!("theme_fork").to_string())
                        .disabled(self.theme_editor.is_some())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.fork_active_theme(window, cx)),
                        ),
                ),
        );
        if let Some(error) = &self.theme_fork_error {
            content = content.child(div().text_sm().text_color(danger).child(error.clone()));
        }
        if let Some(editor) = &self.theme_editor {
            content = content.child(div().text_sm().text_color(fg).child(
                rust_i18n::t!("theme_editing", name = editor.config.name.to_string()).to_string(),
            ));
            content = content.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("theme_editor_description").to_string()),
            );
            for (role, control) in &editor.controls {
                let hex = control
                    .read(cx)
                    .value()
                    .map(|value| value.to_hex())
                    .unwrap_or_default();
                content = content.child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(div().text_sm().text_color(fg).child(role.label()))
                        .child(ColorPicker::new(control).small().label(hex)),
                );
            }
            if let Some(error) = &editor.error {
                content = content.child(div().text_sm().text_color(danger).child(error.clone()));
            }
            content = content.child(
                h_flex()
                    .gap_2()
                    .mt(px(4.0))
                    .child(
                        Button::new("save-custom-theme")
                            .primary()
                            .small()
                            .label(rust_i18n::t!("theme_save").to_string())
                            .disabled(!editor.dirty)
                            .on_click(cx.listener(|this, _, _, cx| this.save_theme_editor(cx))),
                    )
                    .child(
                        Button::new("close-theme-editor")
                            .small()
                            .label(
                                rust_i18n::t!(if editor.dirty {
                                    "theme_discard"
                                } else {
                                    "theme_close_editor"
                                })
                                .to_string(),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.theme_editor = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        content.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn theme_fork_preserves_material_fonts_and_colors_with_a_unique_name() {
        let set: ThemeSet =
            serde_json::from_str(include_str!("../../assets/themes/atlas-themes.json")).unwrap();
        let source = set
            .themes
            .iter()
            .find(|theme| theme.name.ends_with(" Glass"))
            .unwrap();
        let first = fork_config(source, |_| false);
        let second = fork_config(source, |name| name == first.name.as_ref());
        assert_ne!(first.name, second.name);
        assert!(first.name.ends_with(" Glass"));
        assert!(second.name.ends_with(" Glass"));
        assert!(!first.is_default);
        assert_eq!(
            serde_json::to_value(&first.colors).unwrap(),
            serde_json::to_value(&source.colors).unwrap()
        );
        assert_eq!(first.font_family, source.font_family);
        assert_eq!(first.mode, source.mode);
        let serialized = serde_json::to_string(&theme_set(&first)).unwrap();
        let restored: ThemeSet = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.themes[0].name, first.name);
    }
}
