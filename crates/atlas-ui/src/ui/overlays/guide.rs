//! Feature guide: a short, interactive help overlay.
//!
//! The overlay lists topics phrased as what the user wants to do. "Start" turns
//! a topic into a *coach*: a small card that stays out of the way while the user
//! really does each step. Progress is detected from the `Action`s that flow
//! through `process_actions`, so the shortcut and the manual route (menus,
//! palette, clicks) both count.

use crate::app::App;
use crate::input::action::Action;
use crate::ui::icons::{icon, AtlasIcon};
use atlas_model::overlay::GuideCoach;
use gpui::prelude::*;
use gpui::ColorExt as _;
use gpui::{
    deferred, div, hsla, linear_color_stop, linear_gradient, point, px, AnyElement, BoxShadow, Context, MouseButton, SharedString,
};
use gpui_component::kbd::Kbd;
use gpui_component::{h_flex, v_flex, ActiveTheme};

struct Step {
    /// Command id whose (remappable) shortcut is shown next to the step.
    command: Option<&'static str>,
    /// Detects the step being done, by shortcut or by any manual route.
    hit: Option<fn(&Action) -> bool>,
    /// Fingerprint of app state; any change means the user did the step, even
    /// through a path that never goes through `process_actions` (bottom bar,
    /// settings page, ...).
    probe: Option<fn(&App) -> u64>,
    /// i18n key of the palette command that does the same, shown as a second route.
    palette: Option<&'static str>,
    /// Offer the optional icon-theme downloads under this step.
    icons: bool,
}

struct Topic {
    /// i18n key prefix: `guide_<id>_title|lead|s1..s3`.
    id: &'static str,
    icon: AtlasIcon,
    steps: &'static [Step],
}

const fn step(command: Option<&'static str>, hit: fn(&Action) -> bool) -> Step {
    Step { command, hit: Some(hit), probe: None, palette: None, icons: false }
}

const fn read(command: Option<&'static str>) -> Step {
    Step { command, hit: None, probe: None, palette: None, icons: false }
}

impl Step {
    const fn watch(mut self, probe: fn(&App) -> u64) -> Self {
        self.probe = Some(probe);
        self
    }

    const fn icons(mut self) -> Self {
        self.icons = true;
        self
    }

    const fn via(mut self, palette_title_key: &'static str) -> Self {
        self.palette = Some(palette_title_key);
        self
    }

    fn detectable(&self) -> bool {
        self.hit.is_some() || self.probe.is_some()
    }
}

fn fp<T: std::hash::Hash>(v: &T) -> u64 {
    use std::hash::Hasher as _;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

fn watch_view(app: &App) -> u64 {
    fp(&format!("{:?}", app.state.active_tab_data().map(|t| t.view_mode.clone())))
}
fn watch_group(app: &App) -> u64 {
    fp(&format!("{:?}", app.state.active_tab_data().map(|t| t.group_by)))
}
fn watch_inspector(app: &App) -> u64 {
    fp(&app.state.settings.show_inspector)
}
fn watch_panes(app: &App) -> u64 {
    fp(&app.state.all_pane_tile_ids().len())
}
fn watch_goto(app: &App) -> u64 {
    fp(&app.state.overlay.show_goto_bar)
}
fn watch_palette(app: &App) -> u64 {
    fp(&app.state.overlay.show_palette)
}
fn watch_settings(app: &App) -> u64 {
    fp(&app.state.overlay.show_settings)
}
fn watch_theme(app: &App) -> u64 {
    fp(&app.state.profiles.active().theme)
}
fn watch_converter(app: &App) -> u64 {
    fp(&app.converter.is_some())
}
fn watch_extensions(app: &App) -> u64 {
    fp(&app.state.ext.show_manager)
}

fn is_noise(a: &Action) -> bool {
    matches!(
        a,
        Action::OpenPalette
            | Action::ClosePalette
            | Action::CloseOverlays
            | Action::OpenGuide(_)
            | Action::CloseGuide
            | Action::SetGuideTopic(_)
            | Action::StartGuideCoach(_)
            | Action::StopGuideCoach
            | Action::DownloadIconTheme(_)
    )
}

static TOPICS: [Topic; 11] = [
    Topic {
        id: "goto",
        icon: AtlasIcon::Search,
        steps: &[
            step(Some("nav.goto"), |a| matches!(a, Action::OpenGotoBar)).watch(watch_goto),
            step(None, |a| {
                matches!(a, Action::Navigate { .. } | Action::NavigateLocation { .. })
            }),
        ],
    },
    Topic {
        id: "commands",
        icon: AtlasIcon::Lightning,
        steps: &[
            step(Some("sys.palette"), |a| matches!(a, Action::OpenPalette)).watch(watch_palette),
            step(None, |a| !is_noise(a)),
        ],
    },
    Topic {
        id: "views",
        icon: AtlasIcon::Grid,
        steps: &[
            step(None, |a| matches!(a, Action::SetViewMode(_)))
                .watch(watch_view)
                .via("cmd_view_grid_title"),
            step(None, |a| matches!(a, Action::SetGroupBy(_))).watch(watch_group),
            step(Some("view.inspector"), |a| matches!(a, Action::ToggleInspector))
                .watch(watch_inspector)
                .via("cmd_view_inspector_title"),
        ],
    },
    Topic {
        id: "panes",
        icon: AtlasIcon::SplitVertical,
        steps: &[
            step(Some("pane.split-h"), |a| {
                matches!(a, Action::SplitHorizontalFocused | Action::SplitVerticalFocused)
            })
            .watch(watch_panes)
            .via("cmd_pane_split_h_title"),
            step(Some("tab.new"), |a| matches!(a, Action::NewTabFocused | Action::NewTab(_)))
                .watch(watch_panes)
                .via("cmd_tab_new_title"),
            step(Some("pane.close"), |a| matches!(a, Action::CloseFocusedPane))
                .watch(watch_panes)
                .via("cmd_pane_close_title"),
        ],
    },
    Topic {
        id: "convert",
        icon: AtlasIcon::ArrowsLeftRight,
        steps: &[
            read(None),
            step(Some("files.convert"), |a| matches!(a, Action::OpenConverter))
                .watch(watch_converter),
        ],
    },
    Topic {
        id: "analyze",
        icon: AtlasIcon::HardDrives,
        steps: &[
            read(None),
            step(None, |a| matches!(a, Action::OpenSpaceLens { .. })),
        ],
    },
    Topic {
        id: "vault",
        icon: AtlasIcon::Lock,
        steps: &[read(None), read(None), read(None)],
    },
    Topic {
        id: "tags",
        icon: AtlasIcon::Tag,
        steps: &[
            step(None, |a| {
                matches!(a, Action::OpenCreateTagOverlay { .. } | Action::CreateTag { .. })
            }),
            step(None, |a| matches!(a, Action::AssignTagToFiles { .. })),
            step(None, |a| matches!(a, Action::OpenTagResults { .. })),
        ],
    },
    Topic {
        id: "cloud",
        icon: AtlasIcon::Cloud,
        steps: &[read(None), read(None), read(None)],
    },
    Topic {
        id: "themes",
        icon: AtlasIcon::Palette,
        steps: &[
            step(Some("sys.settings"), |a| matches!(a, Action::OpenSettings)).watch(watch_settings),
            step(None, |a| matches!(a, Action::SetTheme(_))).watch(watch_theme),
            step(None, |a| matches!(a, Action::DownloadIconTheme(_))).icons(),
        ],
    },
    Topic {
        id: "extensions",
        icon: AtlasIcon::PuzzlePiece,
        steps: &[
            step(Some("ext.manager"), |a| matches!(a, Action::OpenExtManager))
                .watch(watch_extensions)
                .via("ext_manager_title"),
            read(None),
            read(None),
        ],
    },
];

fn tr(id: &str, part: &str) -> String {
    let key = format!("guide_{id}_{part}");
    rust_i18n::t!(key.as_str()).to_string()
}

fn extension_manager_button(view: &gpui::Entity<App>, fg: gpui::Hsla, fill: gpui::Hsla) -> AnyElement {
    let view = view.clone();
    div().id("guide-open-extensions").px_4().py_2().rounded_full().bg(fill)
        .text_sm().text_color(fg).cursor_pointer()
        .child(rust_i18n::t!("ext_manager_title").to_string())
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            view.update(cx, |this, cx| {
                this.process_actions([Action::CloseGuide, Action::OpenExtManager]);
                cx.notify();
            });
        }).into_any_element()
}

fn practicable(topic: &Topic) -> bool {
    topic.steps.iter().any(|s| s.hit.is_some())
}

fn soft_shadow(alpha: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(0., 0., 0., alpha).into(),
        offset: point(px(0.), px(28.)),
        blur_radius: px(72.),
        spread_radius: px(-28.),
        inset: false,
    }]
}

/// Optional icon-theme downloads: (`icon-manager` command id, label).
const ICON_THEMES: [(&str, &str); 3] = [
    ("icon-manager.download-material", "Material"),
    ("icon-manager.download-catppuccin", "Catppuccin"),
    ("icon-manager.download-flow", "Flow"),
];

/// Row of pills that download an icon theme. Shared by the guide and the
/// first-run setup; colours are passed in because they use different palettes.
pub(crate) fn icon_theme_pills(
    view: &gpui::Entity<App>,
    fg: gpui::Hsla,
    muted: gpui::Hsla,
    border: gpui::Hsla,
    fill: gpui::Hsla,
) -> AnyElement {
    ICON_THEMES
        .iter()
        .fold(h_flex().gap_2().flex_wrap(), |row, (cmd, label)| {
            let v = view.clone();
            row.child(
                h_flex()
                    .id(SharedString::from(format!("icon-theme-{cmd}")))
                    .h(px(32.))
                    .px_4()
                    .gap_2()
                    .items_center()
                    .rounded_full()
                    .border_1()
                    .border_color(border)
                    .bg(fill)
                    .text_xs()
                    .text_color(muted)
                    .cursor_pointer()
                    .hover(move |h| h.text_color(fg).border_color(fg.opacity(0.5)))
                    .child(icon(AtlasIcon::Download, 12., muted))
                    .child(*label)
                    .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                        v.update(cx, |this, cx| {
                            this.process_actions([Action::DownloadIconTheme((*cmd).to_string())]);
                            cx.notify();
                        });
                    }),
            )
        })
        .into_any_element()
}

impl App {
    /// Derive names, descriptions and totals from the same live manifests as
    /// the manager, including optional tool providers in their own count.
    fn guide_extension_inventory(&self, fg: gpui::Hsla, muted: gpui::Hsla) -> AnyElement {
        use atlas_model::ext_model::ExtKind;
        let providers = self.ext_registry.extensions.iter().filter(|e| e.manifest.kind == ExtKind::ToolProvider).count();
        let count = self.ext_registry.extensions.iter().filter(|e| !e.manifest.hidden && e.manifest.kind != ExtKind::ToolProvider).count();
        let mut entries: Vec<_> = self.ext_registry.extensions.iter().filter(|e| !e.manifest.hidden).collect();
        entries.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        v_flex().gap_3()
            .child(div().text_sm().text_color(muted).child(rust_i18n::t!("guide_extensions_count", count = count, tools = providers).to_string()))
            .children(entries.into_iter().map(|e| {
                v_flex().gap_1()
                    .child(div().text_sm().text_color(fg).child(format!("{} · {}", e.manifest.name, e.manifest.version)))
                    .child(div().text_xs().text_color(muted).child(e.manifest.description.clone()))
            })).into_any_element()
    }

    /// "Or: [palette key] “Command name”" — the manual route to the same step.
    fn guide_via(&self, key: Option<&'static str>, muted: gpui::Hsla) -> Option<AnyElement> {
        let key = key?;
        let title = rust_i18n::t!(key).to_string();
        let palette = self.command_shortcut_keystroke("sys.palette");
        Some(
            h_flex()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(muted)
                .child(rust_i18n::t!("guide_or").to_string())
                .when_some(palette, |row, ks| row.child(Kbd::new(ks)))
                .child(format!("“{title}”"))
                .into_any_element(),
        )
    }

    /// Advance the coach when `action` completes its current step.
    pub(crate) fn observe_guide(&mut self, action: &Action) {
        let Some(GuideCoach { topic, step, .. }) = self.state.overlay.guide_coach else {
            return;
        };
        let Some(t) = TOPICS.get(topic) else {
            self.state.overlay.guide_coach = None;
            return;
        };
        let Some(current) = t.steps.get(step) else {
            return;
        };
        // Reading-only steps have no detector; the card's "Next" button moves on.
        if current.hit.is_some_and(|hit| hit(action)) {
            self.state.overlay.guide_coach =
                Some(GuideCoach { topic, step: step + 1, base: u64::MAX });
        }
    }

    /// Advance the coach when the state its current step watches changes.
    fn sync_guide_probe(&mut self) {
        let Some(c) = self.state.overlay.guide_coach else {
            return;
        };
        let Some(probe) = TOPICS
            .get(c.topic)
            .and_then(|t| t.steps.get(c.step))
            .and_then(|s| s.probe)
        else {
            return;
        };
        let now = probe(self);
        if c.base == u64::MAX {
            self.state.overlay.guide_coach = Some(GuideCoach { base: now, ..c });
        } else if now != c.base {
            self.state.overlay.guide_coach =
                Some(GuideCoach { step: c.step + 1, base: u64::MAX, ..c });
        }
    }

    pub(crate) fn render_guide(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (fg, muted, border, accent, accent_fg, panel_bg, hover, tile_bg) = {
            let t = cx.theme();
            (
                t.popover_foreground,
                t.muted_foreground,
                t.border,
                t.primary,
                t.primary_foreground,
                t.popover,
                t.secondary_hover,
                t.secondary,
            )
        };
        let view = cx.entity();
        let active = self.state.overlay.guide_topic.min(TOPICS.len() - 1);
        let topic = &TOPICS[active];
        let faint = border.opacity(0.55);

        let nav = TOPICS.iter().enumerate().fold(
            v_flex().gap_0p5().p_3().pt_4(),
            |col, (i, t)| {
                let selected = i == active;
                let v = view.clone();
                col.child(
                    h_flex()
                        .id(SharedString::from(format!("guide-topic-{}", t.id)))
                        .relative()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .h(px(38.))
                        .rounded(px(10.))
                        .cursor_pointer()
                        .bg(if selected { tile_bg.opacity(0.55) } else { gpui::transparent_black() })
                        .hover(move |h| h.bg(hover))
                        .when(selected, |row| {
                            row.child(
                                div()
                                    .absolute()
                                    .left(px(-2.))
                                    .top(px(11.))
                                    .w(px(2.))
                                    .h(px(16.))
                                    .rounded_full()
                                    .bg(accent),
                            )
                        })
                        .child(icon(t.icon, 15., if selected { fg } else { muted }))
                        .child(
                            div()
                                .text_sm()
                                .text_color(if selected { fg } else { muted })
                                .font_weight(if selected {
                                    gpui::FontWeight::MEDIUM
                                } else {
                                    gpui::FontWeight::NORMAL
                                })
                                .child(tr(t.id, "title")),
                        )
                        .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                            v.update(cx, |this, cx| {
                                this.process_actions([Action::SetGuideTopic(i)]);
                                cx.notify();
                            });
                        }),
                )
            },
        );

        let steps = topic.steps.iter().enumerate().fold(v_flex(), |col, (n, s)| {
            let keystroke = s.command.and_then(|c| self.command_shortcut_keystroke(c));
            let via = self.guide_via(s.palette, muted);
            col.child(
                h_flex()
                    .items_center()
                    .gap_4()
                    .py_4()
                    .border_t_1()
                    .border_color(faint)
                    .child(
                        div()
                            .w(px(28.))
                            .flex_none()
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::LIGHT)
                            .text_color(muted)
                            .child(format!("{:02}", n + 1)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .w_full()
                                    .whitespace_normal()
                                    .text_size(px(16.))
                                    .text_color(fg)
                                    .child(tr(topic.id, &format!("s{}", n + 1))),
                            )
                            .when_some(via, |col, v| col.child(v))
                            .when(s.icons, |col| {
                                col.child(icon_theme_pills(&view, fg, muted, border, tile_bg))
                            }),
                    )
                    .when_some(keystroke, |row, ks| row.child(Kbd::new(ks))),
            )
        });

        let start = practicable(topic).then(|| {
            let v = view.clone();
            h_flex()
                .id("guide-start")
                .h(px(44.))
                .px_6()
                .gap_2()
                .items_center()
                .rounded_full()
                .bg(accent)
                .text_color(accent_fg)
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .cursor_pointer()
                .shadow(soft_shadow(0.25))
                .hover(|h| h.opacity(0.9))
                .child(rust_i18n::t!("guide_start").to_string())
                .child(icon(AtlasIcon::ArrowRight, 13., accent_fg))
                .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                    v.update(cx, |this, cx| {
                        this.process_actions([Action::StartGuideCoach(active)]);
                        cx.notify();
                    });
                })
        });

        let page = v_flex()
            .id("guide-page")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .px(px(56.))
            .py(px(52.))
            .gap_8()
            .child(
                v_flex()
                    .gap_4()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .size(px(40.))
                                    .rounded(px(13.))
                                    .border_1()
                                    .border_color(border.opacity(0.7))
                                    .bg(linear_gradient(
                                        160.,
                                        linear_color_stop(hsla(0., 0., 1., 0.10), 0.0),
                                        linear_color_stop(hsla(0., 0., 1., 0.02), 1.0),
                                    ))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(topic.icon, 18., fg)),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(muted)
                                    .child(format!("{:02} / {:02}", active + 1, TOPICS.len())),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(32.))
                            .line_height(px(38.))
                            .font_weight(gpui::FontWeight::LIGHT)
                            .text_color(fg)
                            .child(tr(topic.id, "title")),
                    )
                    .child(
                        div()
                            .max_w(px(420.))
                            .text_size(px(15.))
                            .line_height(px(22.))
                            .text_color(muted)
                            .child(tr(topic.id, "lead")),
                    )
                    .when(topic.id == "extensions", |col| {
                        col.child(extension_manager_button(&view, fg, tile_bg))
                    }),
            )
            .child(steps)
            .when(topic.id == "extensions", |col| col
                .child(self.guide_extension_inventory(fg, muted)))
            .when_some(start, |col, b| {
                col.child(
                    v_flex()
                        .gap_3()
                        .items_start()
                        .child(b)
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(rust_i18n::t!("guide_hint").to_string()),
                        ),
                )
            });

        let close_view = view.clone();
        let panel = v_flex()
            .id("guide-panel")
            .w(px(880.))
            .max_w_full()
            .h(px(560.))
            .max_h_full()
            .rounded(px(22.))
            .border_1()
            .border_color(border.opacity(0.6))
            .bg(panel_bg)
            .shadow(soft_shadow(0.40))
            .text_color(fg)
            .overflow_hidden()
            .occlude()
            .child(div().absolute().top_0().left_0().right_0().h(px(160.)).bg(linear_gradient(
                180.,
                linear_color_stop(hsla(0., 0., 1., 0.045), 0.0),
                linear_color_stop(hsla(0., 0., 1., 0.0), 1.0),
            )))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        v_flex()
                            .w(px(250.))
                            .h_full()
                            .flex_none()
                            .border_r_1()
                            .border_color(faint)
                            .child(
                                div()
                                    .px_6()
                                    .pt(px(26.))
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(rust_i18n::t!("guide_title").to_string()),
                            )
                            .child(div().id("guide-nav").flex_1().overflow_y_scroll().child(nav)),
                    )
                    .child(page)
                    .child(
                        div().absolute().top(px(14.)).right(px(14.)).child(
                            div()
                                .id("guide-close")
                                .size(px(30.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(move |h| h.bg(hover))
                                .child(icon(AtlasIcon::X, 12., muted))
                                .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                    close_view.update(cx, |this, cx| {
                                        this.process_actions([Action::CloseGuide]);
                                        cx.notify();
                                    });
                                }),
                        ),
                    ),
            );

        let close_view = view;
        let scrim = div()
            .id("scrim-guide")
            .absolute()
            .inset_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .bg(linear_gradient(
                180.,
                linear_color_stop(hsla(0., 0., 0., 0.46), 0.0),
                linear_color_stop(hsla(0., 0., 0., 0.66), 1.0),
            ))
            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                close_view.update(cx, |this, cx| {
                    this.process_actions([Action::CloseGuide]);
                    cx.notify();
                });
            })
            .child(panel);
        deferred(scrim).into_any_element()
    }

    /// Floating card that guides the user through one topic while they use the
    /// real app. It never blocks the window: only the card itself takes clicks.
    pub(crate) fn render_guide_coach(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.sync_guide_probe();
        let Some(GuideCoach { topic, step, .. }) = self.state.overlay.guide_coach else {
            return div().into_any_element();
        };
        let topic_ix = topic.min(TOPICS.len() - 1);
        let t = &TOPICS[topic_ix];
        let (fg, muted, border, accent, accent_fg, panel_bg, hover, tile_bg) = {
            let th = cx.theme();
            (
                th.popover_foreground,
                th.muted_foreground,
                th.border,
                th.primary,
                th.primary_foreground,
                th.popover,
                th.secondary_hover,
                th.secondary,
            )
        };
        let view = cx.entity();
        let total = t.steps.len();
        let finished = step >= total;
        let current = t.steps.get(step);
        let keystroke = current
            .and_then(|s| s.command)
            .and_then(|c| self.command_shortcut_keystroke(c));
        let via = current.and_then(|s| self.guide_via(s.palette, muted));

        let dots = (0..total).fold(h_flex().flex_1().gap_1().items_center(), |row, i| {
            let color = if i < step || finished {
                accent
            } else if i == step {
                fg.opacity(0.55)
            } else {
                border
            };
            row.child(div().flex_1().h(px(2.)).rounded_full().bg(color))
        });

        let stop_view = view.clone();
        let header = h_flex()
            .items_center()
            .gap_4()
            .child(dots)
            .child(
                div()
                    .id("guide-coach-close")
                    .size(px(26.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |h| h.bg(hover))
                    .child(icon(AtlasIcon::X, 11., muted))
                    .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                        stop_view.update(cx, |this, cx| {
                            this.process_actions([Action::StopGuideCoach]);
                            cx.notify();
                        });
                    }),
            );

        let detect = current.is_some_and(Step::detectable);
        let next_view = view.clone();
        let pills_view = view.clone();
        let icons_row = current
            .is_some_and(|s| s.icons)
            .then(|| icon_theme_pills(&pills_view, fg, muted, border, tile_bg));
        let body = if finished {
            let back_view = view.clone();
            let end_view = view;
            v_flex()
                .gap_4()
                .child(
                    h_flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .size(px(32.))
                                .rounded_full()
                                .bg(accent)
                                .text_color(accent_fg)
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_sm()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child("✓"),
                        )
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(rust_i18n::t!("guide_done").to_string()),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .id("guide-coach-back")
                                .h(px(36.))
                                .px_4()
                                .rounded_full()
                                .bg(tile_bg)
                                .flex()
                                .items_center()
                                .text_sm()
                                .cursor_pointer()
                                .hover(move |h| h.bg(hover))
                                .child(rust_i18n::t!("guide_back").to_string())
                                .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                    back_view.update(cx, |this, cx| {
                                        this.process_actions([
                                            Action::StopGuideCoach,
                                            Action::OpenGuide((topic_ix + 1) % TOPICS.len()),
                                        ]);
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(
                            div()
                                .id("guide-coach-end")
                                .h(px(36.))
                                .px_4()
                                .rounded_full()
                                .bg(accent)
                                .text_color(accent_fg)
                                .flex()
                                .items_center()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .cursor_pointer()
                                .hover(|h| h.opacity(0.9))
                                .child(rust_i18n::t!("guide_finish").to_string())
                                .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                    end_view.update(cx, |this, cx| {
                                        this.process_actions([Action::StopGuideCoach]);
                                        cx.notify();
                                    });
                                }),
                        ),
                )
        } else {
            v_flex()
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(format!("{} · {}/{}", tr(t.id, "title"), step + 1, total)),
                )
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_2()
                                .child(
                                    div()
                                        .w_full()
                                        .whitespace_normal()
                                        .text_size(px(20.))
                                        .line_height(px(26.))
                                        .font_weight(gpui::FontWeight::LIGHT)
                                        .child(tr(t.id, &format!("s{}", step + 1))),
                                )
                                .when_some(via, |col, v| col.child(v)),
                        )
                        .when_some(keystroke, |row, ks| row.child(Kbd::new(ks))),
                )
                .when_some(icons_row, |col, r| col.child(r))
                .when(current.is_some_and(|s| s.command == Some("ext.manager")), |col| {
                    col.child(extension_manager_button(&view, fg, tile_bg))
                })
                .child(if detect {
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(div().size(px(6.)).rounded_full().bg(accent))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(rust_i18n::t!("guide_wait").to_string()),
                                ),
                        )
                        .child(
                            div()
                                .id("guide-coach-next")
                                .h(px(30.))
                                .px_3()
                                .rounded_full()
                                .flex()
                                .items_center()
                                .text_xs()
                                .text_color(muted)
                                .cursor_pointer()
                                .hover(move |h| h.bg(hover))
                                .child(rust_i18n::t!("guide_skip").to_string())
                                .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                    next_view.update(cx, |this, cx| {
                                        this.state.overlay.guide_coach = Some(GuideCoach {
                                            topic: topic_ix,
                                            step: step + 1,
                                            base: u64::MAX,
                                        });
                                        cx.notify();
                                    });
                                }),
                        )
                } else {
                    h_flex()
                        .child(
                            div()
                                .id("guide-coach-next")
                                .h(px(36.))
                                .px_4()
                                .rounded_full()
                                .bg(accent)
                                .text_color(accent_fg)
                                .flex()
                                .items_center()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .cursor_pointer()
                                .hover(|h| h.opacity(0.9))
                                .child(rust_i18n::t!("guide_next").to_string())
                                .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                                    next_view.update(cx, |this, cx| {
                                        this.state.overlay.guide_coach = Some(GuideCoach {
                                            topic: topic_ix,
                                            step: step + 1,
                                            base: u64::MAX,
                                        });
                                        cx.notify();
                                    });
                                }),
                        )
                })
        };

        let card = v_flex()
            .id("guide-coach")
            .w(px(460.))
            .max_w_full()
            .p_6()
            .gap_5()
            .rounded(px(22.))
            .border_1()
            .border_color(border.opacity(0.6))
            .bg(panel_bg.opacity(0.97))
            .text_color(fg)
            .shadow(soft_shadow(0.36))
            .occlude()
            .child(header)
            .child(body);

        deferred(
            div()
                .absolute()
                .bottom(px(32.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(card),
        )
        .into_any_element()
    }
}
