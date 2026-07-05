//! Atlas–style right-click menu: a searchable overlay combining Atlas's own
//! commands (top) with the native Windows shell context menu (Open with, Send to,
//! 7-Zip, Properties, …) enumerated by the [`crate::shell_menu`] worker.
//!
//! The Atlas section is cross-platform; native items only appear on Windows.
//! Atlas rows dispatch the same boxed gpui actions the old `PopupMenu` used, so
//! all existing wiring is reused. Native rows are invoked through the worker.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use atlas_model::ext_model::CommandRef;

use gpui::{
    div, img, px, AnyElement, Context, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, Keystroke, MouseButton, ParentElement, Pixels, Point, RenderImage, SharedString,
    StatefulInteractiveElement, Styled, Window,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, kbd::Kbd, v_flex, ActiveTheme};

use crate::app::App;
use crate::input::action::Action;
use crate::ui::icons::AtlasIcon;

const SCROLLBAR_GUTTER: f32 = 16.0;

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
    Trash,
    DeletePerm,
    Restore,
    Properties,
    NewFolder,
    NewFile,
    OpenTerminal,
    SelectAll,
}

impl AtlasCmd {
    fn label(self) -> String {
        match self {
            Self::Open => rust_i18n::t!("root_open").to_string(),
            Self::Rename => rust_i18n::t!("root_rename").to_string(),
            Self::Copy => rust_i18n::t!("root_copy").to_string(),
            Self::NewFolder => rust_i18n::t!("root_new_folder").to_string(),
            Self::NewFile => "New file".to_string(),
            Self::OpenTerminal => "Open terminal".to_string(),
            Self::SelectAll => rust_i18n::t!("root_select_all").to_string(),
            Self::Cut => rust_i18n::t!("root_cut").to_string(),
            Self::Paste => rust_i18n::t!("root_paste").to_string(),
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
            Self::Trash => Some(AtlasIcon::Trash),
            Self::DeletePerm => Some(AtlasIcon::X),
            Self::Restore => Some(AtlasIcon::ArrowCounterClockwise),
            Self::Properties => Some(AtlasIcon::Gear),
            Self::NewFolder => Some(AtlasIcon::Folder),
            Self::NewFile => Some(AtlasIcon::File),
            Self::OpenTerminal => Some(AtlasIcon::Lightning),
            Self::SelectAll => Some(AtlasIcon::SelectionAll),
        }
    }

    fn shortcut(self) -> Option<&'static str> {
        match self {
            Self::Open => Some("enter"),
            Self::Rename => Some("f2"),
            Self::Copy => Some("ctrl-c"),
            Self::Cut => Some("ctrl-x"),
            Self::Paste => Some("ctrl-v"),
            Self::Trash => Some("delete"),
            Self::DeletePerm => Some("shift-delete"),
            Self::SelectAll => Some("ctrl-a"),
            Self::Restore
            | Self::Properties
            | Self::NewFolder
            | Self::NewFile
            | Self::OpenTerminal => None,
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
    (AtlasCmd::SelectAll, false),
];

/// A logical (non-element) row, recomputed each frame and shared by the renderer
/// and the keyboard handler so selection indices always agree.
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
}

impl CtxRow {
    fn selectable(&self) -> bool {
        matches!(
            self,
            CtxRow::Atlas { .. } | CtxRow::Ext { .. } | CtxRow::Native { .. }
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
        if paths.is_empty() {
            return;
        }
        self.ctx_menu_seq += 1;
        let menu_id = self.ctx_menu_seq;

        // Physical screen pixels for the eventual native invoke.
        let screen_pt = cursor_screen_pos();

        let input = crate::ui::text_input::single_line(
            window,
            cx,
            rust_i18n::t!("search_actions").into_owned(),
            "",
        );
        // The menu is painted inline by App; typed characters change the InputState
        // but won't repaint unless we notify. Mirror `PaletteHost`'s subscription.
        self.ctx_menu_sub = Some(cx.subscribe(
            &input,
            |_this, _input, ev: &gpui_component::input::InputEvent, cx| {
                if matches!(ev, gpui_component::input::InputEvent::Change) {
                    cx.notify();
                }
            },
        ));
        self.ctx_menu_input = Some(input);

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
        let ext_cmds = self.ext_registry.commands_for(&selection);

        self.ctx_menu = Some(CtxMenuState {
            menu_id,
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
        });
        cx.notify();
    }

    /// Close the menu, dismissing the worker's COM objects (Windows).
    pub(crate) fn close_ctx_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.ctx_menu.take() {
            #[cfg(windows)]
            if let Some(handle) = &self.shell_menu {
                handle.send(crate::shell_menu::ShellMenuRequest::Dismiss { id: state.menu_id });
            }
            let _ = state;
        }
        self.ctx_menu_input = None;
        self.ctx_menu_sub = None;
        cx.notify();
    }

    /// Drain shell-worker events into the open menu. Returns whether anything
    /// changed (so the 60 Hz loop repaints). Windows-only.
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
                        self.ctx_menu_input = None;
                        self.ctx_menu_sub = None;
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
        self.ctx_menu_input
            .as_ref()
            .map(|i| i.read(cx).focus_handle(cx))
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
            // Search mode: flatten every leaf (Atlas + native + ext), rank, emit flat.
            let _ = scorer_query;
            let mut cand_titles: Vec<String> = Vec::new();
            let mut cands: Vec<CtxRow> = Vec::new();

            for (cmd, _) in atlas_items {
                cand_titles.push(cmd.label().to_string());
                cands.push(CtxRow::Atlas { cmd: *cmd });
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
        let (popover, fg, muted, border, accent, accent_fg) = {
            let t = cx.theme();
            (
                t.popover,
                t.popover_foreground,
                t.muted_foreground,
                t.border,
                t.accent,
                t.accent_foreground,
            )
        };
        let (smooth_ease, smooth_boost) =
            crate::app::scroll_smooth_params(self.state.settings.scroll_smoothness);
        self.ctx_menu_scroll.set_smooth(smooth_ease, smooth_boost);
        let input = self.ctx_menu_input.clone();
        let query = input
            .as_ref()
            .map(|i| i.read(cx).value().to_string())
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
        let (anchor, selected) = state
            .map(|s| (s.anchor, s.selected))
            .unwrap_or((Point::default(), 0));

        // Clamp the panel into the window so it never spills off an edge.
        let win = window.bounds().size;
        const W: f32 = 340.;
        let (ax, ay) = (f32::from(anchor.x), f32::from(anchor.y));
        let (ww, wh) = (f32::from(win.width), f32::from(win.height));
        const MAX_H: f32 = 640.;
        const V_MARGIN: f32 = 72.;
        const INPUT_H: f32 = 40.;
        let content_h = rows.len() as f32 * 30.0 + 8.0;
        let max_h = MAX_H.min((wh - V_MARGIN * 2.0).max(180.0));
        let est_h = (content_h + INPUT_H).clamp(120.0, max_h);
        let list_h = (est_h - INPUT_H).max(72.0);
        let left = ax.min((ww - W - 6.0).max(6.0)).max(6.0);
        let top = ay.min((wh - est_h - 6.0).max(6.0)).max(6.0);

        let view = cx.entity();

        // Map selectable index → element, tracking which selectable index each
        // selectable row occupies so we can highlight `selected`.
        let mut sel_idx = 0usize;
        let mut selected_child_idx = None;
        let mut children: Vec<AnyElement> = Vec::new();
        for row in &rows {
            match row {
                CtxRow::Separator => {
                    children.push(
                        div()
                            .my(px(4.))
                            .h(px(1.))
                            .w_full()
                            .bg(border)
                            .into_any_element(),
                    );
                }
                CtxRow::Loading => {
                    children.push(
                        h_flex()
                            .px_2()
                            .py_1()
                            .gap_2()
                            .text_color(muted)
                            .text_sm()
                            .child(div().child(rust_i18n::t!("loading_actions")))
                            .into_any_element(),
                    );
                }
                CtxRow::Atlas { cmd } => {
                    let this_idx = sel_idx;
                    sel_idx += 1;
                    let is_sel = this_idx == selected;
                    if is_sel {
                        selected_child_idx = Some(children.len());
                    }
                    let cmd = *cmd;
                    let v = view.clone();
                    let icon_el = cmd.icon().map(|i| {
                        crate::ui::icons::icon(i, 16., if is_sel { accent_fg } else { fg })
                    });
                    children.push(
                        menu_row(
                            SharedString::from(format!("ctx-atlas-{this_idx}")),
                            icon_el,
                            cmd.label().to_string(),
                            cmd.shortcut(),
                            false,
                            false,
                            false,
                            is_sel,
                            accent,
                            accent_fg,
                            fg,
                            muted,
                        )
                        .on_mouse_move(ctx_hover_handler(&view, this_idx))
                        .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                            v.update(cx, |this, cx| {
                                this.run_ctx_atlas(cmd, window, cx);
                            });
                        })
                        .into_any_element(),
                    );
                }
                CtxRow::Ext {
                    ext_id,
                    cmd_id,
                    title,
                } => {
                    let this_idx = sel_idx;
                    sel_idx += 1;
                    let is_sel = this_idx == selected;
                    if is_sel {
                        selected_child_idx = Some(children.len());
                    }
                    let ext_id = ext_id.clone();
                    let cmd_id = cmd_id.clone();
                    let title = title.clone();
                    let v = view.clone();
                    let icon_el = Some(crate::ui::icons::icon(
                        AtlasIcon::PuzzlePiece,
                        16.,
                        if is_sel { accent_fg } else { fg },
                    ));
                    children.push(
                        menu_row(
                            SharedString::from(format!("ctx-ext-{this_idx}")),
                            icon_el,
                            title,
                            None,
                            false,
                            false,
                            false,
                            is_sel,
                            accent,
                            accent_fg,
                            fg,
                            muted,
                        )
                        .on_mouse_move(ctx_hover_handler(&view, this_idx))
                        .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
                            v.update(cx, |this, cx| {
                                this.run_ctx_ext(&ext_id, &cmd_id, cx);
                            });
                        })
                        .into_any_element(),
                    );
                }
                CtxRow::Native {
                    id,
                    title,
                    icon,
                    depth,
                    submenu,
                    expanded,
                    disabled,
                    is_default,
                } => {
                    let this_idx = sel_idx;
                    sel_idx += 1;
                    let is_sel = this_idx == selected;
                    if is_sel {
                        selected_child_idx = Some(children.len());
                    }
                    let id = *id;
                    let submenu = *submenu;
                    let disabled = *disabled;
                    let v = view.clone();
                    let icon_el = icon.clone().map(|tex| {
                        img(tex)
                            .w(px(16.))
                            .h(px(16.))
                            .flex_none()
                            .into_any_element()
                    });
                    let mut r = menu_row(
                        SharedString::from(format!("ctx-native-{id}-{this_idx}")),
                        icon_el,
                        title.clone(),
                        None,
                        submenu,
                        *expanded,
                        *is_default,
                        is_sel,
                        accent,
                        accent_fg,
                        if disabled { muted } else { fg },
                        muted,
                    )
                    .pl(px(8.0 + f32::from(*depth) * 14.0))
                    .on_mouse_move(ctx_hover_handler(&view, this_idx));
                    if !disabled {
                        r = r.on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                            v.update(cx, |this, cx| {
                                this.activate_ctx_native(id, submenu, window, cx);
                            });
                        });
                    }
                    children.push(r.into_any_element());
                }
            }
        }

        if self
            .ctx_menu
            .as_ref()
            .is_some_and(|state| state.scroll_to_selected)
        {
            if let Some(idx) = selected_child_idx {
                self.ctx_menu_scroll.scroll_to_item(idx);
            }
            if let Some(state) = self.ctx_menu.as_mut() {
                state.scroll_to_selected = false;
            }
        }

        let input_row = input.as_ref().map(|state| {
            h_flex()
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .border_b_1()
                .border_color(border)
                .items_center()
                .child(crate::ui::icons::icon(AtlasIcon::Search, 14., muted))
                .child(div().flex_1().child(crate::ui::text_input::flat(state)))
        });

        let panel = v_flex()
            .id("ctx-menu-panel")
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(W))
            .h(px(est_h))
            .bg(popover)
            .text_color(fg)
            .border_1()
            .border_color(border)
            .rounded_lg()
            .shadow_lg()
            .occlude()
            .overflow_hidden()
            .children(input_row)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .max_h(px(list_h))
                    .child(
                        v_flex()
                            .id("ctx-menu-list")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.ctx_menu_scroll)
                            .py_1()
                            .pr(px(SCROLLBAR_GUTTER))
                            .children(children),
                    )
                    .vertical_scrollbar(&self.ctx_menu_scroll),
            )
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if this.handle_ctx_menu_key(ev, window, cx) {
                    cx.stop_propagation();
                    cx.notify();
                }
            }));

        // Full-window scrim: any click outside the panel closes the menu.
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
                .child(panel),
        )
        .with_priority(2)
        .into_any_element()
    }

    /// Keyboard model for the open menu. Returns `true` when the key was consumed
    /// (so the search input doesn't also see it).
    fn handle_ctx_menu_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let query = self
            .ctx_menu_input
            .as_ref()
            .map(|i| i.read(cx).value().to_string())
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
        match &rows[row_i] {
            CtxRow::Atlas { cmd } => self.run_ctx_atlas(*cmd, window, cx),
            CtxRow::Ext { ext_id, cmd_id, .. } => {
                let ext_id = ext_id.clone();
                let cmd_id = cmd_id.clone();
                self.run_ctx_ext(&ext_id, &cmd_id, cx);
            }
            CtxRow::Native { id, submenu, .. } => {
                self.activate_ctx_native(*id, *submenu, window, cx)
            }
            _ => {}
        }
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
    fn run_ctx_atlas(&mut self, cmd: AtlasCmd, window: &mut Window, cx: &mut Context<Self>) {
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
            AtlasCmd::Rename => self.begin_rename(window, cx),
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
                if let Some(path) = paths.into_iter().next() {
                    show_properties(&path);
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
                if let Some(dir) = self
                    .state
                    .active_tab_data()
                    .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
                {
                    let tile = self.state.focused_tile;
                    let base = rust_i18n::t!("root_new_folder").to_string();
                    let name = unique_folder_name(&dir, &base);
                    let path = dir.join(&name);
                    self.pending_select = Some((tile, path));
                    self.process_actions(vec![Action::CreateFolder { parent: dir, name }]);
                }
            }
            AtlasCmd::NewFile => {
                if let Some(dir) = self
                    .state
                    .active_tab_data()
                    .and_then(|t| t.location.fs_dir().map(|p| p.to_path_buf()))
                {
                    let tile = self.state.focused_tile;
                    let base = "New file".to_string();
                    let name = unique_folder_name(&dir, &base); // Just use this for uniqueness
                    let path = dir.join(&name);
                    self.pending_select = Some((tile, path));
                    self.process_actions(vec![Action::CreateFile { parent: dir, name }]);
                }
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
        self.ctx_menu_seq += 1;
        let menu_id = self.ctx_menu_seq;
        let screen_pt = cursor_screen_pos();
        let input = crate::ui::text_input::single_line(
            window,
            cx,
            rust_i18n::t!("search_actions").into_owned(),
            "",
        );
        self.ctx_menu_sub = Some(cx.subscribe(
            &input,
            |_this, _input, ev: &gpui_component::input::InputEvent, cx| {
                if matches!(ev, gpui_component::input::InputEvent::Change) {
                    cx.notify();
                }
            },
        ));
        self.ctx_menu_input = Some(input);

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
        self.ctx_menu_input = None;
        self.ctx_menu_sub = None;
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

/// Open the native shell Properties dialog for `path`.
/// On Windows uses `ShellExecuteExW` with the "properties" verb and
/// `SEE_MASK_INVOKEIDLIST` so the dialog works for any file system object.
/// No-op on non-Windows.
#[allow(unsafe_code)]
fn show_properties(path: &std::path::Path) {
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

/// One styled menu row (icon slot, label, optional chevron / default emphasis).
#[allow(clippy::too_many_arguments)]
/// Hover-to-select: moving the mouse over a selectable row makes it the current
/// selection (mirrors keyboard nav). Only notifies on an actual change so a moving
/// cursor doesn't thrash the renderer.
fn ctx_hover_handler(
    view: &gpui::Entity<App>,
    idx: usize,
) -> impl Fn(&gpui::MouseMoveEvent, &mut Window, &mut gpui::App) + 'static {
    let view = view.clone();
    move |_e, _window, cx| {
        view.update(cx, |this, cx| {
            if let Some(m) = this.ctx_menu.as_mut() {
                if m.selected != idx {
                    m.selected = idx;
                    cx.notify();
                }
            }
        });
    }
}

fn menu_row(
    id: SharedString,
    icon: Option<AnyElement>,
    label: String,
    shortcut: Option<&'static str>,
    submenu: bool,
    expanded: bool,
    is_default: bool,
    selected: bool,
    accent: gpui::Hsla,
    accent_fg: gpui::Hsla,
    fg: gpui::Hsla,
    muted: gpui::Hsla,
) -> gpui::Stateful<gpui::Div> {
    let text_color = if selected { accent_fg } else { fg };
    let mut row = h_flex()
        .id(id)
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .rounded_md()
        .cursor_pointer()
        .text_color(text_color)
        .text_sm();
    if selected {
        row = row.bg(accent);
    }
    // Icon slot (fixed width so labels align with and without icons).
    row = row.child(div().w(px(16.)).h(px(16.)).flex_none().children(icon));
    let mut label_div = div().flex_1().min_w(px(0.)).truncate().child(label);
    if is_default {
        label_div = label_div.font_weight(gpui::FontWeight::SEMIBOLD);
    }
    row = row.child(label_div);
    if let Some(shortcut) = shortcut {
        let mut badge = Kbd::new(Keystroke::parse(shortcut).unwrap())
            .outline()
            .flex_shrink_0();
        if selected {
            badge = badge
                .bg(gpui::transparent_black())
                .border_1()
                .border_color(accent_fg.opacity(0.4))
                .text_color(accent_fg);
        } else {
            badge = badge.text_color(muted);
        }
        row = row.child(badge);
    }
    if submenu {
        row = row.child(crate::ui::icons::icon(
            if expanded {
                AtlasIcon::CaretDown
            } else {
                AtlasIcon::CaretRight
            },
            12.,
            text_color,
        ));
    }
    row
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

/// Return a folder name that doesn't already exist under `dir`.
/// Tries `base`, then `base 2`, `base 3`, …
fn unique_folder_name(dir: &std::path::Path, base: &str) -> String {
    if !dir.join(base).exists() {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let name = format!("{base} {n}");
        if !dir.join(&name).exists() {
            return name;
        }
        n += 1;
    }
}
