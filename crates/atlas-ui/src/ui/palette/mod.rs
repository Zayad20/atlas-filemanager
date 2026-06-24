//! Unified fuzzy-palette engine.
//!
//! Every searchable overlay (goto bar, command palette, extension palette) is
//! expressed as a declarative [`PaletteSpec`] — sections of items with icons,
//! descriptions, shortcut badges and per-item buttons — rendered by one shared
//! panel (`render.rs`) with one shared keyboard model: up/down to move, enter to
//! run, escape to dismiss, tab to toggle the description lines, click on a
//! section header to collapse it.
//!
//! The per-overlay builders live in `specs.rs`; neutral state (query, selection,
//! expansion, collapsed sections) lives in `atlas-model`'s `FuzzyOverlayState`.
#![allow(dead_code)] // reason: declarative engine API; variants exist for specs to use

mod render;
mod specs;

use crate::app::App;
use crate::input::action::Action;
use crate::state::overlay::FuzzyOverlayState;
use crate::ui::file_icons::FileIconRef;
use crate::ui::icons::AtlasIcon;
use gpui::{AnyElement, Keystroke, SharedString};

/// A complete palette: what to search, how rows look, and what surrounds them.
pub(crate) struct PaletteSpec {
    pub id: &'static str,
    pub kind: PaletteKind,
    pub sections: Vec<PaletteSection>,
    /// Rendered between the magnifier and the input (e.g. the goto bar's
    /// folder-scope pill while drilled into a folder).
    pub search_prefix: Option<AnyElement>,
    /// Extra controls rendered to the right of the search input (e.g. the goto
    /// bar's "Contenido" pill).
    pub search_buttons: Vec<AnyElement>,
    pub footer: FooterSpec,
    pub empty_text: SharedString,
}

/// A titled, optionally collapsible group of items. `title: None` renders a
/// flat header-less list.
pub(crate) struct PaletteSection {
    /// Stable key for `FuzzyOverlayState::collapsed_sections`.
    pub id: &'static str,
    pub icon: Option<AtlasIcon>,
    pub title: Option<SharedString>,
    pub collapsible: bool,
    pub items: Vec<PaletteItem>,
}

impl PaletteSection {
    /// A flat, header-less list (the whole palette is one anonymous section).
    pub(crate) fn flat(items: Vec<PaletteItem>) -> Self {
        Self {
            id: "",
            icon: None,
            title: None,
            collapsible: false,
            items,
        }
    }
}

/// One selectable row.
pub(crate) struct PaletteItem {
    pub icon: Option<AtlasIcon>,
    pub file_icon: Option<FileIconRef>,
    pub title: SharedString,
    /// Second line, muted. Shown for the selected row, or for every row while
    /// the palette is expanded (Tab).
    pub description: Option<SharedString>,
    /// Badges rendered at the right edge of the row.
    pub shortcuts: Vec<ShortcutHint>,
    /// Small icon buttons shown on hover/selection (star, plus, …).
    pub buttons: Vec<ItemButton>,
    pub action: Action,
    /// Tab "drills in": the search text is replaced with this (e.g. a folder
    /// path ending in a separator, so the fuzzy scope becomes that folder).
    pub expand: Option<SharedString>,
    /// Render the description on the same line as the title, separated by a
    /// chevron and always visible ("name › path") instead of as a second line.
    pub inline_description: bool,
    /// Byte ranges of `title` matched by the query, tinted accent in render.
    pub highlight: Option<Vec<std::ops::Range<usize>>>,
}

impl PaletteItem {
    pub(crate) fn new(title: impl Into<SharedString>, action: Action) -> Self {
        Self {
            icon: None,
            file_icon: None,
            title: title.into(),
            description: None,
            shortcuts: Vec::new(),
            buttons: Vec::new(),
            action,
            expand: None,
            inline_description: false,
            highlight: None,
        }
    }
}

/// How a shortcut badge resolves its text.
pub(crate) enum ShortcutHint {
    /// Look up the live keybinding registered for this gpui action in the
    /// "Atlas" context — stays correct if `lib.rs` bindings change.
    ForAction(Box<dyn gpui::Action>),
    /// A fixed keystroke (`Keystroke::parse`).
    Keys(Keystroke),
    /// Free-form badge text ("MouseMiddle", "↑").
    Label(SharedString),
    /// Command-palette shortcut badge. Click records a replacement; while
    /// recording, Delete/Backspace clears the binding and Esc cancels.
    CommandShortcut {
        command_id: String,
        key: Option<Keystroke>,
        recording: bool,
    },
}

impl ShortcutHint {
    /// Parse a fixed keystroke like `"ctrl-shift-p"`; `None` if unparseable.
    pub(crate) fn keys(s: &str) -> Option<Self> {
        Keystroke::parse(s).ok().map(Self::Keys)
    }
}

/// An icon button on a row, shown on hover/selection (always while `active`).
pub(crate) struct ItemButton {
    pub id: &'static str,
    pub icon: AtlasIcon,
    /// Hover tooltip text.
    pub tooltip: SharedString,
    /// Highlighted state (e.g. a filled star for a pinned command); an active
    /// button stays visible without hover.
    pub active: bool,
    pub action: Action,
    /// Whether clicking it also dismisses the palette.
    pub closes_palette: bool,
}

/// One "keys → label" hint in the bottom bar.
pub(crate) struct FooterHint {
    pub keys: Vec<ShortcutHint>,
    pub label: SharedString,
}

pub(crate) enum FooterSpec {
    /// "↑ ↓ navegar · Enter usar · Esc cerrar · Tab detalles".
    Default,
    Hints(Vec<FooterHint>),
    None,
}

/// Discriminator routing the shared panel to the right `FuzzyOverlayState` and
/// close `Action`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PaletteKind {
    Goto,
    Commands,
    Ext,
}

impl PaletteKind {
    pub(crate) const fn state(self, app: &mut App) -> &mut FuzzyOverlayState {
        match self {
            Self::Goto => &mut app.state.overlay.goto_state,
            Self::Commands => &mut app.state.overlay.palette_state,
            Self::Ext => &mut app.state.overlay.ext_palette_state,
        }
    }

    pub(crate) const fn close_action(self) -> Action {
        match self {
            Self::Goto | Self::Ext => Action::CloseOverlays,
            Self::Commands => Action::ClosePalette,
        }
    }
}
