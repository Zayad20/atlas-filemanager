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

use gpui::{
    div, img, px, AnyElement, Context, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Pixels, Point, RenderImage, SharedString, Styled,
    Window,
};
use gpui_component::{h_flex, v_flex, ActiveTheme};

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
    /// Inline-expanded submenu ids (no query / tree mode).
    pub expanded: HashSet<u32>,
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
}

impl AtlasCmd {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "Abrir",
            Self::Rename => "Renombrar",
            Self::Copy => "Copiar",
            Self::Cut => "Cortar",
            Self::Paste => "Pegar",
            Self::Trash => "Mover a la papelera",
            Self::DeletePerm => "Eliminar permanentemente",
        }
    }
    fn icon(self) -> Option<AtlasIcon> {
        match self {
            Self::Open => Some(AtlasIcon::Eye),
            Self::Rename => Some(AtlasIcon::PencilSimple),
            Self::Copy => Some(AtlasIcon::Copy),
            Self::Trash => Some(AtlasIcon::Trash),
            Self::DeletePerm => Some(AtlasIcon::X),
            Self::Cut | Self::Paste => None,
        }
    }
}

/// The Atlas section layout, in order. `true` = a separator follows the item.
const ATLAS_ITEMS: &[(AtlasCmd, bool)] = &[
    (AtlasCmd::Open, false),
    (AtlasCmd::Rename, true),
    (AtlasCmd::Copy, false),
    (AtlasCmd::Cut, false),
    (AtlasCmd::Paste, true),
    (AtlasCmd::Trash, false),
    (AtlasCmd::DeletePerm, false),
];

/// A logical (non-element) row, recomputed each frame and shared by the renderer
/// and the keyboard handler so selection indices always agree.
enum CtxRow {
    Separator,
    Loading,
    Atlas {
        cmd: AtlasCmd,
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
        matches!(self, CtxRow::Atlas { .. } | CtxRow::Native { .. })
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

        let input = crate::ui::text_input::single_line(window, cx, "Buscar acciones…", "");
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

        #[cfg(windows)]
        let loading = {
            if same_parent {
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
            let _ = same_parent;
            false
        };

        self.ctx_menu = Some(CtxMenuState {
            menu_id,
            anchor,
            screen_pt,
            paths,
            native: Vec::new(),
            loading,
            selected: 0,
            expanded: HashSet::new(),
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
        let q = query.trim();
        if q.is_empty() {
            // Tree mode: Atlas section, separator, native tree (loading skeleton).
            let mut rows = Vec::new();
            for (cmd, sep_after) in ATLAS_ITEMS {
                rows.push(CtxRow::Atlas { cmd: *cmd });
                if *sep_after {
                    rows.push(CtxRow::Separator);
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
            // Search mode: flatten every leaf (Atlas + native), rank, emit flat.
            let _ = scorer_query;
            let mut cand_titles: Vec<String> = Vec::new();
            let mut cands: Vec<CtxRow> = Vec::new();

            for (cmd, _) in ATLAS_ITEMS {
                cand_titles.push(cmd.label().to_string());
                cands.push(CtxRow::Atlas { cmd: *cmd });
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
        }
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
        let input = self.ctx_menu_input.clone();
        let query = input
            .as_ref()
            .map(|i| i.read(cx).value().to_string())
            .unwrap_or_default();

        let rows = self.ctx_menu_rows(&query, &query);
        let state = self.ctx_menu.as_ref();
        let (anchor, selected) = state
            .map(|s| (s.anchor, s.selected))
            .unwrap_or((Point::default(), 0));

        // Clamp the panel into the window so it never spills off an edge.
        let win = window.bounds().size;
        const W: f32 = 290.;
        let est_h = (rows.len() as f32 * 30.0 + 50.0).min(560.0);
        let (ax, ay) = (f32::from(anchor.x), f32::from(anchor.y));
        let (ww, wh) = (f32::from(win.width), f32::from(win.height));
        let left = ax.min((ww - W - 6.0).max(6.0)).max(6.0);
        let top = ay.min((wh - est_h - 6.0).max(6.0)).max(6.0);

        let view = cx.entity();

        // Map selectable index → element, tracking which selectable index each
        // selectable row occupies so we can highlight `selected`.
        let mut sel_idx = 0usize;
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
                            .child(div().child("Cargando acciones del sistema…"))
                            .into_any_element(),
                    );
                }
                CtxRow::Atlas { cmd } => {
                    let this_idx = sel_idx;
                    sel_idx += 1;
                    let is_sel = this_idx == selected;
                    let cmd = *cmd;
                    let v = view.clone();
                    let icon_el = cmd
                        .icon()
                        .map(|i| crate::ui::icons::icon(i, 16., if is_sel { accent_fg } else { fg }));
                    children.push(
                        menu_row(
                            SharedString::from(format!("ctx-atlas-{this_idx}")),
                            icon_el,
                            cmd.label().to_string(),
                            false,
                            false,
                            false,
                            is_sel,
                            accent,
                            accent_fg,
                            fg,
                            muted,
                        )
                        .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                            v.update(cx, |this, cx| {
                                this.run_ctx_atlas(cmd, window, cx);
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
                        submenu,
                        *expanded,
                        *is_default,
                        is_sel,
                        accent,
                        accent_fg,
                        if disabled { muted } else { fg },
                        muted,
                    )
                    .pl(px(8.0 + f32::from(*depth) * 14.0));
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
            .max_h(px(560.))
            .bg(popover)
            .text_color(fg)
            .border_1()
            .border_color(border)
            .rounded_lg()
            .shadow_lg()
            .occlude()
            .overflow_hidden()
            .children(input_row)
            .child(v_flex().py_1().children(children))
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
                    }
                }
                true
            }
            "up" => {
                if let Some(m) = self.ctx_menu.as_mut() {
                    if count > 0 {
                        m.selected = (m.selected + count - 1) % count;
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
            CtxRow::Native { id, submenu, .. } => {
                self.activate_ctx_native(*id, *submenu, window, cx)
            }
            _ => {}
        }
    }

    /// Run an Atlas command against the current selection, then close. We call the
    /// App methods directly (rather than dispatching the gpui action) so the menu's
    /// focused search input can't intercept `Copy`/`Cut`/`Paste` as text edits.
    fn run_ctx_atlas(&mut self, cmd: AtlasCmd, window: &mut Window, cx: &mut Context<Self>) {
        self.close_ctx_menu(cx);
        let is_archive = self
            .state
            .active_tab_data()
            .is_some_and(|t| t.location.is_archive());
        match cmd {
            AtlasCmd::Open => self.open_cursor(),
            AtlasCmd::Rename => self.begin_rename(window, cx),
            AtlasCmd::Copy => {
                if is_archive {
                    self.toast_error(rust_i18n::t!("root_copy_archive_error").into());
                } else {
                    let paths = self.selected_paths();
                    if !paths.is_empty() {
                        self.process_actions(vec![Action::Copy(paths)]);
                    }
                }
            }
            AtlasCmd::Cut => {
                if is_archive {
                    self.toast_error(rust_i18n::t!("root_cut_archive_error").into());
                } else {
                    let paths = self.selected_paths();
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
                    let paths = self.selected_paths();
                    if !paths.is_empty() {
                        self.process_actions(vec![Action::Delete {
                            paths,
                            to_trash: cmd == AtlasCmd::Trash,
                        }]);
                    }
                }
            }
        }
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

/// Rank `titles` against `query` (best first). Standalone so it doesn't need the
/// `App` borrow held by `ctx_menu_rows`.
fn rank_titles(query: &str, titles: &[String]) -> Vec<usize> {
    let mut scorer = crate::search::fuzzy::FuzzyScorer::new();
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
fn menu_row(
    id: SharedString,
    icon: Option<AnyElement>,
    label: String,
    submenu: bool,
    expanded: bool,
    is_default: bool,
    selected: bool,
    accent: gpui::Hsla,
    accent_fg: gpui::Hsla,
    fg: gpui::Hsla,
    _muted: gpui::Hsla,
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
    row = row.child(
        div()
            .w(px(16.))
            .h(px(16.))
            .flex_none()
            .children(icon),
    );
    let mut label_div = div().flex_1().child(label);
    if is_default {
        label_div = label_div.font_weight(gpui::FontWeight::SEMIBOLD);
    }
    row = row.child(label_div);
    if submenu {
        row = row.child(
            crate::ui::icons::icon(
                if expanded {
                    AtlasIcon::CaretDown
                } else {
                    AtlasIcon::CaretRight
                },
                12.,
                text_color,
            ),
        );
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
