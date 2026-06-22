//! FASE B: left sidebar as a Dock `left_dock` panel.
//!
//! `SidebarPanel` is its own gpui `Entity` (like `FilePanel`) so it can live in the
//! `DockArea`'s left dock. It reads `AppState` through a weak handle and navigates
//! the focused pane on click. Places + Bookmarks land here; Tags/Drives/Cloud are
//! added with their subsystems (Fase H).

use crate::app::nav::NavKind;
use crate::app::App as AtlasApp;
use crate::input::action::Action;
use crate::state::location::Location;
use crate::state::profiles::{SectionKind, SidebarSection};
use crate::state::tag_cache::TagId;
use crate::ui::drag_hover::{
    drag_hover_recorder, drag_hover_recorder_with_drop, sidebar_scroll_recorder, DragHoverCommand,
    DragHoverDropCommand, DragHoverId, DragHoverKind,
};
use crate::ui::icons::{atlas_logo, icon, AtlasIcon};
use crate::ui::root::DraggedFiles;
#[cfg(all(feature = "cloud", windows))]
use crate::InstallWinFspForRemote;
#[cfg(feature = "cloud")]
use crate::{RemoveCloudRemote, StartRenameCloudRemote};
use crate::{
    SidebarHideSection, SidebarMoveSectionDown, SidebarMoveSectionUp, SidebarNewGlobalTag,
    SidebarNewProfileTag, SidebarResetSections, SidebarToggleSection,
};
use gpui::prelude::*;
use gpui::{
    auto, div, hsla, point, px, relative, Anchor, App, Context, Entity, EventEmitter, FocusHandle,
    Focusable, Hsla, MouseButton, RenderOnce, SharedString, Styled, WeakEntity, Window,
};
use gpui_component::dock::{Panel, PanelControl, PanelEvent};
use gpui_component::menu::{ContextMenuExt, PopupMenu};
use gpui_component::popover::{Popover, PopoverState};
use gpui_component::scroll::ScrollableElement;
use gpui_component::tooltip::Tooltip;
use gpui_component::{h_flex, v_flex, ActiveTheme, ElementExt, Selectable};
use std::path::PathBuf;

const SCROLLBAR_GUTTER: f32 = 16.0;

pub struct SidebarPanel {
    app: WeakEntity<AtlasApp>,
    focus_handle: FocusHandle,
    scroll: gpui::ScrollHandle,
}

impl SidebarPanel {
    /// `names`/`active_idx` are passed in by the caller (which holds `App` directly):
    /// reading the `app` entity here would re-enter `App` while it is mid-update.
    pub fn new(
        app: Entity<AtlasApp>,
        names: Vec<String>,
        active_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&app, |_, _, cx| cx.notify()).detach();

        let _ = (names, active_idx, window);

        Self {
            app: app.downgrade(),
            focus_handle: cx.focus_handle(),
            scroll: gpui::ScrollHandle::new(),
        }
    }
}

fn profile_menu_panel(
    app: WeakEntity<AtlasApp>,
    popover: Entity<PopoverState>,
    cx: &mut Context<PopoverState>,
    framed: bool,
    keep_profiles_open_on_hover: bool,
) -> gpui::AnyElement {
    let Some(app_entity) = app.upgrade() else {
        return div().into_any_element();
    };

    let (profiles, active_idx, confirming_delete) = {
        let app = app_entity.read(cx);
        (
            app.state
                .profiles
                .profiles
                .iter()
                .map(|profile| profile.name.clone())
                .collect::<Vec<_>>(),
            app.state.profiles.active_idx,
            app.state.profile_ui.confirming_delete_profile,
        )
    };

    let popover_bg = cx.theme().popover.opacity(1.0);
    let fg = cx.theme().popover_foreground;
    let muted = cx.theme().muted_foreground;
    let border = cx.theme().border;
    let accent = cx.theme().accent;
    let accent_fg = cx.theme().accent_foreground;
    let hover = hsla(0., 0., 1., 0.08);
    let action_hover = hsla(0., 0., 1., 0.12);
    let danger = hsla(0., 0.72, 0.52, 1.0);

    let menu_app = app.clone();
    let mut menu = v_flex()
        .w(auto())
        .flex_none()
        .p(px(4.))
        .gap(px(2.))
        .bg(popover_bg)
        .text_sm()
        .text_color(fg)
        .when(framed, |s| {
            s.border_1()
                .border_color(border)
                .rounded(px(8.))
                .rounded_l_none()
                .shadow_lg()
        })
        .when(!framed, |s| s.rounded(px(7.)))
        .occlude();
    if keep_profiles_open_on_hover {
        menu = menu.on_mouse_move(move |_ev, _window, cx| {
            if let Some(app) = menu_app.upgrade() {
                app.update(cx, |this, cx| {
                    if !this.state.profile_ui.atlas_launcher_profiles_open {
                        this.state.profile_ui.atlas_launcher_profiles_open = true;
                        cx.notify();
                    }
                });
            }
        });
    }

    for (index, name) in profiles.into_iter().enumerate() {
        menu = menu.child(profile_menu_profile_row(
            app.clone(),
            popover.clone(),
            index,
            name,
            index == active_idx,
            confirming_delete == Some(index),
            fg,
            muted,
            accent,
            accent_fg,
            hover,
            action_hover,
            danger,
        ));
    }

    menu.child(div().h(px(1.)).w_full().mx(px(8.)).my(px(4.)).bg(border))
        .child(profile_menu_new_row(app, popover, fg, muted, hover))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn profile_menu_profile_row(
    app: WeakEntity<AtlasApp>,
    popover: Entity<PopoverState>,
    index: usize,
    name: String,
    active: bool,
    confirming: bool,
    fg: Hsla,
    muted: Hsla,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
    action_hover: Hsla,
    danger: Hsla,
) -> gpui::AnyElement {
    let row_group = SharedString::from(format!("profile-menu-row-{index}"));
    let row_text = if active { accent_fg } else { fg };
    let row_icon = if active { accent_fg } else { muted };
    let actions = [
        (
            "rename",
            AtlasIcon::PencilSimple,
            rust_i18n::t!("sidebar_profile_rename").to_string(),
        ),
        (
            "duplicate",
            AtlasIcon::Copy,
            rust_i18n::t!("sidebar_profile_duplicate").to_string(),
        ),
        (
            "delete",
            AtlasIcon::Trash,
            if confirming {
                rust_i18n::t!("sidebar_profile_confirm_delete").to_string()
            } else {
                rust_i18n::t!("sidebar_profile_delete").to_string()
            },
        ),
    ];
    let buttons = actions
        .into_iter()
        .map(|(id, glyph, tooltip)| {
            profile_menu_action_button(
                app.clone(),
                popover.clone(),
                row_group.clone(),
                index,
                id,
                glyph,
                tooltip,
                confirming,
                row_icon,
                action_hover,
                danger,
            )
        })
        .collect::<Vec<_>>();

    let row_app = app.clone();
    let row_popover = popover.clone();
    h_flex()
        .id(SharedString::from(format!("profile-menu-row-{index}")))
        .group(row_group)
        .w_full()
        .h(px(30.))
        .px_2()
        .gap_2()
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(row_text)
        .when(active, |s| {
            s.bg(accent).font_weight(gpui::FontWeight::MEDIUM)
        })
        .when(!active, |s| s.hover(move |h| h.bg(hover)))
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .truncate()
                .child(SharedString::from(name)),
        )
        .children(buttons)
        .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
            cx.stop_propagation();
            if let Some(app) = row_app.upgrade() {
                app.update(cx, |this, cx| {
                    this.process_actions(vec![Action::SwitchProfile(index)]);
                    this.state.profile_ui.confirming_delete_profile = None;
                    cx.notify();
                });
            }
            row_popover.update(cx, |state, cx| state.dismiss(window, cx));
        })
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn profile_menu_action_button(
    app: WeakEntity<AtlasApp>,
    popover: Entity<PopoverState>,
    row_group: SharedString,
    index: usize,
    id: &'static str,
    glyph: AtlasIcon,
    tooltip: String,
    confirming: bool,
    icon_color: Hsla,
    action_hover: Hsla,
    danger: Hsla,
) -> gpui::AnyElement {
    let is_delete = id == "delete";
    let is_rename = id == "rename";
    let is_duplicate = id == "duplicate";
    div()
        .id(SharedString::from(format!("profile-menu-row-{index}-{id}")))
        .size(px(22.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .cursor_pointer()
        .when(!confirming, |s| {
            s.invisible()
                .group_hover(row_group.clone(), gpui::Styled::visible)
        })
        .when(confirming && !is_delete, |s| s.invisible())
        .when(confirming && is_delete, |s| {
            s.bg(danger.opacity(0.14)).text_color(danger)
        })
        .hover(move |h| h.bg(action_hover))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(icon(
            glyph,
            13.0,
            if confirming && is_delete {
                danger
            } else {
                icon_color
            },
        ))
        .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
            cx.stop_propagation();
            let Some(app) = app.upgrade() else {
                return;
            };
            app.update(cx, |this, cx| {
                if is_rename {
                    this.process_actions(vec![Action::OpenRenameProfileAt(index)]);
                } else if is_duplicate {
                    this.process_actions(vec![Action::DuplicateProfile(index)]);
                } else if this.state.profile_ui.confirming_delete_profile == Some(index) {
                    this.process_actions(vec![Action::DeleteProfile(index)]);
                } else {
                    this.state.profile_ui.confirming_delete_profile = Some(index);
                }
                cx.notify();
            });
            if is_rename || is_duplicate || (is_delete && confirming) {
                popover.update(cx, |state, cx| state.dismiss(window, cx));
            }
        })
        .into_any_element()
}

fn profile_menu_new_row(
    app: WeakEntity<AtlasApp>,
    popover: Entity<PopoverState>,
    fg: Hsla,
    muted: Hsla,
    hover: Hsla,
) -> gpui::AnyElement {
    h_flex()
        .id("profile-menu-new")
        .w_full()
        .h(px(30.))
        .px_2()
        .gap_2()
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(fg)
        .hover(move |h| h.bg(hover))
        .child(
            div()
                .w(px(16.))
                .h(px(16.))
                .flex_none()
                .child(icon(AtlasIcon::Plus, 14.0, muted)),
        )
        .child(
            div()
                .flex_1()
                .truncate()
                .child(rust_i18n::t!("sidebar_profile_new").to_string()),
        )
        .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
            cx.stop_propagation();
            if let Some(app) = app.upgrade() {
                app.update(cx, |this, cx| {
                    this.process_actions(vec![Action::CreateProfile(String::new())]);
                    cx.notify();
                });
            }
            popover.update(cx, |state, cx| state.dismiss(window, cx));
        })
        .into_any_element()
}

fn atlas_launcher_content(
    app: WeakEntity<AtlasApp>,
    popover: Entity<PopoverState>,
    _window: &mut Window,
    cx: &mut Context<PopoverState>,
) -> gpui::AnyElement {
    let Some(app_entity) = app.upgrade() else {
        return div().into_any_element();
    };

    let (profiles_open, sidebar_collapsed, profiles_anchor) = {
        let app = app_entity.read(cx);
        (
            app.state.profile_ui.atlas_launcher_profiles_open,
            app.state.settings.sidebar_collapsed,
            app.atlas_launcher_profiles_anchor,
        )
    };
    let popover_bg = cx.theme().popover.opacity(1.0);
    let fg = cx.theme().popover_foreground;
    let muted = cx.theme().muted_foreground;
    let border = cx.theme().border;
    let accent = cx.theme().accent;
    let accent_fg = cx.theme().accent_foreground;
    let hover = hsla(0., 0., 1., 0.08);

    let menu = v_flex()
        .id("atlas-launcher-menu")
        .p(px(4.))
        .bg(popover_bg)
        .border_1()
        .border_color(border)
        .rounded(px(10.))
        .shadow_lg()
        .text_sm()
        .text_color(fg)
        .occlude()
        .gap(px(2.))
        .on_prepaint({
            let anchor_app = app.clone();
            move |bounds, _window, cx| {
                if let Some(app) = anchor_app.upgrade() {
                    app.update(cx, |this, _cx| {
                        this.atlas_launcher_profiles_anchor =
                            point(bounds.size.width - px(1.), px(0.));
                    });
                }
            }
        })
        .child(atlas_launcher_row(
            app.clone(),
            popover.clone(),
            "atlas-launcher-profiles",
            AtlasIcon::UserSwitch,
            rust_i18n::t!("atlas_menu_profiles").to_string(),
            Some(rust_i18n::t!("atlas_menu_profiles_hint").to_string()),
            profiles_open,
            true,
            None,
            fg,
            muted,
            accent,
            accent_fg,
            hover,
        ))
        .child(atlas_launcher_row(
            app.clone(),
            popover.clone(),
            "atlas-launcher-toggle-sidebar",
            AtlasIcon::SidebarSimple,
            if sidebar_collapsed {
                rust_i18n::t!("sidebar_expand").to_string()
            } else {
                rust_i18n::t!("sidebar_collapse").to_string()
            },
            None,
            false,
            false,
            Some(Action::ToggleSidebarCollapsed),
            fg,
            muted,
            accent,
            accent_fg,
            hover,
        ))
        .child(atlas_launcher_row(
            app.clone(),
            popover.clone(),
            "atlas-launcher-settings",
            AtlasIcon::Gear,
            rust_i18n::t!("settings_title").to_string(),
            None,
            false,
            false,
            Some(Action::OpenSettings),
            fg,
            muted,
            accent,
            accent_fg,
            hover,
        ))
        .child(atlas_launcher_row(
            app.clone(),
            popover.clone(),
            "atlas-launcher-palette",
            AtlasIcon::Lightning,
            rust_i18n::t!("atlas_menu_command_palette").to_string(),
            None,
            false,
            false,
            Some(Action::OpenPalette),
            fg,
            muted,
            accent,
            accent_fg,
            hover,
        ))
        .child(atlas_launcher_row(
            app.clone(),
            popover.clone(),
            "atlas-launcher-extensions",
            AtlasIcon::PuzzlePiece,
            rust_i18n::t!("ext_manager_title").to_string(),
            None,
            false,
            false,
            Some(Action::OpenExtManager),
            fg,
            muted,
            accent,
            accent_fg,
            hover,
        ));

    div()
        .relative()
        .child(menu)
        .when(profiles_open && profiles_anchor.x > px(0.), |s| {
            let submenu = profile_menu_panel(app, popover, cx, true, true);
            s.child(
                div()
                    .absolute()
                    .top_0()
                    .left(profiles_anchor.x)
                    .child(submenu),
            )
        })
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn atlas_launcher_row(
    app: WeakEntity<AtlasApp>,
    popover: Entity<PopoverState>,
    id: &'static str,
    glyph: AtlasIcon,
    label: String,
    detail: Option<String>,
    selected: bool,
    opens_profiles: bool,
    action: Option<Action>,
    fg: Hsla,
    muted: Hsla,
    accent: Hsla,
    accent_fg: Hsla,
    hover: Hsla,
) -> gpui::AnyElement {
    let row_icon = if selected { accent_fg } else { muted };
    let row_text = if selected { accent_fg } else { fg };
    let mut label_col = v_flex().min_w(px(0.)).flex_1().child(label);
    if let Some(detail) = detail {
        label_col = label_col.child(
            div()
                .text_xs()
                .text_color(if selected {
                    accent_fg.opacity(0.72)
                } else {
                    muted
                })
                .truncate()
                .child(detail),
        );
    }

    let hover_app = app.clone();
    let click_app = app.clone();
    let row = h_flex()
        .id(id)
        .w_full()
        .min_h(px(34.))
        .px_2()
        .py(px(5.))
        .gap_2()
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(row_text)
        .when(selected, |s| {
            s.bg(accent).font_weight(gpui::FontWeight::MEDIUM)
        })
        .when(!selected, |s| s.hover(move |h| h.bg(hover)))
        .child(icon(glyph, 15.0, row_icon))
        .child(label_col)
        .when(opens_profiles, |s| {
            s.child(icon(
                AtlasIcon::CaretRight,
                12.0,
                if selected {
                    accent_fg.opacity(0.78)
                } else {
                    muted.opacity(0.72)
                },
            ))
        })
        .on_mouse_move(move |_ev, _window, cx| {
            if let Some(app) = hover_app.upgrade() {
                app.update(cx, |this, cx| {
                    if this.state.profile_ui.atlas_launcher_profiles_open != opens_profiles {
                        this.state.profile_ui.atlas_launcher_profiles_open = opens_profiles;
                        this.state.profile_ui.confirming_delete_profile = None;
                        cx.notify();
                    }
                });
            }
        });

    if let Some(action) = action {
        row.on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
            cx.stop_propagation();
            if let Some(app) = click_app.upgrade() {
                app.update(cx, |this, cx| {
                    this.process_actions(vec![action.clone()]);
                    cx.notify();
                });
            }
            popover.update(cx, |state, cx| state.dismiss(window, cx));
        })
        .into_any_element()
    } else {
        row.into_any_element()
    }
}

#[derive(IntoElement)]
struct AtlasLauncherTrigger {
    profile_name: String,
    selected: bool,
    collapsed: bool,
    fg: Hsla,
    muted: Hsla,
    bg: Hsla,
    hover: Hsla,
    active: Hsla,
}

impl Selectable for AtlasLauncherTrigger {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for AtlasLauncherTrigger {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let fill = if self.selected { self.active } else { self.bg };
        h_flex()
            .id("sidebar-atlas-trigger")
            .w_full()
            .h(px(34.))
            .px(px(6.))
            .gap(px(6.))
            .items_center()
            .justify_center()
            .rounded(px(9.))
            .bg(fill)
            .hover(move |s| s.bg(self.hover))
            .text_color(self.fg)
            .child(
                div()
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(atlas_logo(if self.collapsed { 22.0 } else { 21.0 })),
            )
            .when(!self.collapsed, |s| {
                s.justify_start()
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Atlas"))
                    .child(icon(AtlasIcon::CaretRight, 11.0, self.muted.opacity(0.78)))
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .truncate()
                            .text_color(self.muted)
                            .child(self.profile_name),
                    )
            })
    }
}

impl EventEmitter<PanelEvent> for SidebarPanel {}

impl Focusable for SidebarPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for SidebarPanel {
    fn panel_name(&self) -> &'static str {
        "Sidebar"
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        SharedString::from(rust_i18n::t!("sidebar_places"))
    }

    fn closable(&self, _cx: &App) -> bool {
        false
    }

    fn zoomable(&self, _cx: &App) -> Option<PanelControl> {
        None
    }

    /// Hide the dock tab/title bar: the sidebar already shows its own section
    /// headers ("Lugares", "Marcadores", …), so the dock title would be a redundant
    /// duplicate. The hosting TabPanel still provides the rounded glass card.
    fn show_title_bar(&self, _cx: &App) -> bool {
        false
    }
}

impl Render for SidebarPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf::count("render.sidebar");
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        let (fg, muted, accent) = {
            let t = cx.theme();
            (t.foreground, t.muted_foreground, t.accent)
        };
        // Subtle fills tuned for the light glass surface (Figma item states).
        let hover = hsla(0., 0., 0., 0.05);
        let active_bg = hsla(0., 0., 0., 0.09);
        // The focused pane's current location — used to mark matching place/virtual row active.
        let current_location = app
            .read(cx)
            .state
            .active_tab_data()
            .map(|t| t.location.clone());
        let current_path = current_location
            .as_ref()
            .and_then(|l| l.fs_dir().map(|p| p.to_path_buf()));

        let (active_profile_name, sidebar_collapsed) = {
            let app_ref = app.read(cx);
            (
                app_ref.state.profiles.active().name.clone(),
                app_ref.state.settings.sidebar_collapsed,
            )
        };
        let launcher_bg = hsla(0., 0., 1., 0.0);
        let atlas_launcher_popover = Popover::new("sidebar-atlas-launcher")
            .appearance(false)
            .anchor(Anchor::TopLeft)
            .trigger(AtlasLauncherTrigger {
                profile_name: active_profile_name,
                selected: false,
                collapsed: sidebar_collapsed,
                fg,
                muted,
                bg: launcher_bg,
                hover: hsla(0., 0., 1., 0.075),
                active: hsla(0., 0., 1., 0.065),
            })
            .content({
                let app = app.downgrade();
                move |_popover, window, cx| {
                    atlas_launcher_content(app.clone(), cx.entity(), window, cx)
                }
            })
            .on_open_change({
                let app = app.downgrade();
                move |_open, _window, cx| {
                    if let Some(app) = app.upgrade() {
                        app.update(cx, |this, cx| {
                            this.state.profile_ui.confirming_delete_profile = None;
                            this.state.profile_ui.atlas_launcher_profiles_open = false;
                            cx.notify();
                        });
                    }
                }
            });
        let collapse_button = {
            let app = app.clone();
            let label = if sidebar_collapsed {
                rust_i18n::t!("sidebar_expand").to_string()
            } else {
                rust_i18n::t!("sidebar_collapse").to_string()
            };
            div()
                .id("sidebar-collapse-toggle")
                .size(px(30.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(9.))
                .bg(hsla(0., 0., 1., 0.0))
                .hover(|s| s.bg(hsla(0., 0., 1., 0.055)).text_color(fg))
                .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                .child(icon(AtlasIcon::SidebarSimple, 16.0, muted))
                .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                    cx.stop_propagation();
                    app.update(cx, |this, cx| {
                        this.process_actions(vec![Action::ToggleSidebarCollapsed]);
                        cx.notify();
                    });
                })
        };
        let atlas_header = if sidebar_collapsed {
            h_flex()
                .w_full()
                .pb(px(8.))
                .items_center()
                .justify_center()
                .child(atlas_launcher_popover)
                .into_any_element()
        } else {
            h_flex()
                .w_full()
                .pb(px(8.))
                .gap(px(5.))
                .child(div().flex_1().child(atlas_launcher_popover))
                .child(collapse_button)
                .into_any_element()
        };

        let mut col = v_flex()
            .size_full()
            .p_2()
            .gap_1()
            // Transparent: the hosting TabPanel (fork) is the rounded glass card. The
            // sidebar just fills it; `show_title_bar` is false so there's no dock title.
            .text_color(fg)
            .text_sm()
            .child(atlas_header)
            .on_action(
                cx.listener(|this, action: &SidebarToggleSection, _window, cx| {
                    if let Some(app) = this.app.upgrade() {
                        let index = action.index;
                        app.update(cx, |app, cx| {
                            app.process_actions(vec![Action::ToggleSidebarSection(index)]);
                            cx.notify();
                        });
                    }
                }),
            )
            .on_action(
                cx.listener(|this, action: &SidebarHideSection, _window, cx| {
                    if let Some(app) = this.app.upgrade() {
                        let index = action.index;
                        app.update(cx, |app, cx| {
                            app.process_actions(vec![Action::SetSidebarSectionHidden {
                                index,
                                hidden: true,
                            }]);
                            cx.notify();
                        });
                    }
                }),
            )
            .on_action(
                cx.listener(|this, action: &SidebarMoveSectionUp, _window, cx| {
                    if let Some(app) = this.app.upgrade() {
                        let index = action.index;
                        app.update(cx, |app, cx| {
                            app.process_actions(vec![Action::MoveSidebarSectionUp(index)]);
                            cx.notify();
                        });
                    }
                }),
            )
            .on_action(
                cx.listener(|this, action: &SidebarMoveSectionDown, _window, cx| {
                    if let Some(app) = this.app.upgrade() {
                        let index = action.index;
                        app.update(cx, |app, cx| {
                            app.process_actions(vec![Action::MoveSidebarSectionDown(index)]);
                            cx.notify();
                        });
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &SidebarResetSections, _window, cx| {
                if let Some(app) = this.app.upgrade() {
                    app.update(cx, |app, cx| {
                        app.process_actions(vec![Action::ResetSidebarSections]);
                        cx.notify();
                    });
                }
            }))
            .on_action(cx.listener(|this, _: &SidebarNewGlobalTag, _window, cx| {
                if let Some(app) = this.app.upgrade() {
                    app.update(cx, |app, cx| {
                        app.process_actions(vec![Action::OpenCreateTagOverlay { global: true }]);
                        cx.notify();
                    });
                }
            }))
            .on_action(cx.listener(|this, _: &SidebarNewProfileTag, _window, cx| {
                if let Some(app) = this.app.upgrade() {
                    app.update(cx, |app, cx| {
                        app.process_actions(vec![Action::OpenCreateTagOverlay { global: false }]);
                        cx.notify();
                    });
                }
            }));
        #[cfg(feature = "cloud")]
        {
            col = col.on_action(
                cx.listener(|this, action: &RemoveCloudRemote, _window, cx| {
                    if let Some(app) = this.app.upgrade() {
                        let id = action.id.clone();
                        app.update(cx, |app, cx| {
                            app.handle_cloud_action(crate::app::cloud::CloudAction::RemoveRemote(
                                id,
                            ));
                            cx.notify();
                        });
                    }
                }),
            );
            col = col.on_action(cx.listener(
                |this, action: &StartRenameCloudRemote, _window, cx| {
                    if let Some(app) = this.app.upgrade() {
                        let id = action.id.clone();
                        app.update(cx, |app, cx| {
                            app.handle_cloud_action(
                                crate::app::cloud::CloudAction::StartRenameRemote(id),
                            );
                            cx.notify();
                        });
                    }
                },
            ));
            #[cfg(windows)]
            {
                col = col.on_action(cx.listener(
                    |this, action: &InstallWinFspForRemote, _window, cx| {
                        if let Some(app) = this.app.upgrade() {
                            let id = action.id.clone();
                            app.update(cx, |app, _cx| {
                                let _ = app.cloud_tx.send(crate::cloud::CloudCmd::InstallWinFsp {
                                    retry_id: Some(id),
                                });
                            });
                        }
                    },
                ));
            }
        }
        if sidebar_collapsed {
            col = render_collapsed_sections(
                col,
                &app,
                hover,
                active_bg,
                accent,
                fg,
                current_path.as_deref(),
                current_location.as_ref(),
                cx,
            );
            return div()
                .id("sidebar-scroll")
                .size_full()
                .relative()
                .on_prepaint(sidebar_scroll_recorder(app.clone(), self.scroll.clone()))
                .child(
                    col.id("sidebar-scroll-content")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll),
                )
                .into_any_element();
        }
        let sections: Vec<(usize, SidebarSection)> = app
            .read(cx)
            .state
            .profiles
            .active()
            .sidebar_sections
            .iter()
            .cloned()
            .enumerate()
            .collect();
        let section_count = sections.len();

        for (idx, section) in sections {
            if section.hidden {
                continue;
            }
            col = col.child(section_header(
                idx,
                section_count,
                &section,
                &app,
                muted,
                hover,
            ));
            if !section.collapsed {
                col = render_section_body(
                    col,
                    &section.kind,
                    &app,
                    muted,
                    hover,
                    active_bg,
                    accent,
                    fg,
                    current_path.as_deref(),
                    current_location.as_ref(),
                    cx,
                );
            }
        }

        if !app
            .read(cx)
            .state
            .profiles
            .active()
            .sidebar_sections
            .iter()
            .any(|section| !section.hidden)
        {
            col = col.child(
                div()
                    .px_2()
                    .py_2()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("sidebar_all_sections_hidden")),
            );
        }

        div()
            .id("sidebar-scroll")
            .size_full()
            .relative()
            .on_prepaint(sidebar_scroll_recorder(app.clone(), self.scroll.clone()))
            .child(
                col.id("sidebar-scroll-content")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .pr(px(SCROLLBAR_GUTTER)),
            )
            .vertical_scrollbar(&self.scroll)
            .into_any_element()
    }
}

/// A 16px Phosphor icon (`icons/ph/<name>.svg`), tinted with `color`.
fn ph(name: &str, color: Hsla) -> impl IntoElement {
    icon(ph_icon(name), 16.0, color)
}

#[inline]
fn ph_icon(name: &str) -> AtlasIcon {
    match name {
        "house" => AtlasIcon::House,
        "monitor" => AtlasIcon::Monitor,
        "file-text" => AtlasIcon::TextFile,
        "download-simple" => AtlasIcon::Download,
        "image" => AtlasIcon::Image,
        "music-notes" => AtlasIcon::Audio,
        "film-strip" => AtlasIcon::Video,
        "folder" => AtlasIcon::Folder,
        "cloud" => AtlasIcon::Cloud,
        "hard-drives" => AtlasIcon::HardDrives,
        "caret-down" => AtlasIcon::CaretDown,
        "caret-right" => AtlasIcon::CaretRight,
        "trash" => AtlasIcon::Trash,
        _ => AtlasIcon::File,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_collapsed_sections(
    mut col: gpui::Div,
    app: &Entity<AtlasApp>,
    hover: Hsla,
    active_bg: Hsla,
    accent: Hsla,
    fg: Hsla,
    current: Option<&std::path::Path>,
    current_location: Option<&Location>,
    cx: &mut Context<SidebarPanel>,
) -> gpui::Div {
    let sections: Vec<SidebarSection> = app
        .read(cx)
        .state
        .profiles
        .active()
        .sidebar_sections
        .iter()
        .filter(|section| !section.hidden)
        .cloned()
        .collect();
    let mut drew_section = false;
    for section in sections {
        let before = drew_section;
        let mut section_has_items = false;
        match section.kind {
            SectionKind::Places => {
                let places: Vec<(&'static str, String, PathBuf)> = [
                    (
                        "house",
                        rust_i18n::t!("sidebar_home").to_string(),
                        dirs::home_dir(),
                    ),
                    (
                        "monitor",
                        rust_i18n::t!("sidebar_desktop").to_string(),
                        dirs::desktop_dir(),
                    ),
                    (
                        "file-text",
                        rust_i18n::t!("sidebar_documents").to_string(),
                        dirs::document_dir(),
                    ),
                    (
                        "download-simple",
                        rust_i18n::t!("sidebar_downloads").to_string(),
                        dirs::download_dir(),
                    ),
                    (
                        "image",
                        rust_i18n::t!("sidebar_pictures").to_string(),
                        dirs::picture_dir(),
                    ),
                    (
                        "music-notes",
                        rust_i18n::t!("sidebar_music").to_string(),
                        dirs::audio_dir(),
                    ),
                    (
                        "film-strip",
                        rust_i18n::t!("sidebar_videos").to_string(),
                        dirs::video_dir(),
                    ),
                ]
                .into_iter()
                .filter_map(|(icon, label, p)| p.map(|p| (icon, label, p)))
                .collect();
                if !places.is_empty() || matches!(current_location, Some(Location::RecycleBin)) {
                    if before {
                        col = col.child(collapsed_divider());
                    }
                }
                for (icon_name, label, path) in places {
                    let active = current.is_some_and(|c| c == path.as_path());
                    col = col.child(collapsed_nav_row(
                        SharedString::from(format!("rail-place-{label}")),
                        label,
                        icon_name,
                        path,
                        app,
                        hover,
                        active_bg,
                        accent,
                        fg,
                        active,
                    ));
                }
                let rb_active = matches!(current_location, Some(Location::RecycleBin));
                col = col.child(collapsed_virtual_nav_row(
                    SharedString::from("rail-place-recycle-bin"),
                    rust_i18n::t!("sidebar_recycle_bin").to_string(),
                    "trash",
                    Location::RecycleBin,
                    app,
                    hover,
                    active_bg,
                    accent,
                    fg,
                    rb_active,
                ));
                section_has_items = true;
            }
            SectionKind::Bookmarks => {
                let bookmarks: Vec<(String, PathBuf)> = app
                    .read(cx)
                    .state
                    .profiles
                    .active()
                    .bookmarks
                    .iter()
                    .map(|b| (b.name.clone(), b.path.clone()))
                    .collect();
                if !bookmarks.is_empty() && before {
                    col = col.child(collapsed_divider());
                }
                for (name, path) in bookmarks {
                    let active = current.is_some_and(|c| c == path.as_path());
                    col = col.child(collapsed_nav_row(
                        SharedString::from(format!("rail-bm-{}", path.display())),
                        name,
                        "folder",
                        path,
                        app,
                        hover,
                        active_bg,
                        accent,
                        fg,
                        active,
                    ));
                    section_has_items = true;
                }
            }
            SectionKind::Tags => {
                let tags: Vec<(String, [u8; 3], TagId, bool)> = {
                    let app_ref = app.read(cx);
                    let profile = app_ref.state.profiles.active().name.clone();
                    app_ref
                        .state
                        .tag_cache
                        .visible_defs(&profile)
                        .map(|d| (d.name.clone(), d.color, d.id, d.profile.is_none()))
                        .collect()
                };
                if !tags.is_empty() && before {
                    col = col.child(collapsed_divider());
                }
                for (name, color, id, _global) in tags {
                    col = col.child(collapsed_tag_row(name, color, id, app, hover));
                    section_has_items = true;
                }
            }
            SectionKind::Drives => {
                let drives = crate::fs::mounts::list_drives();
                if !drives.is_empty() && before {
                    col = col.child(collapsed_divider());
                }
                for drive in drives {
                    let active = current.is_some_and(|c| c == drive.mount_point.as_path());
                    col = col.child(collapsed_drive_row(
                        drive, app, hover, active_bg, accent, fg, active,
                    ));
                    section_has_items = true;
                }
            }
            SectionKind::Recent => {
                let recent: Vec<PathBuf> = app
                    .read(cx)
                    .state
                    .settings
                    .recent_folders
                    .iter()
                    .rev()
                    .take(8)
                    .cloned()
                    .collect();
                if !recent.is_empty() && before {
                    col = col.child(collapsed_divider());
                }
                for path in recent {
                    let label = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    let active = current.is_some_and(|c| c == path.as_path());
                    col = col.child(collapsed_nav_row(
                        SharedString::from(format!("rail-recent-{}", path.display())),
                        label,
                        "folder",
                        path,
                        app,
                        hover,
                        active_bg,
                        accent,
                        fg,
                        active,
                    ));
                    section_has_items = true;
                }
            }
            SectionKind::Cloud => {
                #[cfg(feature = "cloud")]
                {
                    let remotes: Vec<atlas_model::cloud_model::CloudRemote> =
                        app.read(cx).state.cloud.remotes.clone();
                    if !remotes.is_empty() && before {
                        col = col.child(collapsed_divider());
                    }
                    for remote in remotes {
                        col = col.child(collapsed_cloud_remote_row(
                            remote, app, hover, active_bg, accent, fg,
                        ));
                        section_has_items = true;
                    }
                }
            }
        }
        drew_section |= section_has_items;
    }
    col
}

fn collapsed_divider() -> impl IntoElement {
    div()
        .h(px(1.))
        .w_full()
        .my(px(6.))
        .bg(hsla(0., 0., 1., 0.08))
}

#[allow(clippy::too_many_arguments)]
fn collapsed_nav_row(
    id: SharedString,
    label: String,
    icon_name: &'static str,
    path: PathBuf,
    app: &Entity<AtlasApp>,
    hover: Hsla,
    active_bg: Hsla,
    accent: Hsla,
    fg: Hsla,
    active: bool,
) -> impl IntoElement {
    let app = app.clone();
    let tooltip = label.clone();
    let hover_id = DragHoverId::Sidebar(id.to_string());
    let drop_path = path.clone();
    let drop_path_external = path.clone();
    let drop_app = app.clone();
    let drop_app_external = app.clone();
    let icon_color = if active {
        accent
    } else {
        hsla(fg.h, fg.s, fg.l, 0.84)
    };
    h_flex()
        .id(id)
        .w_full()
        .h(px(32.))
        .items_center()
        .justify_center()
        .rounded(px(9.))
        .cursor_pointer()
        .when(active, |s| s.bg(active_bg))
        .when(!active, |s| s.hover(|h| h.bg(hover)))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(icon(ph_icon(icon_name), 17.0, icon_color))
        .on_prepaint(drag_hover_recorder_with_drop(
            hover_id,
            DragHoverKind::Folder,
            DragHoverCommand::NavigatePath(path.clone()),
            DragHoverDropCommand::TransferTo(path.clone()),
            &app,
        ))
        .on_drop::<DraggedFiles>(move |d, _window, cx| {
            let paths = d.paths.clone();
            drop_app.update(cx, |this, cx| {
                this.execute_drag_hover_drop_command(
                    DragHoverDropCommand::TransferTo(drop_path.clone()),
                    paths,
                    true,
                );
                cx.notify();
            });
        })
        .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
            let paths = paths.paths().to_vec();
            drop_app_external.update(cx, |this, cx| {
                this.execute_drag_hover_drop_command(
                    DragHoverDropCommand::TransferTo(drop_path_external.clone()),
                    paths,
                    false,
                );
                cx.notify();
            });
        })
        .on_click(move |_ev, _window, cx| {
            let path = path.clone();
            app.update(cx, |this, cx| {
                let tile = this.state.focused_tile;
                this.navigate(tile, path);
                cx.notify();
            });
        })
}

#[allow(clippy::too_many_arguments)]
fn collapsed_virtual_nav_row(
    id: SharedString,
    label: String,
    icon_name: &'static str,
    loc: Location,
    app: &Entity<AtlasApp>,
    hover: Hsla,
    active_bg: Hsla,
    accent: Hsla,
    fg: Hsla,
    active: bool,
) -> impl IntoElement {
    let app = app.clone();
    let tooltip = label.clone();
    let hover_id = DragHoverId::Sidebar(id.to_string());
    let icon_color = if active {
        accent
    } else {
        hsla(fg.h, fg.s, fg.l, 0.84)
    };
    h_flex()
        .id(id)
        .w_full()
        .h(px(32.))
        .items_center()
        .justify_center()
        .rounded(px(9.))
        .cursor_pointer()
        .when(active, |s| s.bg(active_bg))
        .when(!active, |s| s.hover(|h| h.bg(hover)))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(icon(ph_icon(icon_name), 17.0, icon_color))
        .on_prepaint(drag_hover_recorder(
            hover_id,
            DragHoverKind::Nav,
            DragHoverCommand::NavigateLocation(loc.clone()),
            &app,
        ))
        .on_click(move |_ev, _window, cx| {
            let loc = loc.clone();
            app.update(cx, |this, cx| {
                let tile = this.state.focused_tile;
                this.navigate_to(tile, loc, NavKind::Push);
                cx.notify();
            });
        })
}

fn collapsed_tag_row(
    name: String,
    color: [u8; 3],
    id: TagId,
    app: &Entity<AtlasApp>,
    hover: Hsla,
) -> impl IntoElement {
    let app = app.clone();
    let tooltip = name.clone();
    let rgb =
        gpui::rgb((u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2]));
    let hover_id = DragHoverId::Sidebar(format!("rail-tag-{}", id.0));
    h_flex()
        .id(SharedString::from(format!("rail-tag-{}", id.0)))
        .w_full()
        .h(px(32.))
        .items_center()
        .justify_center()
        .rounded(px(9.))
        .cursor_pointer()
        .hover(|s| s.bg(hover))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(div().size(px(12.)).rounded_full().bg(rgb).shadow_sm())
        .on_prepaint(drag_hover_recorder_with_drop(
            hover_id,
            DragHoverKind::Nav,
            DragHoverCommand::OpenTag(id),
            DragHoverDropCommand::AssignTag(id),
            &app,
        ))
        .on_drop::<DraggedFiles>({
            let app = app.clone();
            move |d, _window, cx| {
                let paths = d.paths.clone();
                app.update(cx, |this, cx| {
                    this.process_actions(vec![Action::AssignTagToFiles { tag: id, paths }]);
                    cx.notify();
                });
            }
        })
        .on_drop::<gpui::ExternalPaths>({
            let app = app.clone();
            move |paths, _window, cx| {
                let paths = paths.paths().to_vec();
                app.update(cx, |this, cx| {
                    this.process_actions(vec![Action::AssignTagToFiles { tag: id, paths }]);
                    cx.notify();
                });
            }
        })
        .on_click(move |_ev, _window, cx| {
            app.update(cx, |this, cx| {
                let tile = this.state.focused_tile;
                this.open_tag_results(tile, id);
                cx.notify();
            });
        })
}

#[allow(clippy::too_many_arguments)]
fn collapsed_drive_row(
    drive: crate::fs::mounts::DriveInfo,
    app: &Entity<AtlasApp>,
    hover: Hsla,
    active_bg: Hsla,
    accent: Hsla,
    fg: Hsla,
    active: bool,
) -> impl IntoElement {
    let free_gb = drive.available_bytes as f64 / 1_073_741_824.0;
    let tooltip = format!(
        "{} · {}",
        drive.label,
        rust_i18n::t!("sidebar_free_space", gb = format!("{free_gb:.0}"))
    );
    collapsed_nav_row(
        SharedString::from(format!("rail-drive-{}", drive.mount_point.display())),
        tooltip,
        "hard-drives",
        drive.mount_point,
        app,
        hover,
        active_bg,
        accent,
        fg,
        active,
    )
}

#[cfg(feature = "cloud")]
#[allow(clippy::too_many_arguments)]
fn collapsed_cloud_remote_row(
    remote: atlas_model::cloud_model::CloudRemote,
    app: &Entity<AtlasApp>,
    hover: Hsla,
    active_bg: Hsla,
    accent: Hsla,
    fg: Hsla,
) -> impl IntoElement {
    use crate::app::cloud::CloudAction;
    use atlas_model::cloud_model::RemoteStatus;

    let is_mounted = matches!(remote.status, RemoteStatus::Mounted(_));
    let target = if is_mounted {
        Action::Cloud(CloudAction::OpenMount(remote.id.clone()))
    } else {
        Action::Cloud(CloudAction::OpenAccess(remote.id.clone()))
    };
    let app_click = app.clone();
    let id = SharedString::from(format!("rail-cloud-{}", remote.id));
    let label = remote.name.clone();
    let hover_id = DragHoverId::Sidebar(id.to_string());
    h_flex()
        .id(id)
        .w_full()
        .h(px(32.))
        .items_center()
        .justify_center()
        .rounded(px(9.))
        .cursor_pointer()
        .when(is_mounted, |s| s.bg(active_bg))
        .when(!is_mounted, |s| s.hover(|h| h.bg(hover)))
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        .child(icon(
            AtlasIcon::Cloud,
            17.0,
            if is_mounted {
                accent
            } else {
                hsla(fg.h, fg.s, fg.l, 0.84)
            },
        ))
        .on_prepaint(drag_hover_recorder(
            hover_id,
            DragHoverKind::Nav,
            DragHoverCommand::Process(target.clone()),
            app,
        ))
        .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
            app_click.update(cx, |this, cx| {
                this.process_actions(vec![target.clone()]);
                cx.notify();
            });
        })
}

#[allow(clippy::too_many_arguments)]
fn render_section_body(
    mut col: gpui::Div,
    kind: &SectionKind,
    app: &Entity<AtlasApp>,
    muted: Hsla,
    hover: Hsla,
    active_bg: Hsla,
    accent: Hsla,
    fg: Hsla,
    current: Option<&std::path::Path>,
    current_location: Option<&Location>,
    cx: &mut Context<SidebarPanel>,
) -> gpui::Div {
    match kind {
        SectionKind::Places => {
            let places: Vec<(&'static str, String, PathBuf)> = [
                (
                    "house",
                    rust_i18n::t!("sidebar_home").to_string(),
                    dirs::home_dir(),
                ),
                (
                    "monitor",
                    rust_i18n::t!("sidebar_desktop").to_string(),
                    dirs::desktop_dir(),
                ),
                (
                    "file-text",
                    rust_i18n::t!("sidebar_documents").to_string(),
                    dirs::document_dir(),
                ),
                (
                    "download-simple",
                    rust_i18n::t!("sidebar_downloads").to_string(),
                    dirs::download_dir(),
                ),
                (
                    "image",
                    rust_i18n::t!("sidebar_pictures").to_string(),
                    dirs::picture_dir(),
                ),
                (
                    "music-notes",
                    rust_i18n::t!("sidebar_music").to_string(),
                    dirs::audio_dir(),
                ),
                (
                    "film-strip",
                    rust_i18n::t!("sidebar_videos").to_string(),
                    dirs::video_dir(),
                ),
            ]
            .into_iter()
            .filter_map(|(icon, label, p)| p.map(|p| (icon, label, p)))
            .collect();
            for (icon, label, path) in places {
                let active = current.is_some_and(|c| c == path.as_path());
                col = col.child(nav_row(
                    SharedString::from(format!("place-{label}")),
                    label.clone(),
                    icon,
                    path,
                    app,
                    hover,
                    active_bg,
                    accent,
                    fg,
                    active,
                ));
            }
            let rb_active = matches!(current_location, Some(Location::RecycleBin));
            col = col.child(virtual_nav_row(
                SharedString::from("place-recycle-bin"),
                rust_i18n::t!("sidebar_recycle_bin").to_string(),
                "trash",
                Location::RecycleBin,
                app,
                hover,
                active_bg,
                accent,
                fg,
                rb_active,
            ));
        }
        SectionKind::Bookmarks => {
            let bookmarks: Vec<(String, PathBuf)> = app
                .read(cx)
                .state
                .profiles
                .active()
                .bookmarks
                .iter()
                .map(|b| (b.name.clone(), b.path.clone()))
                .collect();
            if bookmarks.is_empty() {
                col = col.child(empty_row(
                    rust_i18n::t!("sidebar_no_bookmarks").to_string(),
                    muted,
                ));
            } else {
                for (name, path) in bookmarks {
                    let id = SharedString::from(format!("bm-{}", path.display()));
                    let active = current.is_some_and(|c| c == path.as_path());
                    col = col.child(nav_row(
                        id, name, "folder", path, app, hover, active_bg, accent, fg, active,
                    ));
                }
            }
        }
        SectionKind::Tags => {
            let tags: Vec<(String, [u8; 3], TagId, bool)> = {
                let app_ref = app.read(cx);
                let profile = app_ref.state.profiles.active().name.clone();
                app_ref
                    .state
                    .tag_cache
                    .visible_defs(&profile)
                    .map(|d| (d.name.clone(), d.color, d.id, d.profile.is_none()))
                    .collect()
            };
            if tags.is_empty() {
                col = col.child(empty_row(
                    rust_i18n::t!("sidebar_no_tags").to_string(),
                    muted,
                ));
            } else {
                for (name, color, id, global) in tags {
                    col = col.child(tag_row(name, color, id, global, app, hover, fg));
                }
            }
        }
        SectionKind::Drives => {
            let drives = crate::fs::mounts::list_drives();
            if drives.is_empty() {
                col = col.child(empty_row(
                    rust_i18n::t!("sidebar_no_drives").to_string(),
                    muted,
                ));
            } else {
                for drive in drives {
                    let active = current.is_some_and(|c| c == drive.mount_point.as_path());
                    col = col.child(drive_row(
                        drive, app, hover, active_bg, accent, fg, muted, active,
                    ));
                }
            }
        }
        SectionKind::Recent => {
            let recent: Vec<PathBuf> = app
                .read(cx)
                .state
                .settings
                .recent_folders
                .iter()
                .rev()
                .take(8)
                .cloned()
                .collect();
            if recent.is_empty() {
                col = col.child(empty_row(
                    rust_i18n::t!("sidebar_no_recent").to_string(),
                    muted,
                ));
            } else {
                for path in recent {
                    let label = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    let active = current.is_some_and(|c| c == path.as_path());
                    col = col.child(nav_row(
                        SharedString::from(format!("recent-{}", path.display())),
                        label,
                        "folder",
                        path,
                        app,
                        hover,
                        active_bg,
                        accent,
                        fg,
                        active,
                    ));
                }
            }
        }
        SectionKind::Cloud => {
            #[cfg(feature = "cloud")]
            {
                let remotes: Vec<atlas_model::cloud_model::CloudRemote> =
                    app.read(cx).state.cloud.remotes.clone();
                for remote in remotes {
                    col = col.child(cloud_remote_row(
                        remote, app, hover, active_bg, accent, fg, muted,
                    ));
                }
                col = col.child(add_cloud_row(app, hover, muted, fg));
            }
            #[cfg(not(feature = "cloud"))]
            {
                col = col.child(empty_row(
                    rust_i18n::t!("sidebar_no_cloud").to_string(),
                    muted,
                ));
            }
        }
    }
    col
}

/// One clickable tag row that opens its virtual "tag results" tab.
fn tag_row(
    name: String,
    color: [u8; 3],
    id: TagId,
    global: bool,
    app: &Entity<AtlasApp>,
    hover: Hsla,
    fg: Hsla,
) -> impl IntoElement {
    let app = app.clone();
    let edit_app = app.clone();
    let edit_name = name.clone();
    let group = SharedString::from(format!("tag-row-{}", id.0));
    let rgb =
        gpui::rgb((u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2]));
    let hover_id = DragHoverId::Sidebar(format!("tag-{}", id.0));
    h_flex()
        .id(SharedString::from(format!("tag-{}", id.0)))
        .group(group.clone())
        .w_full()
        .px_2()
        .h(px(24.))
        .gap(px(8.))
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(fg)
        .hover(|s| s.bg(hover))
        .child(div().size(px(11.)).rounded_full().bg(rgb))
        .child(div().min_w(px(0.)).flex_1().truncate().child(name))
        .child(
            div()
                .id(SharedString::from(format!("tag-{}-rename", id.0)))
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .cursor_pointer()
                .invisible()
                .group_hover(group, gpui::Styled::visible)
                .hover(|h| h.bg(hover))
                .tooltip(|window, cx| {
                    Tooltip::new(rust_i18n::t!("tag_rename").to_string()).build(window, cx)
                })
                .child(icon(AtlasIcon::PencilSimple, 12.0, fg))
                .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                    cx.stop_propagation();
                    edit_app.update(cx, |this, cx| {
                        this.process_actions(vec![Action::OpenRenameTagOverlay {
                            tag: id,
                            name: edit_name.clone(),
                            color,
                            global,
                        }]);
                        cx.notify();
                    });
                }),
        )
        .on_prepaint(drag_hover_recorder_with_drop(
            hover_id,
            DragHoverKind::Nav,
            DragHoverCommand::OpenTag(id),
            DragHoverDropCommand::AssignTag(id),
            &app,
        ))
        .on_drop::<DraggedFiles>({
            let app = app.clone();
            move |d, _window, cx| {
                let paths = d.paths.clone();
                app.update(cx, |this, cx| {
                    this.process_actions(vec![Action::AssignTagToFiles { tag: id, paths }]);
                    cx.notify();
                });
            }
        })
        .on_drop::<gpui::ExternalPaths>({
            let app = app.clone();
            move |paths, _window, cx| {
                let paths = paths.paths().to_vec();
                app.update(cx, |this, cx| {
                    this.process_actions(vec![Action::AssignTagToFiles { tag: id, paths }]);
                    cx.notify();
                });
            }
        })
        .on_click(move |_ev, _window, cx| {
            app.update(cx, |this, cx| {
                let tile = this.state.focused_tile;
                this.open_tag_results(tile, id);
                cx.notify();
            });
        })
}

/// One mounted volume: clickable row that navigates to its mount point, with a thin
/// fullness bar (used fraction). The bar turns red when the volume is nearly full
/// (>90%). Taller than a `nav_row` to fit the label + bar (the Figma mock had no bars).
#[allow(clippy::too_many_arguments)]
fn drive_row(
    drive: crate::fs::mounts::DriveInfo,
    app: &Entity<AtlasApp>,
    hover: Hsla,
    active_bg: Hsla,
    accent: Hsla,
    fg: Hsla,
    muted: Hsla,
    active: bool,
) -> impl IntoElement {
    let app = app.clone();
    let frac = drive.used_fraction();
    // Alert color when nearly full, otherwise the accent.
    let bar_color = if frac > 0.9 {
        hsla(0., 0.70, 0.55, 1.0)
    } else {
        accent
    };
    let free_gb = drive.available_bytes as f64 / 1_073_741_824.0;
    let free_label = rust_i18n::t!("sidebar_free_space", gb = format!("{free_gb:.0}"));
    let path = drive.mount_point.clone();
    let drop_path = path.clone();
    let drop_path_external = path.clone();
    let id = SharedString::from(format!("drive-{}", drive.mount_point.display()));
    let hover_id = DragHoverId::Sidebar(format!("drive-{}", drive.mount_point.display()));
    let icon_color = hsla(fg.h, fg.s, fg.l, 0.85);
    let drop_app = app.clone();
    let drop_app_external = app.clone();
    v_flex()
        .id(id)
        .w_full()
        .px_2()
        .py(px(4.))
        .gap(px(3.))
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(fg)
        .when(active, |s| s.bg(active_bg))
        .when(!active, |s| s.hover(|h| h.bg(hover)))
        .child(
            h_flex()
                .w_full()
                .gap(px(8.))
                .items_center()
                .child(ph("hard-drives", icon_color))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .child(SharedString::from(drive.label.clone())),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(SharedString::from(free_label)),
                ),
        )
        .child(
            // Fullness track + filled portion (width = used fraction).
            div()
                .w_full()
                .h(px(4.))
                .rounded(px(2.))
                .bg(crate::ui::glass::track_fill())
                .child(
                    div()
                        .h_full()
                        .w(relative(frac))
                        .rounded(px(2.))
                        .bg(bar_color),
                ),
        )
        .on_prepaint(drag_hover_recorder_with_drop(
            hover_id,
            DragHoverKind::Folder,
            DragHoverCommand::NavigatePath(path.clone()),
            DragHoverDropCommand::TransferTo(path.clone()),
            &app,
        ))
        .on_drop::<DraggedFiles>(move |d, _window, cx| {
            let paths = d.paths.clone();
            drop_app.update(cx, |this, cx| {
                this.execute_drag_hover_drop_command(
                    DragHoverDropCommand::TransferTo(drop_path.clone()),
                    paths,
                    true,
                );
                cx.notify();
            });
        })
        .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
            let paths = paths.paths().to_vec();
            drop_app_external.update(cx, |this, cx| {
                this.execute_drag_hover_drop_command(
                    DragHoverDropCommand::TransferTo(drop_path_external.clone()),
                    paths,
                    false,
                );
                cx.notify();
            });
        })
        .on_click(move |_ev, _window, cx| {
            let path = path.clone();
            app.update(cx, |this, cx| {
                let tile = this.state.focused_tile;
                this.navigate(tile, path);
                cx.notify();
            });
        })
}

fn section_header(
    idx: usize,
    section_count: usize,
    section: &SidebarSection,
    app: &Entity<AtlasApp>,
    muted: Hsla,
    hover: Hsla,
) -> impl IntoElement {
    let label = sidebar_label(&section.kind);
    let app_click = app.clone();
    // Disclosure chevron on the right (Figma "Section Header").
    let chevron = if section.collapsed {
        "caret-right"
    } else {
        "caret-down"
    };
    let menu_kind = section.kind.clone();
    let menu_collapsed = section.collapsed;
    h_flex()
        .id(SharedString::from(format!("sidebar-section-{idx}")))
        .w_full()
        .px_2()
        .pt(px(14.))
        .pb(px(4.))
        .gap_1()
        .items_center()
        .text_color(muted)
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer()
        .rounded(px(6.))
        .hover(|s| s.bg(hover))
        .child(div().flex_1().child(SharedString::from(label.to_string())))
        .child(ph(chevron, muted))
        .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
            app_click.update(cx, |this, cx| {
                this.process_actions(vec![Action::ToggleSidebarSection(idx)]);
                cx.notify();
            });
        })
        .context_menu(move |menu, _window, _cx| {
            sidebar_context_menu(menu, idx, section_count, menu_kind.clone(), menu_collapsed)
        })
}

fn sidebar_context_menu(
    menu: PopupMenu,
    idx: usize,
    section_count: usize,
    kind: SectionKind,
    collapsed: bool,
) -> PopupMenu {
    let mut menu = menu
        .menu(
            if collapsed {
                rust_i18n::t!("sidebar_expand")
            } else {
                rust_i18n::t!("sidebar_collapse")
            },
            Box::new(SidebarToggleSection { index: idx }),
        )
        .menu(
            rust_i18n::t!("sidebar_hide_section"),
            Box::new(SidebarHideSection { index: idx }),
        )
        .separator();
    if idx > 0 {
        menu = menu.menu(
            rust_i18n::t!("sidebar_move_up"),
            Box::new(SidebarMoveSectionUp { index: idx }),
        );
    }
    if idx + 1 < section_count {
        menu = menu.menu(
            rust_i18n::t!("sidebar_move_down"),
            Box::new(SidebarMoveSectionDown { index: idx }),
        );
    }
    menu = menu.separator().menu(
        rust_i18n::t!("sidebar_restore_order"),
        Box::new(SidebarResetSections),
    );
    if kind == SectionKind::Tags {
        menu = menu
            .separator()
            .menu(
                rust_i18n::t!("sidebar_new_global_tag"),
                Box::new(SidebarNewGlobalTag),
            )
            .menu(
                rust_i18n::t!("sidebar_new_profile_tag"),
                Box::new(SidebarNewProfileTag),
            );
    }
    menu
}

fn sidebar_label(kind: &SectionKind) -> String {
    match kind {
        SectionKind::Places => rust_i18n::t!("sidebar_places").to_string(),
        SectionKind::Bookmarks => rust_i18n::t!("sidebar_bookmarks").to_string(),
        SectionKind::Tags => rust_i18n::t!("sidebar_tags").to_string(),
        SectionKind::Drives => rust_i18n::t!("sidebar_drives").to_string(),
        SectionKind::Cloud => rust_i18n::t!("sidebar_cloud").to_string(),
        SectionKind::Recent => rust_i18n::t!("sidebar_recent").to_string(),
    }
}

fn empty_row(label: String, muted: Hsla) -> impl IntoElement {
    div().px_2().py_1().text_xs().text_color(muted).child(label)
}

/// One clickable row that navigates the focused pane to `path`. Styled per the Figma
/// sidebar item: Phosphor icon + label, rounded, with hover and an active (current
/// folder) state that tints the icon/label with the accent.
#[allow(clippy::too_many_arguments)]
fn nav_row(
    id: SharedString,
    label: String,
    icon: &'static str,
    path: PathBuf,
    app: &Entity<AtlasApp>,
    hover: Hsla,
    active_bg: Hsla,
    accent: Hsla,
    fg: Hsla,
    active: bool,
) -> impl IntoElement {
    let app = app.clone();
    let hover_id = DragHoverId::Sidebar(id.to_string());
    let drop_path = path.clone();
    let drop_path_external = path.clone();
    let drop_app = app.clone();
    let drop_app_external = app.clone();
    let content = if active { accent } else { fg };
    let icon_color = if active {
        accent
    } else {
        // Slightly dimmed icon for inactive rows (Figma: ~85% ink).
        hsla(fg.h, fg.s, fg.l, 0.85)
    };
    h_flex()
        .id(id)
        .w_full()
        .h(px(24.))
        .px_2()
        .gap(px(8.))
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(content)
        .when(active, |s| {
            s.bg(active_bg).font_weight(gpui::FontWeight::MEDIUM)
        })
        .when(!active, |s| s.hover(|h| h.bg(hover)))
        .child(ph(icon, icon_color))
        .child(div().flex_1().truncate().child(SharedString::from(label)))
        .on_prepaint(drag_hover_recorder_with_drop(
            hover_id,
            DragHoverKind::Folder,
            DragHoverCommand::NavigatePath(path.clone()),
            DragHoverDropCommand::TransferTo(path.clone()),
            &app,
        ))
        .on_drop::<DraggedFiles>(move |d, _window, cx| {
            let paths = d.paths.clone();
            drop_app.update(cx, |this, cx| {
                this.execute_drag_hover_drop_command(
                    DragHoverDropCommand::TransferTo(drop_path.clone()),
                    paths,
                    true,
                );
                cx.notify();
            });
        })
        .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
            let paths = paths.paths().to_vec();
            drop_app_external.update(cx, |this, cx| {
                this.execute_drag_hover_drop_command(
                    DragHoverDropCommand::TransferTo(drop_path_external.clone()),
                    paths,
                    false,
                );
                cx.notify();
            });
        })
        .on_click(move |_ev, _window, cx| {
            let path = path.clone();
            app.update(cx, |this, cx| {
                let tile = this.state.focused_tile;
                this.navigate(tile, path);
                cx.notify();
            });
        })
}

/// Like `nav_row` but navigates to an arbitrary `Location` (for virtual places
/// like RecycleBin that have no real `PathBuf`).
#[allow(clippy::too_many_arguments)]
fn virtual_nav_row(
    id: SharedString,
    label: String,
    icon: &'static str,
    loc: Location,
    app: &Entity<AtlasApp>,
    hover: Hsla,
    active_bg: Hsla,
    accent: Hsla,
    fg: Hsla,
    active: bool,
) -> impl IntoElement {
    let app = app.clone();
    let hover_id = DragHoverId::Sidebar(id.to_string());
    let content = if active { accent } else { fg };
    let icon_color = if active {
        accent
    } else {
        hsla(fg.h, fg.s, fg.l, 0.85)
    };
    h_flex()
        .id(id)
        .w_full()
        .h(px(24.))
        .px_2()
        .gap(px(8.))
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(content)
        .when(active, |s| {
            s.bg(active_bg).font_weight(gpui::FontWeight::MEDIUM)
        })
        .when(!active, |s| s.hover(|h| h.bg(hover)))
        .child(ph(icon, icon_color))
        .child(div().flex_1().truncate().child(SharedString::from(label)))
        .on_prepaint(drag_hover_recorder(
            hover_id,
            DragHoverKind::Nav,
            DragHoverCommand::NavigateLocation(loc.clone()),
            &app,
        ))
        .on_click(move |_ev, _window, cx| {
            let loc = loc.clone();
            app.update(cx, |this, cx| {
                let tile = this.state.focused_tile;
                this.navigate_to(tile, loc, NavKind::Push);
                cx.notify();
            });
        })
}

/// Row for a configured cloud remote. Status label on the right; click opens access/mount.
/// Right-click menu: pending — needs dedicated gpui Action types (PopupMenu only accepts
/// Box<dyn gpui::Action>, not closures). For now, use the Add button to manage remotes.
#[cfg(feature = "cloud")]
fn cloud_remote_row(
    remote: atlas_model::cloud_model::CloudRemote,
    app: &Entity<AtlasApp>,
    hover: Hsla,
    _active_bg: Hsla,
    _accent: Hsla,
    fg: Hsla,
    muted: Hsla,
) -> impl IntoElement {
    use crate::app::cloud::CloudAction;
    use crate::input::action::Action;
    use atlas_model::cloud_model::RemoteStatus;

    let id = remote.id.clone();
    let name = remote.name.clone();
    let status = remote.status.clone();

    let (status_label, status_color): (Option<String>, Option<Hsla>) = match &status {
        RemoteStatus::Idle => (None, None),
        RemoteStatus::Connecting => (
            Some(rust_i18n::t!("cloud_status_connecting").to_string()),
            Some(muted),
        ),
        RemoteStatus::Mounting => (
            Some(rust_i18n::t!("cloud_status_mounting").to_string()),
            Some(muted),
        ),
        RemoteStatus::Mounted(_) => (
            Some(rust_i18n::t!("cloud_status_mounted").to_string()),
            Some(hsla(120. / 360., 0.55, 0.45, 1.0)),
        ),
        // Browse fallback: neutral badge, not an error. Click still opens Access tab.
        RemoteStatus::BrowseFallback(_) => (
            Some(rust_i18n::t!("cloud_status_browse").to_string()),
            Some(muted),
        ),
        RemoteStatus::Error(_) => (
            Some(rust_i18n::t!("cloud_status_error").to_string()),
            Some(hsla(0., 0.70, 0.55, 1.0)),
        ),
    };

    let is_mounted = matches!(status, RemoteStatus::Mounted(_));
    // A mounted remote has a real local path, so files can be dropped onto it like
    // any folder. Unmounted remotes stay hover-to-browse only (no transfer target).
    let mount_path = match &status {
        RemoteStatus::Mounted(p) => Some(p.clone()),
        _ => None,
    };
    let is_busy = matches!(status, RemoteStatus::Connecting | RemoteStatus::Mounting);
    let hover_action = if is_busy {
        None
    } else if is_mounted {
        Some(Action::Cloud(CloudAction::OpenMount(id.clone())))
    } else {
        Some(Action::Cloud(CloudAction::OpenAccess(id.clone())))
    };
    let hover_id = DragHoverId::Sidebar(format!("cloud-remote-{id}"));
    let icon_color = hsla(fg.h, fg.s, fg.l, 0.85);
    let app = app.clone();

    h_flex()
        .id(SharedString::from(format!("cloud-remote-{id}")))
        .w_full()
        .h(px(24.))
        .px_2()
        .gap(px(8.))
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(fg)
        .when(!is_busy, |s| s.hover(|h| h.bg(hover)))
        .child(ph("cloud", icon_color))
        .child(div().flex_1().truncate().child(SharedString::from(name)))
        .when_some(status_label, |row, label| {
            let color = status_color.unwrap_or(muted);
            row.child(
                div()
                    .text_xs()
                    .text_color(color)
                    .child(SharedString::from(label)),
            )
        })
        .when_some(hover_action.clone(), |row, action| {
            // Mounted: hover-browse + accept drops to the mount path. Otherwise hover only.
            match mount_path.clone() {
                Some(dest) => row.on_prepaint(drag_hover_recorder_with_drop(
                    hover_id,
                    DragHoverKind::Nav,
                    DragHoverCommand::Process(action),
                    DragHoverDropCommand::TransferTo(dest),
                    &app,
                )),
                None => row.on_prepaint(drag_hover_recorder(
                    hover_id,
                    DragHoverKind::Nav,
                    DragHoverCommand::Process(action),
                    &app,
                )),
            }
        })
        .when_some(mount_path.clone(), |row, dest| {
            let drop_dest = dest.clone();
            let drop_dest_ext = dest;
            let drop_app = app.clone();
            let drop_app_ext = app.clone();
            row.on_drop::<DraggedFiles>(move |d, _window, cx| {
                let paths = d.paths.clone();
                drop_app.update(cx, |this, cx| {
                    this.execute_drag_hover_drop_command(
                        DragHoverDropCommand::TransferTo(drop_dest.clone()),
                        paths,
                        true,
                    );
                    cx.notify();
                });
            })
            .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
                let paths = paths.paths().to_vec();
                drop_app_ext.update(cx, |this, cx| {
                    this.execute_drag_hover_drop_command(
                        DragHoverDropCommand::TransferTo(drop_dest_ext.clone()),
                        paths,
                        false,
                    );
                    cx.notify();
                });
            })
        })
        .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
            if is_busy {
                return;
            }
            let action = if is_mounted {
                Action::Cloud(CloudAction::OpenMount(id.clone()))
            } else {
                Action::Cloud(CloudAction::OpenAccess(id.clone()))
            };
            app.update(cx, |this, cx| {
                this.process_actions(vec![action]);
                cx.notify();
            });
        })
        .context_menu({
            let id2 = remote.id.clone();
            let is_browse_fallback = matches!(status, RemoteStatus::BrowseFallback(_));
            move |menu, _window, _cx| {
                let menu = menu
                    .menu(
                        rust_i18n::t!("cloud_ctx_rename"),
                        Box::new(StartRenameCloudRemote { id: id2.clone() }),
                    )
                    .menu(
                        rust_i18n::t!("cloud_ctx_remove"),
                        Box::new(RemoveCloudRemote { id: id2.clone() }),
                    );
                #[cfg(windows)]
                if is_browse_fallback {
                    return menu.menu(
                        rust_i18n::t!("cloud_ctx_install_winfsp"),
                        Box::new(InstallWinFspForRemote { id: id2.clone() }),
                    );
                }
                menu
            }
        })
}

/// Always-visible "+ Add cloud drive" row that incites the user to connect.
#[cfg(feature = "cloud")]
fn add_cloud_row(app: &Entity<AtlasApp>, hover: Hsla, muted: Hsla, fg: Hsla) -> impl IntoElement {
    use crate::app::cloud::CloudAction;
    use crate::input::action::Action;

    let app = app.clone();
    let icon_color = hsla(muted.h, muted.s, muted.l, 0.7);
    h_flex()
        .id("cloud-add-row")
        .w_full()
        .h(px(24.))
        .px_2()
        .gap(px(8.))
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(muted)
        .hover(|s| s.bg(hover).text_color(fg))
        .child(ph("cloud", icon_color))
        .child(
            div()
                .flex_1()
                .truncate()
                .child(rust_i18n::t!("cloud_add_remote").to_string()),
        )
        .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
            app.update(cx, |this, cx| {
                this.process_actions(vec![Action::Cloud(CloudAction::ShowAddDialog(true))]);
                cx.notify();
            });
        })
}
