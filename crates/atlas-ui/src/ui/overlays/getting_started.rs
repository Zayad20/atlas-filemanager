use crate::app::App;
use crate::input::action::Action;
use crate::ui::icons::{atlas_logo, icon, AtlasIcon};
use atlas_model::settings::ViewMode;
use gpui::prelude::*;
use gpui::ColorExt as _;
use gpui::{
    deferred, div, hsla, linear_color_stop, linear_gradient, point, px, AnyElement, BoxShadow,
    Context, Entity, FocusHandle, Focusable, KeyDownEvent, MouseButton, SharedString, Window,
};
use gpui_component::input::InputState;
use gpui_component::{h_flex, scroll::ScrollableElement, v_flex};

const LOAD_MS: u128 = 5000;
const WELCOME_MS: u128 = 2600;
fn setup_bg() -> gpui::Hsla {
    hsla(0., 0., 0.08, 1.0)
}
fn setup_panel() -> gpui::Hsla {
    hsla(0., 0., 0.13, 0.96)
}
fn setup_tile() -> gpui::Hsla {
    hsla(0., 0., 0.19, 0.90)
}
fn setup_fg() -> gpui::Hsla {
    hsla(0., 0., 0.94, 1.0)
}
fn setup_muted() -> gpui::Hsla {
    hsla(0., 0., 0.68, 1.0)
}
fn setup_border() -> gpui::Hsla {
    hsla(0., 0., 0.36, 1.0)
}
fn setup_accent() -> gpui::Hsla {
    hsla(0., 0., 0.90, 1.0)
}

impl App {
    pub(crate) fn render_getting_started(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let loading_since = self.state.overlay.getting_started_loading_since;
        if let Some(started) = loading_since {
            if started.elapsed().as_millis() >= LOAD_MS {
                self.finish_getting_started(cx);
                return div().into_any_element();
            }
            window.request_animation_frame();
        }

        let view = cx.entity();
        let focus = self.overlay_focus(cx);
        let step = self.state.overlay.getting_started_step.min(2);
        let loading = loading_since.is_some();
        let reveal = loading_since
            .map(|started| (started.elapsed().as_millis() as f32 / LOAD_MS as f32).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        let (fg, muted, border) = (setup_fg(), setup_muted(), setup_border());

        let content = match step {
            0 => self.setup_welcome_page(window, cx, fg, muted, border),
            1 => self.setup_theme_page(cx, fg, muted, border),
            _ => self.setup_workspace_page(fg, muted, border, cx),
        };

        let close_view = view.clone();
        let primary_view = view.clone();
        let back_view = view.clone();
        let guide_view = view.clone();
        let primary_label = if step == 2 {
            rust_i18n::t!("getting_started_primary").to_string()
        } else {
            rust_i18n::t!("getting_started_next").to_string()
        };

        let loader = loading.then(|| {
            let pulse = (reveal * std::f32::consts::TAU * 5.0).sin() * 0.04;
            div()
                .absolute()
                .inset_0()
                .bg(hsla(0., 0., 0.04, (0.88 * (1.0 - reveal)).max(0.0)))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    v_flex()
                        .items_center()
                        .gap_4()
                        .opacity(0.72 + 0.22 * (1.0 - reveal))
                        .child(
                            div()
                                .scale(0.98 + pulse)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(atlas_logo(74.0)),
                        )
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(rust_i18n::t!("getting_started_loading").to_string()),
                        ),
                )
        });

        let page = div()
            .id("getting-started-fullscreen")
            .absolute()
            .inset_0()
            .size_full()
            .bg(setup_bg())
            .text_color(fg)
            .track_focus(&focus)
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| {
                if ev.keystroke.key == "escape" {
                    this.submit_getting_started_profile_name(cx);
                    this.process_actions([Action::CloseGettingStarted]);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(div().absolute().inset_0().bg(linear_gradient(
                135.,
                linear_color_stop(hsla(0., 0., 0.24, 0.30), 0.0),
                linear_color_stop(hsla(0., 0., 0.08, 0.56), 1.0),
            )))
            .child(div().absolute().inset_0().bg(linear_gradient(
                35.,
                linear_color_stop(hsla(0., 0., 0.34, 0.10), 0.0),
                linear_color_stop(hsla(0., 0., 0.04, 0.72), 1.0),
            )))
            .child(
                v_flex()
                    .absolute()
                    .inset_0()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .px_6()
                    .children((!loading).then(|| {
                        v_flex()
                            .id("getting-started-card")
                            .w(px(760.))
                            .max_w(px(760.))
                            .min_h(px(520.))
                            .p_6()
                            .gap_5()
                            .rounded(px(22.))
                            .border_1()
                            .border_color(border.opacity(0.70))
                            .bg(setup_panel())
                            .shadow(soft_shadow())
                            .occlude()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .child(step_dots(step, muted))
                                    .child(
                                        div()
                                            .id("getting-started-close")
                                            .size(px(30.))
                                            .rounded(px(8.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_color(muted)
                                            .cursor_pointer()
                                            .hover(move |h| h.bg(setup_tile()).text_color(fg))
                                            .child(icon(AtlasIcon::X, 13., muted))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                move |_ev, _w, cx| {
                                                    close_view.update(cx, |this, cx| {
                                                        this.submit_getting_started_profile_name(cx);
                                                        this.process_actions([
                                                            Action::CloseGettingStarted,
                                                        ]);
                                                        cx.notify();
                                                    });
                                                },
                                            ),
                                    ),
                            )
                            .child(content)
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .child(
                                        div()
                                            .id("getting-started-back")
                                            .px_3()
                                            .py_2()
                                            .rounded(px(8.))
                                            .text_sm()
                                            .text_color(muted)
                                            .cursor_pointer()
                                            .when(step == 0, |d| d.opacity(0.0))
                                            .hover(move |h| h.bg(setup_tile()))
                                            .child(rust_i18n::t!("overlay_back").to_string())
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                move |_ev, _w, cx| {
                                                    back_view.update(cx, |this, cx| {
                                                        this.state.overlay.getting_started_step =
                                                            this.state
                                                                .overlay
                                                                .getting_started_step
                                                                .saturating_sub(1);
                                                        cx.notify();
                                                    });
                                                },
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .id("getting-started-skip")
                                                    .px_3()
                                                    .py_2()
                                                    .rounded(px(8.))
                                                    .text_sm()
                                                    .text_color(muted)
                                                    .cursor_pointer()
                                                    .hover(move |h| h.text_color(fg))
                                                    .child(
                                                        rust_i18n::t!("getting_started_secondary")
                                                            .to_string(),
                                                    )
                                                    .on_mouse_down(MouseButton::Left, {
                                                        let view = view.clone();
                                                        move |_ev, _w, cx| {
                                                            view.update(cx, |this, cx| {
                                                                this.submit_getting_started_profile_name(cx);
                                                                this.process_actions([
                                                                    Action::CloseGettingStarted,
                                                                ]);
                                                                cx.notify();
                                                            });
                                                        }
                                                    }),
                                            )
                                            .children((step == 2).then(|| {
                                                div()
                                                    .id("getting-started-guide")
                                                    .px_4()
                                                    .py_2()
                                                    .rounded(px(8.))
                                                    .border_1()
                                                    .border_color(border.opacity(0.70))
                                                    .text_sm()
                                                    .text_color(fg)
                                                    .cursor_pointer()
                                                    .hover(move |h| h.bg(setup_tile()))
                                                    .child(
                                                        rust_i18n::t!("getting_started_guide_btn")
                                                            .to_string(),
                                                    )
                                                    .on_mouse_down(MouseButton::Left, {
                                                        let guide_view = guide_view.clone();
                                                        move |_ev, _w, cx| {
                                                            guide_view.update(cx, |this, cx| {
                                                                this.submit_getting_started_profile_name(cx);
                                                                this.process_actions([
                                                                    Action::CloseGettingStarted,
                                                                    Action::OpenGuide(0),
                                                                ]);
                                                                cx.notify();
                                                            });
                                                        }
                                                    })
                                            }))
                                            .child(
                                                div()
                                                    .id("getting-started-primary")
                                                    .px_4()
                                                    .py_2()
                                                    .rounded(px(8.))
                                                    .text_sm()
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .bg(setup_accent())
                                                    .text_color(hsla(0., 0., 0.07, 1.0))
                                                    .cursor_pointer()
                                                    .when(loading, |d| d.opacity(0.72))
                                                    .child(primary_label)
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        move |_ev, _w, cx| {
                                                            primary_view.update(cx, |this, cx| {
                                                                if this
                                                                    .state
                                                                    .overlay
                                                                    .getting_started_step
                                                                    >= 2
                                                                {
                                                                    this.submit_getting_started_profile_name(cx);
                                                                    this.process_actions([
                                                                        Action::StartGettingStarted,
                                                                    ]);
                                                                } else {
                                                                    if this
                                                                        .state
                                                                        .overlay
                                                                        .getting_started_step
                                                                        == 0
                                                                    {
                                                                        this.submit_getting_started_profile_name(cx);
                                                                    }
                                                                    this.state
                                                                        .overlay
                                                                        .getting_started_step += 1;
                                                                }
                                                                cx.notify();
                                                            });
                                                        },
                                                    ),
                                            ),
                                    ),
                            )
                    })),
            )
            .children(loader);

        deferred(page).into_any_element()
    }

    fn finish_getting_started(&mut self, cx: &mut Context<Self>) {
        self.submit_getting_started_profile_name(cx);
        self.mark_getting_started_seen();
        self.state.overlay.show_getting_started = false;
        self.state.overlay.getting_started_loading_since = None;
        self.state.overlay.getting_started_welcome_since = Some(std::time::Instant::now());
        self.getting_started_profile_input = None;
    }

    pub(crate) fn mark_getting_started_seen(&mut self) {
        if !self.state.settings.getting_started_seen {
            self.state.settings.getting_started_seen = true;
            atlas_core::util::config::save_settings(&self.state.settings);
        }
    }

    fn submit_getting_started_profile_name(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.getting_started_profile_input.clone() else {
            return;
        };
        let value = input.read(cx).value().trim().to_string();
        if !value.is_empty() && value != self.state.profiles.active().name {
            self.rename_active_profile(value);
        }
    }

    fn getting_started_profile_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let name = self.state.profiles.active().name.clone();
        self.getting_started_profile_input
            .get_or_insert_with(|| {
                crate::ui::text_input::single_line(
                    window,
                    cx,
                    rust_i18n::t!("getting_started_profile_placeholder").to_string(),
                    name,
                )
            })
            .clone()
    }

    pub(crate) fn getting_started_profile_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> FocusHandle {
        self.getting_started_profile_input(window, cx)
            .read(cx)
            .focus_handle(cx)
    }

    pub(crate) fn render_getting_started_welcome(
        &mut self,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(started) = self.state.overlay.getting_started_welcome_since else {
            return div().into_any_element();
        };
        let elapsed = started.elapsed().as_millis();
        if elapsed >= WELCOME_MS {
            self.state.overlay.getting_started_welcome_since = None;
            return div().into_any_element();
        }
        window.request_animation_frame();

        let t = (elapsed as f32 / WELCOME_MS as f32).clamp(0.0, 1.0);
        let fade = 1.0 - t;
        let fg = setup_fg();

        deferred(
            div()
                .id("getting-started-welcome")
                .absolute()
                .inset_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(hsla(0., 0., 0.04, 0.62 * fade))
                .child(
                    v_flex()
                        .items_center()
                        .gap_4()
                        .opacity(fade)
                        .child(atlas_logo(82.0))
                        .child(
                            div()
                                .text_3xl()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(fg)
                                .child(rust_i18n::t!("getting_started_welcome").to_string()),
                        ),
                ),
        )
        .into_any_element()
    }

    fn setup_welcome_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        fg: gpui::Hsla,
        muted: gpui::Hsla,
        border: gpui::Hsla,
    ) -> AnyElement {
        let profile_input = self.getting_started_profile_input(window, cx);
        v_flex()
            .gap_4()
            .child(
                v_flex()
                    .gap_2()
                    .items_center()
                    .child(atlas_logo(72.0))
                    .child(
                        div()
                            .text_xl()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(rust_i18n::t!("getting_started_title").to_string()),
                    )
                    .child(
                        div()
                            .max_w(px(520.))
                            .text_align(gpui::TextAlign::Center)
                            .text_sm()
                            .line_height(px(20.))
                            .text_color(muted)
                            .child(rust_i18n::t!("getting_started_subtitle").to_string()),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(muted)
                            .child(rust_i18n::t!("getting_started_profile_name").to_string()),
                    )
                    .child(crate::ui::text_input::bordered(&profile_input)),
            )
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_3()
                            .child(welcome_tile(
                                "getting-started-profile",
                                AtlasIcon::UserSwitch,
                                "getting_started_profile",
                                fg,
                                border,
                            ))
                            .child(welcome_tile(
                                "getting-started-palette",
                                AtlasIcon::Lightning,
                                "getting_started_palette",
                                fg,
                                border,
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(welcome_tile(
                                "getting-started-ext",
                                AtlasIcon::PuzzlePiece,
                                "getting_started_extensions",
                                fg,
                                border,
                            ))
                            .child(welcome_tile(
                                "getting-started-integrations",
                                AtlasIcon::TerminalWindow,
                                "getting_started_integrations",
                                fg,
                                border,
                            )),
                    ),
            )
            .into_any_element()
    }

    fn setup_theme_page(
        &self,
        cx: &mut Context<Self>,
        fg: gpui::Hsla,
        muted: gpui::Hsla,
        border: gpui::Hsla,
    ) -> AnyElement {
        let active = self.state.profiles.active().theme.clone();
        let view = cx.entity();
        let themes = gpui_component::ThemeRegistry::global(cx)
            .sorted_themes()
            .iter()
            .filter(|cfg| crate::app::theme::theme_is_available(cfg.name.as_ref()))
            .map(|cfg| {
                let name = cfg.name.to_string();
                let selected = name == active;
                let pick = name.clone();
                let view = view.clone();
                div()
                    .id(SharedString::from(format!("setup-theme-{name}")))
                    .px_3()
                    .py_2()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if selected {
                        setup_accent().opacity(0.76)
                    } else {
                        border.opacity(0.46)
                    })
                    .bg(if selected {
                        setup_accent().opacity(0.14)
                    } else {
                        setup_tile()
                    })
                    .text_color(if selected { fg } else { muted })
                    .text_sm()
                    .cursor_pointer()
                    .hover(move |h| h.border_color(setup_accent().opacity(0.64)).text_color(fg))
                    .child(name)
                    .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                        let pick = pick.clone();
                        view.update(cx, |this, cx| {
                            this.process_actions([Action::SetTheme(pick)]);
                            cx.notify();
                        });
                    })
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        let theme_list = div()
            .relative()
            .h(px(172.))
            .child(
                v_flex()
                    .id("getting-started-theme-list")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.getting_started_theme_scroll)
                    .pr(px(10.))
                    .gap_2()
                    .children(themes),
            )
            .vertical_scrollbar(&self.getting_started_theme_scroll)
            .into_any_element();

        let icons = v_flex()
            .gap_2()
            .child(category_label("getting_started_icons_category", muted))
            .child(super::guide::icon_theme_pills(
                &view,
                fg,
                muted,
                border.opacity(0.46),
                setup_tile(),
            ));

        setup_page(
            "getting_started_theme_title",
            "getting_started_theme_desc",
            v_flex().gap_4().child(theme_list).child(icons).into_any_element(),
            muted,
        )
    }

    fn setup_workspace_page(
        &self,
        fg: gpui::Hsla,
        muted: gpui::Hsla,
        border: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let view_mode = self
            .state
            .active_tab_data()
            .map(|tab| tab.view_mode.clone())
            .unwrap_or_else(|| self.state.settings.view_mode.clone());
        let mode_tiles = [
            (
                ViewMode::Details,
                "settings_view_details_btn",
                AtlasIcon::Details,
            ),
            (ViewMode::Grid, "settings_view_grid_btn", AtlasIcon::Grid),
            (ViewMode::List, "settings_view_list_btn", AtlasIcon::List),
            (
                ViewMode::Miller,
                "settings_view_miller_btn",
                AtlasIcon::Columns,
            ),
        ]
        .into_iter()
        .map(|(mode, label, icon_kind)| {
            let selected = view_mode == mode;
            let view = view.clone();
            option_tile(label, icon_kind, selected, fg, muted, border, move |cx| {
                let mode = mode.clone();
                view.update(cx, |this, cx| {
                    this.process_actions([Action::SetViewMode(mode)]);
                    cx.notify();
                });
            })
        })
        .collect::<Vec<_>>();

        let sidebar_on = self.state.settings.show_sidebar;
        let inspector_on = self.state.settings.show_inspector;
        let hidden_on = self.state.settings.show_hidden;
        let sidebar_view = view.clone();
        let inspector_view = view.clone();
        let hidden_view = view.clone();
        let sidebar = option_tile(
            "getting_started_show_sidebar",
            AtlasIcon::SidebarSimple,
            sidebar_on,
            fg,
            muted,
            border,
            move |cx| {
                sidebar_view.update(cx, |this, cx| {
                    this.process_actions([Action::ToggleSidebar]);
                    cx.notify();
                });
            },
        );
        let inspector = option_tile(
            "getting_started_show_inspector",
            AtlasIcon::Eye,
            inspector_on,
            fg,
            muted,
            border,
            move |cx| {
                inspector_view.update(cx, |this, cx| {
                    this.state.settings.show_inspector = !this.state.settings.show_inspector;
                    atlas_core::util::config::save_settings(&this.state.settings);
                    cx.notify();
                });
            },
        );
        let hidden = option_tile(
            "getting_started_show_hidden",
            AtlasIcon::File,
            hidden_on,
            fg,
            muted,
            border,
            move |cx| {
                hidden_view.update(cx, |this, cx| {
                    this.process_actions([Action::ToggleHidden]);
                    cx.notify();
                });
            },
        );

        let mut mode_tiles = mode_tiles.into_iter();
        let layout = v_flex()
            .gap_4()
            .child(
                v_flex()
                    .gap_2()
                    .child(category_label("getting_started_view_category", muted))
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                mode_tiles
                                    .next()
                                    .unwrap_or_else(|| div().into_any_element()),
                            )
                            .child(
                                mode_tiles
                                    .next()
                                    .unwrap_or_else(|| div().into_any_element()),
                            )
                            .child(
                                mode_tiles
                                    .next()
                                    .unwrap_or_else(|| div().into_any_element()),
                            )
                            .child(
                                mode_tiles
                                    .next()
                                    .unwrap_or_else(|| div().into_any_element()),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(category_label("getting_started_workspace_category", muted))
                    .child(
                        h_flex()
                            .gap_3()
                            .child(sidebar)
                            .child(inspector)
                            .child(hidden),
                    ),
            );

        setup_page(
            "getting_started_workspace_title",
            "getting_started_workspace_desc",
            layout.into_any_element(),
            muted,
        )
    }
}

fn welcome_tile(
    id: &'static str,
    icon_kind: AtlasIcon,
    title_key: &'static str,
    fg: gpui::Hsla,
    border: gpui::Hsla,
) -> AnyElement {
    v_flex()
        .id(SharedString::from(id))
        .flex_1()
        .h(px(132.))
        .items_center()
        .justify_center()
        .gap_3()
        .rounded(px(14.))
        .border_1()
        .border_color(border.opacity(0.46))
        .bg(setup_tile())
        .shadow(soft_card_shadow())
        .child(icon(icon_kind, 38., setup_accent()))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(fg)
                .child(rust_i18n::t!(title_key).to_string()),
        )
        .into_any_element()
}

fn option_tile(
    label: &'static str,
    icon_kind: AtlasIcon,
    selected: bool,
    fg: gpui::Hsla,
    muted: gpui::Hsla,
    border: gpui::Hsla,
    on_click: impl Fn(&mut gpui::App) + 'static,
) -> AnyElement {
    v_flex()
        .flex_1()
        .h(px(92.))
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(px(14.))
        .border_1()
        .border_color(if selected {
            hsla(0., 0., 1., 0.30)
        } else {
            border.opacity(0.38)
        })
        .bg(if selected {
            setup_accent().opacity(0.14)
        } else {
            setup_tile()
        })
        .shadow(soft_card_shadow())
        .cursor_pointer()
        .child(icon(
            icon_kind,
            24.,
            if selected {
                setup_accent()
            } else {
                muted.opacity(0.86)
            },
        ))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if selected { fg } else { muted })
                .child(rust_i18n::t!(label).to_string()),
        )
        .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
            on_click(cx);
        })
        .into_any_element()
}

fn category_label(label: &'static str, muted: gpui::Hsla) -> AnyElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(muted)
        .child(rust_i18n::t!(label).to_string())
        .into_any_element()
}

fn setup_page(
    title: &'static str,
    desc: &'static str,
    content: AnyElement,
    muted: gpui::Hsla,
) -> AnyElement {
    v_flex()
        .gap_4()
        .child(
            v_flex()
                .gap_2()
                .items_center()
                .child(atlas_logo(72.0))
                .child(
                    div()
                        .text_xl()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(rust_i18n::t!(title).to_string()),
                )
                .child(
                    div()
                        .max_w(px(520.))
                        .text_align(gpui::TextAlign::Center)
                        .text_sm()
                        .line_height(px(20.))
                        .text_color(muted)
                        .child(rust_i18n::t!(desc).to_string()),
                ),
        )
        .child(content)
        .into_any_element()
}

fn step_dots(step: usize, muted: gpui::Hsla) -> AnyElement {
    h_flex()
        .gap_2()
        .children((0..3).map(|i| {
            div()
                .size(px(if i == step { 9. } else { 7. }))
                .rounded_full()
                .bg(if i <= step {
                    setup_accent()
                } else {
                    muted.opacity(0.45)
                })
                .into_any_element()
        }))
        .into_any_element()
}

fn soft_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(0., 0., 0., 0.24).into(),
        offset: point(px(0.0), px(24.0)),
        blur_radius: px(64.0),
        spread_radius: px(-24.0),
        inset: false,
    }]
}

fn soft_card_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(0., 0., 0., 0.18).into(),
        offset: point(px(0.0), px(14.0)),
        blur_radius: px(34.0),
        spread_radius: px(-20.0),
        inset: false,
    }]
}
