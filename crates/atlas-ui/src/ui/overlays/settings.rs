//! Settings overlay: language, zoom, toggles, view mode, theme picker, sizes.
#![allow(clippy::too_many_lines)] // reason: cohesive overlay kept whole

use crate::app::App;
use crate::input::action::Action;
use crate::ui::motion;
use atlas_model::settings::{InspectorPlacement, ViewMode};
use gpui::prelude::*;
use gpui::AnimationExt;
use gpui::{div, px, AnyElement, Context, Entity, MouseButton, SharedString};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex, ActiveTheme};

impl App {
    pub(crate) fn render_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
        // Theme picker data: the registered theme names and the active profile's pick.
        let active_theme = self.state.profiles.active().theme.clone();
        let theme_names: Vec<String> = gpui_component::ThemeRegistry::global(cx)
            .sorted_themes()
            .iter()
            .map(|c| c.name.to_string())
            .collect();
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

        // Theme picker: one wrap-flow button per registered theme (bundled + user).
        let theme_buttons: Vec<AnyElement> = theme_names
            .iter()
            .map(|name| {
                let selected = *name == active_theme;
                let pick = name.clone();
                let v = view.clone();
                div()
                    .id(SharedString::from(format!("set-theme-{name}")))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(if selected { accent } else { border })
                    .cursor_pointer()
                    .when(selected, |d| d.bg(accent))
                    .when(!selected, |d| d.hover(|h| h.bg(border)))
                    .child(name.clone())
                    .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                        let pick = pick.clone();
                        v.update(cx, |this, cx| {
                            this.set_theme(pick);
                            cx.notify();
                        });
                    })
                    .into_any_element()
            })
            .collect();
        let theme_section = h_flex()
            .gap_2()
            .text_xs()
            .flex_wrap()
            .children(theme_buttons);
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

        let panel = v_flex()
            .id("settings-content")
            .size_full()
            .p_4()
            .gap_2()
            .overflow_y_scroll()
            .track_scroll(&self.settings_scroll)
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(rust_i18n::t!("settings_title").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("settings_general").to_string()),
            )
            .child(
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
                                    .text_color(if is_selected { accent_fg } else { fg })
                                    .child("Auto")
                                    .on_mouse_down(MouseButton::Left, {
                                        let view = view.clone();
                                        move |_ev, _w, cx| {
                                            view.update(cx, |app, cx| {
                                                let mut settings = app.state.settings.clone();
                                                settings.language = None;
                                                atlas_core::util::config::save_settings(&settings);
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
                                    .text_color(if is_selected { accent_fg } else { fg })
                                    .child("EN")
                                    .on_mouse_down(MouseButton::Left, {
                                        let view = view.clone();
                                        move |_ev, _w, cx| {
                                            view.update(cx, |app, cx| {
                                                let mut settings = app.state.settings.clone();
                                                settings.language = Some("en".to_string());
                                                atlas_core::util::config::save_settings(&settings);
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
                                    .text_color(if is_selected { accent_fg } else { fg })
                                    .child("ES")
                                    .on_mouse_down(MouseButton::Left, {
                                        let view = view.clone();
                                        move |_ev, _w, cx| {
                                            view.update(cx, |app, cx| {
                                                let mut settings = app.state.settings.clone();
                                                settings.language = Some("es".to_string());
                                                atlas_core::util::config::save_settings(&settings);
                                                app.state.settings = settings;
                                                rust_i18n::set_locale("es");
                                                cx.notify();
                                            })
                                        }
                                    })
                            }),
                    ),
            )
            .child(
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
                                                app.process_actions(vec![Action::ZoomOut]);
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
                                                app.process_actions(vec![Action::ZoomReset]);
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
                                                app.process_actions(vec![Action::ZoomIn]);
                                                cx.notify();
                                            });
                                        }
                                    }),
                            ),
                    ),
            )
            .child(toggle(
                "Eco Mode (Reduce RAM when inactive)",
                s.eco_mode,
                "set-eco-mode",
                |app| {
                    app.state.settings.eco_mode = !app.state.settings.eco_mode;
                    atlas_core::util::config::save_settings(&app.state.settings);
                },
                view.clone(),
            ))
            .child(toggle(
                &rust_i18n::t!("settings_show_hidden"),
                s.show_hidden,
                "set-hidden",
                |app| app.process_actions(vec![Action::ToggleHidden]),
                view.clone(),
            ))
            .child(toggle(
                &rust_i18n::t!("settings_show_sidebar"),
                s.show_sidebar,
                "set-sidebar",
                |app| app.process_actions(vec![Action::ToggleSidebar]),
                view.clone(),
            ))
            .child(toggle(
                &rust_i18n::t!("settings_show_inspector"),
                s.show_inspector,
                "set-inspector",
                |app| app.state.settings.show_inspector = !app.state.settings.show_inspector,
                view.clone(),
            ))
            .child(toggle(
                &rust_i18n::t!("settings_disable_animations"),
                s.disable_animations,
                "set-disable-anim",
                |app| {
                    app.state.settings.disable_animations = !app.state.settings.disable_animations;
                },
                view.clone(),
            ))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings_scroll_smoothness").to_string()),
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
                                    .child(format!("{:.0}%", s.scroll_smoothness * 100.0)),
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
            .child(toggle(
                &rust_i18n::t!("settings_mica"),
                s.mica,
                "set-mica",
                |app| {
                    app.state.settings.mica = !app.state.settings.mica;
                    app.state.profiles.active_mut().workspace = app.state.settings.workspace();
                    // Re-apply so the glass swap + window backdrop flip happen this frame
                    // (root.rs consumes `theme_dirty` before reading colors).
                    app.theme_dirty = true;
                    atlas_core::util::config::save_profiles(&app.state.profiles);
                    atlas_core::util::config::save_settings(&app.state.settings);
                },
                view.clone(),
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("settings_mica_desc").to_string()),
            )
            .child(div().text_xs().text_color(muted).child(
                rust_i18n::t!("settings_inspector_placement", placement = placement).to_string(),
            ))
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .child(segment(
                        "set-placement-global",
                        &rust_i18n::t!("settings_placement_global"),
                        inspector_placement == InspectorPlacement::Global,
                        |app| app.state.settings.inspector_placement = InspectorPlacement::Global,
                        view.clone(),
                    ))
                    .child(segment(
                        "set-placement-tab",
                        &rust_i18n::t!("settings_placement_tab"),
                        inspector_placement == InspectorPlacement::ActiveTab,
                        |app| {
                            app.state.settings.inspector_placement = InspectorPlacement::ActiveTab;
                        },
                        view.clone(),
                    )),
            )
            .child(
                div().text_xs().text_color(muted).child(
                    rust_i18n::t!("settings_default_view", view_mode = view_mode).to_string(),
                ),
            )
            .child(
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
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("settings_theme", theme = active_theme).to_string()),
            )
            .child(theme_section)
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("settings_sizes").to_string()),
            )
            .child(stepper(
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
            .child(stepper(
                "set-inspector-width",
                &rust_i18n::t!("settings_inspector"),
                inspector_width,
                " px",
                |app| {
                    app.state.settings.inspector_width =
                        (app.state.settings.inspector_width - 20.0).clamp(180.0, 520.0);
                },
                |app| {
                    app.state.settings.inspector_width =
                        (app.state.settings.inspector_width + 20.0).clamp(180.0, 520.0);
                },
                view.clone(),
            ))
            .child(stepper(
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
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .flex_wrap()
                    .children(font_buttons),
            )
            .child(stepper(
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
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("settings_about").to_string()),
            )
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("settings_icons_by").to_string())
                    .child(
                        div()
                            .id("about-icons8-link")
                            .text_color(accent)
                            .cursor_pointer()
                            .child("Icons8")
                            .on_mouse_down(MouseButton::Left, move |_ev, _w, _cx| {
                                let _ = open::that("https://icons8.com");
                            }),
                    ),
            );

        // Settings has no dedicated close action; closing sets the flag directly.
        let view_close = cx.entity();
        let scrim_focus = focus;
        self.overlay_scrim_custom(
            "scrim-settings",
            view_close,
            move |this| this.state.overlay.show_settings = false,
            scrim_focus,
            // FASE I: subtle fade-in for the settings panel.
            div()
                .id("settings-panel")
                .w(px(480.))
                .max_h(px(560.))
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
