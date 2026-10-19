//! Atlas–style right-click menu: a searchable overlay combining Atlas's own
//! commands (top) with the native Windows shell context menu (Open with, Send to,
//! 7-Zip, Properties, …) enumerated by the [`crate::shell_menu`] worker.
//!
//! The Atlas section is cross-platform; native items only appear on Windows.
//! Atlas rows dispatch the same boxed gpui actions the old `PopupMenu` used, so
//! all existing wiring is reused. Native rows are invoked through the worker.

use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use atlas_model::ext_model::CommandRef;

use crate::ui::imm::{motion::RetainedSpring, ImmTextInputState};
use gpui::{
    div, px, AnyElement, Context, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, ParentElement, Pixels, Point, RenderImage, Styled, Window,
};
use gpui_component::{kbd::Kbd, ActiveTheme};

use crate::app::App;
use crate::input::action::Action;
use crate::ui::icons::AtlasIcon;

/// One native shell entry, ready for rendering (icon already a GPU texture).
pub(crate) struct UiNativeItem {
    pub id: u32,
    pub title: String,
    pub icon: Option<Arc<RenderImage>>,
    pub kind: UiItemKind,
    pub disabled: bool,
    pub is_default: bool,
}

pub(crate) enum UiItemKind {
    Normal,
    Separator,
    Submenu { children: Option<Vec<UiNativeItem>> },
}

/// State for the open right-click menu. `None` on `App` when closed.
pub(crate) struct CtxMenuState {
    /// Monotonic id; stale `Built` events from a prior open are ignored.
    pub menu_id: u64,
    pub focus: FocusHandle,
    pub input: ImmTextInputState,
    pub scroll_top: f32,
    pub scroll_motion: RetainedSpring,
    pub input_bounds: Option<gpui::Bounds<Pixels>>,
    pub input_line: Option<gpui::ShapedLine>,
    pub input_scroll_x: f32,
    pub input_drag: bool,
    pub scroll_drag: Option<f32>,
    pub blink_visible: bool,
    /// One timeline per opening; search, hover and native results never replay it.
    pub opened_at: Instant,
    /// Top-left of the panel, in window-logical pixels.
    pub anchor: Point<Pixels>,
    /// Physical screen pixels for the native `InvokeCommand` (`ptInvoke`).
    pub screen_pt: (i32, i32),
    // reason: retained for a future "rebuild menu" path; not read yet.
    #[allow(dead_code)]
    pub paths: Vec<PathBuf>,
    pub native: Vec<UiNativeItem>,
    /// True until the worker's `Built` event arrives (shows a skeleton row).
    pub loading: bool,
    /// Keyboard cursor into the flattened list of selectable rows.
    pub selected: usize,
    /// Keyboard navigation requested that the selected row be scrolled into view.
    pub scroll_to_selected: bool,
    /// Inline-expanded submenu ids (no query / tree mode).
    pub expanded: HashSet<u32>,
    /// True when opened on empty space (background), not on a file selection.
    pub background: bool,
    /// Extension commands that apply to the current selection (prefix `ext.`).
    pub ext_cmds: Vec<CommandRef>,
    /// Associated system desktop applications for "Open with..."
    pub desktop_apps: Vec<atlas_core::fs::desktop_apps::DesktopApp>,
}

/// Atlas's own commands. Each maps to a label, icon and a boxed gpui action that
/// the root `Atlas` key context handles against the current selection.
#[derive(Clone, Copy, PartialEq)]
enum AtlasCmd {
    Open,
    Rename,
    Copy,
    Cut,
    Paste,
    #[cfg(windows)]
    PasteShortcut,
    Trash,
    DeletePerm,
    Restore,
    Properties,
    NewFolder,
    NewFile,
    OpenTerminal,
    Refresh,
    SelectAll,
}

impl AtlasCmd {
    fn label(self) -> String {
        match self {
            Self::Open => rust_i18n::t!("root_open").to_string(),
            Self::Rename => rust_i18n::t!("root_rename").to_string(),
            Self::Copy => rust_i18n::t!("root_copy").to_string(),
            Self::NewFolder => rust_i18n::t!("root_new_folder").to_string(),
            Self::NewFile => rust_i18n::t!("cmd_files_create_file_title").to_string(),
            Self::OpenTerminal => rust_i18n::t!("cmd_files_open_terminal_title").to_string(),
            Self::Refresh => rust_i18n::t!("cmd_nav_refresh_title").to_string(),
            Self::SelectAll => rust_i18n::t!("root_select_all").to_string(),
            Self::Cut => rust_i18n::t!("root_cut").to_string(),
            Self::Paste => rust_i18n::t!("root_paste").to_string(),
            #[cfg(windows)]
            Self::PasteShortcut => rust_i18n::t!("cmd_paste_shortcut_title").to_string(),
            Self::Trash => rust_i18n::t!("root_move_to_trash").to_string(),
            Self::DeletePerm => rust_i18n::t!("root_delete_permanently").to_string(),
            Self::Restore => rust_i18n::t!("root_restore").to_string(),
            Self::Properties => rust_i18n::t!("root_properties").to_string(),
        }
    }
    fn icon(self) -> Option<AtlasIcon> {
        match self {
            Self::Open => Some(AtlasIcon::Eye),
            Self::Rename => Some(AtlasIcon::PencilSimple),
            Self::Copy => Some(AtlasIcon::Copy),
            Self::Cut => Some(AtlasIcon::Scissors),
            Self::Paste => Some(AtlasIcon::Clipboard),
            #[cfg(windows)]
            Self::PasteShortcut => Some(AtlasIcon::Copy),
            Self::Trash => Some(AtlasIcon::Trash),
            Self::DeletePerm => Some(AtlasIcon::X),
            Self::Restore => Some(AtlasIcon::ArrowCounterClockwise),
            Self::Properties => Some(AtlasIcon::Gear),
            Self::NewFolder => Some(AtlasIcon::Folder),
            Self::NewFile => Some(AtlasIcon::File),
            Self::OpenTerminal => Some(AtlasIcon::Lightning),
            Self::Refresh => Some(AtlasIcon::ArrowClockwise),
            Self::SelectAll => Some(AtlasIcon::SelectionAll),
        }
    }

    fn command_id(self) -> &'static str {
        match self {
            Self::Open => "selection.open",
            Self::Rename => "files.rename",
            Self::Copy => "files.copy",
            Self::Cut => "files.cut",
            Self::Paste => "files.paste",
            #[cfg(windows)]
            Self::PasteShortcut => "files.paste-shortcut",
            Self::Trash => "files.trash",
            Self::DeletePerm => "files.delete",
            Self::Refresh => "nav.refresh",
            Self::SelectAll => "edit.select-all",
            Self::Restore => "recycle.restore",
            Self::Properties => "files.properties",
            Self::NewFolder => "files.create-folder",
            Self::NewFile => "files.create-file",
            Self::OpenTerminal => "files.open-terminal",
        }
    }
}

/// Atlas items for normal directory views. `true` = separator follows the item.
const ATLAS_ITEMS: &[(AtlasCmd, bool)] = &[
    (AtlasCmd::Open, false),
    (AtlasCmd::Rename, true),
    (AtlasCmd::Copy, false),
    (AtlasCmd::Cut, false),
    (AtlasCmd::Paste, true),
    #[cfg(windows)]
    (AtlasCmd::PasteShortcut, true),
    (AtlasCmd::Trash, false),
    (AtlasCmd::DeletePerm, true),
    (AtlasCmd::Properties, false),
];

/// Atlas items shown when the current tab is the Recycle Bin.
const ATLAS_ITEMS_RECYCLE: &[(AtlasCmd, bool)] = &[
    (AtlasCmd::Restore, false),
    (AtlasCmd::Cut, false),
    (AtlasCmd::DeletePerm, true),
    (AtlasCmd::Properties, false),
];

/// Atlas items shown on right-click on empty background (no file selection).
const ATLAS_ITEMS_BACKGROUND: &[(AtlasCmd, bool)] = &[
    (AtlasCmd::NewFile, false),
    (AtlasCmd::NewFolder, false),
    (AtlasCmd::OpenTerminal, true),
    (AtlasCmd::Paste, false),
    #[cfg(windows)]
    (AtlasCmd::PasteShortcut, false),
    (AtlasCmd::Refresh, false),
    (AtlasCmd::SelectAll, false),
];

/// A logical (non-element) row, recomputed each frame and shared by the renderer
/// and the keyboard handler so selection indices always agree.
#[derive(Clone)]
enum CtxRow {
    Separator,
    Loading,
    Atlas {
        cmd: AtlasCmd,
    },
    Ext {
        ext_id: String,
        cmd_id: String,
        title: String,
    },
    Native {
        id: u32,
        title: String,
        icon: Option<Arc<RenderImage>>,
        depth: u8,
        submenu: bool,
        expanded: bool,
        disabled: bool,
        is_default: bool,
    },
    DesktopApp {
        app: atlas_core::fs::desktop_apps::DesktopApp,
    },
}

impl CtxRow {
    fn selectable(&self) -> bool {
        matches!(
            self,
            CtxRow::Atlas { .. }
                | CtxRow::Ext { .. }
                | CtxRow::Native { .. }
                | CtxRow::DesktopApp { .. }
        )
    }
}

impl App {
    /// Open the right-click menu for `paths` anchored at `anchor` (window-logical
    /// px). Sends a `Build` request to the shell worker (Windows).
    pub(crate) fn open_ctx_menu(
        &mut self,
        paths: Vec<PathBuf>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.suppress_next_shell_ctx_menu {
            self.suppress_next_shell_ctx_menu = false;
            self.close_ctx_menu(cx);
            return;
        }
        if paths.is_empty() {
            return;
        }
        self.close_type_filter_menu(cx);
        self.close_breadcrumb_menu(cx);
        self.ctx_menu_seq += 1;
        let menu_id = self.ctx_menu_seq;

        // Physical screen pixels for the eventual native invoke.
        let screen_pt = cursor_screen_pos();

        // Same-parent grouping: the native shell menu needs one parent folder.
        // If the selection spans parents we still show Atlas items (native empty).
        let same_parent = paths
            .first()
            .and_then(|p| p.parent())
            .map(|p0| paths.iter().all(|p| p.parent() == Some(p0)))
            .unwrap_or(false);

        let is_recycle_bin = self
            .state
            .active_tab_data()
            .is_some_and(|t| matches!(t.location, atlas_model::location::Location::RecycleBin));

        #[cfg(windows)]
        let loading = {
            // Skip the native shell Build for RecycleBin items: the $R paths are not
            // valid shell namespace objects and the native menu would duplicate Restore.
            if same_parent && !is_recycle_bin {
                if let Some(handle) = &self.shell_menu {
                    handle.send(crate::shell_menu::ShellMenuRequest::Build {
                        id: menu_id,
                        paths: paths.clone(),
                    });
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        #[cfg(not(windows))]
        let loading = {
            let _ = (same_parent, is_recycle_bin);
            false
        };

        let selection: Vec<(Option<String>, bool)> = paths
            .iter()
            .map(|p| {
                (
                    p.extension().map(|e| e.to_string_lossy().to_lowercase()),
                    false,
                )
            })
            .collect();
        let ext_cmds = self.ext_registry.context_commands_for(&selection);
        let desktop_apps = if paths.is_empty() || is_recycle_bin {
            Vec::new()
        } else {
            paths
                .first()
                .map(|p| atlas_core::fs::desktop_apps::apps_for_file(p))
                .unwrap_or_default()
        };

        self.ctx_menu = Some(CtxMenuState {
            menu_id,
            focus: cx.focus_handle(),
            input: ImmTextInputState::new(""),
            scroll_top: 0.0,
            scroll_motion: RetainedSpring::new(0.0, Instant::now()),
            input_bounds: None,
            input_line: None,
            input_scroll_x: 0.0,
            input_drag: false,
            scroll_drag: None,
            blink_visible: true,
            opened_at: Instant::now(),
            anchor,
            screen_pt,
            paths,
            native: Vec::new(),
            loading,
            selected: 0,
            scroll_to_selected: true,
            expanded: HashSet::new(),
            background: false,
            ext_cmds,
            desktop_apps,
        });
        cx.notify();
    }

    /// Close the menu, dismissing the worker's COM objects (Windows).
    pub(crate) fn close_ctx_menu(&mut self, cx: &mut Context<Self>) {
        if self.ctx_menu.is_some() {
            self.cancel_command_shortcut_capture();
        }
        if let Some(state) = self.ctx_menu.take() {
            #[cfg(windows)]
            if let Some(handle) = &self.shell_menu {
                handle.send(crate::shell_menu::ShellMenuRequest::Dismiss { id: state.menu_id });
            }
            let _ = state;
        }

        cx.notify();
    }

    /// The existing foreground drain drives only blink edges, not continuous frames.
    pub(crate) fn tick_ctx_input_blink(&mut self) -> bool {
        let Some(menu) = self.ctx_menu.as_mut() else {
            return false;
        };
        let visible = self.command_shortcut_capture.is_none()
            && (menu.input.blink_epoch.elapsed().as_millis() / 530) % 2 == 0;
        let changed = visible != menu.blink_visible;
        menu.blink_visible = visible;
        changed
    }

    /// Drain shell-worker events into the open menu. Returns whether anything
    /// changed (so the event-driven drain repaints). Windows-only.
    #[cfg(windows)]
    pub(crate) fn drain_shell_menu_events(&mut self, cx: &mut Context<Self>) -> bool {
        use crate::shell_menu::ShellMenuEvent;
        let Some(handle) = self.shell_menu.clone() else {
            return false;
        };
        let events = handle.drain_events();
        if events.is_empty() {
            return false;
        }
        let mut dirty = false;
        for ev in events {
            match ev {
                ShellMenuEvent::Built { id, items } => {
                    let stale = self.ctx_menu.as_ref().map(|m| m.menu_id) != Some(id);
                    if stale {
                        handle.send(crate::shell_menu::ShellMenuRequest::Dismiss { id });
                        continue;
                    }
                    let ui = native_items_to_ui(items, cx);
                    if let Some(m) = self.ctx_menu.as_mut() {
                        m.native = ui;
                        m.loading = false;
                    }
                    dirty = true;
                }
                ShellMenuEvent::SubmenuExpanded { id, item_id, items } => {
                    if self.ctx_menu.as_ref().map(|m| m.menu_id) != Some(id) {
                        continue;
                    }
                    let ui = native_items_to_ui(items, cx);
                    if let Some(m) = self.ctx_menu.as_mut() {
                        let mut slot = Some(ui);
                        fill_submenu(&mut m.native, item_id, &mut slot);
                    }
                    dirty = true;
                }
                ShellMenuEvent::InvokeDone { id, result } => {
                    if let Err(e) = result {
                        tracing::warn!(menu_id = id, "shell invoke failed: {e}");
                    }
                    handle.send(crate::shell_menu::ShellMenuRequest::Dismiss { id });
                    if self.ctx_menu.as_ref().map(|m| m.menu_id) == Some(id) {
                        self.ctx_menu = None;
                    }
                    dirty = true;
                }
            }
        }
        if dirty {
            cx.notify();
        }
        dirty
    }

    /// The focus handle of the context menu's search input, for `sync_overlay_focus`.
    pub(crate) fn ctx_menu_focus(&self, cx: &Context<Self>) -> Option<FocusHandle> {
        let _ = cx;
        self.ctx_menu.as_ref().map(|menu| menu.focus.clone())
    }

    /// Build this frame's logical rows from the current query + native tree.
    fn ctx_menu_rows(&self, query: &str, scorer_query: &str) -> Vec<CtxRow> {
        let Some(state) = self.ctx_menu.as_ref() else {
            return Vec::new();
        };
        let is_recycle_bin = self
            .state
            .active_tab_data()
            .is_some_and(|t| matches!(t.location, atlas_model::location::Location::RecycleBin));
        let atlas_items = if state.background {
            ATLAS_ITEMS_BACKGROUND
        } else if is_recycle_bin {
            ATLAS_ITEMS_RECYCLE
        } else {
            ATLAS_ITEMS
        };

        let q = query.trim();
        let rows = if q.is_empty() {
            // Tree mode: Atlas section, separator, native tree (loading skeleton).
            let mut rows = Vec::new();
            for (cmd, sep_after) in atlas_items {
                rows.push(CtxRow::Atlas { cmd: *cmd });
                if *cmd == AtlasCmd::Open && !state.desktop_apps.is_empty() {
                    for app in state.desktop_apps.iter().take(4) {
                        rows.push(CtxRow::DesktopApp { app: app.clone() });
                    }
                }
                if *sep_after {
                    rows.push(CtxRow::Separator);
                }
            }
            if !state.ext_cmds.is_empty() {
                rows.push(CtxRow::Separator);
                for cmd in &state.ext_cmds {
                    rows.push(CtxRow::Ext {
                        ext_id: cmd.ext_id.clone(),
                        cmd_id: cmd.command.id.clone(),
                        title: cmd.command.title.clone(),
                    });
                }
            }
            if !state.native.is_empty() || state.loading {
                rows.push(CtxRow::Separator);
            }
            push_native_tree(&state.native, 0, &state.expanded, &mut rows);
            if state.loading {
                rows.push(CtxRow::Loading);
            }
            rows
        } else {
            // Search mode: flatten every leaf (Atlas + native + ext + apps), rank, emit flat.
            let _ = scorer_query;
            let mut cand_titles: Vec<String> = Vec::new();
            let mut cands: Vec<CtxRow> = Vec::new();

            for (cmd, _) in atlas_items {
                cand_titles.push(cmd.label().to_string());
                cands.push(CtxRow::Atlas { cmd: *cmd });
            }
            for app in &state.desktop_apps {
                cand_titles.push(format!("{} {}", rust_i18n::t!("open_with"), app.name));
                cands.push(CtxRow::DesktopApp { app: app.clone() });
            }
            for cmd in &state.ext_cmds {
                cand_titles.push(cmd.command.title.clone());
                cands.push(CtxRow::Ext {
                    ext_id: cmd.ext_id.clone(),
                    cmd_id: cmd.command.id.clone(),
                    title: cmd.command.title.clone(),
                });
            }
            collect_native_leaves(&state.native, "", &mut cand_titles, &mut cands);

            // Rank via the shared fuzzy matcher (best first). The scorer needs
            // &mut self, which we don't have here; ranking is done in the caller.
            let order = rank_titles(q, &cand_titles);
            order
                .into_iter()
                .map(|i| std::mem::replace(&mut cands[i], CtxRow::Separator))
                .filter(|r| r.selectable())
                .collect()
        };
        sanitize_separators(rows)
    }

    /// Render the context-menu overlay layer (scrim + anchored panel).
    pub(crate) fn render_ctx_menu_layer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (fg, muted, border, accent, accent_fg, bg, radius) = {
            let t = cx.theme();
            (
                t.popover_foreground,
                t.muted_foreground,
                t.border,
                t.accent,
                t.accent_foreground,
                t.popover,
                f32::from(t.radius),
            )
        };
        let query = self
            .ctx_menu
            .as_ref()
            .map(|menu| menu.input.text.clone())
            .unwrap_or_default();

        let rows = self.ctx_menu_rows(&query, &query);
        let selectable_count = rows.iter().filter(|row| row.selectable()).count();
        if let Some(state) = self.ctx_menu.as_mut() {
            if selectable_count == 0 {
                state.selected = 0;
            } else if state.selected >= selectable_count {
                state.selected = selectable_count - 1;
                state.scroll_to_selected = true;
            }
        }
        let state = self.ctx_menu.as_ref();
        let reveal_ms = if self.state.settings.disable_animations {
            f32::MAX
        } else {
            state.map_or(f32::MAX, |s| s.opened_at.elapsed().as_secs_f32() * 1000.0)
        };
        let reveal_duration = crate::ui::motion::MENU_REVEAL_MS
            + crate::ui::motion::MENU_STAGGER_MS * crate::ui::motion::MENU_STAGGER_CAP as u64;
        if reveal_ms < reveal_duration as f32 {
            window.request_animation_frame();
        }
        let (anchor, selected) = state
            .map(|s| (s.anchor, s.selected))
            .unwrap_or((Point::default(), 0));

        // Clamp the panel into the window so it never spills off an edge.
        let win = window.bounds().size;
        let zoom = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        let width = (340.0 * zoom).min((f32::from(win.width) - 12.0).max(1.0));
        let (ax, ay) = (f32::from(anchor.x), f32::from(anchor.y));
        let (ww, wh) = (f32::from(win.width), f32::from(win.height));
        const MAX_H: f32 = 640.;
        const V_MARGIN: f32 = 72.;
        let input_h = 40.0 * zoom;
        let content_h = (rows.iter().map(ctx_row_height).sum::<f32>() + 8.0) * zoom;
        let max_h = MAX_H.min((wh - V_MARGIN * 2.0).max(180.0));
        let est_h = (content_h + input_h).min(max_h);
        let list_h = est_h - input_h;
        let left = ax.min((ww - width - 6.0).max(6.0)).max(6.0);
        let top = ay.min((wh - est_h - 6.0).max(6.0)).max(6.0);

        let view = cx.entity();

        // Logical rows remain the command authority; IMM only receives ink and ids.
        let selected_child_idx = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.selectable())
            .nth(selected)
            .map(|(index, _)| index);
        let mut ink = Vec::with_capacity(rows.len());
        for row in &rows {
            use crate::ui::imm::menu::{MenuIcon, MenuRow, MenuRowKind};
            let mut item = MenuRow {
                kind: MenuRowKind::Item,
                label: "".into(),
                icon: None,
                shortcut: None,
                shortcut_editable: false,
                shortcut_recording: false,
                depth: 0,
                disabled: false,
                emphasized: false,
                submenu: false,
                expanded: false,
            };
            match row {
                CtxRow::Separator => item.kind = MenuRowKind::Separator,
                CtxRow::Loading => {
                    item.kind = MenuRowKind::Loading;
                    item.label = rust_i18n::t!("palette_searching").to_string().into();
                }
                CtxRow::Atlas { cmd } => {
                    item.label = cmd.label().to_string().into();
                    item.icon = cmd.icon().map(MenuIcon::Theme);
                    let command_id = cmd.command_id();
                    item.shortcut_editable = crate::input::commands::command_remappable(command_id);
                    item.shortcut_recording =
                        self.command_shortcut_capture.as_deref() == Some(command_id);
                    item.shortcut = Some(if item.shortcut_recording {
                        rust_i18n::t!("palette_shortcut_recording").into()
                    } else if let Some(key) = self.command_shortcut_keystroke(command_id) {
                        Kbd::format(&key).into()
                    } else {
                        rust_i18n::t!("palette_shortcut_none").into()
                    });
                }
                CtxRow::Ext { title, .. } => {
                    item.label = title.clone().into();
                    item.icon = Some(MenuIcon::Theme(AtlasIcon::PuzzlePiece));
                }
                CtxRow::Native {
                    title,
                    icon,
                    depth,
                    submenu,
                    expanded,
                    disabled,
                    is_default,
                    ..
                } => {
                    item.label = title.clone().into();
                    item.icon = icon.clone().map(MenuIcon::Image);
                    item.depth = *depth;
                    item.submenu = *submenu;
                    item.expanded = *expanded;
                    item.disabled = *disabled;
                    item.emphasized = *is_default;
                }
                CtxRow::DesktopApp { app } => {
                    item.label = format!("{} {}", rust_i18n::t!("open_with"), app.name).into();
                    item.icon = Some(MenuIcon::Theme(AtlasIcon::Eye));
                }
            }
            ink.push(item);
        }
        // The list now has a single layout child. Keyboard visibility uses pixel slots.
        if self
            .ctx_menu
            .as_ref()
            .is_some_and(|state| state.scroll_to_selected)
        {
            if let Some(index) = selected_child_idx {
                let row_top =
                    4.0 * zoom + ink.iter().take(index).map(|r| r.height(zoom)).sum::<f32>();
                let row_bottom = row_top + ink[index].height(zoom);
                let scroll_top = self.ctx_menu.as_ref().map_or(0.0, |menu| menu.scroll_top);
                let viewport = (list_h - zoom).max(1.0);
                let next = crate::ui::imm::menu::visible_scroll_top(
                    scroll_top, viewport, row_top, row_bottom,
                );
                if let Some(menu) = self.ctx_menu.as_mut() {
                    menu.scroll_top = next;
                }
            }
            if let Some(state) = self.ctx_menu.as_mut() {
                state.scroll_to_selected = false;
            }
        }
        let menu_id = self.ctx_menu.as_ref().map_or(0, |state| state.menu_id);
        let snapshot = Rc::new(rows);
        let hover_rows = snapshot.clone();
        let hover_view = view.clone();
        let on_hover = Rc::new(
            move |index: usize, _window: &mut Window, cx: &mut gpui::App| {
                let ordinal = hover_rows
                    .iter()
                    .take(index)
                    .filter(|row| row.selectable())
                    .count();
                hover_view.update(cx, |this, cx| {
                    if let Some(state) = this
                        .ctx_menu
                        .as_mut()
                        .filter(|state| state.menu_id == menu_id)
                    {
                        if state.selected != ordinal {
                            state.selected = ordinal;
                            cx.notify();
                        }
                    }
                });
            },
        );
        let activate_view = view.clone();
        let edit_rows = snapshot.clone();
        let edit_view = view.clone();
        let on_edit_shortcut = Rc::new(
            move |index: usize, _window: &mut Window, cx: &mut gpui::App| {
                edit_view.update(cx, |this, cx| {
                    if this
                        .ctx_menu
                        .as_ref()
                        .is_some_and(|state| state.menu_id == menu_id)
                    {
                        if let Some(CtxRow::Atlas { cmd }) = edit_rows.get(index) {
                            this.process_actions(vec![Action::StartCommandShortcutCapture(
                                cmd.command_id().to_string(),
                            )]);
                            cx.notify();
                        }
                    }
                });
            },
        );
        let on_activate = Rc::new(
            move |index: usize, window: &mut Window, cx: &mut gpui::App| {
                activate_view.update(cx, |this, cx| {
                    if this
                        .ctx_menu
                        .as_ref()
                        .is_some_and(|state| state.menu_id == menu_id)
                    {
                        if let Some(row) = snapshot.get(index) {
                            this.activate_ctx_row(row, window, cx);
                        }
                    }
                });
            },
        );
        let canvas = crate::ui::imm::menu::MenuCanvas::new(
            ink,
            selected_child_idx,
            crate::ui::imm::menu::MenuColors {
                foreground: fg,
                muted,
                border,
                accent,
                accent_foreground: accent_fg,
            },
            reveal_ms,
            on_hover,
            on_activate,
            on_edit_shortcut,
        );

        let Some(menu) = self.ctx_menu.as_mut() else {
            return div().into_any_element();
        };
        let viewport_h = (list_h - zoom).max(0.0);
        menu.scroll_top = menu
            .scroll_top
            .clamp(0.0, (content_h - viewport_h).max(0.0));
        let instant = self.state.settings.disable_animations
            || self.state.settings.scroll_smoothness <= 0.0
            || menu.scroll_drag.is_some();
        let scroll_config = crate::ui::motion::Spring {
            omega: 46.0 - 26.0 * self.state.settings.scroll_smoothness.clamp(0.0, 1.0),
            zeta: 1.0,
        }
        .config();
        let (scroll, scrolling) = menu.scroll_motion.sample_with_config(
            menu.scroll_top,
            instant,
            Instant::now(),
            scroll_config,
        );
        let scroll = scroll.clamp(0.0, (content_h - viewport_h).max(0.0));
        if scrolling {
            window.request_animation_frame();
        }
        let focus = menu.focus.clone();
        let panel = crate::ui::imm::menu_panel::MenuPanel {
            app: view.downgrade(),
            menu_id,
            focus: focus.clone(),
            input: menu.input.paint_snapshot(),
            recording: self.command_shortcut_capture.is_some(),
            scroll,
            content_h,
            height: est_h,
            colors: crate::ui::imm::menu_panel::PanelColors {
                background: bg,
                foreground: fg,
                muted,
                border,
                accent,
                radius: if bg.alpha < 1.0 { 10.0 * zoom } else { radius },
            },
            rows: crate::ui::imm::menu_panel::MenuPanel::rows(canvas),
        };
        // Only generic GPUI boxes remain: anchoring/focus and the fullscreen scrim.
        // All visible components, search editing and scrolling belong to IMM.
        let v = view.clone();
        gpui::deferred(
            div()
                .id("ctx-menu-scrim")
                .absolute()
                .inset_0()
                .size_full()
                .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                    v.update(cx, |this, cx| this.close_ctx_menu(cx));
                })
                .on_mouse_down(MouseButton::Right, {
                    let v = view.clone();
                    move |_e, _w, cx| {
                        v.update(cx, |this, cx| this.close_ctx_menu(cx));
                    }
                })
                .child(
                    div()
                        .absolute()
                        .left(px(left))
                        .top(px(top))
                        .w(px(width))
                        .h(px(est_h))
                        .track_focus(&focus)
                        .opacity(0.92 + 0.08 * crate::ui::motion::menu_row_progress(reveal_ms, 0))
                        .paint_offset(gpui::point(
                            px(0.0),
                            px(4.0
                                * zoom
                                * (1.0 - crate::ui::motion::menu_row_progress(reveal_ms, 0))),
                        ))
                        .child(panel),
                ),
        )
        .priority(2)
        .into_any_element()
    }

    /// Keyboard model for the open menu. Returns `true` when the key was consumed
    /// (so the search input doesn't also see it).
    pub(crate) fn handle_ctx_menu_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.command_shortcut_capture.is_some() {
            return self.handle_command_shortcut_key(ev);
        }
        let ctrl = ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
        let shift = ev.keystroke.modifiers.shift;
        let key = ev.keystroke.key.as_str();
        if !ctrl
            && self
                .ctx_menu
                .as_ref()
                .is_some_and(|menu| menu.input.marked.is_some())
        {
            // Candidate navigation/confirmation belongs to the platform IME.
            return false;
        }
        let editing_arrow = matches!(key, "left" | "right")
            && (ctrl
                || shift
                || self
                    .ctx_menu
                    .as_ref()
                    .is_some_and(|menu| !menu.input.text.is_empty()));
        if editing_arrow
            || matches!(key, "home" | "end" | "backspace" | "delete")
            || (ctrl && matches!(key, "a" | "c" | "v" | "x" | "z" | "y"))
        {
            let Some(menu) = self.ctx_menu.as_mut() else {
                return false;
            };
            let before = menu.input.text.clone();
            match key {
                "c" | "x" if ctrl => {
                    if menu.input.has_selection() {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                            menu.input.selected_text().to_string(),
                        ));
                        if key == "x" {
                            menu.input.delete_selection();
                        }
                    }
                }
                "v" if ctrl => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        menu.input.replace_native(None, &text, None, false);
                    }
                }
                _ => {
                    menu.input.handle_key(key, None, ctrl, shift);
                }
            }
            if before != menu.input.text {
                menu.selected = 0;
                menu.scroll_to_selected = true;
                menu.scroll_top = 0.0;
                menu.scroll_motion = RetainedSpring::new(0.0, Instant::now());
            }
            return true;
        }
        let query = self
            .ctx_menu
            .as_ref()
            .map(|menu| menu.input.text.clone())
            .unwrap_or_default();
        let rows = self.ctx_menu_rows(&query, &query);
        let selectable: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selectable())
            .map(|(i, _)| i)
            .collect();
        let count = selectable.len();

        match ev.keystroke.key.as_str() {
            "escape" => {
                self.close_ctx_menu(cx);
                true
            }
            "down" => {
                if let Some(m) = self.ctx_menu.as_mut() {
                    if count > 0 {
                        m.selected = (m.selected + 1) % count;
                        m.scroll_to_selected = true;
                    }
                }
                true
            }
            "up" => {
                if let Some(m) = self.ctx_menu.as_mut() {
                    if count > 0 {
                        m.selected = (m.selected + count - 1) % count;
                        m.scroll_to_selected = true;
                    }
                }
                true
            }
            "right" => {
                self.ctx_expand_selected(&rows, &selectable, true);
                true
            }
            "left" => {
                self.ctx_expand_selected(&rows, &selectable, false);
                true
            }
            "enter" => {
                self.ctx_activate_selected(&rows, &selectable, window, cx);
                true
            }
            _ => false,
        }
    }

    fn ctx_expand_selected(&mut self, rows: &[CtxRow], selectable: &[usize], expand: bool) {
        let Some(m) = self.ctx_menu.as_mut() else {
            return;
        };
        let Some(&row_i) = selectable.get(m.selected) else {
            return;
        };
        if let CtxRow::Native { id, submenu, .. } = &rows[row_i] {
            if *submenu {
                if expand {
                    m.expanded.insert(*id);
                    // Lazy children: ask the worker if not loaded.
                    let need = submenu_children_missing(&m.native, *id);
                    #[cfg(windows)]
                    if need {
                        if let Some(handle) = &self.shell_menu {
                            handle.send(crate::shell_menu::ShellMenuRequest::ExpandSubmenu {
                                id: m.menu_id,
                                item_id: *id,
                            });
                        }
                    }
                    #[cfg(not(windows))]
                    let _ = need;
                } else {
                    m.expanded.remove(id);
                }
            }
        }
    }

    fn ctx_activate_selected(
        &mut self,
        rows: &[CtxRow],
        selectable: &[usize],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = self.ctx_menu.as_ref().map(|m| m.selected).unwrap_or(0);
        let Some(&row_i) = selectable.get(selected) else {
            return;
        };
        self.activate_ctx_row(&rows[row_i], window, cx);
    }

    fn activate_ctx_row(&mut self, row: &CtxRow, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_command_shortcut_capture();
        match row {
            CtxRow::Atlas { cmd } => self.run_ctx_atlas(*cmd, window, cx),
            CtxRow::Ext { ext_id, cmd_id, .. } => {
                let ext_id = ext_id.clone();
                let cmd_id = cmd_id.clone();
                self.run_ctx_ext(&ext_id, &cmd_id, cx);
            }
            CtxRow::Native { id, submenu, .. } => {
                self.activate_ctx_native(*id, *submenu, window, cx)
            }
            CtxRow::DesktopApp { app } => {
                let app = app.clone();
                self.run_ctx_desktop_app(&app, cx);
            }
            _ => {}
        }
    }

    pub(crate) fn run_ctx_desktop_app(
        &mut self,
        app: &atlas_core::fs::desktop_apps::DesktopApp,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = self.ctx_menu.take() {
            let _ = atlas_core::fs::desktop_apps::launch_desktop_app(app, &state.paths);
        }

        cx.notify();
    }

    fn run_ctx_ext(&mut self, ext_id: &str, cmd_id: &str, cx: &mut Context<Self>) {
        self.close_ctx_menu(cx);
        self.handle_ext_action(crate::app::ext::ExtAction::Run {
            ext_id: ext_id.to_string(),
            cmd_id: cmd_id.to_string(),
        });
        cx.notify();
    }

    /// Run an Atlas command against the current selection, then close. We call the
    /// App methods directly (rather than dispatching the gpui action) so the menu's
    /// focused search input can't intercept `Copy`/`Cut`/`Paste` as text edits.
    fn run_ctx_atlas(&mut self, cmd: AtlasCmd, _window: &mut Window, cx: &mut Context<Self>) {
        let is_archive = self
            .state
            .active_tab_data()
            .is_some_and(|t| t.location.is_archive());
        let menu_paths = self
            .ctx_menu
            .as_ref()
            .filter(|m| !m.background)
            .map(|m| m.paths.clone())
            .unwrap_or_default();
        self.close_ctx_menu(cx);
        match cmd {
            AtlasCmd::Open => self.open_cursor(),
            AtlasCmd::Rename => self.process_actions(vec![Action::RenameFocused]),
            AtlasCmd::Copy => {
                if is_archive {
                    self.toast_error(rust_i18n::t!("root_copy_archive_error").into());
                } else {
                    let paths = if menu_paths.is_empty() {
                        self.selected_paths()
                    } else {
                        menu_paths.clone()
                    };
                    if !paths.is_empty() {
                        self.process_actions(vec![Action::Copy(paths)]);
                    }
                }
            }
            AtlasCmd::Cut => {
                if is_archive {
                    self.toast_error(rust_i18n::t!("root_cut_archive_error").into());
                } else {
                    let paths = if menu_paths.is_empty() {
                        self.selected_paths()
                    } else {
                        menu_paths.clone()
                    };
                    if !paths.is_empty() {
                        self.process_actions(vec![Action::Cut(paths)]);
                    }
                }
            }
            AtlasCmd::Paste => {
                if let Some(dest) = self
                    .state
                    .active_tab_data()
                    .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
                {
                    self.process_actions(vec![Action::Paste { dest }]);
                }
            }
            #[cfg(windows)]
            AtlasCmd::PasteShortcut => self.paste_shortcuts_focused(),
            AtlasCmd::Trash | AtlasCmd::DeletePerm => {
                if is_archive {
                    self.toast_error(rust_i18n::t!("root_delete_archive_error").into());
                } else {
                    let paths = if menu_paths.is_empty() {
                        self.selected_paths()
                    } else {
                        menu_paths.clone()
                    };
                    if !paths.is_empty() {
                        self.process_actions(vec![Action::Delete {
                            paths,
                            to_trash: cmd == AtlasCmd::Trash,
                        }]);
                    }
                }
            }
            AtlasCmd::Restore => {
                self.restore_selected_from_recycle_bin();
            }
            AtlasCmd::Properties => {
                let paths = if menu_paths.is_empty() {
                    self.selected_paths()
                } else {
                    menu_paths.clone()
                };
                #[cfg(windows)]
                match paths.as_slice() {
                    [] => {}
                    [single] => show_properties(single),
                    many => show_properties_multi(many),
                }
                // No native "properties" verb outside Windows — use our own
                // themed overlay instead (F3 #15).
                #[cfg(not(windows))]
                self.open_properties(paths);
            }
            AtlasCmd::SelectAll => {
                self.process_actions(vec![Action::SelectAll]);
            }
            AtlasCmd::NewFolder => {
                self.process_actions(vec![Action::CreateFolderFocused]);
            }
            AtlasCmd::NewFile => {
                self.process_actions(vec![Action::CreateFileFocused]);
            }
            AtlasCmd::OpenTerminal => {
                if let Some(dir) = self
                    .state
                    .active_tab_data()
                    .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
                {
                    self.process_actions(vec![Action::OpenTerminal { dir }]);
                }
            }
            AtlasCmd::Refresh => {
                self.process_actions(vec![Action::Refresh(self.state.focused_tile)]);
            }
        }
        cx.notify();
    }

    /// Open the background right-click menu anchored at `anchor` (no file selection).
    #[allow(dead_code)]
    pub(crate) fn open_background_ctx_menu(
        &mut self,
        current_dir: Option<PathBuf>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.suppress_next_shell_ctx_menu {
            self.suppress_next_shell_ctx_menu = false;
            self.close_ctx_menu(cx);
            return;
        }
        self.close_type_filter_menu(cx);
        self.close_breadcrumb_menu(cx);
        self.ctx_menu_seq += 1;
        let menu_id = self.ctx_menu_seq;
        let screen_pt = cursor_screen_pos();

        // Request native shell items for the current directory (background menu).

        #[cfg(windows)]
        let loading = {
            if let Some(ref dir) = current_dir {
                if let Some(handle) = &self.shell_menu {
                    handle.send(crate::shell_menu::ShellMenuRequest::Build {
                        id: menu_id,
                        paths: vec![dir.clone()],
                    });
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        #[cfg(not(windows))]
        let loading = {
            let _ = &current_dir;
            false
        };

        self.ctx_menu = Some(CtxMenuState {
            menu_id,
            focus: cx.focus_handle(),
            input: ImmTextInputState::new(""),
            scroll_top: 0.0,
            scroll_motion: RetainedSpring::new(0.0, Instant::now()),
            input_bounds: None,
            input_line: None,
            input_scroll_x: 0.0,
            input_drag: false,
            scroll_drag: None,
            blink_visible: true,
            opened_at: Instant::now(),
            anchor,
            screen_pt,
            paths: current_dir.into_iter().collect(),
            native: Vec::new(),
            loading,
            selected: 0,
            scroll_to_selected: true,
            expanded: HashSet::new(),
            background: true,
            ext_cmds: vec![],
            desktop_apps: Vec::new(),
        });
        cx.notify();
    }

    /// Native row: a submenu toggles inline expansion; a leaf invokes the verb.
    fn activate_ctx_native(
        &mut self,
        id: u32,
        submenu: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if submenu {
            let expand = self
                .ctx_menu
                .as_ref()
                .map(|m| !m.expanded.contains(&id))
                .unwrap_or(false);
            if let Some(m) = self.ctx_menu.as_mut() {
                if expand {
                    m.expanded.insert(id);
                } else {
                    m.expanded.remove(&id);
                }
            }
            #[cfg(windows)]
            if expand {
                let (need, menu_id) = self
                    .ctx_menu
                    .as_ref()
                    .map(|m| (submenu_children_missing(&m.native, id), m.menu_id))
                    .unwrap_or((false, 0));
                if need {
                    if let Some(handle) = &self.shell_menu {
                        handle.send(crate::shell_menu::ShellMenuRequest::ExpandSubmenu {
                            id: menu_id,
                            item_id: id,
                        });
                    }
                }
            }
            cx.notify();
            return;
        }
        // Leaf: invoke through the worker, close the overlay immediately.
        #[cfg(windows)]
        if let Some(m) = self.ctx_menu.as_ref() {
            if let Some(handle) = &self.shell_menu {
                handle.send(crate::shell_menu::ShellMenuRequest::Invoke {
                    id: m.menu_id,
                    item_id: id,
                    screen_pt: m.screen_pt,
                });
            }
        }
        #[cfg(not(windows))]
        let _ = id;
        // Visually close now; the worker frees its COM objects on InvokeDone.
        self.ctx_menu = None;

        cx.notify();
    }
}

/// Remove leading, trailing, and consecutive `Separator` rows so no orphaned
/// dividers appear when a context-menu section is empty.
fn sanitize_separators(rows: Vec<CtxRow>) -> Vec<CtxRow> {
    let mut out: Vec<CtxRow> = Vec::with_capacity(rows.len());
    for row in rows {
        let is_sep = matches!(row, CtxRow::Separator);
        if is_sep && matches!(out.last(), None | Some(CtxRow::Separator)) {
            continue;
        }
        out.push(row);
    }
    // Strip trailing separator.
    if matches!(out.last(), Some(CtxRow::Separator)) {
        out.pop();
    }
    out
}

fn ctx_row_height(row: &CtxRow) -> f32 {
    match row {
        CtxRow::Separator => 9.0,
        CtxRow::Loading
        | CtxRow::Atlas { .. }
        | CtxRow::Ext { .. }
        | CtxRow::Native { .. }
        | CtxRow::DesktopApp { .. } => 30.0,
    }
}

/// Open the native shell Properties dialog for `path`.
/// On Windows uses `ShellExecuteExW` with the "properties" verb and
/// `SEE_MASK_INVOKEIDLIST` so the dialog works for any file system object.
/// No-op on non-Windows.
#[allow(unsafe_code)]
pub(crate) fn show_properties(path: &std::path::Path) {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::Shell::{
            ShellExecuteExW, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW,
        };
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;

        let verb: Vec<u16> = "properties\0".encode_utf16().collect();
        let file: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_INVOKEIDLIST,
            hwnd: HWND::default(),
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            nShow: SW_SHOW.0,
            ..Default::default()
        };
        unsafe {
            let _ = ShellExecuteExW(&mut info);
        }
    }
    #[cfg(not(windows))]
    let _ = path;
}

/// Open the shell Properties dialog for "This PC".
#[allow(unsafe_code)]
pub(crate) fn show_this_pc_properties() {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::Shell::{
            ShellExecuteExW, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW,
        };
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;

        let verb: Vec<u16> = "properties\0".encode_utf16().collect();
        let this_pc: Vec<u16> = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}\0"
            .encode_utf16()
            .collect();
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_INVOKEIDLIST,
            hwnd: HWND::default(),
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(this_pc.as_ptr()),
            nShow: SW_SHOW.0,
            ..Default::default()
        };
        unsafe {
            let _ = ShellExecuteExW(&mut info);
        }
    }
}

/// Open the combined native Properties dialog for several paths at once
/// (`SHMultiFileProperties`, the same sheet Explorer shows for a multi-select).
/// Falls back to one dialog per path if the data object can't be built (e.g.
/// items don't share a common shell parent).
#[cfg(windows)]
#[allow(unsafe_code)]
fn show_properties_multi(paths: &[PathBuf]) {
    use crate::dnd::windows::data_object::AtlasDataObject;
    use std::sync::atomic::AtomicU32;
    use windows::Win32::System::Ole::DROPEFFECT_COPY;
    use windows::Win32::UI::Shell::SHMultiFileProperties;

    let data_obj = AtlasDataObject::new(paths, DROPEFFECT_COPY, Arc::new(AtomicU32::new(0)));
    let ok = data_obj.is_some_and(|obj| unsafe { SHMultiFileProperties(&obj, 0).is_ok() });
    if !ok {
        for path in paths {
            show_properties(path);
        }
    }
}

/// Rank `titles` against `query` (best first). Standalone so it doesn't need the
/// `App` borrow held by `ctx_menu_rows`.
fn rank_titles(query: &str, titles: &[String]) -> Vec<usize> {
    let mut scorer = atlas_core::search::fuzzy::FuzzyScorer::new();
    scorer.rank(query, titles.iter().map(|s| s.as_str()))
}

/// Append the native tree (depth-aware, expansion-aware) as logical rows.
fn push_native_tree(
    items: &[UiNativeItem],
    depth: u8,
    expanded: &HashSet<u32>,
    out: &mut Vec<CtxRow>,
) {
    for it in items {
        match &it.kind {
            UiItemKind::Separator => out.push(CtxRow::Separator),
            UiItemKind::Normal => out.push(CtxRow::Native {
                id: it.id,
                title: it.title.clone(),
                icon: it.icon.clone(),
                depth,
                submenu: false,
                expanded: false,
                disabled: it.disabled,
                is_default: it.is_default,
            }),
            UiItemKind::Submenu { children } => {
                let is_exp = expanded.contains(&it.id);
                out.push(CtxRow::Native {
                    id: it.id,
                    title: it.title.clone(),
                    icon: it.icon.clone(),
                    depth,
                    submenu: true,
                    expanded: is_exp,
                    disabled: it.disabled,
                    is_default: it.is_default,
                });
                if is_exp {
                    if let Some(ch) = children {
                        push_native_tree(ch, depth + 1, expanded, out);
                    }
                }
            }
        }
    }
}

/// Collect leaf candidates for search mode, prefixing nested titles ("A › B").
fn collect_native_leaves(
    items: &[UiNativeItem],
    prefix: &str,
    titles: &mut Vec<String>,
    cands: &mut Vec<CtxRow>,
) {
    for it in items {
        match &it.kind {
            UiItemKind::Separator => {}
            UiItemKind::Normal => {
                let full = if prefix.is_empty() {
                    it.title.clone()
                } else {
                    format!("{prefix}{}", it.title)
                };
                titles.push(full.clone());
                cands.push(CtxRow::Native {
                    id: it.id,
                    title: full,
                    icon: it.icon.clone(),
                    depth: 0,
                    submenu: false,
                    expanded: false,
                    disabled: it.disabled,
                    is_default: it.is_default,
                });
            }
            UiItemKind::Submenu { children } => {
                if let Some(ch) = children {
                    let next = format!("{prefix}{} › ", it.title);
                    collect_native_leaves(ch, &next, titles, cands);
                }
            }
        }
    }
}

/// True when submenu `id` exists but its children are not yet loaded.
fn submenu_children_missing(items: &[UiNativeItem], id: u32) -> bool {
    for it in items {
        if let UiItemKind::Submenu { children } = &it.kind {
            if it.id == id {
                return children.is_none();
            }
            if let Some(ch) = children {
                if submenu_children_missing(ch, id) {
                    return true;
                }
            }
        }
    }
    false
}

/// Place freshly-expanded children under submenu `id` (depth ≥ 2 lazy path).
/// `slot` is taken once the target is found, so recursion stops cleanly.
fn fill_submenu(items: &mut [UiNativeItem], id: u32, slot: &mut Option<Vec<UiNativeItem>>) {
    for it in items.iter_mut() {
        if let UiItemKind::Submenu { children } = &mut it.kind {
            if it.id == id {
                *children = slot.take();
                return;
            }
            if let Some(ch) = children {
                fill_submenu(ch, id, slot);
                if slot.is_none() {
                    return;
                }
            }
        }
    }
}

/// Capture the cursor position in physical screen pixels (for `ptInvoke`).
#[cfg(windows)]
#[allow(unsafe_code)]
fn cursor_screen_pos() -> (i32, i32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut pt = POINT { x: 0, y: 0 };
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    (pt.x, pt.y)
}

#[cfg(not(windows))]
fn cursor_screen_pos() -> (i32, i32) {
    (0, 0)
}

/// Convert worker `NativeMenuItem`s (with `RawIcon`) to UI items whose icons are
/// GPU textures. Recurses into eagerly-loaded submenu children.
#[cfg(windows)]
fn native_items_to_ui(
    items: Vec<crate::shell_menu::NativeMenuItem>,
    cx: &mut Context<App>,
) -> Vec<UiNativeItem> {
    use crate::shell_menu::NativeItemKind;
    let _ = cx;
    items
        .into_iter()
        .map(|it| {
            let icon = it.icon.and_then(raw_icon_to_texture);
            let kind = match it.kind {
                NativeItemKind::Normal => UiItemKind::Normal,
                NativeItemKind::Separator => UiItemKind::Separator,
                NativeItemKind::Submenu { children } => UiItemKind::Submenu {
                    children: children.map(|ch| native_items_to_ui(ch, cx)),
                },
            };
            UiNativeItem {
                id: it.id,
                title: it.title,
                icon,
                kind,
                disabled: it.disabled,
                is_default: it.is_default,
            }
        })
        .collect()
}

/// `RawIcon` (straight RGBA) → `RenderImage` (gpui expects BGRA, so swap R↔B).
#[cfg(windows)]
fn raw_icon_to_texture(raw: crate::shell_menu::RawIcon) -> Option<Arc<RenderImage>> {
    let mut bytes = raw.rgba;
    for px in bytes.chunks_exact_mut(4) {
        px.swap(0, 2); // RGBA → BGRA
    }
    let buf = image::RgbaImage::from_raw(raw.w, raw.h, bytes)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_atlas_menu_command_uses_the_palette_binding_registry() {
        for items in [ATLAS_ITEMS, ATLAS_ITEMS_BACKGROUND, ATLAS_ITEMS_RECYCLE] {
            for (cmd, _) in items {
                assert!(
                    crate::input::commands::command_remappable(cmd.command_id()),
                    "{}",
                    cmd.command_id()
                );
            }
        }
    }
}
