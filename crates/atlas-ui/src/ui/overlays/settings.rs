//! Settings overlay: language, zoom, toggles, view mode, appearance, sizes.
#![allow(clippy::too_many_lines)] // reason: cohesive overlay kept whole

use crate::app::App;
use crate::input::action::Action;
use crate::ui::icons::{icon, AtlasIcon};
use crate::ui::motion;
use atlas_model::settings::{InspectorPlacement, ViewMode};
use gpui::prelude::*;
use gpui::AnimationExt;
use gpui::{div, px, AnyElement, Context, Entity, MouseButton, SharedString, Window};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex, ActiveTheme};

impl App {
    pub(crate) fn render_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (popover, fg, muted, border, accent, surface, accent_fg) = {
            let t = cx.theme();
            (
                t.popover,
                t.popover_foreground,
                t.muted_foreground,
                t.border,
                t.accent,
                t.background,
                t.accent_foreground,
            )
        };
        let view = cx.entity();
        let focus = self.overlay_focus(cx);
        let (smooth_ease, smooth_boost) =
            crate::app::scroll_smooth_params(self.state.settings.scroll_smoothness);
        self.settings_scroll.set_smooth(smooth_ease, smooth_boost);
        let input_entity = {
            if self.settings_input.is_none() {
                self.settings_input = Some(crate::ui::text_input::single_line(
                    window,
                    cx,
                    "Search settings",
                    "",
                ));
            }
            self.settings_input.clone().unwrap()
        };
        let query = input_entity.read(cx).value().to_string();
        let query_lower = query.to_lowercase();
        let show_all = query_lower.trim().is_empty();
        let active_section = self.settings_section.min(5);
        let language_row = show_all
            || setting_matches(&query_lower, &["language", "idioma", "english", "spanish"]);
        let zoom_row = show_all || setting_matches(&query_lower, &["zoom", "scale", "escala"]);
        let eco_row = show_all || setting_matches(&query_lower, &["eco", "ram", "memory"]);
        let hidden_row =
            show_all || setting_matches(&query_lower, &["show", "hidden", "ocultos", "files"]);
        let sidebar_row =
            show_all || setting_matches(&query_lower, &["show", "sidebar", "barra", "lateral"]);
        let inspector_row =
            show_all || setting_matches(&query_lower, &["show", "inspector", "preview", "panel"]);
        let animations_row = show_all
            || setting_matches(
                &query_lower,
                &[
                    "disable animations",
                    "animation",
                    "animations",
                    "animaciones",
                ],
            );
        let autosave_row =
            show_all || setting_matches(&query_lower, &["autosave", "session", "sesion"]);
        let dim_panes_row = show_all
            || setting_matches(
                &query_lower,
                &["dim", "inactive", "pane", "atenuar", "panel", "inactivo"],
            );
        let smooth_row =
            show_all || setting_matches(&query_lower, &["smooth", "scroll", "suavidad"]);
        let titlebar_row = show_all
            || setting_matches(
                &query_lower,
                &[
                    "titlebar",
                    "title bar",
                    "window bar",
                    "chrome",
                    "linux",
                    "barra",
                    "ventana",
                ],
            );
        let glass_opacity_row = show_all
            || setting_matches(
                &query_lower,
                &[
                    "glass",
                    "transparency",
                    "transparencia",
                    "opacity",
                    "opacidad",
                    "cristal",
                    "blur",
                    "desenfoque",
                ],
            );
        let placement_row =
            show_all || setting_matches(&query_lower, &["placement", "inspector", "global", "tab"]);
        let view_row = show_all
            || setting_matches(
                &query_lower,
                &["view", "vista", "details", "grid", "list", "miller"],
            );
        let sidebar_width_row =
            show_all || setting_matches(&query_lower, &["sidebar", "width", "ancho"]);
        let inspector_width_row =
            show_all || setting_matches(&query_lower, &["inspector", "width", "ancho"]);
        let font_row = show_all || setting_matches(&query_lower, &["font", "fuente", "text"]);
        let thumbnail_row =
            show_all || setting_matches(&query_lower, &["thumbnail", "thumb", "miniatura"]);
        let about_row = show_all
            || setting_matches(
                &query_lower,
                &[
                    "about",
                    "support",
                    "community",
                    "discord",
                    "website",
                    "web",
                    "icons",
                    "acerca",
                ],
            );
        let show_general = if show_all {
            active_section == 0
        } else {
            language_row || zoom_row || setting_matches(&query_lower, &["general"])
        };
        let show_behavior = if show_all {
            active_section == 1
        } else {
            eco_row
                || hidden_row
                || sidebar_row
                || inspector_row
                || animations_row
                || autosave_row
                || dim_panes_row
                || smooth_row
                || setting_matches(&query_lower, &["behavior"])
        };
        let show_theme = if show_all {
            active_section == 2
        } else {
            titlebar_row
                || glass_opacity_row
                || setting_matches(&query_lower, &["appearance", "theme", "tema", "aspecto"])
        };
        let show_layout = if show_all {
            active_section == 3
        } else {
            placement_row || view_row || setting_matches(&query_lower, &["layout"])
        };
        let show_sizes = if show_all {
            active_section == 4
        } else {
            sidebar_width_row
                || inspector_width_row
                || font_row
                || thumbnail_row
                || setting_matches(&query_lower, &["size", "sizes", "tamano"])
        };
        let show_about = if show_all {
            active_section == 5
        } else {
            about_row
        };
        let s = &self.state.settings;

        // A simple labeled toggle row that flips a settings bool via a closure.
        let toggle = move |label: &str,
                           value: bool,
                           id: &'static str,
                           set: fn(&mut Self),
                           view: Entity<Self>| {
            h_flex()
                .id(SharedString::from(id))
                .w_full()
                .justify_between()
                .items_center()
                .py_1()
                .cursor_pointer()
                .child(label.to_string())
                .child(
                    div()
                        .w(px(36.))
                        .h(px(20.))
                        .rounded_full()
                        .p(px(2.))
                        .when(value, |d| d.bg(accent))
                        .when(!value, |d| d.bg(border))
                        .child(
                            div()
                                .size(px(16.))
                                .rounded_full()
                                .bg(fg)
                                .when(value, |d| d.ml(px(16.))),
                        ),
                )
                .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                    view.update(cx, |this, cx| {
                        set(this);
                        cx.notify();
                    });
                })
        };

        let placement = match s.inspector_placement {
            InspectorPlacement::Global => rust_i18n::t!("settings_inspector_global"),
            InspectorPlacement::ActiveTab => rust_i18n::t!("settings_inspector_tab"),
        };
        let view_mode = match s.view_mode {
            ViewMode::Details => rust_i18n::t!("settings_view_details"),
            ViewMode::Grid => rust_i18n::t!("settings_view_grid"),
            ViewMode::List => rust_i18n::t!("settings_view_list"),
            ViewMode::Miller => rust_i18n::t!("settings_view_miller"),
        };
        let inspector_placement = s.inspector_placement.clone();
        let default_view_mode = s.view_mode.clone();
        let sidebar_width = s.sidebar_width;
        let inspector_width = s.inspector_width;
        let font_size = s.font_size;
        let font_family = s.font_family.clone();
        let thumbnail_size = s.thumbnail_size;

        let segment = move |id: &'static str,
                            label: &str,
                            selected: bool,
                            apply: fn(&mut Self),
                            view: Entity<Self>| {
            div()
                .id(id)
                .px_2()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(if selected { accent } else { border })
                .cursor_pointer()
                .when(selected, |s| s.bg(accent))
                .when(!selected, |s| s.hover(|h| h.bg(border)))
                .child(label.to_string())
                .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                    view.update(cx, |this, cx| {
                        apply(this);
                        cx.notify();
                    });
                })
        };

        let stepper = move |id: &'static str,
                            label: &str,
                            value: f32,
                            unit: &str,
                            dec: fn(&mut Self),
                            inc: fn(&mut Self),
                            view: Entity<Self>| {
            h_flex()
                .id(id)
                .w_full()
                .justify_between()
                .items_center()
                .py_1()
                .child(label.to_string())
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .border_1()
                                .border_color(border)
                                .cursor_pointer()
                                .child("−")
                                .on_mouse_down(MouseButton::Left, {
                                    let view = view.clone();
                                    move |_ev, _w, cx| {
                                        view.update(cx, |this, cx| {
                                            dec(this);
                                            cx.notify();
                                        });
                                    }
                                }),
                        )
                        .child(
                            div()
                                .w(px(72.))
                                .text_align(gpui::TextAlign::Center)
                                .child(format!("{value:.0}{unit}")),
                        )
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .border_1()
                                .border_color(border)
                                .cursor_pointer()
                                .child("+")
                                .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                    view.update(cx, |this, cx| {
                                        inc(this);
                                        cx.notify();
                                    });
                                }),
                        ),
                )
        };

        let font_buttons: Vec<AnyElement> = ["Geist", "Segoe UI Variable", ".SystemUIFont"]
            .into_iter()
            .map(|name| {
                let selected = name == font_family;
                let pick = name.to_string();
                let v = view.clone();
                div()
                    .id(SharedString::from(format!("set-font-family-{name}")))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(if selected { accent } else { border })
                    .cursor_pointer()
                    .when(selected, |d| d.bg(accent).text_color(accent_fg))
                    .when(!selected, |d| d.bg(surface).hover(|h| h.bg(border)))
                    .child(name.to_string())
                    .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                        let pick = pick.clone();
                        v.update(cx, |app, cx| {
                            app.state.settings.font_family = pick;
                            app.state.profiles.active_mut().workspace =
                                app.state.settings.workspace();
                            atlas_core::util::config::save_profiles(&app.state.profiles);
                            atlas_core::util::config::save_settings(&app.state.settings);
                            cx.notify();
                        });
                    })
                    .into_any_element()
            })
            .collect();
        let link_button = move |id: &'static str,
                                item_icon: AtlasIcon,
                                label: &'static str,
                                url: &'static str| {
            div()
                .id(id)
                .px_2()
                .py_1()
                .rounded(px(crate::ui::RADIUS))
                .border_1()
                .border_color(border)
                .text_sm()
                .text_color(fg)
                .cursor_pointer()
                .hover(|h| h.bg(border))
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(icon(item_icon, 13.0, fg))
                        .child(label),
                )
                .on_mouse_down(MouseButton::Left, move |_ev, _w, _cx| {
                    let _ = open::that(url);
                })
        };

        let sidebar_items: Vec<AnyElement> = [
            ("General", AtlasIcon::Gear, show_general),
            ("Behavior", AtlasIcon::Lightning, show_behavior),
            ("Appearance", AtlasIcon::Palette, show_theme),
            ("Layout", AtlasIcon::SidebarSimple, show_layout),
            ("Sizes", AtlasIcon::ArrowsLeftRight, show_sizes),
            ("About", AtlasIcon::House, show_about),
        ]
        .into_iter()
        .enumerate()
        .filter_map(|(idx, (label, item_icon, visible))| {
            if !show_all && !visible {
                return None;
            }
            let selected = show_all && active_section == idx;
            let v = view.clone();
            Some(
                h_flex()
                    .id(SharedString::from(format!("settings-section-{idx}")))
                    .w_full()
                    .gap_2()
                    .items_center()
                    .px_2()
                    .py_1()
                    .rounded(px(crate::ui::RADIUS))
                    .cursor_pointer()
                    .text_sm()
                    .when(selected, |row| row.bg(accent).text_color(accent_fg))
                    .when(!selected, |row| {
                        row.text_color(if show_all { fg } else { muted })
                            .hover(|h| h.bg(border))
                    })
                    .child(icon(
                        item_icon,
                        14.0,
                        if selected { accent_fg } else { muted },
                    ))
                    .child(label)
                    .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                        v.update(cx, |app, cx| {
                            app.settings_section = idx;
                            if let Some(input) = app.settings_input.clone() {
                                input.update(cx, |state, cx| state.set_value("", window, cx));
                            }
                            cx.notify();
                        });
                    })
                    .into_any_element(),
            )
        })
        .collect();

        let content = v_flex()
            .id("settings-content")
            .size_full()
            .flex_1()
            .min_h(px(0.))
            .px_4()
            .py_3()
            .gap_2()
            .overflow_y_scroll()
            .track_scroll(&self.settings_scroll)
            .when(show_general, |panel| {
                panel
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(rust_i18n::t!("settings_general").to_string()),
                    )
                    .when(language_row, |panel| {
                        panel.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(rust_i18n::t!("settings_language").to_string()),
                                )
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .child({
                                            let is_selected = s.language.is_none();
                                            div()
                                                .cursor_pointer()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .bg(if is_selected { accent } else { surface })
                                                .text_color(if is_selected {
                                                    accent_fg
                                                } else {
                                                    fg
                                                })
                                                .child("Auto")
                                                .on_mouse_down(MouseButton::Left, {
                                                    let view = view.clone();
                                                    move |_ev, _w, cx| {
                                                        view.update(cx, |app, cx| {
                                                            let mut settings =
                                                                app.state.settings.clone();
                                                            settings.language = None;
                                                            atlas_core::util::config::save_settings(
                                                                &settings,
                                                            );
                                                            app.state.settings = settings;
                                                            rust_i18n::set_locale("en"); // auto fallback for now
                                                            cx.notify();
                                                        })
                                                    }
                                                })
                                        })
                                        .child({
                                            let is_selected = s.language.as_deref() == Some("en");
                                            div()
                                                .cursor_pointer()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .bg(if is_selected { accent } else { surface })
                                                .text_color(if is_selected {
                                                    accent_fg
                                                } else {
                                                    fg
                                                })
                                                .child("EN")
                                                .on_mouse_down(MouseButton::Left, {
                                                    let view = view.clone();
                                                    move |_ev, _w, cx| {
                                                        view.update(cx, |app, cx| {
                                                            let mut settings =
                                                                app.state.settings.clone();
                                                            settings.language =
                                                                Some("en".to_string());
                                                            atlas_core::util::config::save_settings(
                                                                &settings,
                                                            );
                                                            app.state.settings = settings;
                                                            rust_i18n::set_locale("en");
                                                            cx.notify();
                                                        })
                                                    }
                                                })
                                        })
                                        .child({
                                            let is_selected = s.language.as_deref() == Some("es");
                                            div()
                                                .cursor_pointer()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .bg(if is_selected { accent } else { surface })
                                                .text_color(if is_selected {
                                                    accent_fg
                                                } else {
                                                    fg
                                                })
                                                .child("ES")
                                                .on_mouse_down(MouseButton::Left, {
                                                    let view = view.clone();
                                                    move |_ev, _w, cx| {
                                                        view.update(cx, |app, cx| {
                                                            let mut settings =
                                                                app.state.settings.clone();
                                                            settings.language =
                                                                Some("es".to_string());
                                                            atlas_core::util::config::save_settings(
                                                                &settings,
                                                            );
                                                            app.state.settings = settings;
                                                            rust_i18n::set_locale("es");
                                                            cx.notify();
                                                        })
                                                    }
                                                })
                                        }),
                                ),
                        )
                    })
                    .when(zoom_row, |panel| {
                        panel.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(rust_i18n::t!("settings_zoom").to_string()),
                                )
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .items_center()
                                        .child(
                                            div()
                                                .cursor_pointer()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .bg(surface)
                                                .text_color(fg)
                                                .child("-")
                                                .on_mouse_down(MouseButton::Left, {
                                                    let view = view.clone();
                                                    move |_ev, _w, cx| {
                                                        view.update(cx, |app, cx| {
                                                            app.process_actions(vec![
                                                                Action::ZoomOut,
                                                            ]);
                                                            cx.notify();
                                                        });
                                                    }
                                                }),
                                        )
                                        .child(
                                            div()
                                                .cursor_pointer()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .bg(surface)
                                                .text_color(fg)
                                                .child(format!("{:.0}%", s.global_zoom * 100.0))
                                                .on_mouse_down(MouseButton::Left, {
                                                    let view = view.clone();
                                                    move |_ev, _w, cx| {
                                                        view.update(cx, |app, cx| {
                                                            app.process_actions(vec![
                                                                Action::ZoomReset,
                                                            ]);
                                                            cx.notify();
                                                        });
                                                    }
                                                }),
                                        )
                                        .child(
                                            div()
                                                .cursor_pointer()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .bg(surface)
                                                .text_color(fg)
                                                .child("+")
                                                .on_mouse_down(MouseButton::Left, {
                                                    let view = view.clone();
                                                    move |_ev, _w, cx| {
                                                        view.update(cx, |app, cx| {
                                                            app.process_actions(vec![
                                                                Action::ZoomIn,
                                                            ]);
                                                            cx.notify();
                                                        });
                                                    }
                                                }),
                                        ),
                                ),
                        )
                    })
            })
            .when(show_behavior, |panel| {
                panel
                    .when(eco_row, |panel| {
                        panel.child(toggle(
                            "Eco Mode (Reduce RAM when inactive)",
                            s.eco_mode,
                            "set-eco-mode",
                            |app| {
                                app.state.settings.eco_mode = !app.state.settings.eco_mode;
                                atlas_core::util::config::save_settings(&app.state.settings);
                            },
                            view.clone(),
                        ))
                    })
                    .when(hidden_row, |panel| {
                        panel.child(toggle(
                            &rust_i18n::t!("settings_show_hidden"),
                            s.show_hidden,
                            "set-hidden",
                            |app| app.process_actions(vec![Action::ToggleHidden]),
                            view.clone(),
                        ))
                    })
                    .when(sidebar_row, |panel| {
                        panel.child(toggle(
                            &rust_i18n::t!("settings_show_sidebar"),
                            s.show_sidebar,
                            "set-sidebar",
                            |app| app.process_actions(vec![Action::ToggleSidebar]),
                            view.clone(),
                        ))
                    })
                    .when(inspector_row, |panel| {
                        panel.child(toggle(
                            &rust_i18n::t!("settings_show_inspector"),
                            s.show_inspector,
                            "set-inspector",
                            |app| {
                                app.state.settings.show_inspector =
                                    !app.state.settings.show_inspector
                            },
                            view.clone(),
                        ))
                    })
                    .when(animations_row, |panel| {
                        panel.child(toggle(
                            &rust_i18n::t!("settings_disable_animations"),
                            s.disable_animations,
                            "set-disable-anim",
                            |app| {
                                app.state.settings.disable_animations =
                                    !app.state.settings.disable_animations;
                            },
                            view.clone(),
                        ))
                    })
                    .when(autosave_row, |panel| {
                        panel.child(toggle(
                            &rust_i18n::t!("settings_autosave_session"),
                            s.autosave_session,
                            "set-autosave-session",
                            |app| {
                                app.state.settings.autosave_session =
                                    !app.state.settings.autosave_session;
                                atlas_core::util::config::save_settings(&app.state.settings);
                            },
                            view.clone(),
                        ))
                    })
                    .when(dim_panes_row, |panel| {
                        panel.child(toggle(
                            &rust_i18n::t!("settings_dim_inactive_panes"),
                            s.dim_inactive_panes,
                            "set-dim-inactive-panes",
                            |app| {
                                app.state.settings.dim_inactive_panes =
                                    !app.state.settings.dim_inactive_panes;
                                atlas_core::util::config::save_settings(&app.state.settings);
                            },
                            view.clone(),
                        ))
                    })
                    .when(smooth_row, |panel| {
                        panel.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .justify_between()
                                .child(
                                    div().text_sm().child(
                                        rust_i18n::t!("settings_scroll_smoothness").to_string(),
                                    ),
                                )
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .items_center()
                                        .child(
                                            div()
                                                .cursor_pointer()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .bg(surface)
                                                .text_color(fg)
                                                .child("-")
                                                .on_mouse_down(MouseButton::Left, {
                                                    let view = view.clone();
                                                    move |_ev, _w, cx| {
                                                        view.update(cx, |app, cx| {
                                                            set_scroll_smoothness(app, -0.1);
                                                            cx.notify();
                                                        });
                                                    }
                                                }),
                                        )
                                        .child(
                                            div()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .bg(surface)
                                                .text_color(fg)
                                                .child(format!(
                                                    "{:.0}%",
                                                    s.scroll_smoothness * 100.0
                                                )),
                                        )
                                        .child(
                                            div()
                                                .cursor_pointer()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .bg(surface)
                                                .text_color(fg)
                                                .child("+")
                                                .on_mouse_down(MouseButton::Left, {
                                                    let view = view.clone();
                                                    move |_ev, _w, cx| {
                                                        view.update(cx, |app, cx| {
                                                            set_scroll_smoothness(app, 0.1);
                                                            cx.notify();
                                                        });
                                                    }
                                                }),
                                        ),
                                ),
                        )
                    })
            })
            .when(show_theme, |panel| {
                panel
                    .when(titlebar_row, |panel| {
                        panel.child(toggle(
                            &rust_i18n::t!("settings_show_titlebar"),
                            s.show_titlebar,
                            "set-show-titlebar",
                            |app| {
                                app.state.settings.show_titlebar = !app.state.settings.show_titlebar;
                                atlas_core::util::config::save_settings(&app.state.settings);
                            },
                            view.clone(),
                        ))
                    })
                    .when(glass_opacity_row, |panel| {
                        let glass_pct = (s.glass_opacity * 100.0).round();
                        panel.child(
                            h_flex()
                                .id("setting-glass-opacity")
                                .w_full()
                                .justify_between()
                                .items_center()
                                .py_1()
                                .child(
                                    v_flex()
                                        .gap(px(2.))
                                        .child(rust_i18n::t!("settings_glass_opacity").to_string())
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(muted)
                                                .child(rust_i18n::t!("settings_glass_opacity_desc").to_string()),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .items_center()
                                        .child(
                                            div()
                                                .cursor_pointer()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .bg(surface)
                                                .text_color(fg)
                                                .child("−")
                                                .on_mouse_down(MouseButton::Left, {
                                                    let view = view.clone();
                                                    move |_ev, _w, cx| {
                                                        view.update(cx, |this, cx| {
                                                            dec_glass_opacity(this);
                                                            cx.notify();
                                                        });
                                                    }
                                                }),
                                        )
                                        .child(
                                            div()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .bg(surface)
                                                .text_color(fg)
                                                .child(format!("{glass_pct:.0}%")),
                                        )
                                        .child(
                                            div()
                                                .cursor_pointer()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .bg(surface)
                                                .text_color(fg)
                                                .child("+")
                                                .on_mouse_down(MouseButton::Left, {
                                                    let view = view.clone();
                                                    move |_ev, _w, cx| {
                                                        view.update(cx, |this, cx| {
                                                            inc_glass_opacity(this);
                                                            cx.notify();
                                                        });
                                                    }
                                                }),
                                        ),
                                ),
                        )
                    })
            })
            .when(show_layout, |panel| {
                panel
                    .when(placement_row, |panel| {
                        panel.child(
                            div().text_xs().text_color(muted).child(
                                rust_i18n::t!(
                                    "settings_inspector_placement",
                                    placement = placement
                                )
                                .to_string(),
                            ),
                        )
                    })
                    .when(placement_row, |panel| {
                        panel.child(
                            h_flex()
                                .gap_2()
                                .text_xs()
                                .child(segment(
                                    "set-placement-global",
                                    &rust_i18n::t!("settings_placement_global"),
                                    inspector_placement == InspectorPlacement::Global,
                                    |app| {
                                        app.state.settings.inspector_placement =
                                            InspectorPlacement::Global
                                    },
                                    view.clone(),
                                ))
                                .child(segment(
                                    "set-placement-tab",
                                    &rust_i18n::t!("settings_placement_tab"),
                                    inspector_placement == InspectorPlacement::ActiveTab,
                                    |app| {
                                        app.state.settings.inspector_placement =
                                            InspectorPlacement::ActiveTab;
                                    },
                                    view.clone(),
                                )),
                        )
                    })
                    .when(view_row, |panel| {
                        panel.child(
                            div().text_xs().text_color(muted).child(
                                rust_i18n::t!("settings_default_view", view_mode = view_mode)
                                    .to_string(),
                            ),
                        )
                    })
                    .when(view_row, |panel| {
                        panel.child(
                            h_flex()
                                .gap_2()
                                .text_xs()
                                .flex_wrap()
                                .child(segment(
                                    "set-view-details",
                                    &rust_i18n::t!("settings_view_details_btn"),
                                    default_view_mode == ViewMode::Details,
                                    |app| app.state.settings.view_mode = ViewMode::Details,
                                    view.clone(),
                                ))
                                .child(segment(
                                    "set-view-grid",
                                    &rust_i18n::t!("settings_view_grid_btn"),
                                    default_view_mode == ViewMode::Grid,
                                    |app| app.state.settings.view_mode = ViewMode::Grid,
                                    view.clone(),
                                ))
                                .child(segment(
                                    "set-view-list",
                                    &rust_i18n::t!("settings_view_list_btn"),
                                    default_view_mode == ViewMode::List,
                                    |app| app.state.settings.view_mode = ViewMode::List,
                                    view.clone(),
                                ))
                                .child(segment(
                                    "set-view-miller",
                                    &rust_i18n::t!("settings_view_miller_btn"),
                                    default_view_mode == ViewMode::Miller,
                                    |app| app.state.settings.view_mode = ViewMode::Miller,
                                    view.clone(),
                                )),
                        )
                    })
            })
            .when(show_sizes, |panel| {
                panel
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(rust_i18n::t!("settings_sizes").to_string()),
                    )
                    .when(sidebar_width_row, |panel| {
                        panel.child(stepper(
                            "set-sidebar-width",
                            &rust_i18n::t!("settings_sidebar"),
                            sidebar_width,
                            " px",
                            |app| {
                                app.state.settings.sidebar_width =
                                    (app.state.settings.sidebar_width - 20.0).clamp(160.0, 420.0);
                            },
                            |app| {
                                app.state.settings.sidebar_width =
                                    (app.state.settings.sidebar_width + 20.0).clamp(160.0, 420.0);
                            },
                            view.clone(),
                        ))
                    })
                    .when(inspector_width_row, |panel| {
                        panel.child(stepper(
                            "set-inspector-width",
                            &rust_i18n::t!("settings_inspector"),
                            inspector_width,
                            " px",
                            |app| {
                                app.state.settings.inspector_width =
                                    (app.state.settings.inspector_width - 20.0)
                                        .clamp(180.0, 2000.0);
                            },
                            |app| {
                                app.state.settings.inspector_width =
                                    (app.state.settings.inspector_width + 20.0)
                                        .clamp(180.0, 2000.0);
                            },
                            view.clone(),
                        ))
                    })
                    .when(font_row, |panel| {
                        panel.child(stepper(
                            "set-font-size",
                            &rust_i18n::t!("settings_font"),
                            font_size,
                            " px",
                            |app| {
                                app.state.settings.font_size =
                                    (app.state.settings.font_size - 1.0).clamp(10.0, 24.0);
                            },
                            |app| {
                                app.state.settings.font_size =
                                    (app.state.settings.font_size + 1.0).clamp(10.0, 24.0);
                            },
                            view.clone(),
                        ))
                    })
                    .when(font_row, |panel| {
                        panel.child(
                            h_flex()
                                .gap_2()
                                .text_xs()
                                .flex_wrap()
                                .children(font_buttons),
                        )
                    })
                    .when(thumbnail_row, |panel| {
                        panel.child(stepper(
                            "set-thumbnail-size",
                            &rust_i18n::t!("settings_thumbnails"),
                            thumbnail_size,
                            " px",
                            |app| {
                                app.state.settings.thumbnail_size =
                                    (app.state.settings.thumbnail_size - 16.0).clamp(64.0, 256.0);
                            },
                            |app| {
                                app.state.settings.thumbnail_size =
                                    (app.state.settings.thumbnail_size + 16.0).clamp(64.0, 256.0);
                            },
                            view,
                        ))
                    })
            })
            .when(show_about, |panel| {
                panel
                    .when(about_row, |panel| {
                        panel.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(rust_i18n::t!("settings_about").to_string()),
                        )
                    })
                    .when(about_row, |panel| {
                        panel.child(
                            v_flex()
                                .gap_2()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .flex_wrap()
                                        .child(link_button(
                                            "about-community-link",
                                            AtlasIcon::DiscordLogo,
                                            "Community",
                                            "https://discord.gg/aeAvQ27bgG",
                                        ))
                                        .child(link_button(
                                            "about-support-link",
                                            AtlasIcon::Lifebuoy,
                                            "Support",
                                            "https://discord.gg/aeAvQ27bgG",
                                        ))
                                        .child(link_button(
                                            "about-web-link",
                                            AtlasIcon::GlobeSimple,
                                            "Website",
                                            "https://atlasnative.com/",
                                        )),
                                )
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .items_center()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(rust_i18n::t!("settings_icons_by").to_string())
                                        .child(link_button(
                                            "about-icons8-link",
                                            AtlasIcon::GlobeSimple,
                                            "Icons8",
                                            "https://icons8.com",
                                        )),
                                ),
                        )
                    })
            })
            .when(
                !show_general
                    && !show_behavior
                    && !show_layout
                    && !show_theme
                    && !show_sizes
                    && !show_about,
                |panel| {
                    panel.child(
                        div()
                            .py_4()
                            .text_center()
                            .text_sm()
                            .text_color(muted)
                            .child("No settings match your search"),
                    )
                },
            );

        let viewport_h = f32::from(window.bounds().size.height);
        let panel_h = (viewport_h * 0.52).clamp(340.0, 460.0);

        let panel = h_flex()
            .id("settings-root")
            .size_full()
            .items_stretch()
            .child(
                v_flex()
                    .w(px(164.))
                    .h_full()
                    .gap_1()
                    .p_3()
                    .border_r_1()
                    .border_color(border)
                    .bg(surface)
                    .child(
                        div()
                            .pb_2()
                            .text_lg()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(rust_i18n::t!("settings_title").to_string()),
                    )
                    .children(sidebar_items),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h(px(0.))
                    .h_full()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .items_center()
                            .border_b_1()
                            .border_color(border)
                            .px_4()
                            .py_3()
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .border_1()
                                    .border_color(border)
                                    .rounded(px(crate::ui::RADIUS))
                                    .bg(surface)
                                    .px_2()
                                    .py_1()
                                    .child(icon(AtlasIcon::Search, 14.0, muted))
                                    .child(crate::ui::text_input::flat(&input_entity)),
                            ),
                    )
                    .child(content),
            );

        // Settings has no dedicated close action; closing sets the flag directly.
        let view_close = cx.entity();
        let scrim_focus = focus;
        self.overlay_scrim_custom(
            "scrim-settings",
            view_close,
            move |this| {
                this.state.overlay.show_settings = false;
                this.settings_input = None;
            },
            scrim_focus,
            // Phase I: subtle fade-in for the settings panel.
            div()
                .id("settings-panel")
                .w(px(720.))
                .h(px(panel_h))
                .bg(popover)
                .text_color(fg)
                .border_1()
                .border_color(border)
                .rounded_lg()
                .shadow_lg()
                .overflow_hidden()
                .occlude()
                .relative()
                .child(panel)
                .vertical_scrollbar(&self.settings_scroll)
                .with_animation(
                    "settings-fade",
                    motion::animation(motion::PANEL_MS),
                    gpui::Styled::opacity,
                )
                .into_any_element(),
            cx,
        )
    }

    // ── batch rename ─────────────────────────────────────────────────────────
}

/// Adjust the global scroll-smoothness setting by `delta` (clamped 0..1) and persist.
fn set_scroll_smoothness(app: &mut App, delta: f32) {
    app.state.settings.scroll_smoothness =
        (app.state.settings.scroll_smoothness + delta).clamp(0.0, 1.0);
    atlas_core::util::config::save_settings(&app.state.settings);
}

fn set_glass_opacity(app: &mut App, delta: f32) {
    let raw = (app.state.settings.glass_opacity + delta) * 20.0;
    app.state.settings.glass_opacity = (raw.round() / 20.0).clamp(0.20, 1.0);
    app.theme_dirty = true;
    atlas_core::util::config::save_settings(&app.state.settings);
}

fn dec_glass_opacity(app: &mut App) {
    set_glass_opacity(app, -0.05);
}

fn inc_glass_opacity(app: &mut App) {
    set_glass_opacity(app, 0.05);
}

fn setting_matches(query: &str, terms: &[&str]) -> bool {
    let query = query.trim();
    !query.is_empty()
        && terms.iter().any(|term| {
            let term = term.to_lowercase();
            term.contains(query) || query.contains(&term) || fuzzy_subsequence(query, &term)
        })
}

fn fuzzy_subsequence(query: &str, term: &str) -> bool {
    let mut chars = query.chars();
    let Some(mut needle) = chars.next() else {
        return true;
    };
    for c in term.chars() {
        if c == needle {
            match chars.next() {
                Some(next) => needle = next,
                None => return true,
            }
        }
    }
    false
}
