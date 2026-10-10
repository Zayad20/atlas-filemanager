//! Shared panel renderer + keyboard model for the palette engine.
//!
//! One visual for every palette: search row (lupa + native input + optional
//! buttons), collapsible sections of rich rows, and a bottom hint bar. The
//! flattened list of *visible* rows — section headers included — is the single
//! source for both the highlight index and the keyboard handler, so selection
//! can never land on a hidden row:
//!   * up/down walk headers and items alike;
//!   * Enter runs an item (and closes) or toggles a header's collapse;
//!   * Tab toggles a header, drills into an expandable item (replaces the
//!     query with its `expand` text, e.g. a folder path), or toggles the
//!     description lines for plain items.

use super::{FooterHint, FooterSpec, PaletteItem, PaletteKind, PaletteSpec, ShortcutHint};
use crate::app::App;
use crate::input::action::Action;
use crate::ui::icons::{icon, AtlasIcon};
use crate::ui::motion;
use std::path::PathBuf;
use gpui::prelude::*;
use gpui::AnimationExt;
use gpui::ColorExt as _;
use gpui::{
    div, px, relative, AnyElement, Context, Entity, FontWeight, HighlightStyle, Hsla, KeyDownEvent,
    MouseButton, SharedString, StyledText, Window,
};
use gpui_component::input::{
    Backspace as InputBackspace, Delete as InputDelete, Escape as InputEscape, IndentInline,
    InputState,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::StyledExt as _;
use gpui_component::{h_flex, kbd::Kbd, tooltip::Tooltip, v_flex, ActiveTheme};
use std::rc::Rc;

pub(crate) const PANEL_WIDTH: f32 = 620.0;
const PANEL_MAX_VIEWPORT_RATIO: f32 = 0.60;
const PANEL_TOP_VIEWPORT_RATIO: f32 = 0.20;
const PANEL_MIN_VISIBLE_ROWS: f32 = 2.0;
const PANEL_HEADER_HEIGHT: f32 = 26.0;
const PANEL_ITEM_HEIGHT: f32 = 34.0;
const PANEL_COMPACT_ITEM_HEIGHT: f32 = 26.0;
const PANEL_ITEM_WITH_DESC_HEIGHT: f32 = 52.0;
const PANEL_LIST_PADDING: f32 = 8.0;
const PANEL_SEARCH_HEIGHT: f32 = 45.0;
const PANEL_FOOTER_HEIGHT: f32 = 32.0;
const SCROLLBAR_GUTTER: f32 = 16.0;
// Open/close tween shared by every palette. gpui can't transform-scale a div, so
// the "grow" is the panel's width going 80%→100% (centered by the scrim)
// alongside opacity. `PALETTE_CLOSE_MS` must be ≤ `App::PALETTE_CLOSE_FINALIZE`
// so the fade-out finishes before the hosting view is torn down.
pub(crate) const PALETTE_OPEN_MS: u64 = 150;
pub(crate) const PALETTE_CLOSE_MS: u64 = 130;
/// Left inset for items under a titled header (chevron + icon width).
const SECTION_INDENT: f32 = 26.0;

/// One keyboard-selectable row in the flattened, collapse-aware list.
enum KeyRow {
    Header(&'static str),
    Item {
        action: Action,
        /// Tab replaces the search text with this (drill into a folder).
        expand: Option<SharedString>,
    },
}

#[derive(Clone, Copy)]
pub(crate) struct PaletteColors {
    pub fg: Hsla,
    pub muted: Hsla,
    pub muted_bg: Hsla,
    pub accent: Hsla,
    pub accent_fg: Hsla,
    pub border: Hsla,
}

struct PaletteHeaderProps<'a> {
    spec: &'a PaletteSpec,
    section_id: &'static str,
    section_icon: Option<AtlasIcon>,
    title: SharedString,
    is_collapsed: bool,
    selected: bool,
    colors: PaletteColors,
}

struct PaletteItemProps<'a> {
    spec: &'a PaletteSpec,
    item: &'a PaletteItem,
    i: usize,
    selected: bool,
    expanded: bool,
    indent: bool,
    colors: PaletteColors,
}

struct PaletteKeyProps<'a> {
    kind: PaletteKind,
    ev: &'a KeyDownEvent,
    rows: &'a [KeyRow],
    close_action: &'a Action,
    input: &'a Entity<InputState>,
}

/// An item title with the query-matched characters tinted (and bolded) so the
/// eye lands on why each row matched.
fn highlighted_title(
    item: &PaletteItem,
    base_color: Hsla,
    match_color: Hsla,
    window: &Window,
) -> AnyElement {
    match &item.highlight {
        Some(ranges) if !ranges.is_empty() => {
            let mut base = window.text_style();
            base.color = base_color;
            let highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = ranges
                .iter()
                .map(|r| {
                    (
                        r.clone(),
                        HighlightStyle {
                            color: Some(match_color),
                            font_weight: Some(FontWeight::SEMIBOLD),
                            ..HighlightStyle::default()
                        },
                    )
                })
                .collect();
            StyledText::new(item.title.clone())
                .with_default_highlights(&base, highlights)
                .into_any_element()
        }
        _ => item.title.clone().into_any_element(),
    }
}

/// Free-form badge ("MouseMiddle", "↑") visually matching `Kbd`.
fn label_badge(text: SharedString, bg: Hsla, fg: Hsla) -> AnyElement {
    div()
        .px_1()
        .py(px(1.))
        .rounded_sm()
        .bg(bg)
        .text_color(fg)
        .text_xs()
        .flex_shrink_0()
        .child(text)
        .into_any_element()
}

/// Resolve one shortcut hint into a badge element. `ForAction` hints with no
/// registered binding render nothing. On a selected (accent) row the badge
/// switches to an outline style so it doesn't fight the highlight color.
fn shortcut_badge(
    hint: &ShortcutHint,
    selected: bool,
    badge_bg: Hsla,
    badge_fg: Hsla,
    accent_fg: Hsla,
    window: &Window,
    view: Option<Entity<App>>,
) -> Option<AnyElement> {
    // Restyle a Kbd for a selected row: no fill, faint outline, highlight text.
    let restyle = move |kbd: Kbd| -> AnyElement {
        if selected {
            kbd.bg(gpui::transparent_black())
                .border_1()
                .border_color(accent_fg.opacity(0.4))
                .text_color(accent_fg)
                .into_any_element()
        } else {
            kbd.into_any_element()
        }
    };
    match hint {
        ShortcutHint::ForAction(action) => {
            Kbd::binding_for_action(action.as_ref(), Some("Atlas"), window).map(restyle)
        }
        ShortcutHint::Keys(ks) => Some(restyle(Kbd::new(ks.clone()))),
        ShortcutHint::Label(text) => Some(if selected {
            div()
                .px_1()
                .py(px(1.))
                .rounded_sm()
                .border_1()
                .border_color(accent_fg.opacity(0.4))
                .text_color(accent_fg)
                .text_xs()
                .flex_shrink_0()
                .child(text.clone())
                .into_any_element()
        } else {
            label_badge(text.clone(), badge_bg, badge_fg)
        }),
        ShortcutHint::CommandShortcut {
            command_id,
            key,
            recording,
        } => {
            let view = view?;
            let badge = if *recording {
                label_badge(
                    rust_i18n::t!("palette_shortcut_recording").into(),
                    badge_bg,
                    badge_fg,
                )
            } else if let Some(key) = key {
                restyle(Kbd::new(key.clone()))
            } else {
                label_badge(
                    rust_i18n::t!("palette_shortcut_none").into(),
                    badge_bg,
                    badge_fg,
                )
            };
            let start_view = view.clone();
            let start_id = command_id.clone();
            Some(
                div()
                    .id(SharedString::from(format!("command-shortcut-{command_id}")))
                    .cursor_pointer()
                    .tooltip({
                        let tooltip: SharedString = if *recording {
                            rust_i18n::t!("palette_shortcut_recording_tooltip").into()
                        } else {
                            rust_i18n::t!("palette_shortcut_edit_tooltip").into()
                        };
                        move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)
                    })
                    .child(badge)
                    .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                        cx.stop_propagation();
                        start_view.update(cx, |this, cx| {
                            this.process_actions(vec![Action::StartCommandShortcutCapture(
                                start_id.clone(),
                            )]);
                            cx.notify();
                        });
                    })
                    .into_any_element(),
            )
        }
    }
}

impl App {
    /// Get-or-create the native search input for a palette. Cleared to `None`
    /// on close so each open starts empty.
    pub(crate) fn palette_input_for(
        &mut self,
        kind: PaletteKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        match kind {
            PaletteKind::Goto => {
                // A pending pre-fill (Ctrl+L) seeds the input on creation only.
                let prefill = if self.goto_input.is_none() {
                    self.goto_prefill.take().unwrap_or_default()
                } else {
                    String::new()
                };
                self.goto_input
                    .get_or_insert_with(|| {
                        crate::ui::text_input::single_line(
                            window,
                            cx,
                            rust_i18n::t!("palette_search_files"),
                            prefill,
                        )
                    })
                    .clone()
            }
            PaletteKind::Commands => self
                .palette_input
                .get_or_insert_with(|| {
                    crate::ui::text_input::single_line(
                        window,
                        cx,
                        rust_i18n::t!("palette_search_commands"),
                        "",
                    )
                })
                .clone(),
            PaletteKind::Ext => self
                .ext_palette_input
                .get_or_insert_with(|| {
                    crate::ui::text_input::single_line(
                        window,
                        cx,
                        rust_i18n::t!("palette_search_options"),
                        "",
                    )
                })
                .clone(),
        }
    }

    /// Mirror the native input's text into the palette's neutral state. Returns
    /// `true` when the query changed (the caller may re-feed a search).
    pub(crate) fn sync_palette_query(
        &mut self,
        kind: PaletteKind,
        input: &Entity<InputState>,
        cx: &Context<Self>,
    ) -> bool {
        let query = input.read(cx).value().to_string();
        let state = kind.state(self);
        if query == state.query {
            return false;
        }
        state.query = query;
        state.selected_idx = 0;
        true
    }

    /// Build the full palette layer (scrim + animated panel) for this frame.
    pub(crate) fn palette_panel(
        &mut self,
        spec: PaletteSpec,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = {
            let t = cx.theme();
            PaletteColors {
                fg: t.popover_foreground,
                muted: t.muted_foreground,
                muted_bg: t.muted,
                accent: t.accent,
                accent_fg: t.accent_foreground,
                border: t.border,
            }
        };
        let PaletteColors {
            fg,
            muted,
            muted_bg: _,
            accent: _,
            accent_fg: _,
            border,
        } = colors;
        let view = cx.entity();
        let focus = self.overlay_focus(cx);
        let kind = spec.kind;
        let (smooth_ease, smooth_boost) =
            crate::app::scroll_smooth_params(self.state.settings.scroll_smoothness);
        self.palette_scroll.set_smooth(smooth_ease, smooth_boost);
        let close_action = kind.close_action();
        let closing = matches!(self.palette_closing, Some((k, _)) if k == kind);

        let (expanded, collapsed) = {
            let state = kind.state(self);
            (state.expanded, state.collapsed_sections.clone())
        };

        // Flatten the *visible* rows — headers included, collapsed items
        // excluded. This list drives both the highlight and the keyboard.
        let mut key_rows: Vec<KeyRow> = Vec::new();
        for section in &spec.sections {
            let has_header = section.title.is_some();
            if has_header {
                key_rows.push(KeyRow::Header(section.id));
            }
            if has_header && collapsed.contains(section.id) {
                continue;
            }
            for item in &section.items {
                key_rows.push(KeyRow::Item {
                    action: item.action.clone(),
                    expand: item.expand.clone(),
                });
            }
        }
        let key_rows = Rc::new(key_rows);
        let row_count = key_rows.len();
        let mut selected_idx = kind
            .state(self)
            .selected_idx
            .min(row_count.saturating_sub(1));
        // Fresh open / query reset land on 0: snap past a leading header so
        // Enter runs the best match instead of folding its section. Headers
        // stay reachable with the up arrow.
        if selected_idx == 0 && matches!(key_rows.first(), Some(KeyRow::Header(_))) {
            if let Some(first_item) = key_rows
                .iter()
                .position(|r| matches!(r, KeyRow::Item { .. }))
            {
                selected_idx = first_item;
                kind.state(self).selected_idx = first_item;
            }
        }
        self.preview_selected_theme(kind, &key_rows, selected_idx);
        // Keyboard nav requested a scroll: keep the selected row in view (the
        // handle applies it on the next layout of the tracked list).
        {
            let state = kind.state(self);
            if state.scroll_to_selected {
                state.scroll_to_selected = false;
                self.palette_scroll.scroll_to_item(selected_idx);
            }
        }
        let mut rows_h = PANEL_LIST_PADDING;
        let mut row_i = 0usize;
        for section in &spec.sections {
            let has_header = section.title.is_some();
            if has_header {
                rows_h += PANEL_HEADER_HEIGHT;
                row_i += 1;
                if collapsed.contains(section.id) {
                    continue;
                }
            }
            for item in &section.items {
                let shows_description = !item.inline_description
                    && item.description.is_some()
                    && (expanded || row_i == selected_idx);
                rows_h += if kind == PaletteKind::Goto && item.inline_description {
                    PANEL_COMPACT_ITEM_HEIGHT
                } else if shows_description {
                    PANEL_ITEM_WITH_DESC_HEIGHT
                } else {
                    PANEL_ITEM_HEIGHT
                };
                row_i += 1;
            }
        }

        // ── rows ────────────────────────────────────────────────────────────
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut gi = 0usize;
        for section in &spec.sections {
            let has_header = section.title.is_some();
            let is_collapsed = has_header && collapsed.contains(section.id);
            if let Some(title) = &section.title {
                let i = gi;
                gi += 1;
                rows.push(self.palette_section_header(
                    PaletteHeaderProps {
                        spec: &spec,
                        section_id: section.id,
                        section_icon: section.icon,
                        title: title.clone(),
                        is_collapsed,
                        selected: i == selected_idx,
                        colors,
                    },
                    cx,
                ));
            }
            if is_collapsed {
                continue;
            }
            for item in &section.items {
                let i = gi;
                gi += 1;
                rows.push(self.palette_item_row(
                    PaletteItemProps {
                        spec: &spec,
                        item,
                        i,
                        selected: i == selected_idx,
                        expanded,
                        indent: has_header,
                        colors,
                    },
                    window,
                    cx,
                ));
            }
        }
        let empty = rows.is_empty();

        // ── search row ──────────────────────────────────────────────────────
        let search_row = h_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(border)
            .child(icon(AtlasIcon::Search, 16.0, muted))
            .children(spec.search_prefix)
            .child(div().flex_1().child(crate::ui::text_input::flat(input)))
            .children(spec.search_buttons);

        // ── results ─────────────────────────────────────────────────────────
        // Rows are direct children of the tracked scroll container so
        // `scroll_to_item(row_index)` maps 1:1 onto them.
        let results = div()
            .relative()
            .flex_1()
            .min_h(px(0.))
            .child(
                v_flex()
                    .id(SharedString::from(format!("{}-list", spec.id)))
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.palette_scroll)
                    .p_1()
                    .pr(px(SCROLLBAR_GUTTER))
                    .gap(px(1.))
                    .when(empty, |s| {
                        s.child(div().p_3().text_color(muted).child(spec.empty_text))
                    })
                    .children(rows),
            )
            .vertical_scrollbar(&self.palette_scroll);

        let result_count = key_rows
            .iter()
            .filter(|r| matches!(r, KeyRow::Item { .. }))
            .count();
        let footer = palette_footer(&spec.footer, result_count, colors, window);
        let viewport_h = f32::from(window.bounds().size.height);
        let max_panel_h = viewport_h * PANEL_MAX_VIEWPORT_RATIO;
        let chrome_h = PANEL_SEARCH_HEIGHT
            + if footer.is_some() {
                PANEL_FOOTER_HEIGHT
            } else {
                0.0
            };
        let min_panel_h = chrome_h + PANEL_ITEM_HEIGHT * PANEL_MIN_VISIBLE_ROWS;
        let rows_h = if empty {
            PANEL_ITEM_HEIGHT * PANEL_MIN_VISIBLE_ROWS
        } else {
            rows_h
        };
        let panel_h = (chrome_h + rows_h).clamp(min_panel_h.min(max_panel_h), max_panel_h);
        let panel_width = PANEL_WIDTH.min((f32::from(window.bounds().size.width) - 24.0).max(1.0));

        // Capture phase: intercept navigation/confirm/dismiss/expand before the
        // focused `TextInput` consumes them. Everything else falls through to
        // the input for real editing. Tab needs a *capture action* handler:
        // gpui dispatches keybindings before key listeners, and the input binds
        // "tab" to `IndentInline` (a silent no-op in single-line inputs) which
        // would otherwise eat the key.
        let close_for_keys = close_action.clone();
        let input_for_keys = input.clone();
        let rows_for_keys = key_rows.clone();
        let input_for_tab = input.clone();
        let panel = v_flex()
            .w(px(panel_width))
            .h(px(panel_h))
            .popover_style(cx)
            .text_color(fg)
            .overflow_hidden()
            .occlude()
            .capture_action(cx.listener(move |this, _: &IndentInline, window, cx| {
                cx.stop_propagation();
                if this.command_shortcut_capture.is_some() {
                    if let Some(command_id) = this.command_shortcut_capture.clone() {
                        this.set_command_shortcut(command_id, "tab".to_string());
                    }
                } else {
                    this.palette_tab_key(kind, &key_rows, &input_for_tab, window, cx);
                }
                cx.notify();
            }))
            .capture_action(cx.listener(|this, _: &InputBackspace, _window, cx| {
                if let Some(command_id) = this.command_shortcut_capture.clone() {
                    cx.stop_propagation();
                    this.remove_command_shortcut(command_id);
                    cx.notify();
                }
            }))
            .capture_action(cx.listener(|this, _: &InputDelete, _window, cx| {
                if let Some(command_id) = this.command_shortcut_capture.clone() {
                    cx.stop_propagation();
                    this.remove_command_shortcut(command_id);
                    cx.notify();
                }
            }))
            // Same trick for Escape: the input's own Escape action can consume
            // the key (context menu / IME / clean-on-escape paths); the palette
            // owns Esc semantics instead.
            .capture_action(cx.listener({
                let close = close_action.clone();
                let input = input.clone();
                move |this, _: &InputEscape, window, cx| {
                    cx.stop_propagation();
                    if this.command_shortcut_capture.is_some() {
                        this.cancel_command_shortcut_capture();
                    } else {
                        this.palette_escape(kind, &close, &input, window, cx);
                    }
                    cx.notify();
                }
            }))
            .capture_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                if this.handle_command_shortcut_key(ev)
                    || this.handle_palette_key(
                        PaletteKeyProps {
                            kind,
                            ev,
                            rows: &rows_for_keys,
                            close_action: &close_for_keys,
                            input: &input_for_keys,
                        },
                        window,
                        cx,
                    )
                {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(search_row)
            .child(results)
            .children(footer);

        // Distinct ids so switching open↔close restarts the animation from 0.
        let anim_id = if closing {
            format!("{}-close", spec.id)
        } else {
            format!("{}-appear", spec.id)
        };
        let ms = if closing {
            PALETTE_CLOSE_MS
        } else {
            PALETTE_OPEN_MS
        };
        let animated = panel
            .with_animation(
                SharedString::from(anim_id),
                motion::animation(ms),
                move |this, delta| {
                    // `p` = openness (1 = fully open). Reversed while closing.
                    let p = if closing { 1.0 - delta } else { delta };
                    motion::panel_pop(this, p, panel_width)
                },
            )
            .into_any_element();

        let positioned = div()
            .absolute()
            .top(relative(PANEL_TOP_VIEWPORT_RATIO))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(animated)
            .into_any_element();

        self.overlay_scrim(view, close_action, focus, positioned, false, cx)
    }

    /// A section header: chevron + optional icon + title. Selectable like any
    /// row; click, Enter or Tab toggle its collapse.
    fn palette_section_header(
        &mut self,
        props: PaletteHeaderProps,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kind = props.spec.kind;
        let view = cx.entity();
        let text_color = if props.selected {
            props.colors.fg
        } else {
            props.colors.muted
        };
        h_flex()
            .id(SharedString::from(format!(
                "{}-sec-{}",
                props.spec.id, props.section_id
            )))
            .w_full()
            .px_2()
            .py_1()
            .mt_1()
            .gap_1()
            .items_center()
            .rounded_sm()
            .text_xs()
            .text_color(text_color)
            .cursor_pointer()
            .when(props.selected, |s| s.bg(props.colors.border))
            .when(!props.selected, |s| s.hover(|h| h.bg(props.colors.border)))
            .child(icon(
                if props.is_collapsed {
                    AtlasIcon::CaretRight
                } else {
                    AtlasIcon::CaretDown
                },
                12.0,
                text_color,
            ))
            .children(props.section_icon.map(|ic| icon(ic, 14.0, text_color)))
            .child(props.title)
            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                view.update(cx, |this, cx| {
                    this.toggle_palette_section(kind, props.section_id);
                    cx.notify();
                });
            })
            .into_any_element()
    }

    fn toggle_palette_section(&mut self, kind: PaletteKind, section_id: &str) {
        let state = kind.state(self);
        if !state.collapsed_sections.remove(section_id) {
            state.collapsed_sections.insert(section_id.to_string());
        }
    }

    /// One selectable row: icon · title/description · buttons · shortcut badges.
    fn palette_item_row(
        &mut self,
        props: PaletteItemProps,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let kind = props.spec.kind;
        let group_name = SharedString::from(format!("{}-row-{}", props.spec.id, props.i));
        let icon_color = if props.selected {
            props.colors.accent_fg
        } else {
            props.colors.muted
        };
        let desc_color = if props.selected {
            props.colors.accent_fg
        } else {
            props.colors.muted
        };
        if let Some(file_icon) = &props.item.file_icon {
            self.maybe_request_file_icon(file_icon);
        }
        let icon_el = props
            .item
            .file_icon
            .as_ref()
            .map(|file_icon| self.file_icon(file_icon).element(16.0, icon_color))
            .or_else(|| props.item.icon.map(|ic| icon(ic, 16.0, icon_color)));
        // Matched characters: accent on normal rows, bold-bright on selected.
        let title_el = highlighted_title(
            props.item,
            if props.selected {
                props.colors.accent_fg
            } else {
                props.colors.fg
            },
            if props.selected {
                props.colors.accent_fg
            } else {
                props.colors.accent
            },
            window,
        );

        // Title + description: inline ("name › path", always visible) or
        // stacked (second line, shown when selected or globally expanded).
        let text_col = if props.item.inline_description {
            let mut row = h_flex()
                .min_w(px(0.))
                .flex_1()
                .gap_1()
                .items_center()
                .child(
                    div()
                        .max_w(relative(0.60))
                        .flex_shrink_0()
                        .truncate()
                        .child(title_el),
                );
            if let Some(desc) = props.item.description.clone() {
                row = row
                    .child(icon(AtlasIcon::CaretRight, 11.0, desc_color))
                    .child(
                        div()
                            .min_w(px(0.))
                            .text_xs()
                            .text_color(desc_color)
                            .opacity(0.7)
                            .truncate()
                            .child(desc),
                    );
            }
            row.into_any_element()
        } else {
            let show_desc = props.item.description.is_some() && (props.selected || props.expanded);
            v_flex()
                .min_w(px(0.))
                .flex_1()
                .child(div().truncate().child(title_el))
                .when(show_desc, |col| {
                    col.children(props.item.description.clone().map(|desc| {
                        div()
                            .text_xs()
                            .text_color(desc_color)
                            .truncate()
                            .child(desc)
                    }))
                })
                .into_any_element()
        };

        let buttons: Vec<AnyElement> = props
            .item
            .buttons
            .iter()
            .map(|b| {
                let btn_view = view.clone();
                let btn_action = b.action.clone();
                let btn_close = b.closes_palette.then(|| kind.close_action());
                let btn_tooltip = b.tooltip.clone();
                let color = if props.selected {
                    props.colors.accent_fg
                } else if b.active {
                    props.colors.accent
                } else {
                    props.colors.muted
                };
                div()
                    .id(SharedString::from(format!(
                        "{}-row-{}-btn-{}",
                        props.spec.id, props.i, b.id
                    )))
                    .p(px(2.))
                    .rounded_sm()
                    .cursor_pointer()
                    .when(!props.selected && !b.active, |s| {
                        s.invisible()
                            .group_hover(group_name.clone(), gpui::Styled::visible)
                    })
                    .hover(|h| h.bg(props.colors.border))
                    .tooltip(move |window, cx| Tooltip::new(btn_tooltip.clone()).build(window, cx))
                    .child(icon(b.icon, 14.0, color))
                    .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                        cx.stop_propagation();
                        let mut actions = vec![btn_action.clone()];
                        if let Some(close) = &btn_close {
                            actions.push(close.clone());
                        }
                        btn_view.update(cx, |this, cx| {
                            this.process_actions(actions);
                            this.flush_dirty_panes(cx);
                        });
                    })
                    .into_any_element()
            })
            .collect();

        let badges: Vec<AnyElement> = props
            .item
            .shortcuts
            .iter()
            .filter_map(|hint| {
                shortcut_badge(
                    hint,
                    props.selected,
                    props.colors.muted_bg,
                    props.colors.muted,
                    props.colors.accent_fg,
                    window,
                    Some(view.clone()),
                )
            })
            .collect();

        let row_action = props.item.action.clone();
        let path_tooltip = match &row_action {
            Action::Navigate { path, .. } => Some(path.to_string_lossy().into_owned()),
            Action::Open(paths) => paths.first().map(|path| path.to_string_lossy().into_owned()),
            _ => None,
        };
        let row_close = kind.close_action();
        let row_view = view;
        div()
            .id(SharedString::from(format!(
                "{}-row-{}",
                props.spec.id, props.i
            )))
            .group(group_name)
            .w_full()
            .when(props.indent, |s| s.pl(px(SECTION_INDENT)))
            .when(!props.indent, |s| s.pl_2())
            .pr_2()
            .py_1()
            .when(kind == PaletteKind::Goto, |row| {
                row.h(px(PANEL_COMPACT_ITEM_HEIGHT))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .text_xs()
                    .py_0()
            })
            .rounded_sm()
            .cursor_pointer()
            .when_some(path_tooltip, |row, path| {
                row.tooltip(move |window, cx| Tooltip::new(path.clone()).build(window, cx))
            })
            .when(props.selected, |s| {
                s.bg(props.colors.accent).text_color(props.colors.accent_fg)
            })
            .when(!props.selected, |s| s.hover(|h| h.bg(props.colors.border)))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .children(icon_el)
                    .child(text_col)
                    .children(buttons)
                    .children(badges),
            )
            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                let actions = vec![row_action.clone(), row_close.clone()];
                row_view.update(cx, |this, cx| {
                    this.process_actions(actions);
                    this.flush_dirty_panes(cx);
                });
            })
            .into_any_element()
    }

    /// Navigation/confirm/dismiss/expand for the palettes. Text editing &
    /// clipboard are owned by the native `TextInput`; this only handles the
    /// keys that drive the row list. Returns `true` when consumed.
    fn handle_palette_key(
        &mut self,
        props: PaletteKeyProps,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let row_count = props.rows.len();
        match props.ev.keystroke.key.as_str() {
            "escape" => {
                self.palette_escape(props.kind, props.close_action, props.input, window, cx);
                true
            }
            "enter" => {
                let idx = props
                    .kind
                    .state(self)
                    .selected_idx
                    .min(row_count.saturating_sub(1));
                match props.rows.get(idx) {
                    Some(KeyRow::Item { action, .. }) => {
                        let m = &props.ev.keystroke.modifiers;
                        let mut actions_to_run = Vec::new();
                        let action = if matches!(props.kind, PaletteKind::Goto) {
                            let (path, is_create) = match action {
                                Action::Navigate { path, .. } => (Some(path.clone()), false),
                                Action::Open(paths) => (paths.first().cloned(), false),
                                Action::CreateFolder { parent, name } => {
                                    (Some(parent.join(name)), true)
                                }
                                Action::CreateFile { parent, name } => {
                                    (Some(parent.join(name)), true)
                                }
                                _ => (None, false),
                            };
                            let focused_tile = self.state.focused_tile;
                            let pane = self
                                .state
                                .find_tabs_container(focused_tile)
                                .or_else(|| self.state.find_any_tabs_container())
                                .unwrap_or(focused_tile);
                            if (m.control || m.platform) && !m.shift && !m.alt {
                                if is_create {
                                    actions_to_run.push(action.clone());
                                }
                                path.map(|p| Action::OpenDirInRightSplit { pane, path: p })
                                    .unwrap_or_else(|| action.clone())
                            } else if m.alt || ((m.control || m.platform) && m.shift) {
                                if is_create {
                                    actions_to_run.push(action.clone());
                                }
                                path.map(|p| Action::OpenDirInBottomSplit { pane, path: p })
                                    .unwrap_or_else(|| action.clone())
                            } else if m.shift && !m.control && !m.platform && !m.alt {
                                if is_create {
                                    actions_to_run.push(action.clone());
                                }
                                path.map(Action::OpenPathInNewWindow)
                                    .unwrap_or_else(|| action.clone())
                            } else {
                                action.clone()
                            }
                        } else {
                            action.clone()
                        };
                        actions_to_run.push(action);
                        actions_to_run.push(props.close_action.clone());
                        self.process_actions(actions_to_run);
                        self.flush_dirty_panes(cx);
                    }
                    Some(KeyRow::Header(section_id)) => {
                        self.toggle_palette_section(props.kind, section_id);
                    }
                    None => {
                        if matches!(props.kind, PaletteKind::Goto) {
                            let query = props.input.read(cx).value().trim().to_string();
                            if !query.is_empty() {
                                let focused_tile = self.state.focused_tile;
                                let pane = self
                                    .state
                                    .find_tabs_container(focused_tile)
                                    .or_else(|| self.state.find_any_tabs_container())
                                    .unwrap_or(focused_tile);
                                let resolved = {
                                    let expanded = dirs::home_dir().and_then(|h| {
                                        super::specs::expand_home_query(&query, &h)
                                    });
                                    let q = expanded.as_deref().unwrap_or(&query);
                                    let p = PathBuf::from(q);
                                    if p.exists() {
                                        p
                                    } else {
                                        let base = self.goto_default_root();
                                        let joined = base.join(q);
                                        if joined.exists() {
                                            joined
                                        } else {
                                            p
                                        }
                                    }
                                };
                                let m = &props.ev.keystroke.modifiers;
                                let action = if (m.control || m.platform) && !m.shift && !m.alt {
                                    Action::OpenDirInRightSplit { pane, path: resolved }
                                } else if m.alt || ((m.control || m.platform) && m.shift) {
                                    Action::OpenDirInBottomSplit { pane, path: resolved }
                                } else if m.shift && !m.control && !m.platform && !m.alt {
                                    Action::OpenPathInNewWindow(resolved)
                                } else if resolved.is_dir() {
                                    Action::Navigate {
                                        tab: focused_tile,
                                        path: resolved,
                                    }
                                } else {
                                    Action::Open(vec![resolved])
                                };
                                self.process_actions(vec![action, props.close_action.clone()]);
                                self.flush_dirty_panes(cx);
                            }
                        }
                    }
                }
                true
            }
            "down" => {
                let state = props.kind.state(self);
                state.selected_idx = (state.selected_idx + 1).min(row_count.saturating_sub(1));
                state.scroll_to_selected = true;
                true
            }
            "up" => {
                let state = props.kind.state(self);
                state.selected_idx = state.selected_idx.saturating_sub(1);
                state.scroll_to_selected = true;
                true
            }
            // Fallback only: when the input is focused, Tab arrives as the
            // `IndentInline` action (see `capture_action` above), not here.
            "tab" => {
                self.palette_tab_key(props.kind, props.rows, props.input, window, cx);
                true
            }
            _ => false,
        }
    }

    /// Esc semantics: in the goto with text written (e.g. drilled into a
    /// folder), the first Esc clears the query back to the root/home view;
    /// an empty query — or any other palette — dismisses.
    fn palette_escape(
        &mut self,
        kind: PaletteKind,
        close_action: &Action,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let has_query =
            matches!(kind, PaletteKind::Goto) && !input.read(cx).value().trim().is_empty();
        if has_query {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        } else {
            self.process_actions(vec![close_action.clone()]);
        }
    }

    /// Tab on the selected row: fold/unfold a header, drill into an expandable
    /// item (its `expand` text becomes the query), or toggle the description
    /// lines for plain items.
    fn palette_tab_key(
        &mut self,
        kind: PaletteKind,
        rows: &[KeyRow],
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let idx = kind
            .state(self)
            .selected_idx
            .min(rows.len().saturating_sub(1));
        match rows.get(idx) {
            Some(KeyRow::Header(section_id)) => {
                self.toggle_palette_section(kind, section_id);
            }
            Some(KeyRow::Item {
                expand: Some(text), ..
            }) => {
                // Drill in: the query becomes the item's expand text
                // (e.g. "C:\dir\"); next frame re-syncs and re-matches.
                let text = text.clone();
                input.update(cx, |state, cx| state.set_value(text, window, cx));
            }
            _ => {
                // Plain item: toggle the description lines.
                let state = kind.state(self);
                state.expanded = !state.expanded;
            }
        }
    }

    fn preview_selected_theme(&mut self, kind: PaletteKind, rows: &[KeyRow], selected_idx: usize) {
        if kind != PaletteKind::Commands {
            return;
        }
        match rows.get(selected_idx) {
            Some(KeyRow::Item {
                action: Action::SetTheme(name),
                ..
            }) => self.process_actions([Action::PreviewTheme(name.clone())]),
            _ => self.restore_theme_preview(),
        }
    }
}

/// The bottom hint bar: result count on the left, key hints on the right.
/// `None` for `FooterSpec::None`.
fn palette_footer(
    footer: &FooterSpec,
    result_count: usize,
    colors: PaletteColors,
    window: &Window,
) -> Option<AnyElement> {
    let default_hints;
    let hints: &[FooterHint] = match footer {
        FooterSpec::None => return None,
        FooterSpec::Hints(hints) => hints,
        FooterSpec::Default => {
            default_hints = vec![
                FooterHint {
                    keys: vec![
                        ShortcutHint::Label("↑".into()),
                        ShortcutHint::Label("↓".into()),
                    ],
                    label: rust_i18n::t!("palette_footer_navigate").into(),
                },
                FooterHint {
                    keys: vec![ShortcutHint::Label("Enter".into())],
                    label: rust_i18n::t!("palette_footer_use").into(),
                },
                FooterHint {
                    keys: vec![ShortcutHint::Label("Tab".into())],
                    label: rust_i18n::t!("palette_footer_expand").into(),
                },
                FooterHint {
                    keys: vec![ShortcutHint::Label("Esc".into())],
                    label: rust_i18n::t!("palette_footer_close").into(),
                },
            ];
            &default_hints
        }
    };
    let mut clusters = h_flex().gap_3().items_center();
    for hint in hints {
        let mut cluster = h_flex().gap_1().items_center();
        for key in &hint.keys {
            cluster = cluster.children(shortcut_badge(
                key,
                false,
                colors.muted_bg,
                colors.muted,
                colors.muted,
                window,
                None,
            ));
        }
        cluster = cluster.child(hint.label.clone());
        clusters = clusters.child(cluster);
    }
    let bar = h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .gap_3()
        .px_3()
        .py_1()
        .border_t_1()
        .border_color(colors.border)
        .text_xs()
        .text_color(colors.muted)
        .child(div().child(rust_i18n::t!("palette_results_count", count = result_count)))
        .child(clusters);
    Some(bar.into_any_element())
}
