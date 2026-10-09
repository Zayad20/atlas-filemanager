//! Phase B: left sidebar as a Dock `left_dock` panel.
//!
//! `SidebarPanel` is its own gpui `Entity` (like `FilePanel`) so it can live in the
//! `DockArea`'s left dock. It reads `AppState` through a weak handle and navigates
//! the focused pane on click. Places + Bookmarks land here; Tags/Drives/Cloud are
//! added with their subsystems (Phase H).

use crate::app::nav::NavKind;
use crate::app::App as AtlasApp;
use crate::input::action::Action;
use crate::ui::drag::{file_drag_value, DragView};
use crate::ui::drag_hover::{
    drag_hover_recorder, drag_hover_recorder_with_drop, sidebar_scroll_recorder, DragHoverCommand,
    DragHoverDropCommand, DragHoverId, DragHoverKind,
};
use crate::ui::glass::{control_colors, control_hover, layer_color, GlassExt};
use crate::ui::icons::{atlas_logo, file_type_icon, icon, is_windows_theme, theme_icon, AtlasIcon};
use crate::ui::root::DraggedFiles;
#[cfg(all(feature = "provider-host", windows))]
use crate::InstallWinFspForRemote;
use crate::{
    OpenPropertiesForPath, OpenSpaceLensForPath, OpenThisPcProperties, SidebarHideSection,
    SidebarMoveSectionDown, SidebarMoveSectionUp, SidebarNewGlobalTag, SidebarNewProfileTag,
    SidebarResetSections, SidebarToggleSection,
};
#[cfg(feature = "provider-host")]
use crate::{RemoveCloudRemote, StartRenameCloudRemote};
use atlas_model::file_entry::FileKind;
use atlas_model::location::Location;
use atlas_model::profiles::{SectionKind, SidebarSection};
use atlas_model::tag_cache::TagId;
use gpui::prelude::*;
use gpui::ColorExt as _;
use gpui::{
    auto, div, hsla, img, point, px, relative, Anchor, AnimationExt, AnyElement, App, Context,
    Entity, EventEmitter, FocusHandle, Focusable, Hsla, MouseButton, MouseDownEvent, RenderOnce,
    SharedString, Styled, WeakEntity, Window,
};
use gpui_component::dock::{Panel, PanelControl, PanelEvent};
use gpui_component::menu::{ContextMenuExt, PopupMenu};
use gpui_component::popover::{Popover, PopoverState};
use gpui_component::scroll::ScrollableElement;
use gpui_component::tooltip::Tooltip;
use gpui_component::{h_flex, kbd::Kbd, v_flex, ActiveTheme, ElementExt, Selectable};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct SidebarStyle {
    pub hover: Hsla,
    pub active_bg: Hsla,
    pub accent: Hsla,
    pub fg: Hsla,
    pub muted: Hsla,
    pub windows_theme: bool,
}

#[derive(Clone, Copy)]
pub struct MenuColors {
    pub fg: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub accent_fg: Hsla,
    pub hover: Hsla,
    pub action_hover: Hsla,
    pub danger: Hsla,
}

const SCROLLBAR_GUTTER: f32 = 16.0;
const ATLAS_LAUNCHER_MENU_WIDTH: f32 = 260.0;
const PROFILE_MENU_WIDTH: f32 = 236.0;

pub struct SidebarPanel {
    app: WeakEntity<AtlasApp>,
    focus_handle: FocusHandle,
    scroll: gpui::ScrollHandle,
    /// App is intentionally observed broadly so state changes made by actions,
    /// background drains, and filesystem notifications remain visible. This
    /// fingerprint gates the expensive retained-element rebuild: split/hover
    /// updates that do not change sidebar-visible state do not notify this view.
    sidebar_fingerprint: u64,
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
        cx.observe(&app, |this: &mut Self, app, cx| {
            let next = sidebar_fingerprint(app.read(cx));
            if this.sidebar_fingerprint != next {
                this.sidebar_fingerprint = next;
                crate::perf::count("sidebar.invalidate.changed");
                cx.notify();
            } else {
                crate::perf::count("sidebar.invalidate.skipped");
            }
        })
        .detach();

        let _ = (names, active_idx, window);

        Self {
            app: app.downgrade(),
            focus_handle: cx.focus_handle(),
            scroll: gpui::ScrollHandle::new(),
            // `new` is called while App is leased by its root render, so the
            // initial value is filled by the first observer callback.
            sidebar_fingerprint: 0,
        }
    }
}

/// Hash only state that can alter the sidebar's visual tree or active markers.
///
/// This is deliberately not a revision counter on `App`: App is also updated
/// for pane animation, hover, preview decoding, and other state that should not
/// invalidate the sidebar. The observer can stay broad while the render
/// invalidation remains narrow.
fn sidebar_fingerprint(app: &AtlasApp) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let state = &app.state;

    state.settings.sidebar_collapsed.hash(&mut hasher);
    state.settings.scroll_smoothness.to_bits().hash(&mut hasher);
    state.settings.glass_opacity.to_bits().hash(&mut hasher);
    dropzone_drag_over(app).hash(&mut hasher);
    app.drag_hover_maybe_active().hash(&mut hasher);
    state.profiles.active_idx.hash(&mut hasher);

    for profile in &state.profiles.profiles {
        profile.name.hash(&mut hasher);
        profile.theme.hash(&mut hasher);
        for bookmark in &profile.bookmarks {
            bookmark.name.hash(&mut hasher);
            bookmark.path.hash(&mut hasher);
        }
        for section in &profile.sidebar_sections {
            section.kind.label().hash(&mut hasher);
            section.collapsed.hash(&mut hasher);
            section.hidden.hash(&mut hasher);
        }
    }

    state
        .profile_ui
        .atlas_launcher_profiles_open
        .hash(&mut hasher);
    state.profile_ui.confirming_delete_profile.hash(&mut hasher);

    if let Some(tab) = state.active_tab_data() {
        tab.location.hash(&mut hasher);
        tab.preview_enabled.hash(&mut hasher);
    } else {
        0u8.hash(&mut hasher);
    }

    for item in &state.sidebar_drop_zone {
        item.path.hash(&mut hasher);
        item.from_self.hash(&mut hasher);
    }
    // HashSet iteration order is not stable; sort before hashing so a
    // selection update changes the fingerprint without causing false rebuilds.
    let mut selected: Vec<_> = app.dropzone_selected.iter().collect();
    selected.sort_unstable();
    selected.len().hash(&mut hasher);
    for path in selected {
        path.hash(&mut hasher);
    }

    // Recent folders are capped in the renderer, so hash only the same visible
    // suffix rather than making every historical entry invalidate the panel.
    let recent = app.recent_folders();
    recent.len().hash(&mut hasher);
    for path in recent.iter().rev().take(8) {
        path.hash(&mut hasher);
    }

    // Cloud is extension-driven rather than AppState-driven, but its presence
    // changes whether the Network section is emitted.
    app.ext_registry
        .has_sidebar_section("cloud")
        .hash(&mut hasher);

    hasher.finish()
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
    let hover = control_hover(fg);
    let action_hover = fg.opacity(0.12);
    let danger = hsla(0., 0.72, 0.52, 1.0);

    let menu_app = app.clone();
    let mut menu = v_flex()
        .w(crate::ui::rem_px(PROFILE_MENU_WIDTH))
        .flex_none()
        .p(crate::ui::rem_px(4.))
        .gap(crate::ui::rem_px(2.))
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

    let colors = MenuColors {
        fg,
        muted,
        accent,
        accent_fg,
        hover,
        action_hover,
        danger,
    };

    for (index, name) in profiles.into_iter().enumerate() {
        menu = menu.child(profile_menu_profile_row(
            app.clone(),
            popover.clone(),
            index,
            name,
            index == active_idx,
            confirming_delete == Some(index),
            colors,
        ));
    }

    menu.child(
        div()
            .h(px(1.))
            .w_full()
            .mx(crate::ui::rem_px(8.))
            .my(crate::ui::rem_px(4.))
            .bg(border),
    )
    .child(profile_menu_new_row(app, popover, fg, muted, hover))
    .with_animation(
        SharedString::from("sidebar-profile-menu-appear"),
        crate::ui::motion::animation(crate::ui::motion::MICRO_MS),
        |this, t| this.opacity(t).ml(px(5.0 * (1.0 - t))),
    )
    .into_any_element()
}

fn profile_menu_profile_row(
    app: WeakEntity<AtlasApp>,
    popover: Entity<PopoverState>,
    index: usize,
    name: String,
    active: bool,
    confirming: bool,
    colors: MenuColors,
) -> gpui::AnyElement {
    let MenuColors {
        fg,
        muted,
        accent,
        accent_fg,
        hover,
        ..
    } = colors;
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
                colors,
            )
        })
        .collect::<Vec<_>>();

    let row_app = app.clone();
    let row_popover = popover.clone();
    h_flex()
        .id(SharedString::from(format!("profile-menu-row-{index}")))
        .group(row_group)
        .w_full()
        .h(crate::ui::rem_px(30.))
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
    colors: MenuColors,
) -> gpui::AnyElement {
    let MenuColors {
        action_hover,
        danger,
        ..
    } = colors;
    let is_delete = id == "delete";
    let is_rename = id == "rename";
    let is_duplicate = id == "duplicate";
    div()
        .id(SharedString::from(format!("profile-menu-row-{index}-{id}")))
        .size(crate::ui::rem_px(22.))
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
        .h(crate::ui::rem_px(30.))
        .px_2()
        .gap_2()
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(fg)
        .hover(move |h| h.bg(hover))
        .child(
            div()
                .w(crate::ui::rem_px(16.))
                .h(crate::ui::rem_px(16.))
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
    let hover = control_hover(fg);

    let colors = MenuColors {
        fg,
        muted,
        accent,
        accent_fg,
        hover,
        action_hover: hover,
        danger: hover,
    };

    let menu = v_flex()
        .id("atlas-launcher-menu")
        .w(crate::ui::rem_px(ATLAS_LAUNCHER_MENU_WIDTH))
        .p(crate::ui::rem_px(4.))
        .bg(popover_bg)
        .border_1()
        .border_color(border)
        .rounded(px(10.))
        .shadow_lg()
        .text_sm()
        .text_color(fg)
        .occlude()
        .gap(crate::ui::rem_px(2.))
        .on_prepaint_child({
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
            None,
            colors,
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
            app_entity
                .read(cx)
                .command_shortcut_keystroke("view.sidebar-collapse"),
            colors,
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
            app_entity
                .read(cx)
                .command_shortcut_keystroke("sys.settings"),
            colors,
        ))
        .child(atlas_launcher_row(
            app.clone(),
            popover.clone(),
            "atlas-launcher-goto",
            AtlasIcon::Search,
            rust_i18n::t!("cmd_nav_goto_title").to_string(),
            None,
            false,
            false,
            Some(Action::OpenGotoBar),
            app_entity.read(cx).command_shortcut_keystroke("nav.goto"),
            colors,
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
            app_entity
                .read(cx)
                .command_shortcut_keystroke("sys.palette"),
            colors,
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
            app_entity
                .read(cx)
                .command_shortcut_keystroke("ext.manager"),
            colors,
        ))
        .child(atlas_launcher_row(
            app.clone(),
            popover.clone(),
            "atlas-launcher-toggle-preview",
            AtlasIcon::Eye,
            rust_i18n::t!("cmd_view_inspector_title").to_string(),
            None,
            false,
            false,
            Some(Action::ToggleInspector),
            app_entity
                .read(cx)
                .command_shortcut_keystroke("view.inspector"),
            colors,
        ))
        .child(atlas_launcher_row(
            app.clone(),
            popover.clone(),
            "atlas-launcher-about",
            AtlasIcon::House,
            rust_i18n::t!("settings_about").to_string(),
            None,
            false,
            false,
            Some(Action::OpenSettingsAbout),
            None,
            colors,
        ));

    let menu = menu.with_animation(
        SharedString::from("sidebar-atlas-launcher-menu-appear"),
        crate::ui::motion::animation(crate::ui::motion::MICRO_MS),
        |this, t| this.opacity(t).mt(px(5.0 * (1.0 - t))),
    );

    div()
        .relative()
        .w(crate::ui::rem_px(ATLAS_LAUNCHER_MENU_WIDTH))
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
    shortcut: Option<gpui::Keystroke>,
    colors: MenuColors,
) -> gpui::AnyElement {
    let MenuColors {
        fg,
        muted,
        accent,
        accent_fg,
        hover,
        ..
    } = colors;
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
        .min_h(crate::ui::rem_px(34.))
        .px_2()
        .py(crate::ui::rem_px(5.))
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
        .when_some(shortcut, |s, shortcut| s.child(Kbd::new(shortcut)))
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
                    this.flush_dirty_panes(cx);
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
    app: WeakEntity<AtlasApp>,
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
        let rename_app = self.app.clone();
        h_flex()
            .id("sidebar-atlas-trigger")
            .w_full()
            .h(crate::ui::rem_px(34.))
            .px(crate::ui::rem_px(6.))
            .gap(crate::ui::rem_px(6.))
            .items_center()
            .justify_center()
            .rounded(px(9.))
            .bg(fill)
            .hover(move |s| s.bg(self.hover))
            .text_color(self.fg)
            .on_mouse_down(
                MouseButton::Left,
                move |ev: &MouseDownEvent, _window, cx| {
                    if ev.click_count < 2 {
                        return;
                    }
                    cx.stop_propagation();
                    if let Some(app) = rename_app.upgrade() {
                        app.update(cx, |this, cx| {
                            this.process_actions(vec![Action::OpenRenameProfile]);
                            cx.notify();
                        });
                    }
                },
            )
            .when(self.collapsed, |s| {
                s.child(
                    div()
                        .size(crate::ui::rem_px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(atlas_logo(22.0)),
                )
            })
            .when(!self.collapsed, |s| {
                s.justify_start()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(gpui::FontWeight::BOLD)
                            .truncate()
                            .child(self.profile_name),
                    )
                    .child(icon(AtlasIcon::CaretDown, 11.0, self.muted.opacity(0.78)))
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
        // Replace geometry only when this retained view rebuilds. App renders
        // alone must preserve the dropzone's occlusion and selection rectangles.
        app.update(cx, |this, _| {
            this.dropzone_rects.clear();
            this.pending_drag_hover_targets.retain(|target| !matches!(target.id, DragHoverId::Sidebar(_)));
        });
        let (fg, muted, accent) = {
            let t = cx.theme();
            (t.foreground, t.muted_foreground, t.accent)
        };
        let active_theme = app.read(cx).state.profiles.active().theme.clone();
        let (smooth_ease, smooth_boost) =
            crate::app::scroll_smooth_params(app.read(cx).state.settings.scroll_smoothness);
        self.scroll.set_smooth(smooth_ease, smooth_boost);
        let windows_theme = is_windows_theme(&active_theme);
        let is_glass = active_theme.ends_with(" Glass");
        let hover = control_hover(fg);
        let active_bg = layer_color(accent, 0.0, 0.10);
        let style = SidebarStyle {
            hover,
            active_bg,
            accent,
            fg,
            muted,
            windows_theme,
        };
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
                app: app.downgrade(),
                profile_name: active_profile_name,
                selected: false,
                collapsed: sidebar_collapsed,
                fg,
                muted,
                bg: launcher_bg,
                hover,
                active: fg.opacity(0.12),
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
                .block_mouse_except_scroll()
                .size(crate::ui::rem_px(30.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(9.))
                .bg(hsla(0., 0., 1., 0.0))
                .hover(move |s| s.bg(hover).text_color(fg))
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
                .pb(crate::ui::rem_px(8.))
                .items_center()
                .justify_center()
                .child(
                    div()
                        .block_mouse_except_scroll()
                        .child(atlas_launcher_popover),
                )
                .into_any_element()
        } else {
            h_flex()
                .w_full()
                .pb(crate::ui::rem_px(8.))
                .gap(crate::ui::rem_px(5.))
                // `min_w(0)` overrides flexbox's default `min-width: auto`, which
                // otherwise refuses to shrink this item below the trigger's natural
                // content width — without it, a long profile name grows the whole
                // row past the sidebar and gets hard-clipped upstream instead of
                // eliding via the trigger's own `.truncate()`.
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .block_mouse_except_scroll()
                        .child(atlas_launcher_popover),
                )
                .child(collapse_button)
                .into_any_element()
        };
        let mut col = v_flex()
            .size_full()
            .p_2()
            .gap(crate::ui::rem_px(if windows_theme { 3.0 } else { 4.0 }))
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
            }))
            .on_action(
                cx.listener(|this, action: &OpenSpaceLensForPath, _window, cx| {
                    if let Some(app) = this.app.upgrade() {
                        let path = action.path.clone();
                        app.update(cx, |app, cx| {
                            let pane = app.state.focused_tile;
                            app.process_actions(vec![Action::OpenSpaceLens { pane, path }]);
                            cx.notify();
                        });
                    }
                }),
            )
            .on_action(
                cx.listener(|_this, action: &OpenPropertiesForPath, _window, _cx| {
                    let path = action.path.clone();
                    #[cfg(windows)]
                    crate::ui::shell_context_menu::show_properties(&path);
                    #[cfg(not(windows))]
                    if let Some(app) = _this.app.upgrade() {
                        app.update(_cx, |app, cx| {
                            app.open_properties(vec![path]);
                            cx.notify();
                        });
                    }
                }),
            )
            .on_action(
                cx.listener(|_this, _: &OpenThisPcProperties, _window, _cx| {
                    crate::ui::shell_context_menu::show_this_pc_properties();
                }),
            );
        #[cfg(feature = "provider-host")]
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
                                let _ =
                                    app.cloud_tx
                                        .send(atlas_core::cloud::CloudCmd::InstallWinFsp {
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
                style,
                current_path.as_deref(),
                current_location.as_ref(),
                cx,
            );
            return div()
                .id("sidebar-scroll")
                .size_full()
                .relative()
                .on_prepaint_child(sidebar_scroll_recorder(app.clone(), self.scroll.clone()))
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
            if section.kind == SectionKind::Linux
                && atlas_core::fs::paths::list_wsl_roots().is_empty()
            {
                continue;
            }
            if section.kind == SectionKind::Cloud
                && !app.read(cx).ext_registry.has_sidebar_section("cloud")
            {
                continue;
            }
            col = col.child(section_header(
                idx,
                section_count,
                &section,
                &app,
                muted,
                hover,
                cx,
            ));
            if !section.collapsed {
                col = render_section_body(
                    col,
                    &section.kind,
                    &app,
                    style,
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
        let scroll = div()
            .id("sidebar-scroll")
            .flex_1()
            .min_h(px(0.))
            .relative()
            .overflow_hidden()
            .on_prepaint_child(sidebar_scroll_recorder(app.clone(), self.scroll.clone()))
            .child(
                col.id("sidebar-scroll-content")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .pr(crate::ui::rem_px(SCROLLBAR_GUTTER)),
            )
            // The scrollbar paints above this gutter. Protect its track from
            // native caption dragging without blocking the scroll container.
            .child(
                div()
                    .absolute()
                    .right_0()
                    .top_0()
                    .w(crate::ui::rem_px(SCROLLBAR_GUTTER))
                    .h_full()
                    .block_mouse_except_scroll(),
            )
            .vertical_scrollbar(&self.scroll);

        if sidebar_collapsed {
            app.update(cx, |this, _| {
                this.dropzone_bounds = None;
                this.dropzone_rects.clear();
            });
            return scroll.size_full().into_any_element();
        }

        let staged = app.read(cx).state.sidebar_drop_zone.clone();
        let staged_set = staged
            .iter()
            .map(|item| item.path.clone())
            .collect::<HashSet<_>>();
        let selected = app
            .read(cx)
            .dropzone_selected
            .iter()
            .filter(|path| staged_set.contains(*path))
            .cloned()
            .collect::<HashSet<_>>();
        v_flex()
            .size_full()
            .child(scroll)
            .child(sidebar_drop_zone(
                &app,
                staged,
                selected,
                current_path.clone(),
                accent,
                fg,
                muted,
                hover,
                windows_theme,
                is_glass,
                cx,
            ))
            .into_any_element()
    }
}

fn dropzone_drag_over(app: &AtlasApp) -> bool {
    app.drag_hover_maybe_active()
        && app
            .dropzone_bounds
            .zip(app.drag_hover.cursor)
            .is_some_and(|(bounds, cursor)| bounds.contains(&cursor))
}

fn sidebar_drop_zone(
    app: &Entity<AtlasApp>,
    staged: Vec<atlas_model::app_state::StagedDrop>,
    selected: HashSet<PathBuf>,
    current_path: Option<PathBuf>,
    accent: Hsla,
    fg: Hsla,
    muted: Hsla,
    hover: Hsla,
    windows_theme: bool,
    is_glass: bool,
    cx: &mut Context<SidebarPanel>,
) -> impl IntoElement {
    let count = staged.len();
    let app_drop = app.clone();
    let app_drop_ext = app.clone();
    let app_prime = app.clone();
    let app_place = app.clone();
    let app_clear_all = app.clone();
    let dest = current_path.clone();
    let place_tip = rust_i18n::t!("sidebar_drop_zone_place").to_string();
    let clear_tip = rust_i18n::t!("sidebar_drop_zone_clear").to_string();
    let staged_paths = staged
        .iter()
        .map(|item| item.path.clone())
        .collect::<Vec<_>>();

    let mut grid = h_flex()
        .w(auto())
        .gap(crate::ui::rem_px(if windows_theme { 14.0 } else { 12.0 }))
        .items_center();
    for (index, item) in staged.iter().enumerate() {
        let (kind, extension) = staged_file_meta(&item.path);
        let (thumb_tex, icon_tex) = {
            let app_ref = app.read(cx);
            (
                app_ref.grid_thumbnail(&item.path),
                app_ref.icon_tex_for_entry(&kind, extension.as_deref()),
            )
        };
        grid = grid.child(drop_zone_item(
            app,
            item.path.clone(),
            index,
            kind,
            extension,
            thumb_tex,
            icon_tex,
            fg,
            muted,
            hover,
            accent,
            selected.contains(&item.path),
        ));
    }
    let controls = control_colors(cx.theme(), is_glass);
    let drop_bg = cx.theme().foreground.opacity(0.025);
    let drag_over = dropzone_drag_over(app.read(cx));
    let action_color = muted.opacity(0.82);
    let can_place = count > 0 && current_path.is_some();
    let app_start_rubber = app.clone();
    let app_record_bounds = app.clone();

    div()
        .id("sidebar-drop-zone")
        .relative()
        .flex_shrink_0()
        .w(auto())
        .h(crate::ui::rem_px(if count == 0 { 88.0 } else { 144.0 }))
        .mx(crate::ui::rem_px(8.))
        .my(crate::ui::rem_px(6.))
        .p(crate::ui::rem_px(10.))
        .overflow_hidden()
        .rounded(px(12.))
        .bg(if drag_over {
            accent.opacity(0.12)
        } else {
            drop_bg
        })
        .on_drag_over::<DraggedFiles>(move |s, _, _, _| s.bg(accent.opacity(0.12)))
        .on_drag_over::<gpui::ExternalPaths>(move |s, _, _, _| s.bg(accent.opacity(0.12)))
        .occlude()
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            let additive = ev.modifiers.control || ev.modifiers.platform || ev.modifiers.shift;
            let pos = window.mouse_position();
            app_start_rubber.update(cx, |this, cx| {
                this.begin_dropzone_rubber(pos, additive);
                cx.notify();
            });
        })
        .on_prepaint_child(move |bounds, _window, cx| {
            app_record_bounds.update(cx, |this, _cx| {
                this.dropzone_bounds = Some(bounds);
            });
        })
        .on_prepaint_child(move |_bounds, _window, cx| {
            app_prime.update(cx, |this, _cx| {
                prime_drop_zone_thumbnails(this, &staged_paths, windows_theme)
            });
        })
        .child(
            v_flex()
                .relative()
                .size_full()
                .gap(crate::ui::rem_px(7.))
                .child(
                    h_flex()
                        .w_full()
                        .gap_1()
                        .items_center()
                        .text_color(muted)
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(div().flex_1().truncate().child("Dropzone"))
                        .child(
                            div()
                                .id("sidebar-drop-zone-place")
                                .size(crate::ui::rem_px(24.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(6.))
                                .text_color(action_color)
                                .when(count == 0, |s| s.invisible())
                                .when(can_place, |s| {
                                    s.cursor_pointer()
                                        .hover(move |s| s.bg(controls.hover).text_color(fg))
                                })
                                .when(count > 0 && !can_place, |s| s.opacity(0.4))
                                .tooltip(move |window, cx| {
                                    Tooltip::new(place_tip.clone()).build(window, cx)
                                })
                                .child(icon(AtlasIcon::ArrowUp, 14.0, action_color))
                                .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                                    cx.stop_propagation();
                                    if !can_place {
                                        return;
                                    }
                                    let Some(dest) = dest.clone() else {
                                        return;
                                    };
                                    app_place.update(cx, |this, cx| {
                                        place_sidebar_drop_zone(this, dest);
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(
                            div()
                                .id("sidebar-drop-zone-clear")
                                .size(crate::ui::rem_px(24.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(6.))
                                .text_color(action_color)
                                .when(count == 0, |s| s.invisible())
                                .hover(move |s| s.bg(controls.hover).text_color(fg))
                                .tooltip(move |window, cx| {
                                    Tooltip::new(clear_tip.clone()).build(window, cx)
                                })
                                .child(icon(AtlasIcon::X, 13.0, action_color))
                                .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                                    cx.stop_propagation();
                                    app_clear_all.update(cx, |this, cx| {
                                        this.state.sidebar_drop_zone.clear();
                                        this.dropzone_selected.clear();
                                        this.dropzone_rects.clear();
                                        this.dropzone_pending_tags.clear();
                                        this.cancel_dropzone_file_drag();
                                        cx.notify();
                                    });
                                }),
                        ),
                )
                .when(count == 0, |s| {
                    s.child(
                        h_flex()
                            .flex_1()
                            .relative()
                            .items_center()
                            .justify_center()
                            .gap(crate::ui::rem_px(6.))
                            .text_xs()
                            .text_color(muted)
                            .child(div().flex_none().child(icon(
                                AtlasIcon::TrayArrowDown,
                                13.0,
                                muted,
                            )))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .text_center()
                                    .child(rust_i18n::t!("sidebar_drop_zone_hint").to_string()),
                            ),
                    )
                })
                .when(count > 0, |s| {
                    s.child(
                        div()
                            .w_full()
                            .flex_1()
                            .relative()
                            .overflow_x_scrollbar()
                            .child(grid),
                    )
                }),
        )
        .on_drop::<DraggedFiles>(move |d, _window, cx| {
            app_drop.update(cx, |this, cx| {
                this.execute_drag_hover_drop_command(
                    DragHoverDropCommand::StageInSidebar,
                    d.paths.clone(),
                    true,
                );
                cx.notify();
            });
            cx.stop_propagation();
        })
        .on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
            app_drop_ext.update(cx, |this, cx| {
                this.execute_drag_hover_drop_command(
                    DragHoverDropCommand::StageInSidebar,
                    paths.paths().to_vec(),
                    false,
                );
                cx.notify();
            });
            cx.stop_propagation();
        })
}

fn staged_file_meta(path: &std::path::Path) -> (FileKind, Option<String>) {
    let kind = if path.is_dir() {
        FileKind::Dir
    } else {
        FileKind::File
    };
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    (kind, extension)
}

fn prime_drop_zone_thumbnails(app: &mut AtlasApp, paths: &[PathBuf], windows_theme: bool) {
    {
        let Ok(mut wanted) = app.thumb_wanted.lock() else {
            return;
        };
        wanted.extend(paths.iter().cloned());
    }
    for path in paths {
        let (kind, extension) = staged_file_meta(path);
        if app.thumb_textures.contains_key(path) {
            continue;
        }
        #[cfg(target_os = "windows")]
        if windows_theme && kind == FileKind::Dir {
            app.request_shell_thumbnail_or_icon(path);
            continue;
        }
        if kind != FileKind::File {
            continue;
        }
        if extension.as_deref().is_some_and(|ext| {
            [
                "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tif", "tiff",
            ]
            .contains(&ext)
        }) {
            app.request_thumbnail(path);
        } else {
            #[cfg(target_os = "windows")]
            app.request_shell_thumbnail(path);
        }
    }
}

fn drop_zone_item(
    app: &Entity<AtlasApp>,
    path: PathBuf,
    index: usize,
    kind: FileKind,
    extension: Option<String>,
    thumb_tex: Option<Arc<gpui::RenderImage>>,
    icon_tex: Option<Arc<gpui::RenderImage>>,
    fg: Hsla,
    muted: Hsla,
    hover: Hsla,
    accent: Hsla,
    selected: bool,
) -> impl IntoElement {
    let group = SharedString::from(format!("drop-zone-item-{index}"));
    let remove_app = app.clone();
    let app_record_item = app.clone();
    let select_app = app.clone();
    let remove_path = path.clone();
    let drag_path = path.clone();
    let rect_path = path.clone();
    let select_path = path.clone();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    div()
        .id(group.clone())
        .group(group.clone())
        .relative()
        .flex_none()
        .w(crate::ui::rem_px(84.))
        .min_h(crate::ui::rem_px(74.))
        .p(crate::ui::rem_px(6.))
        .flex()
        .flex_col()
        .items_center()
        .gap(crate::ui::rem_px(5.))
        .rounded(px(8.))
        .cursor_pointer()
        .when(selected, |s| s.bg(accent.opacity(0.18)))
        .when(!selected, |s| s.hover(move |s| s.bg(hover)))
        .on_prepaint_child(move |bounds, _window, cx| {
            app_record_item.update(cx, |this, _cx| {
                if let Some((_, old)) = this
                    .dropzone_rects
                    .iter_mut()
                    .find(|(path, _)| path == &rect_path)
                {
                    *old = bounds;
                } else {
                    this.dropzone_rects.push((rect_path.clone(), bounds));
                }
            });
        })
        .child(drop_zone_preview(
            kind, extension, thumb_tex, icon_tex, accent,
        ))
        .child(
            div()
                .w_full()
                .text_xs()
                .text_color(fg)
                .text_center()
                .truncate()
                .child(SharedString::from(name)),
        )
        .on_click(move |ev, _window, cx| {
            let ctrl = ev.modifiers().control || ev.modifiers().platform;
            select_app.update(cx, |this, cx| {
                let staged = this
                    .state
                    .sidebar_drop_zone
                    .iter()
                    .map(|item| item.path.clone())
                    .collect::<HashSet<_>>();
                this.dropzone_selected.retain(|path| staged.contains(path));
                if ctrl {
                    if !this.dropzone_selected.remove(&select_path) {
                        this.dropzone_selected.insert(select_path.clone());
                    }
                } else {
                    this.dropzone_selected.clear();
                    this.dropzone_selected.insert(select_path.clone());
                }
                cx.notify();
            });
        })
        .child(
            div()
                .absolute()
                .top(crate::ui::rem_px(3.))
                .right(crate::ui::rem_px(3.))
                .size(crate::ui::rem_px(18.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .bg(hsla(0., 0., 0., 0.18))
                .text_color(muted)
                .invisible()
                .group_hover(group, gpui::Styled::visible)
                .child(icon(AtlasIcon::X, 10.0, fg))
                .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                    cx.stop_propagation();
                    remove_app.update(cx, |this, cx| {
                        if let Some(pos) = this
                            .state
                            .sidebar_drop_zone
                            .iter()
                            .position(|item| item.path == remove_path)
                        {
                            this.state.sidebar_drop_zone.remove(pos);
                            this.dropzone_selected.remove(&remove_path);
                            this.dropzone_pending_tags.remove(&remove_path);
                            this.dropzone_rects.retain(|(path, _)| path != &remove_path);
                        }
                        cx.notify();
                    });
                }),
        )
        .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
            cx.stop_propagation();
        })
        .on_drag(
            file_drag_value(vec![drag_path.clone()]),
            staged_drop_drag_handler(app.clone(), drag_path),
        )
}

fn drop_zone_preview(
    kind: FileKind,
    extension: Option<String>,
    thumb_tex: Option<Arc<gpui::RenderImage>>,
    icon_tex: Option<Arc<gpui::RenderImage>>,
    accent: Hsla,
) -> AnyElement {
    let area = div()
        .w(crate::ui::rem_px(72.))
        .h(crate::ui::rem_px(44.))
        .flex()
        .items_center()
        .justify_center();
    match thumb_tex {
        Some(tex) => area
            .child(
                img(tex)
                    .max_w(crate::ui::rem_px(72.))
                    .max_h(crate::ui::rem_px(44.))
                    .rounded(px(3.)),
            )
            .into_any_element(),
        None => match icon_tex {
            Some(tex) => area
                .child(
                    img(tex)
                        .max_w(crate::ui::rem_px(38.))
                        .max_h(crate::ui::rem_px(38.))
                        .flex_none(),
                )
                .into_any_element(),
            None => area
                .child(file_type_icon(&kind, extension.as_deref(), 38.0))
                .text_color(accent)
                .into_any_element(),
        },
    }
}

fn staged_drop_drag_handler(
    app: Entity<AtlasApp>,
    path: PathBuf,
) -> impl Fn(&DragView, gpui::Point<gpui::Pixels>, &mut Window, &mut gpui::App) -> Entity<DragView>
       + 'static {
    move |_d, _pos, _window, cx| {
        let paths = sidebar_drop_drag_paths(&app.read(&*cx), &path);
        app.update(cx, |app, cx| {
            app.begin_dropzone_file_drag(&paths);
            app.begin_drag_hover();
            cx.notify();
        });
        #[cfg(windows)]
        {
            app.read(&*cx).trigger_os_drag_copy_only(paths.clone());
            cx.new(|_| crate::ui::root::DragNoop)
        }
        #[cfg(not(windows))]
        {
            cx.new(|_| DraggedFiles { paths })
        }
    }
}

fn sidebar_drop_drag_paths(app: &AtlasApp, path: &std::path::Path) -> Vec<PathBuf> {
    if app.dropzone_selected.contains(path) {
        let selected = &app.dropzone_selected;
        let paths = app
            .state
            .sidebar_drop_zone
            .iter()
            .filter(|item| selected.contains(&item.path))
            .map(|item| item.path.clone())
            .collect::<Vec<_>>();
        if !paths.is_empty() {
            return paths;
        }
    }
    vec![path.to_path_buf()]
}

fn place_sidebar_drop_zone(app: &mut AtlasApp, dest: PathBuf) {
    let paths = app.state.sidebar_drop_zone.iter().map(|item| item.path.clone()).collect::<Vec<_>>();
    if !paths.is_empty() {
        // One request means one conflict dialog, including mixed-origin shelves.
        // Completion removes only the sources that actually transferred.
        app.execute_drag_hover_drop_command(DragHoverDropCommand::TransferTo(dest), paths, true);
    }
}

/// A 16px Phosphor icon (`icons/ph/<name>.svg`), tinted with `color`.
fn ph(name: &str, color: Hsla) -> impl IntoElement {
    icon(ph_icon(name), 16.0, color)
}

fn sidebar_shell_tex(
    app: &Entity<AtlasApp>,
    path: &std::path::Path,
    windows_theme: bool,
    cx: &mut Context<SidebarPanel>,
) -> Option<Arc<gpui::RenderImage>> {
    windows_theme
        .then(|| app.read(cx).grid_thumbnail(path))
        .flatten()
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
        "terminal-window" => AtlasIcon::TerminalWindow,
        "caret-down" => AtlasIcon::CaretDown,
        "caret-right" => AtlasIcon::CaretRight,
        "trash" => AtlasIcon::Trash,
        _ => AtlasIcon::File,
    }
}

fn render_collapsed_sections(
    mut col: gpui::Div,
    app: &Entity<AtlasApp>,
    style: SidebarStyle,
    current: Option<&std::path::Path>,
    current_location: Option<&Location>,
    cx: &mut Context<SidebarPanel>,
) -> gpui::Div {
    let SidebarStyle {
        hover,
        windows_theme,
        ..
    } = style;
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
        if section.kind == SectionKind::Cloud
            && !app.read(cx).ext_registry.has_sidebar_section("cloud")
        {
            continue;
        }
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
                    let shell_tex = sidebar_shell_tex(app, &path, windows_theme, cx);
                    col = col.child(collapsed_nav_row(
                        SharedString::from(format!("rail-place-{label}")),
                        label,
                        icon_name,
                        path,
                        shell_tex,
                        app,
                        style,
                        active,
                    ));
                }
                let rb_active = matches!(current_location, Some(Location::RecycleBin));
                #[cfg(target_os = "windows")]
                let rb_shell_tex = windows_theme
                    .then(|| app.read(cx).shell_recycle_bin_icon())
                    .flatten();
                #[cfg(not(target_os = "windows"))]
                let rb_shell_tex = None;
                col = col.child(collapsed_virtual_nav_row(
                    SharedString::from("rail-place-recycle-bin"),
                    rust_i18n::t!("sidebar_recycle_bin").to_string(),
                    "trash",
                    Location::RecycleBin,
                    rb_shell_tex,
                    app,
                    style,
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
                    let shell_tex = sidebar_shell_tex(app, &path, windows_theme, cx);
                    col = col.child(collapsed_nav_row(
                        SharedString::from(format!("rail-bm-{}", path.display())),
                        name,
                        "folder",
                        path,
                        shell_tex,
                        app,
                        style,
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
            SectionKind::Linux => {
                let roots = atlas_core::fs::paths::list_wsl_roots();
                if !roots.is_empty() && before {
                    col = col.child(collapsed_divider());
                }
                for path in roots {
                    let label = linux_root_label(&path);
                    let active = current.is_some_and(|c| c == path.as_path());
                    let shell_tex = sidebar_shell_tex(app, &path, windows_theme, cx);
                    col = col.child(collapsed_nav_row(
                        SharedString::from(format!("rail-linux-{}", path.display())),
                        label,
                        "terminal-window",
                        path,
                        shell_tex,
                        app,
                        style,
                        active,
                    ));
                    section_has_items = true;
                }
            }
            SectionKind::Drives => {
                let drives = atlas_core::fs::mounts::list_drives();
                if !drives.is_empty() && before {
                    col = col.child(collapsed_divider());
                }
                for drive in drives {
                    let active = current.is_some_and(|c| c == drive.mount_point.as_path());
                    let shell_tex = sidebar_shell_tex(app, &drive.mount_point, windows_theme, cx);
                    col = col.child(collapsed_drive_row(drive, shell_tex, app, style, active));
                    section_has_items = true;
                }
            }
            SectionKind::Recent => {
                let recent: Vec<PathBuf> = app
                    .read(cx)
                    .recent_folders()
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
                    let shell_tex = sidebar_shell_tex(app, &path, windows_theme, cx);
                    col = col.child(collapsed_nav_row(
                        SharedString::from(format!("rail-recent-{}", path.display())),
                        label,
                        "folder",
                        path,
                        shell_tex,
                        app,
                        style,
                        active,
                    ));
                    section_has_items = true;
                }
            }
            SectionKind::Cloud => {
                #[cfg(feature = "provider-host")]
                {
                    let native = native_cloud_folders();
                    let remotes: Vec<atlas_model::cloud_model::CloudRemote> =
                        app.read(cx).state.cloud.remotes.clone();
                    if (!native.is_empty() || !remotes.is_empty()) && before {
                        col = col.child(collapsed_divider());
                    }
                    for (label, path) in native {
                        let active = current.is_some_and(|c| c == path.as_path());
                        let shell_tex = sidebar_shell_tex(app, &path, windows_theme, cx);
                        col = col.child(collapsed_nav_row(
                            SharedString::from(format!("rail-native-cloud-{}", path.display())),
                            label,
                            "cloud",
                            path,
                            shell_tex,
                            app,
                            style,
                            active,
                        ));
                        section_has_items = true;
                    }
                    for remote in remotes {
                        col = col.child(collapsed_cloud_remote_row(remote, app, style));
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
        .my(crate::ui::rem_px(6.))
        .bg(hsla(0., 0., 1., 0.08))
}

fn is_sidebar_drop_allowed(
    app: &Entity<AtlasApp>,
    pos: gpui::Point<gpui::Pixels>,
    cx: &gpui::App,
) -> bool {
    let app_ref = app.read(cx);
    if app_ref.dropzone_bounds.is_some_and(|b| b.contains(&pos)) {
        return false;
    }
    if let Some((scroll_bounds, _)) = app_ref.sidebar_scroll_region {
        if !scroll_bounds.contains(&pos) {
            return false;
        }
    }
    true
}

fn collapsed_nav_row(
    id: SharedString,
    label: String,
    icon_name: &'static str,
    path: PathBuf,
    shell_tex: Option<Arc<gpui::RenderImage>>,
    app: &Entity<AtlasApp>,
    style: SidebarStyle,
    active: bool,
) -> impl IntoElement {
    let SidebarStyle {
        hover,
        active_bg,
        accent,
        fg,
        windows_theme,
        ..
    } = style;
    let app = app.clone();
    let tooltip = label.clone();
    let hover_id = DragHoverId::Sidebar(id.to_string());
    let drop_path = path.clone();
    let drop_path_external = path.clone();
    let drop_app = app.clone();
    let drop_app_external = app.clone();
    let shell_path = path.clone();
    let shell_app = app.clone();
    let ctx_app = app.clone();
    let ctx_path = path.clone();
    let icon_color = if active {
        accent
    } else {
        hsla(
            (fg.hue.into_positive_degrees() / 360.0),
            fg.saturation,
            fg.lightness,
            0.84,
        )
    };
    h_flex()
        .id(id)
        .w_full()
        .h(crate::ui::rem_px(32.))
        .block_mouse_except_scroll()
        .items_center()
        .justify_center()
        .rounded(px(9.))
        .cursor_pointer()
        .when(active, |s| s.bg(active_bg))
        .when(!active, |s| s.hover(|h| h.bg(hover)))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(match shell_tex {
            Some(tex) => img(tex)
                .size(crate::ui::rem_px(18.0))
                .flex_none()
                .into_any_element(),
            None => theme_icon(ph_icon(icon_name), 17.0, icon_color, windows_theme),
        })
        .on_prepaint_child(move |_bounds, _window, cx| {
            #[cfg(target_os = "windows")]
            if windows_theme {
                shell_app.update(cx, |this, _cx| {
                    this.request_shell_thumbnail_or_icon(&shell_path);
                });
            }
        })
        .on_prepaint_child(drag_hover_recorder_with_drop(
            hover_id,
            DragHoverKind::Folder,
            DragHoverCommand::NavigatePath(path.clone()),
            DragHoverDropCommand::TransferTo(path.clone()),
            &app,
        ))
        .on_drop::<DraggedFiles>(move |d, window, cx| {
            if !is_sidebar_drop_allowed(&drop_app, window.mouse_position(), cx) {
                return;
            }
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
        .on_drop::<gpui::ExternalPaths>(move |paths, window, cx| {
            if !is_sidebar_drop_allowed(&drop_app_external, window.mouse_position(), cx) {
                return;
            }
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
        .on_mouse_down(MouseButton::Right, move |ev, window, cx| {
            ctx_app.update(cx, |this, cx| {
                this.open_ctx_menu(vec![ctx_path.clone()], ev.position, window, cx);
                cx.notify();
            });
            cx.stop_propagation();
        })
}

fn collapsed_virtual_nav_row(
    id: SharedString,
    label: String,
    icon_name: &'static str,
    loc: Location,
    shell_tex: Option<Arc<gpui::RenderImage>>,
    app: &Entity<AtlasApp>,
    style: SidebarStyle,
    active: bool,
) -> impl IntoElement {
    let SidebarStyle {
        hover,
        active_bg,
        accent,
        fg,
        windows_theme,
        ..
    } = style;
    let app = app.clone();
    let tooltip = label.clone();
    let hover_id = DragHoverId::Sidebar(id.to_string());
    let request_recycle_icon = windows_theme && loc == Location::RecycleBin && shell_tex.is_none();
    let recycle_icon_app = app.clone();
    let icon_color = if active {
        accent
    } else {
        hsla(
            (fg.hue.into_positive_degrees() / 360.0),
            fg.saturation,
            fg.lightness,
            0.84,
        )
    };
    h_flex()
        .id(id)
        .w_full()
        .h(crate::ui::rem_px(32.))
        .block_mouse_except_scroll()
        .items_center()
        .justify_center()
        .rounded(px(9.))
        .cursor_pointer()
        .when(active, |s| s.bg(active_bg))
        .when(!active, |s| s.hover(|h| h.bg(hover)))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(match shell_tex {
            Some(tex) => img(tex).size(px(18.0)).flex_none().into_any_element(),
            None => theme_icon(ph_icon(icon_name), 17.0, icon_color, windows_theme),
        })
        .when(request_recycle_icon, |row| {
            row.on_prepaint_child(move |_bounds, _window, cx| {
                recycle_icon_app.update(cx, |this, _cx| {
                    this.request_shell_recycle_bin_icon();
                });
            })
        })
        .map(|row| {
            // Recycle Bin row doubles as a drop target: files dropped on it go
            // to the OS trash instead of just spring-navigating.
            if loc == Location::RecycleBin {
                row.on_prepaint_child(drag_hover_recorder_with_drop(
                    hover_id,
                    DragHoverKind::Nav,
                    DragHoverCommand::NavigateLocation(loc.clone()),
                    DragHoverDropCommand::TrashPaths,
                    &app,
                ))
            } else {
                row.on_prepaint_child(drag_hover_recorder(
                    hover_id,
                    DragHoverKind::Nav,
                    DragHoverCommand::NavigateLocation(loc.clone()),
                    &app,
                ))
            }
        })
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
        .block_mouse_except_scroll()
        .w_full()
        .h(crate::ui::rem_px(32.))
        .items_center()
        .justify_center()
        .rounded(px(9.))
        .cursor_pointer()
        .hover(|s| s.bg(hover))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(
            div()
                .size(crate::ui::rem_px(12.))
                .rounded_full()
                .bg(rgb)
                .shadow_sm(),
        )
        .on_prepaint_child(drag_hover_recorder_with_drop(
            hover_id,
            DragHoverKind::Nav,
            DragHoverCommand::OpenTag(id),
            DragHoverDropCommand::AssignTag(id),
            &app,
        ))
        .on_drop::<DraggedFiles>({
            let app = app.clone();
            move |d, window, cx| {
                if !is_sidebar_drop_allowed(&app, window.mouse_position(), cx) {
                    return;
                }
                let paths = d.paths.clone();
                app.update(cx, |this, cx| {
                    this.process_actions(vec![Action::AssignTagToFiles { tag: id, paths }]);
                    cx.notify();
                });
            }
        })
        .on_drop::<gpui::ExternalPaths>({
            let app = app.clone();
            move |paths, window, cx| {
                if !is_sidebar_drop_allowed(&app, window.mouse_position(), cx) {
                    return;
                }
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

fn collapsed_drive_row(
    drive: atlas_core::fs::mounts::DriveInfo,
    shell_tex: Option<Arc<gpui::RenderImage>>,
    app: &Entity<AtlasApp>,
    style: SidebarStyle,
    active: bool,
) -> impl IntoElement {
    let tooltip = if drive.total_bytes == 0 {
        drive.label.clone()
    } else {
        let free_gb = drive.available_bytes as f64 / 1_073_741_824.0;
        format!(
            "{} · {}",
            drive.label,
            rust_i18n::t!("sidebar_free_space", gb = format!("{free_gb:.0}"))
        )
    };
    collapsed_nav_row(
        SharedString::from(format!("rail-drive-{}", drive.mount_point.display())),
        tooltip,
        "hard-drives",
        drive.mount_point,
        shell_tex,
        app,
        style,
        active,
    )
}

#[cfg(feature = "provider-host")]
fn collapsed_cloud_remote_row(
    remote: atlas_model::cloud_model::CloudRemote,
    app: &Entity<AtlasApp>,
    style: SidebarStyle,
) -> impl IntoElement {
    let SidebarStyle {
        hover,
        active_bg,
        accent,
        fg,
        windows_theme,
        ..
    } = style;
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
        .h(crate::ui::rem_px(32.))
        .block_mouse_except_scroll()
        .items_center()
        .justify_center()
        .rounded(px(9.))
        .cursor_pointer()
        .when(is_mounted, |s| s.bg(active_bg))
        .when(!is_mounted, |s| s.hover(|h| h.bg(hover)))
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        .child(theme_icon(
            AtlasIcon::Cloud,
            17.0,
            if is_mounted {
                accent
            } else {
                hsla(
                    (fg.hue.into_positive_degrees() / 360.0),
                    fg.saturation,
                    fg.lightness,
                    0.84,
                )
            },
            windows_theme,
        ))
        .on_prepaint_child(drag_hover_recorder(
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

fn render_section_body(
    mut col: gpui::Div,
    kind: &SectionKind,
    app: &Entity<AtlasApp>,
    style: SidebarStyle,
    current: Option<&std::path::Path>,
    current_location: Option<&Location>,
    cx: &mut Context<SidebarPanel>,
) -> gpui::Div {
    let SidebarStyle {
        hover,
        fg,
        muted,
        windows_theme,
        ..
    } = style;
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
                let shell_tex = sidebar_shell_tex(app, &path, windows_theme, cx);
                col = col.child(nav_row(
                    SharedString::from(format!("place-{label}")),
                    label.clone(),
                    icon,
                    path,
                    shell_tex,
                    app,
                    style,
                    active,
                ));
            }
            let rb_active = matches!(current_location, Some(Location::RecycleBin));
            #[cfg(target_os = "windows")]
            let rb_shell_tex = windows_theme
                .then(|| app.read(cx).shell_recycle_bin_icon())
                .flatten();
            #[cfg(not(target_os = "windows"))]
            let rb_shell_tex = None;
            col = col.child(virtual_nav_row(
                SharedString::from("place-recycle-bin"),
                rust_i18n::t!("sidebar_recycle_bin").to_string(),
                "trash",
                Location::RecycleBin,
                rb_shell_tex,
                app,
                style,
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
                    let shell_tex = sidebar_shell_tex(app, &path, windows_theme, cx);
                    col = col.child(nav_row(
                        id, name, "folder", path, shell_tex, app, style, active,
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
                    col = col.child(tag_row(name, color, id, global, app, style));
                }
            }
        }
        SectionKind::Linux => {
            for path in atlas_core::fs::paths::list_wsl_roots() {
                let active = current.is_some_and(|c| c == path.as_path());
                let shell_tex = sidebar_shell_tex(app, &path, windows_theme, cx);
                col = col.child(nav_row(
                    SharedString::from(format!("linux-{}", path.display())),
                    linux_root_label(&path),
                    "terminal-window",
                    path,
                    shell_tex,
                    app,
                    style,
                    active,
                ));
            }
        }
        SectionKind::Drives => {
            let drives = atlas_core::fs::mounts::list_drives();
            if drives.is_empty() {
                col = col.child(empty_row(
                    rust_i18n::t!("sidebar_no_drives").to_string(),
                    muted,
                ));
            } else {
                for drive in drives {
                    let active = current.is_some_and(|c| c == drive.mount_point.as_path());
                    let shell_tex = sidebar_shell_tex(app, &drive.mount_point, windows_theme, cx);
                    col = col.child(drive_row(drive, shell_tex, app, style, active));
                }
            }
        }
        SectionKind::Recent => {
            let recent: Vec<PathBuf> = app
                .read(cx)
                .recent_folders()
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
                    let shell_tex = sidebar_shell_tex(app, &path, windows_theme, cx);
                    col = col.child(nav_row(
                        SharedString::from(format!("recent-{}", path.display())),
                        label,
                        "folder",
                        path,
                        shell_tex,
                        app,
                        style,
                        active,
                    ));
                }
            }
        }
        SectionKind::Cloud => {
            #[cfg(feature = "provider-host")]
            {
                let current = app
                    .read(cx)
                    .state
                    .active_tab_data()
                    .and_then(|tab| tab.location.fs_dir().map(std::path::Path::to_path_buf));
                for (label, path) in native_cloud_folders() {
                    let active = current.as_deref().is_some_and(|c| c == path.as_path());
                    let shell_tex = sidebar_shell_tex(app, &path, windows_theme, cx);
                    col = col.child(nav_row(
                        SharedString::from(format!("native-cloud-{}", path.display())),
                        label,
                        "cloud",
                        path,
                        shell_tex,
                        app,
                        style,
                        active,
                    ));
                }
                let remotes: Vec<atlas_model::cloud_model::CloudRemote> =
                    app.read(cx).state.cloud.remotes.clone();
                for remote in remotes {
                    col = col.child(cloud_remote_row(remote, app, style));
                }
                col = col.child(add_cloud_row(app, hover, muted, fg));
            }
            #[cfg(not(feature = "provider-host"))]
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
    style: SidebarStyle,
) -> impl IntoElement {
    let SidebarStyle {
        hover,
        fg,
        windows_theme,
        ..
    } = style;
    let app = app.clone();
    let edit_app = app.clone();
    let edit_name = name.clone();
    let group = SharedString::from(format!("tag-row-{}", id.0));
    let rgb =
        gpui::rgb((u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2]));
    let hover_id = DragHoverId::Sidebar(format!("tag-{}", id.0));
    h_flex()
        .id(SharedString::from(format!("tag-{}", id.0)))
        .block_mouse_except_scroll()
        .group(group.clone())
        .w_full()
        .px_2()
        .h(crate::ui::rem_px(if windows_theme { 30.0 } else { 24.0 }))
        .gap(crate::ui::rem_px(if windows_theme { 10.0 } else { 8.0 }))
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(fg)
        .hover(|s| s.bg(hover))
        .child(div().size(crate::ui::rem_px(11.)).rounded_full().bg(rgb))
        .child(div().min_w(px(0.)).flex_1().truncate().child(name))
        .child(
            div()
                .id(SharedString::from(format!("tag-{}-rename", id.0)))
                .size(crate::ui::rem_px(20.))
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
        .when(global, |s| {
            s.child(
                div()
                    .id(SharedString::from(format!("tag-{}-global", id.0)))
                    .flex_none()
                    .child(icon(AtlasIcon::GlobeSimple, 12.0, fg.opacity(0.6)))
                    .tooltip(|window, cx| {
                        Tooltip::new(rust_i18n::t!("tag_global_hint").to_string()).build(window, cx)
                    }),
            )
        })
        .on_prepaint_child(drag_hover_recorder_with_drop(
            hover_id,
            DragHoverKind::Nav,
            DragHoverCommand::OpenTag(id),
            DragHoverDropCommand::AssignTag(id),
            &app,
        ))
        .on_drop::<DraggedFiles>({
            let app = app.clone();
            move |d, window, cx| {
                if !is_sidebar_drop_allowed(&app, window.mouse_position(), cx) {
                    return;
                }
                let paths = d.paths.clone();
                app.update(cx, |this, cx| {
                    this.process_actions(vec![Action::AssignTagToFiles { tag: id, paths }]);
                    cx.notify();
                });
            }
        })
        .on_drop::<gpui::ExternalPaths>({
            let app = app.clone();
            move |paths, window, cx| {
                if !is_sidebar_drop_allowed(&app, window.mouse_position(), cx) {
                    return;
                }
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
fn drive_row(
    drive: atlas_core::fs::mounts::DriveInfo,
    shell_tex: Option<Arc<gpui::RenderImage>>,
    app: &Entity<AtlasApp>,
    style: SidebarStyle,
    active: bool,
) -> gpui::AnyElement {
    let SidebarStyle {
        hover,
        active_bg,
        accent,
        fg,
        muted,
        windows_theme,
        ..
    } = style;
    let app = app.clone();
    let frac = drive.used_fraction();
    // Alert color when nearly full, otherwise the accent.
    let bar_color = if frac > 0.9 {
        hsla(0., 0.70, 0.55, 1.0)
    } else {
        accent
    };
    let free_label = if drive.total_bytes == 0 {
        String::new()
    } else {
        let free_gb = drive.available_bytes as f64 / 1_073_741_824.0;
        rust_i18n::t!("sidebar_free_space", gb = format!("{free_gb:.0}")).to_string()
    };
    let path = drive.mount_point.clone();
    let drop_path = path.clone();
    let drop_path_external = path.clone();
    let id = SharedString::from(format!("drive-{}", drive.mount_point.display()));
    let hover_id = DragHoverId::Sidebar(format!("drive-{}", drive.mount_point.display()));
    let icon_color = hsla(
        (fg.hue.into_positive_degrees() / 360.0),
        fg.saturation,
        fg.lightness,
        0.85,
    );
    let drop_app = app.clone();
    let drop_app_external = app.clone();
    let shell_path = path.clone();
    let shell_app = app.clone();
    let row = v_flex()
        .id(id)
        .block_mouse_except_scroll()
        .w_full()
        .px_2()
        .py(crate::ui::rem_px(if windows_theme { 6.0 } else { 4.0 }))
        .gap(crate::ui::rem_px(3.))
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(fg)
        .when(active, |s| s.bg(active_bg))
        .when(!active, |s| s.hover(|h| h.bg(hover)))
        .child(
            h_flex()
                .w_full()
                .gap(crate::ui::rem_px(if windows_theme { 10.0 } else { 8.0 }))
                .items_center()
                .child(match shell_tex {
                    Some(tex) => img(tex)
                        .size(crate::ui::rem_px(if windows_theme { 18.0 } else { 16.0 }))
                        .flex_none()
                        .into_any_element(),
                    None => theme_icon(
                        AtlasIcon::HardDrives,
                        if windows_theme { 18.0 } else { 16.0 },
                        icon_color,
                        windows_theme,
                    ),
                })
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .child(SharedString::from(drive.label.clone())),
                )
                .when(!free_label.is_empty(), |row| {
                    row.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(SharedString::from(free_label)),
                    )
                }),
        )
        .when(drive.total_bytes > 0, |row| {
            // Fullness track + filled portion (width = used fraction).
            row.child(
                div()
                    .w_full()
                    .h(crate::ui::rem_px(4.))
                    .rounded(px(2.))
                    .bg(crate::ui::glass::track_fill())
                    .sunken_surface()
                    .child(
                        div()
                            .h_full()
                            .w(relative(frac))
                            .rounded(px(2.))
                            .bg(bar_color),
                    ),
            )
        })
        .on_prepaint_child(move |_bounds, _window, cx| {
            #[cfg(target_os = "windows")]
            if windows_theme {
                shell_app.update(cx, |this, _cx| {
                    this.request_shell_thumbnail_or_icon(&shell_path);
                });
            }
        })
        .on_prepaint_child(drag_hover_recorder_with_drop(
            hover_id,
            DragHoverKind::Folder,
            DragHoverCommand::NavigatePath(path.clone()),
            DragHoverDropCommand::TransferTo(path.clone()),
            &app,
        ))
        .on_drop::<DraggedFiles>(move |d, window, cx| {
            if !is_sidebar_drop_allowed(&drop_app, window.mouse_position(), cx) {
                return;
            }
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
        .on_drop::<gpui::ExternalPaths>(move |paths, window, cx| {
            if !is_sidebar_drop_allowed(&drop_app_external, window.mouse_position(), cx) {
                return;
            }
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
        });
    #[cfg(target_os = "windows")]
    {
        row.context_menu({
            let lens_path = drive.mount_point.clone();
            let properties_path = drive.mount_point.clone();
            move |menu, _window, _cx| {
                menu.menu(
                    rust_i18n::t!("sidebar_analyze_space"),
                    Box::new(OpenSpaceLensForPath {
                        path: lens_path.clone(),
                    }),
                )
                .menu(
                    rust_i18n::t!("root_properties"),
                    Box::new(OpenPropertiesForPath {
                        path: properties_path.clone(),
                    }),
                )
            }
        })
        .into_any_element()
    }
    #[cfg(not(target_os = "windows"))]
    {
        row.into_any_element()
    }
}

fn section_header(
    idx: usize,
    section_count: usize,
    section: &SidebarSection,
    app: &Entity<AtlasApp>,
    muted: Hsla,
    hover: Hsla,
    cx: &Context<SidebarPanel>,
) -> impl IntoElement {
    let label = if section.kind == SectionKind::Cloud {
        app.read(cx)
            .ext_registry
            .sidebar_section("cloud")
            .map_or_else(
                || sidebar_label(&section.kind),
                |section| section.title.clone(),
            )
    } else {
        sidebar_label(&section.kind)
    };
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
        .block_mouse_except_scroll()
        .w_full()
        .mt(crate::ui::rem_px(10.))
        .px_2()
        .py(crate::ui::rem_px(4.))
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
    if kind == SectionKind::Drives {
        menu = menu
            .separator()
            .menu("This PC Properties", Box::new(OpenThisPcProperties));
    }
    menu
}

fn sidebar_label(kind: &SectionKind) -> String {
    match kind {
        SectionKind::Places => rust_i18n::t!("sidebar_places").to_string(),
        SectionKind::Bookmarks => rust_i18n::t!("sidebar_bookmarks").to_string(),
        SectionKind::Tags => rust_i18n::t!("sidebar_tags").to_string(),
        SectionKind::Linux => rust_i18n::t!("sidebar_linux").to_string(),
        SectionKind::Drives => rust_i18n::t!("sidebar_drives").to_string(),
        SectionKind::Cloud => rust_i18n::t!("sidebar_cloud").to_string(),
        SectionKind::Recent => rust_i18n::t!("sidebar_recent").to_string(),
    }
}

fn linux_root_label(path: &std::path::Path) -> String {
    atlas_core::fs::paths::wsl_distro_name(path).unwrap_or_else(|| path.display().to_string())
}

fn empty_row(label: String, muted: Hsla) -> impl IntoElement {
    div().px_2().py_1().text_xs().text_color(muted).child(label)
}

/// One clickable row that navigates the focused pane to `path`. Styled per the Figma
/// sidebar item: Phosphor icon + label, rounded, with hover and an active (current
/// folder) state that tints the icon/label with the accent.
fn nav_row(
    id: SharedString,
    label: String,
    icon_name: &'static str,
    path: PathBuf,
    shell_tex: Option<Arc<gpui::RenderImage>>,
    app: &Entity<AtlasApp>,
    style: SidebarStyle,
    active: bool,
) -> impl IntoElement {
    let SidebarStyle {
        hover,
        active_bg,
        accent,
        fg,
        windows_theme,
        ..
    } = style;
    let app = app.clone();
    let hover_id = DragHoverId::Sidebar(id.to_string());
    let drop_path = path.clone();
    let drop_path_external = path.clone();
    let drop_app = app.clone();
    let drop_app_external = app.clone();
    let shell_path = path.clone();
    let shell_app = app.clone();
    let ctx_app = app.clone();
    let ctx_path = path.clone();
    let content = if active { accent } else { fg };
    let icon_color = if active {
        accent
    } else {
        // Slightly dimmed icon for inactive rows (Figma: ~85% ink).
        hsla(
            (fg.hue.into_positive_degrees() / 360.0),
            fg.saturation,
            fg.lightness,
            0.85,
        )
    };
    h_flex()
        .id(id)
        .w_full()
        .h(crate::ui::rem_px(if windows_theme { 30.0 } else { 24.0 }))
        .block_mouse_except_scroll()
        .px_2()
        .gap(crate::ui::rem_px(if windows_theme { 10.0 } else { 8.0 }))
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(content)
        .when(active, |s| {
            s.bg(active_bg).font_weight(gpui::FontWeight::MEDIUM)
        })
        .when(!active, |s| s.hover(|h| h.bg(hover)))
        .child(match shell_tex {
            Some(tex) => img(tex)
                .size(crate::ui::rem_px(if windows_theme { 18.0 } else { 16.0 }))
                .flex_none()
                .into_any_element(),
            None => theme_icon(
                ph_icon(icon_name),
                if windows_theme { 18.0 } else { 16.0 },
                icon_color,
                windows_theme,
            ),
        })
        .child(div().flex_1().truncate().child(SharedString::from(label)))
        .on_prepaint_child(move |_bounds, _window, cx| {
            #[cfg(target_os = "windows")]
            if windows_theme {
                shell_app.update(cx, |this, _cx| {
                    this.request_shell_thumbnail_or_icon(&shell_path);
                });
            }
        })
        .on_prepaint_child(drag_hover_recorder_with_drop(
            hover_id,
            DragHoverKind::Folder,
            DragHoverCommand::NavigatePath(path.clone()),
            DragHoverDropCommand::TransferTo(path.clone()),
            &app,
        ))
        .on_drop::<DraggedFiles>(move |d, window, cx| {
            if !is_sidebar_drop_allowed(&drop_app, window.mouse_position(), cx) {
                return;
            }
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
        .on_drop::<gpui::ExternalPaths>(move |paths, window, cx| {
            if !is_sidebar_drop_allowed(&drop_app_external, window.mouse_position(), cx) {
                return;
            }
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
        .on_mouse_down(MouseButton::Right, move |ev, window, cx| {
            ctx_app.update(cx, |this, cx| {
                this.open_ctx_menu(vec![ctx_path.clone()], ev.position, window, cx);
                cx.notify();
            });
            cx.stop_propagation();
        })
}

/// Like `nav_row` but navigates to an arbitrary `Location` (for virtual places
/// like RecycleBin that have no real `PathBuf`).
fn virtual_nav_row(
    id: SharedString,
    label: String,
    icon_name: &'static str,
    loc: Location,
    shell_tex: Option<Arc<gpui::RenderImage>>,
    app: &Entity<AtlasApp>,
    style: SidebarStyle,
    active: bool,
) -> impl IntoElement {
    let SidebarStyle {
        hover,
        active_bg,
        accent,
        fg,
        windows_theme,
        ..
    } = style;
    let app = app.clone();
    let hover_id = DragHoverId::Sidebar(id.to_string());
    let recycle_icon_app = app.clone();
    let request_recycle_icon = windows_theme && loc == Location::RecycleBin && shell_tex.is_none();
    let content = if active { accent } else { fg };
    let icon_color = if active {
        accent
    } else {
        hsla(
            (fg.hue.into_positive_degrees() / 360.0),
            fg.saturation,
            fg.lightness,
            0.85,
        )
    };
    h_flex()
        .id(id)
        .w_full()
        .h(crate::ui::rem_px(if windows_theme { 30.0 } else { 24.0 }))
        .block_mouse_except_scroll()
        .px_2()
        .gap(crate::ui::rem_px(if windows_theme { 10.0 } else { 8.0 }))
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .text_color(content)
        .when(active, |s| {
            s.bg(active_bg).font_weight(gpui::FontWeight::MEDIUM)
        })
        .when(!active, |s| s.hover(|h| h.bg(hover)))
        .child(match shell_tex {
            Some(tex) => img(tex)
                .size(px(if windows_theme { 18.0 } else { 16.0 }))
                .flex_none()
                .into_any_element(),
            None => theme_icon(
                ph_icon(icon_name),
                if windows_theme { 18.0 } else { 16.0 },
                icon_color,
                windows_theme,
            ),
        })
        .when(request_recycle_icon, |row| {
            row.on_prepaint_child(move |_bounds, _window, cx| {
                recycle_icon_app.update(cx, |this, _cx| {
                    this.request_shell_recycle_bin_icon();
                });
            })
        })
        .child(div().flex_1().truncate().child(SharedString::from(label)))
        .map(|row| {
            // Recycle Bin row doubles as a drop target (drop = send to trash).
            if loc == Location::RecycleBin {
                row.on_prepaint_child(drag_hover_recorder_with_drop(
                    hover_id,
                    DragHoverKind::Nav,
                    DragHoverCommand::NavigateLocation(loc.clone()),
                    DragHoverDropCommand::TrashPaths,
                    &app,
                ))
            } else {
                row.on_prepaint_child(drag_hover_recorder(
                    hover_id,
                    DragHoverKind::Nav,
                    DragHoverCommand::NavigateLocation(loc.clone()),
                    &app,
                ))
            }
        })
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
#[cfg(feature = "provider-host")]
fn cloud_remote_row(
    remote: atlas_model::cloud_model::CloudRemote,
    app: &Entity<AtlasApp>,
    style: SidebarStyle,
) -> impl IntoElement {
    let SidebarStyle {
        hover, fg, muted, ..
    } = style;
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
    // Mounted remotes use their filesystem path; Access remotes upload to root.
    let mount_path = match &status {
        RemoteStatus::Mounted(p) => Some(p.clone()),
        _ => None,
    };
    let is_busy = matches!(status, RemoteStatus::Connecting | RemoteStatus::Mounting);
    let drop_dest = (!is_busy).then(|| mount_path.clone().unwrap_or_else(|| crate::app::cloud::remote_sentinel(&id, "")));
    let hover_action = if is_busy {
        None
    } else if is_mounted {
        Some(Action::Cloud(CloudAction::OpenMount(id.clone())))
    } else {
        Some(Action::Cloud(CloudAction::OpenAccess(id.clone())))
    };
    let hover_id = DragHoverId::Sidebar(format!("cloud-remote-{id}"));
    let icon_color = hsla(
        (fg.hue.into_positive_degrees() / 360.0),
        fg.saturation,
        fg.lightness,
        0.85,
    );
    let app = app.clone();

    h_flex()
        .id(SharedString::from(format!("cloud-remote-{id}")))
        .block_mouse_except_scroll()
        .w_full()
        .h(crate::ui::rem_px(24.))
        .px_2()
        .gap(crate::ui::rem_px(8.))
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
            match drop_dest.clone() {
                Some(dest) => row.on_prepaint_child(drag_hover_recorder_with_drop(
                    hover_id,
                    DragHoverKind::Nav,
                    DragHoverCommand::Process(action),
                    DragHoverDropCommand::TransferTo(dest),
                    &app,
                )),
                None => row.on_prepaint_child(drag_hover_recorder(
                    hover_id,
                    DragHoverKind::Nav,
                    DragHoverCommand::Process(action),
                    &app,
                )),
            }
        })
        .when_some(drop_dest.clone(), |row, dest| {
            let drop_dest = dest.clone();
            let drop_dest_ext = dest;
            let drop_app = app.clone();
            let drop_app_ext = app.clone();
            row.on_drop::<DraggedFiles>(move |d, window, cx| {
                if !is_sidebar_drop_allowed(&drop_app, window.mouse_position(), cx) {
                    return;
                }
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
            .on_drop::<gpui::ExternalPaths>(move |paths, window, cx| {
                if !is_sidebar_drop_allowed(&drop_app_ext, window.mouse_position(), cx) {
                    return;
                }
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

#[cfg(feature = "provider-host")]
fn native_cloud_folders() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut push = |label: &str, path: Option<PathBuf>| {
        let Some(path) = path else {
            return;
        };
        if !path.is_dir() || out.iter().any(|(_, p)| *p == path) {
            return;
        }
        out.push((label.to_string(), path));
    };

    push("OneDrive", std::env::var_os("OneDrive").map(PathBuf::from));
    push(
        "OneDrive",
        std::env::var_os("OneDriveCommercial").map(PathBuf::from),
    );
    push(
        "OneDrive",
        std::env::var_os("OneDriveConsumer").map(PathBuf::from),
    );

    if let Some(home) = dirs::home_dir() {
        push("Google Drive", Some(home.join("Google Drive")));
        push("iCloud Drive", Some(home.join("iCloudDrive")));
        push("MEGA", Some(home.join("MEGA")));
        push("MEGA", Some(home.join("MEGAsync")));
        push("MEGA", Some(home.join("MEGA Sync")));

        #[cfg(target_os = "macos")]
        {
            let cloud_storage = home.join("Library").join("CloudStorage");
            if let Ok(entries) = std::fs::read_dir(cloud_storage) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with("GoogleDrive") {
                        push("Google Drive", Some(path));
                    } else if name.starts_with("OneDrive") {
                        push("OneDrive", Some(path));
                    } else if name == "iCloudDrive" {
                        push("iCloud Drive", Some(path));
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        for letter in b'A'..=b'Z' {
            let root = PathBuf::from(format!("{}:\\", letter as char));
            push("Google Drive", Some(root.join("My Drive")));
        }
    }

    out
}

/// Always-visible "+ Add cloud drive" row that incites the user to connect.
#[cfg(feature = "provider-host")]
fn add_cloud_row(app: &Entity<AtlasApp>, hover: Hsla, muted: Hsla, fg: Hsla) -> impl IntoElement {
    use crate::app::cloud::CloudAction;
    use crate::input::action::Action;

    let app = app.clone();
    let icon_color = hsla(
        (muted.hue.into_positive_degrees() / 360.0),
        muted.saturation,
        muted.lightness,
        0.7,
    );
    h_flex()
        .id("cloud-add-row")
        .block_mouse_except_scroll()
        .w_full()
        .h(crate::ui::rem_px(24.))
        .px_2()
        .gap(crate::ui::rem_px(8.))
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
