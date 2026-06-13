//! FASE B: left sidebar as a Dock `left_dock` panel.
//!
//! `SidebarPanel` is its own gpui `Entity` (like `FilePanel`) so it can live in the
//! `DockArea`'s left dock. It reads `AppState` through a weak handle and navigates
//! the focused pane on click. Places + Bookmarks land here; Tags/Drives/Cloud are
//! added with their subsystems (Fase H).

use crate::app::App as AtlasApp;
use crate::input::action::Action;
use crate::state::profiles::{SectionKind, SidebarSection};
use crate::state::tag_cache::TagId;
use crate::ui::icons::{icon, AtlasIcon};
use crate::{
    SidebarHideSection, SidebarMoveSectionDown, SidebarMoveSectionUp, SidebarNewGlobalTag,
    SidebarNewProfileTag, SidebarResetSections, SidebarToggleSection,
};
use gpui::prelude::*;
use gpui::{
    div, hsla, px, relative, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    MouseButton, SharedString, WeakEntity, Window,
};
use gpui_component::dock::{Panel, PanelControl, PanelEvent};
use gpui_component::menu::{ContextMenuExt, PopupMenu};
use gpui_component::searchable_list::{SearchableListDelegate, SearchableListItem};
use gpui_component::select::{Select, SelectEvent, SelectState};
use gpui_component::{h_flex, v_flex, ActiveTheme, IndexPath, Sizable, Size};
use std::path::PathBuf;

pub struct SidebarPanel {
    app: WeakEntity<AtlasApp>,
    focus_handle: FocusHandle,
    /// Profile dropdown (Figma "Pulldown Button"). Rows are profile names plus two
    /// trailing action rows (new / rename) separated by a divider — see [`ProfileMenu`].
    profile_select: Entity<SelectState<ProfileMenu>>,
    /// (profile count, active_idx) last pushed into `profile_select`, to detect
    /// external profile changes (add/remove/switch) and resync the dropdown.
    profile_sig: (usize, usize),
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

        let profile_sig = (names.len(), active_idx);
        let profile_select = cx.new(|cx| {
            SelectState::new(
                profile_menu(names),
                Some(IndexPath::default().row(active_idx)),
                window,
                cx,
            )
        });
        cx.subscribe(
            &profile_select,
            |this, _select, event: &SelectEvent<ProfileMenu>, cx| {
                let SelectEvent::Confirm(Some(value)) = event else {
                    return;
                };
                let Some(app) = this.app.upgrade() else {
                    return;
                };
                // The two trailing rows are actions, not profiles — match by label.
                if value == &profile_action_label(Action::CreateProfile(String::new())) {
                    app.update(cx, |a, cx| {
                        a.process_actions(vec![Action::CreateProfile(String::new())]);
                        cx.notify();
                    });
                    this.force_profile_resync();
                    cx.notify();
                    return;
                }
                if value == &profile_action_label(Action::OpenRenameProfile) {
                    app.update(cx, |a, cx| {
                        a.process_actions(vec![Action::OpenRenameProfile]);
                        cx.notify();
                    });
                    this.force_profile_resync();
                    cx.notify();
                    return;
                }
                let idx = app
                    .read(cx)
                    .state
                    .profiles
                    .profiles
                    .iter()
                    .position(|p| value.as_ref() == p.name);
                if let Some(idx) = idx {
                    app.update(cx, |a, cx| {
                        a.switch_profile(idx);
                        cx.notify();
                    });
                }
            },
        )
        .detach();

        Self {
            app: app.downgrade(),
            focus_handle: cx.focus_handle(),
            profile_select,
            profile_sig,
        }
    }

    /// Force the next render to rebuild the dropdown items and reset the selected
    /// row to the active profile. Used after a menu *action* row (new / rename)
    /// fires, so the pill never lingers showing the action label.
    fn force_profile_resync(&mut self) {
        self.profile_sig = (usize::MAX, usize::MAX);
    }
}

/// Localized label for a profile *action* row, shared by the menu builder and the
/// confirm-matcher so the two never drift.
fn profile_action_label(action: Action) -> SharedString {
    match action {
        Action::CreateProfile(_) => rust_i18n::t!("sidebar_profile_new").to_string().into(),
        Action::OpenRenameProfile => rust_i18n::t!("sidebar_profile_rename").to_string().into(),
        _ => SharedString::default(),
    }
}

/// One dropdown row: either a profile (selectable, gets the active check) or an
/// action (rendered with an icon; the first one draws a divider above it to split
/// the two zones).
#[derive(Clone)]
struct ProfileRow {
    value: SharedString,
    /// `Some(icon)` marks an action row; `None` is a plain profile row.
    action_icon: Option<AtlasIcon>,
    /// Draw a separator line above this row (true only on the first action row).
    divider_above: bool,
}

impl SearchableListItem for ProfileRow {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.value.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }

    fn render(&self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(glyph) = self.action_icon else {
            // Profile row: plain label; the adapter adds the trailing check icon.
            return div().child(self.value.clone()).into_any_element();
        };
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let mut col = v_flex().w_full().gap_1();
        if self.divider_above {
            col = col.child(div().h(px(1.)).w_full().bg(border));
        }
        col.child(
            h_flex()
                .gap_2()
                .items_center()
                .text_color(muted)
                .child(icon(glyph, 14.0, muted))
                .child(self.value.clone()),
        )
        .into_any_element()
    }
}

/// Delegate backing the profile dropdown: the real profiles followed by the two
/// action rows, so create/rename live in the menu without stealing pill width.
struct ProfileMenu {
    rows: Vec<ProfileRow>,
}

impl SearchableListDelegate for ProfileMenu {
    type Item = ProfileRow;

    fn items_count(&self, _section: usize) -> usize {
        self.rows.len()
    }

    fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
        self.rows.get(ix.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.rows
            .iter()
            .position(|r| r.value() == value)
            .map(|i| IndexPath::default().row(i))
    }
}

/// Build the dropdown delegate from the profile names.
fn profile_menu(names: Vec<String>) -> ProfileMenu {
    let mut rows: Vec<ProfileRow> = names
        .into_iter()
        .map(|n| ProfileRow {
            value: n.into(),
            action_icon: None,
            divider_above: false,
        })
        .collect();
    rows.push(ProfileRow {
        value: profile_action_label(Action::CreateProfile(String::new())),
        action_icon: Some(AtlasIcon::Plus),
        divider_above: true,
    });
    rows.push(ProfileRow {
        value: profile_action_label(Action::OpenRenameProfile),
        action_icon: Some(AtlasIcon::PencilSimple),
        divider_above: false,
    });
    ProfileMenu { rows }
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        // The focused pane's current folder — used to mark the matching place active.
        let current_path = app.read(cx).state.active_tab_data().and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()));

        // Resync the profile dropdown when the profile list/active changed externally
        // (e.g. a profile was added/removed or switched from elsewhere).
        let cur_sig = {
            let a = app.read(cx);
            (a.state.profiles.profiles.len(), a.state.profiles.active_idx)
        };
        if cur_sig != self.profile_sig {
            let names: Vec<String> = app
                .read(cx)
                .state
                .profiles
                .profiles
                .iter()
                .map(|p| p.name.clone())
                .collect();
            let active = cur_sig.1;
            self.profile_select.update(cx, |st, cx| {
                st.set_items(profile_menu(names), window, cx);
                st.set_selected_index(Some(IndexPath::default().row(active)), window, cx);
            });
            self.profile_sig = cur_sig;
        }

        // Profile selector pill (Figma "Pulldown Button"), sits above the sections.
        // The dropdown both switches profiles and — via the two action rows at the
        // bottom of its menu (see `profile_menu_items`) — creates / renames, so no
        // buttons steal width from the pill.
        let profile_pill = h_flex().w_full().pb(px(8.)).child(
            Select::new(&self.profile_select)
                .placeholder(rust_i18n::t!("sidebar_profile_placeholder"))
                .cleanable(false)
                .with_size(Size::Small)
                .menu_width(px(193.)),
        );

        let mut col = v_flex()
            .size_full()
            .p_2()
            .gap_1()
            // Transparent: the hosting TabPanel (fork) is the rounded glass card. The
            // sidebar just fills it; `show_title_bar` is false so there's no dock title.
            .text_color(fg)
            .text_sm()
            .child(profile_pill)
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

        col.into_any_element()
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
        _ => AtlasIcon::File,
    }
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
    cx: &mut Context<SidebarPanel>,
) -> gpui::Div {
    match kind {
        SectionKind::Places => {
            let places: Vec<(&'static str, String, PathBuf)> = [
                ("house", rust_i18n::t!("sidebar_home").to_string(), dirs::home_dir()),
                ("monitor", rust_i18n::t!("sidebar_desktop").to_string(), dirs::desktop_dir()),
                ("file-text", rust_i18n::t!("sidebar_documents").to_string(), dirs::document_dir()),
                ("download-simple", rust_i18n::t!("sidebar_downloads").to_string(), dirs::download_dir()),
                ("image", rust_i18n::t!("sidebar_pictures").to_string(), dirs::picture_dir()),
                ("music-notes", rust_i18n::t!("sidebar_music").to_string(), dirs::audio_dir()),
                ("film-strip", rust_i18n::t!("sidebar_videos").to_string(), dirs::video_dir()),
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
                col = col.child(empty_row(rust_i18n::t!("sidebar_no_bookmarks").to_string(), muted));
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
            let tags: Vec<(String, [u8; 3], TagId)> = {
                let app_ref = app.read(cx);
                let profile = app_ref.state.profiles.active().name.clone();
                app_ref
                    .state
                    .tag_cache
                    .visible_defs(&profile)
                    .map(|d| (d.name.clone(), d.color, d.id))
                    .collect()
            };
            if tags.is_empty() {
                col = col.child(empty_row(rust_i18n::t!("sidebar_no_tags").to_string(), muted));
            } else {
                for (name, color, id) in tags {
                    col = col.child(tag_row(name, color, id, app, hover, fg));
                }
            }
        }
        SectionKind::Drives => {
            let drives = crate::fs::mounts::list_drives();
            if drives.is_empty() {
                col = col.child(empty_row(rust_i18n::t!("sidebar_no_drives").to_string(), muted));
            } else {
                for drive in drives {
                    let active = current.is_some_and(|c| c == drive.mount_point.as_path());
                    col = col.child(drive_row(
                        drive, app, hover, active_bg, accent, fg, muted, active,
                    ));
                }
            }
        }
        SectionKind::Recent => col = col.child(empty_row(rust_i18n::t!("sidebar_no_recent").to_string(), muted)),
        SectionKind::Cloud => col = col.child(empty_row(rust_i18n::t!("sidebar_no_cloud").to_string(), muted)),
    }
    col
}

/// One clickable tag row that opens its virtual "tag results" tab.
fn tag_row(
    name: String,
    color: [u8; 3],
    id: TagId,
    app: &Entity<AtlasApp>,
    hover: Hsla,
    fg: Hsla,
) -> impl IntoElement {
    let app = app.clone();
    let rgb =
        gpui::rgb((u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2]));
    h_flex()
        .id(SharedString::from(format!("tag-{}", id.0)))
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
        .child(SharedString::from(name))
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
    let id = SharedString::from(format!("drive-{}", drive.mount_point.display()));
    let icon_color = hsla(fg.h, fg.s, fg.l, 0.85);
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
            if collapsed { rust_i18n::t!("sidebar_expand") } else { rust_i18n::t!("sidebar_collapse") },
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
    menu = menu
        .separator()
        .menu(rust_i18n::t!("sidebar_restore_order"), Box::new(SidebarResetSections));
    if kind == SectionKind::Tags {
        menu = menu
            .separator()
            .menu(rust_i18n::t!("sidebar_new_global_tag"), Box::new(SidebarNewGlobalTag))
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
        .on_click(move |_ev, _window, cx| {
            let path = path.clone();
            app.update(cx, |this, cx| {
                let tile = this.state.focused_tile;
                this.navigate(tile, path);
                cx.notify();
            });
        })
}
