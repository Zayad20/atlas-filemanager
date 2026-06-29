//! Per-palette `PaletteSpec` builders.

use super::{
    FooterSpec, ItemButton, PaletteItem, PaletteKind, PaletteSection, PaletteSpec, ShortcutHint,
};
use crate::app::App;
use crate::input::action::Action;
use crate::state::pane::PaneId;
use crate::state::settings::ViewMode;
use crate::ui::icons::AtlasIcon;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, MouseButton};
use gpui_component::ActiveTheme;
use std::path::PathBuf;

/// How many matches to pull from the streaming index per frame.
const GOTO_LIMIT: usize = 100;
/// Per-section cap for goto results (Carpetas / Archivos).
const GOTO_SECTION_LIMIT: usize = 25;
/// Per-category cap for the goto's empty-query home sections.
const GOTO_HOME_LIMIT: usize = 8;

/// One command-palette entry: a stable id (for pinning), display strings, and
/// both the runtime `Action` to run and the gpui action whose live keybinding
/// is shown as the shortcut badge.
struct CommandDef {
    id: String,
    title: String,
    description: std::borrow::Cow<'static, str>,
    icon: AtlasIcon,
    action: Action,
}

impl CommandDef {
    fn new(
        id: &str,
        title: impl Into<String>,
        description: impl Into<std::borrow::Cow<'static, str>>,
        icon: AtlasIcon,
        action: Action,
    ) -> Self {
        Self {
            id: id.to_string(),
            title: title.into(),
            description: description.into(),
            icon,
            action,
        }
    }

    /// Convert into a palette row: star button pins/unpins, badge shows the
    /// live keybinding for the associated gpui action.
    fn item(
        &self,
        pinned: bool,
        shortcut: Option<gpui::Keystroke>,
        recording: bool,
    ) -> PaletteItem {
        let mut item = PaletteItem::new(self.title.clone(), self.action.clone());
        item.icon = Some(self.icon);
        if !self.description.is_empty() {
            item.description = Some(self.description.clone().into());
        }
        if crate::input::shortcuts::has_default_command_shortcut(&self.id) {
            item.shortcuts.push(ShortcutHint::CommandShortcut {
                command_id: self.id.clone(),
                key: shortcut,
                recording,
            });
        }
        item.buttons.push(ItemButton {
            id: "pin",
            icon: if pinned {
                AtlasIcon::StarFill
            } else {
                AtlasIcon::Star
            },
            tooltip: if pinned {
                rust_i18n::t!("palette_unpin_quick").into()
            } else {
                rust_i18n::t!("palette_pin_quick").into()
            },
            active: pinned,
            action: Action::ToggleCommandPin(self.id.clone()),
            closes_palette: false,
        });
        item
    }
}

/// A goto row for `path`: navigate on Enter, bookmark/new-tab side buttons,
/// description = full path. Directories (`expandable`) drill in with Tab: the
/// query becomes "<path><sep>" and the fuzzy scope moves into that folder.
fn goto_entry(
    title: String,
    path: PathBuf,
    icon: AtlasIcon,
    expandable: bool,
    tab: crate::state::pane::TabId,
    pane: PaneId,
) -> PaletteItem {
    let mut item = PaletteItem::new(
        title.clone(),
        Action::Navigate {
            tab,
            path: path.clone(),
        },
    );
    item.icon = Some(icon);
    item.description = Some(path.to_string_lossy().into_owned().into());
    // Files read as one line: "name › path" with the path always visible.
    item.inline_description = true;
    if expandable {
        item.expand = Some(format!("{}{}", path.display(), std::path::MAIN_SEPARATOR).into());
    }
    item.file_icon = Some(App::file_icon_ref_for_path(path.clone(), expandable));
    item.buttons = vec![
        ItemButton {
            id: "bookmark",
            icon: AtlasIcon::Star,
            tooltip: rust_i18n::t!("palette_add_bookmark").into(),
            active: false,
            action: Action::AddBookmark {
                name: title,
                path: path.clone(),
            },
            closes_palette: false,
        },
        ItemButton {
            id: "new-tab",
            icon: AtlasIcon::Plus,
            tooltip: rust_i18n::t!("palette_open_new_tab").into(),
            active: false,
            action: Action::NewTabWithPath { pane, path },
            closes_palette: true,
        },
    ];
    item
}

/// `goto_entry` with the file name as the title and a heuristic icon (an
/// extension usually means a file; no fs metadata on the render path).
fn goto_item(path: PathBuf, tab: crate::state::pane::TabId, pane: PaneId) -> PaletteItem {
    let name = path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let is_dir_guess = path.extension().is_none();
    let icon = if is_dir_guess {
        AtlasIcon::Folder
    } else {
        AtlasIcon::File
    };
    goto_entry(name, path, icon, is_dir_guess, tab, pane)
}

/// `goto_entry` for an indexed result, where `is_dir` is known from the walk.
fn goto_index_item(
    entry: crate::search::nucleo_pool::IndexEntry,
    tab: crate::state::pane::TabId,
    pane: PaneId,
) -> PaletteItem {
    let name = entry.path.file_name().map_or_else(
        || entry.path.to_string_lossy().into_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let icon = if entry.is_dir {
        AtlasIcon::Folder
    } else {
        AtlasIcon::File
    };
    goto_entry(name, entry.path, icon, entry.is_dir, tab, pane)
}

/// The folder pill shown left of the input while the search is scoped to a
/// drilled-into folder.
fn goto_scope_pill(base: &std::path::Path, cx: &Context<App>) -> gpui::AnyElement {
    let name = base.file_name().map_or_else(
        || base.to_string_lossy().into_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let (muted, border) = {
        let t = cx.theme();
        (t.muted_foreground, t.border)
    };
    gpui::prelude::IntoElement::into_any_element(
        gpui_component::h_flex()
            .flex_shrink_0()
            .gap_1()
            .px_2()
            .py(px(2.))
            .rounded_md()
            .border_1()
            .border_color(border)
            .text_xs()
            .text_color(muted)
            .child(crate::ui::icons::icon(AtlasIcon::Folder, 12.0, muted))
            .child(name),
    )
}

/// If the query starts with an absolute path, split it at the last separator
/// into (base dir, needle): "C:\dev\atl" → ("C:\dev\", "atl"). The fuzzy
/// index is then scoped to the base dir.
fn split_query_base(query: &str) -> Option<(PathBuf, String)> {
    let q = query.trim_start();
    let bytes = q.as_bytes();
    let absolute = q.starts_with('/')
        || q.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/'));
    if !absolute {
        return None;
    }
    let pos = q.rfind(['/', '\\'])?;
    let (base, rest) = q.split_at(pos + 1);
    Some((PathBuf::from(base), rest.to_string()))
}

/// Wrap `items` in one titled section, or no sections when empty (so the panel
/// shows its empty placeholder instead of a lone header).
fn section_or_empty(
    id: &'static str,
    icon: AtlasIcon,
    title: impl Into<gpui::SharedString>,
    items: Vec<PaletteItem>,
) -> Vec<PaletteSection> {
    if items.is_empty() {
        Vec::new()
    } else {
        vec![PaletteSection {
            id,
            icon: Some(icon),
            title: Some(title.into()),
            collapsible: true,
            items,
        }]
    }
}

impl App {
    /// Goto bar: fuzzy folder/file navigation. Empty query → sidebar-like home
    /// sections (Recientes/Almacenamiento/Marcadores/Lugares); content mode →
    /// files whose contents matched the background grep; otherwise the
    /// streaming nucleo index, optionally scoped to a folder when the query
    /// starts with an absolute path ("C:\dev\" + needle).
    pub(crate) fn goto_spec(&mut self, cx: &mut Context<Self>) -> PaletteSpec {
        let focused_tile = self.state.focused_tile;
        let pane = self
            .state
            .find_tabs_container(focused_tile)
            .or_else(|| self.state.find_any_tabs_container())
            .unwrap_or(focused_tile);
        let content_mode = self.state.profiles.active().search_in_content;
        let query = self.state.overlay.goto_state.query.clone();
        let trimmed = query.trim();

        let mut search_prefix: Option<gpui::AnyElement> = None;
        let sections = if trimmed.is_empty() {
            self.goto_home_sections(focused_tile, pane)
        } else if content_mode {
            // Files whose *contents* matched the latest grep.
            let items: Vec<PaletteItem> = self
                .content_results
                .iter()
                .take(GOTO_LIMIT)
                .map(|p| {
                    let name = p.file_name().map_or_else(
                        || p.to_string_lossy().into_owned(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    goto_entry(
                        name,
                        p.clone(),
                        AtlasIcon::TextFile,
                        false,
                        focused_tile,
                        pane,
                    )
                })
                .collect();
            section_or_empty(
                "content",
                AtlasIcon::TextFile,
                rust_i18n::t!("palette_sec_content"),
                items,
            )
        } else {
            // Scoped fuzzy: an absolute-path prefix moves the index root into
            // that folder (relative-path haystacks, files included) and
            // matches the remainder inside it.
            let scoped = split_query_base(trimmed);
            let (root, needle) = scoped
                .clone()
                .unwrap_or_else(|| (self.goto_default_root(), trimmed.to_string()));
            if let Some((base, _)) = &scoped {
                search_prefix = Some(goto_scope_pill(base, cx));
            }
            self.ensure_goto_index(root, scoped.is_some());
            self.nucleo.set_pattern(&needle);
            self.nucleo.tick();
            let entries = self.nucleo.results(GOTO_LIMIT);

            // Name-first re-rank: nucleo scored the whole (relative) path, so
            // a strong match buried in a parent segment can outrank a weak one
            // in the file name. Entries whose *name* matches the needle come
            // first (best name score wins); pure path matches keep their
            // nucleo order after them.
            let names: Vec<String> = entries
                .iter()
                .map(|e| {
                    e.path.file_name().map_or_else(
                        || e.path.to_string_lossy().into_owned(),
                        |n| n.to_string_lossy().into_owned(),
                    )
                })
                .collect();
            let name_scores = self
                .fuzzy_scorer
                .scores(&needle, names.iter().map(String::as_str));
            let mut order: Vec<usize> = (0..entries.len()).collect();
            order.sort_by(|&a, &b| match (name_scores[a], name_scores[b]) {
                (Some(x), Some(y)) => y.cmp(&x).then(a.cmp(&b)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.cmp(&b),
            });

            // The category holding the overall best match leads.
            let files_first = order
                .first()
                .is_some_and(|&i| entries.get(i).is_some_and(|e| !e.is_dir));

            // Each category capped — and the matched characters of the name
            // tinted for scanability.
            let mut folders: Vec<PaletteItem> = Vec::new();
            let mut files: Vec<PaletteItem> = Vec::new();
            for idx in order {
                let entry = entries[idx].clone();
                let bucket = if entry.is_dir {
                    &mut folders
                } else {
                    &mut files
                };
                if bucket.len() >= GOTO_SECTION_LIMIT {
                    if folders.len() >= GOTO_SECTION_LIMIT && files.len() >= GOTO_SECTION_LIMIT {
                        break;
                    }
                    continue;
                }
                let mut item = goto_index_item(entry, focused_tile, pane);
                item.highlight = self.fuzzy_scorer.match_ranges(&needle, &item.title);
                bucket.push(item);
            }
            let dirs_sec = section_or_empty(
                "dirs",
                AtlasIcon::Folder,
                rust_i18n::t!("palette_sec_folders"),
                folders,
            );
            let files_sec = section_or_empty(
                "files",
                AtlasIcon::File,
                rust_i18n::t!("palette_sec_files"),
                files,
            );
            let mut sections = Vec::new();
            if files_first {
                sections.extend(files_sec);
                sections.extend(dirs_sec);
            } else {
                sections.extend(dirs_sec);
                sections.extend(files_sec);
            }
            sections
        };

        PaletteSpec {
            id: "goto-bar",
            kind: PaletteKind::Goto,
            sections,
            search_prefix,
            search_buttons: vec![self.content_toggle_pill(content_mode, cx)],
            footer: FooterSpec::Default,
            empty_text: rust_i18n::t!("palette_no_results").into(),
        }
    }

    /// The default fuzzy scope: the focused tab's directory (home as fallback).
    fn goto_default_root(&self) -> PathBuf {
        self.state
            .active_tab_data()
            .and_then(|td| td.location.fs_dir().map(std::path::Path::to_path_buf))
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    /// Re-point the streaming nucleo index at `root` if it isn't there already
    /// (drill-in/out re-indexes; same-root keystrokes don't). A scoped index
    /// walks only that folder (files included, relative haystacks); the
    /// unscoped one is the global folder index.
    fn ensure_goto_index(&mut self, root: PathBuf, scoped: bool) {
        if self.goto_index_root.as_ref() == Some(&(root.clone(), scoped)) {
            return;
        }
        self.nucleo.reset();
        let injector = self.nucleo.injector();
        if scoped {
            crate::fs::index::build_index_scoped(root.clone(), injector);
        } else {
            crate::fs::index::build_index(root.clone(), injector);
        }
        self.goto_index_root = Some((root, scoped));
    }

    /// Empty-query home view, mirroring the sidebar: Recientes, Almacenamiento
    /// (drives), Marcadores (profile bookmarks) and Lugares (user dirs).
    fn goto_home_sections(
        &self,
        tab: crate::state::pane::TabId,
        pane: PaneId,
    ) -> Vec<PaletteSection> {
        let mut sections: Vec<PaletteSection> = Vec::new();

        // Recientes (most-recent first, excluding the current dir).
        let cur = self
            .state
            .active_tab_data()
            .and_then(|td| td.location.fs_dir().map(std::path::Path::to_path_buf));
        let recent: Vec<PaletteItem> = self
            .state
            .settings
            .recent_folders
            .iter()
            .rev()
            .filter(|p| cur.as_deref() != Some(p.as_path()))
            .take(GOTO_HOME_LIMIT)
            .map(|p| goto_item(p.clone(), tab, pane))
            .collect();
        sections.extend(section_or_empty(
            "recent",
            AtlasIcon::ClockCounterClockwise,
            rust_i18n::t!("palette_sec_recent"),
            recent,
        ));

        // Almacenamiento: mounted drives (cached; same source as the sidebar).
        let drives: Vec<PaletteItem> = crate::fs::mounts::list_drives()
            .into_iter()
            .take(GOTO_HOME_LIMIT)
            .map(|d| {
                goto_entry(
                    d.label.clone(),
                    d.mount_point,
                    AtlasIcon::HardDrives,
                    true,
                    tab,
                    pane,
                )
            })
            .collect();
        sections.extend(section_or_empty(
            "storage",
            AtlasIcon::HardDrives,
            rust_i18n::t!("palette_sec_storage"),
            drives,
        ));

        // Marcadores del perfil activo.
        let bookmarks: Vec<PaletteItem> = self
            .state
            .profiles
            .active()
            .bookmarks
            .iter()
            .take(GOTO_HOME_LIMIT)
            .map(|b| {
                goto_entry(
                    b.name.clone(),
                    b.path.clone(),
                    AtlasIcon::BookmarkSimple,
                    true,
                    tab,
                    pane,
                )
            })
            .collect();
        sections.extend(section_or_empty(
            "bookmarks",
            AtlasIcon::BookmarkSimple,
            rust_i18n::t!("palette_sec_bookmarks"),
            bookmarks,
        ));

        // Lugares: user dirs (same set/icons as the sidebar).
        let places: Vec<PaletteItem> = [
            (
                AtlasIcon::House,
                rust_i18n::t!("palette_places_home"),
                dirs::home_dir(),
            ),
            (
                AtlasIcon::Monitor,
                rust_i18n::t!("palette_places_desktop"),
                dirs::desktop_dir(),
            ),
            (
                AtlasIcon::TextFile,
                rust_i18n::t!("palette_places_documents"),
                dirs::document_dir(),
            ),
            (
                AtlasIcon::Download,
                rust_i18n::t!("palette_places_downloads"),
                dirs::download_dir(),
            ),
            (
                AtlasIcon::Image,
                rust_i18n::t!("palette_places_pictures"),
                dirs::picture_dir(),
            ),
            (
                AtlasIcon::Music,
                rust_i18n::t!("palette_places_music"),
                dirs::audio_dir(),
            ),
            (
                AtlasIcon::Video,
                rust_i18n::t!("palette_places_videos"),
                dirs::video_dir(),
            ),
        ]
        .into_iter()
        .filter_map(|(ic, label, p)| {
            p.map(|p| goto_entry(label.to_string(), p, ic, true, tab, pane))
        })
        .collect();
        sections.extend(section_or_empty(
            "places",
            AtlasIcon::House,
            rust_i18n::t!("palette_sec_places"),
            places,
        ));

        sections
    }

    /// The "search in content" pill at the right of the goto search row.
    /// Toggles the active profile's `search_in_content`, persists it, and
    /// restarts the search so the results switch modes immediately.
    fn content_toggle_pill(&mut self, active: bool, cx: &mut Context<Self>) -> AnyElement {
        let (accent, accent_fg, border, muted) = {
            let t = cx.theme();
            (t.accent, t.accent_foreground, t.border, t.muted_foreground)
        };
        let view = cx.entity();
        div()
            .id("goto-content-toggle")
            .px_2()
            .py(px(2.))
            .rounded_md()
            .border_1()
            .border_color(if active { accent } else { border })
            .when(active, |s| s.bg(accent).text_color(accent_fg))
            .when(!active, |s| s.text_color(muted))
            .cursor_pointer()
            .text_xs()
            .child(rust_i18n::t!("palette_btn_content"))
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new(
                    rust_i18n::t!("palette_btn_content_tooltip").to_string(),
                )
                .build(window, cx)
            })
            .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                view.update(cx, |this, cx| {
                    let now = !this.state.profiles.active().search_in_content;
                    this.state.profiles.active_mut().search_in_content = now;
                    crate::util::config::save_profiles(&this.state.profiles);
                    // Reset results and relaunch in the new mode.
                    this.relaunch_content_search();
                    this.state.overlay.goto_state.selected_idx = 0;
                    cx.notify();
                });
            })
            .into_any_element()
    }

    /// Extension palette: a flat pick-list of strings provided by a WASM
    /// extension; the chosen label is sent back over the reply channel.
    pub(crate) fn ext_palette_spec(&mut self) -> PaletteSpec {
        let query = self.state.overlay.ext_palette_state.query.clone();
        let labels = self.state.overlay.ext_palette_items.clone();
        let mut items: Vec<PaletteItem> = self
            .fuzzy_scorer
            .rank(&query, labels.iter().map(String::as_str))
            .into_iter()
            .map(|i| {
                let mut item = PaletteItem::new(
                    labels[i].clone(),
                    Action::SubmitExtPalette(labels[i].clone()),
                );
                item.icon = Some(AtlasIcon::Palette);
                item
            })
            .collect();
        for item in &mut items {
            item.highlight = self.fuzzy_scorer.match_ranges(&query, &item.title);
        }
        PaletteSpec {
            id: "ext-palette",
            kind: PaletteKind::Ext,
            sections: vec![PaletteSection::flat(items)],
            search_prefix: None,
            search_buttons: Vec::new(),
            footer: FooterSpec::Default,
            empty_text: rust_i18n::t!("palette_no_results").into(),
        }
    }

    /// Command palette: every app command grouped into sections, with icons,
    /// descriptions, live shortcut badges, and a pinned "Quick" section on top.
    /// A non-empty query fuzzy-ranks within each section, drops empty ones and
    /// re-orders the sections themselves by their best-matching item, so the
    /// most relevant category surfaces first.
    pub(crate) fn command_palette_spec(&mut self, cx: &gpui::App) -> PaletteSpec {
        let groups = self.command_groups(cx);
        let pinned = self.state.profiles.active().pinned_commands.clone();
        let query = self.state.overlay.palette_state.query.clone();
        let searching = !query.trim().is_empty();

        // (best item score, declaration order, section) — declaration order is
        // the tie-breaker and the whole order when not searching.
        let mut scored: Vec<(u32, usize, PaletteSection)> = Vec::new();

        // "Quick": the pinned commands, in the order they were pinned.
        let quick_defs: Vec<&CommandDef> = pinned
            .iter()
            .filter_map(|id| {
                groups
                    .iter()
                    .flat_map(|(_, _, _, defs)| defs.iter())
                    .find(|def| def.id == *id)
            })
            .collect();
        if let Some((best, items)) = self.scored_command_items(&quick_defs, true, &pinned, &query) {
            scored.push((
                best,
                0,
                PaletteSection {
                    id: "quick",
                    icon: Some(AtlasIcon::Lightning),
                    title: Some(rust_i18n::t!("palette_sec_quick").into()),
                    collapsible: true,
                    items,
                },
            ));
        }

        for (decl, (id, title, icon, defs)) in groups.into_iter().enumerate() {
            let refs: Vec<&CommandDef> = defs.iter().collect();
            let icon = icon.clone();
            let title = title.clone();
            let id = id;
            if let Some((best, items)) = self.scored_command_items(&refs, false, &pinned, &query) {
                scored.push((
                    best,
                    decl + 1,
                    PaletteSection {
                        id,
                        icon: Some(icon),
                        title: Some(title.into()),
                        collapsible: true,
                        items,
                    },
                ));
            }
        }

        if searching {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        let sections: Vec<PaletteSection> = scored.into_iter().map(|(_, _, s)| s).collect();

        PaletteSpec {
            id: "command-palette",
            kind: PaletteKind::Commands,
            sections,
            search_prefix: None,
            search_buttons: Vec::new(),
            footer: FooterSpec::Default,
            empty_text: rust_i18n::t!("palette_no_results").into(),
        }
    }

    /// Score `defs` against `query`: matching items as palette rows (best
    /// first, highlights applied) plus the section's best score for category
    /// re-ordering. `None` when nothing matches. An empty query keeps the
    /// declared order with score 0.
    fn scored_command_items(
        &mut self,
        defs: &[&CommandDef],
        force_pinned: bool,
        pinned: &[String],
        query: &str,
    ) -> Option<(u32, Vec<PaletteItem>)> {
        let searching = !query.trim().is_empty();
        let scores = self
            .fuzzy_scorer
            .scores(query, defs.iter().map(|d| d.title.as_str()));
        let order: Vec<usize> = if searching {
            let mut v: Vec<usize> = (0..defs.len()).filter(|&i| scores[i].is_some()).collect();
            v.sort_by(|&a, &b| scores[b].cmp(&scores[a]).then(a.cmp(&b)));
            v
        } else {
            (0..defs.len()).collect()
        };
        if order.is_empty() {
            return None;
        }
        let best = order.first().and_then(|&i| scores[i]).unwrap_or(0);
        let items: Vec<PaletteItem> = order
            .into_iter()
            .map(|i| {
                let def = defs[i];
                let shortcut = self.command_shortcut_keystroke(&def.id);
                let recording = self
                    .command_shortcut_capture
                    .as_ref()
                    .is_some_and(|id| id == &def.id);
                let mut item = def.item(
                    force_pinned || pinned.contains(&def.id),
                    shortcut,
                    recording,
                );
                item.highlight = self.fuzzy_scorer.match_ranges(query, &item.title);
                item
            })
            .collect();
        Some((best, items))
    }

    /// The command table: `(section id, section title, section icon, commands)`.
    #[allow(clippy::type_complexity)] // local table shape, read once below
    fn command_groups(
        &self,
        cx: &gpui::App,
    ) -> Vec<(&'static str, gpui::SharedString, AtlasIcon, Vec<CommandDef>)> {
        let focused = self.state.focused_tile;

        let tabs = vec![
            CommandDef::new(
                "tab.new",
                rust_i18n::t!("cmd_tab_new_title"),
                rust_i18n::t!("cmd_tab_new_desc"),
                AtlasIcon::Plus,
                Action::NewTabFocused,
            ),
            CommandDef::new(
                "tab.close",
                rust_i18n::t!("cmd_tab_close_title"),
                rust_i18n::t!("cmd_tab_close_desc"),
                AtlasIcon::X,
                Action::CloseFocusedTab,
            ),
            CommandDef::new(
                "pane.split-h",
                rust_i18n::t!("cmd_pane_split_h_title"),
                rust_i18n::t!("cmd_pane_split_h_desc"),
                AtlasIcon::SplitHorizontal,
                Action::SplitHorizontalFocused,
            ),
            CommandDef::new(
                "pane.split-v",
                rust_i18n::t!("cmd_pane_split_v_title"),
                rust_i18n::t!("cmd_pane_split_v_desc"),
                AtlasIcon::SplitVertical,
                Action::SplitVerticalFocused,
            ),
            CommandDef::new(
                "pane.close",
                rust_i18n::t!("cmd_pane_close_title"),
                rust_i18n::t!("cmd_pane_close_desc"),
                AtlasIcon::X,
                Action::CloseFocusedPane,
            ),
        ];

        let nav = vec![
            CommandDef::new(
                "nav.back",
                rust_i18n::t!("cmd_nav_back_title"),
                rust_i18n::t!("cmd_nav_back_desc"),
                AtlasIcon::ArrowLeft,
                Action::NavigateBack(focused),
            ),
            CommandDef::new(
                "nav.forward",
                rust_i18n::t!("cmd_nav_forward_title"),
                rust_i18n::t!("cmd_nav_forward_desc"),
                AtlasIcon::ArrowRight,
                Action::NavigateForward(focused),
            ),
            CommandDef::new(
                "nav.goto",
                rust_i18n::t!("cmd_nav_goto_title"),
                rust_i18n::t!("cmd_nav_goto_desc"),
                AtlasIcon::Search,
                Action::OpenGotoBar,
            ),
        ];

        let view = vec![
            CommandDef::new(
                "view.hidden",
                rust_i18n::t!("cmd_view_hidden_title"),
                rust_i18n::t!("cmd_view_hidden_desc"),
                AtlasIcon::Eye,
                Action::ToggleHidden,
            ),
            CommandDef::new(
                "view.inspector",
                rust_i18n::t!("cmd_view_inspector_title"),
                rust_i18n::t!("cmd_view_inspector_desc"),
                AtlasIcon::Columns,
                Action::ToggleInspector,
            ),
            CommandDef::new(
                "view.sidebar",
                rust_i18n::t!("cmd_view_sidebar_title"),
                rust_i18n::t!("cmd_view_sidebar_desc"),
                AtlasIcon::List,
                Action::ToggleSidebar,
            ),
            CommandDef::new(
                "view.details",
                rust_i18n::t!("cmd_view_details_title"),
                rust_i18n::t!("cmd_view_details_desc"),
                AtlasIcon::Details,
                Action::SetViewMode(ViewMode::Details),
            ),
            CommandDef::new(
                "view.grid",
                rust_i18n::t!("cmd_view_grid_title"),
                rust_i18n::t!("cmd_view_grid_desc"),
                AtlasIcon::Grid,
                Action::SetViewMode(ViewMode::Grid),
            ),
            CommandDef::new(
                "view.list",
                rust_i18n::t!("cmd_view_list_title"),
                rust_i18n::t!("cmd_view_list_desc"),
                AtlasIcon::List,
                Action::SetViewMode(ViewMode::List),
            ),
            CommandDef::new(
                "view.miller",
                rust_i18n::t!("cmd_view_miller_title"),
                rust_i18n::t!("cmd_view_miller_desc"),
                AtlasIcon::Columns,
                Action::SetViewMode(ViewMode::Miller),
            ),
            CommandDef::new(
                "view.zoom-in",
                rust_i18n::t!("cmd_zoom_in_title"),
                rust_i18n::t!("cmd_zoom_in_desc"),
                AtlasIcon::Plus,
                Action::ZoomIn,
            ),
            CommandDef::new(
                "view.zoom-out",
                rust_i18n::t!("cmd_zoom_out_title"),
                rust_i18n::t!("cmd_zoom_out_desc"),
                AtlasIcon::ArrowCounterClockwise,
                Action::ZoomOut,
            ),
            CommandDef::new(
                "view.zoom-reset",
                rust_i18n::t!("cmd_zoom_reset_title"),
                rust_i18n::t!("cmd_zoom_reset_desc"),
                AtlasIcon::ArrowClockwise,
                Action::ZoomReset,
            ),
        ];

        let edit = vec![
            CommandDef::new(
                "edit.select-all",
                rust_i18n::t!("cmd_edit_select_all_title"),
                rust_i18n::t!("cmd_edit_select_all_desc"),
                AtlasIcon::SelectionAll,
                Action::SelectAll,
            ),
            CommandDef::new(
                "edit.invert",
                rust_i18n::t!("cmd_edit_invert_title"),
                rust_i18n::t!("cmd_edit_invert_desc"),
                AtlasIcon::SelectionInverse,
                Action::InvertSelection,
            ),
            CommandDef::new(
                "edit.undo",
                rust_i18n::t!("cmd_edit_undo_title"),
                rust_i18n::t!("cmd_edit_undo_desc"),
                AtlasIcon::ArrowCounterClockwise,
                Action::Undo,
            ),
            CommandDef::new(
                "edit.redo",
                rust_i18n::t!("cmd_edit_redo_title"),
                rust_i18n::t!("cmd_edit_redo_desc"),
                AtlasIcon::ArrowClockwise,
                Action::Redo,
            ),
            CommandDef::new(
                "edit.batch-rename",
                rust_i18n::t!("cmd_edit_batch_rename_title"),
                rust_i18n::t!("cmd_edit_batch_rename_desc"),
                AtlasIcon::PencilSimple,
                Action::OpenBatchRename,
            ),
        ];

        let selected_paths = self.state.selected_paths();
        let files = if selected_paths.is_empty() {
            Vec::new()
        } else {
            vec![
                CommandDef::new(
                    "files.trash",
                    rust_i18n::t!("cmd_files_trash_title"),
                    rust_i18n::t!("cmd_files_trash_desc"),
                    AtlasIcon::Trash,
                    Action::Delete {
                        paths: selected_paths.clone(),
                        to_trash: true,
                    },
                ),
                CommandDef::new(
                    "files.delete",
                    rust_i18n::t!("cmd_files_delete_title"),
                    rust_i18n::t!("cmd_files_delete_desc"),
                    AtlasIcon::Trash,
                    Action::Delete {
                        paths: selected_paths,
                        to_trash: false,
                    },
                ),
            ]
        };

        let mut profiles: Vec<CommandDef> = self
            .state
            .profiles
            .profiles
            .iter()
            .enumerate()
            .map(|(idx, profile)| {
                CommandDef::new(
                    &format!("profile:{}", profile.name),
                    rust_i18n::t!("cmd_profile_switch_title", name = profile.name.clone()),
                    rust_i18n::t!("cmd_profile_switch_desc"),
                    AtlasIcon::UserSwitch,
                    Action::SwitchProfile(idx),
                )
            })
            .collect();
        // Management commands: create a fresh profile (auto-named, inherits the
        // current view) and delete the active one.
        profiles.push(CommandDef::new(
            "profile.new",
            rust_i18n::t!("cmd_profile_new_title"),
            rust_i18n::t!("cmd_profile_new_desc"),
            AtlasIcon::UserSwitch,
            Action::CreateProfile(String::new()),
        ));
        profiles.push(CommandDef::new(
            "profile.rename",
            rust_i18n::t!(
                "cmd_profile_rename_title",
                name = self.state.profiles.active().name.clone()
            ),
            rust_i18n::t!("cmd_profile_rename_desc"),
            AtlasIcon::PencilSimple,
            Action::OpenRenameProfile,
        ));
        if self.state.profiles.profiles.len() > 1 {
            profiles.push(CommandDef::new(
                "profile.delete",
                rust_i18n::t!(
                    "cmd_profile_delete_title",
                    name = self.state.profiles.active().name.clone()
                ),
                rust_i18n::t!("cmd_profile_delete_desc"),
                AtlasIcon::Trash,
                Action::DeleteActiveProfile,
            ));
        }

        let themes: Vec<CommandDef> = gpui_component::ThemeRegistry::global(cx)
            .sorted_themes()
            .iter()
            .map(|cfg| {
                let name = cfg.name.to_string();
                CommandDef::new(
                    &format!("theme:{name}"),
                    rust_i18n::t!("cmd_theme_switch_title", name = name.clone()),
                    rust_i18n::t!("cmd_theme_switch_desc"),
                    AtlasIcon::Palette,
                    Action::SetTheme(name),
                )
            })
            .collect();

        let mut ext = vec![CommandDef::new(
            "ext.manager",
            rust_i18n::t!("ext_manager_title"),
            rust_i18n::t!("ext_manager_desc"),
            AtlasIcon::PuzzlePiece,
            Action::OpenExtManager,
        )];
        // Dynamically register every enabled non-builtin, non-thumbnailer command
        // with prefix `ext.<ext_id>.<cmd_id>` so they appear in the palette.
        for extension in self.ext_registry.extensions.iter().filter(|e| e.enabled) {
            if extension.manifest.hidden
                || matches!(
                    extension.manifest.kind,
                    crate::state::ext_model::ExtKind::Builtin
                        | crate::state::ext_model::ExtKind::ToolProvider
                )
            {
                continue;
            }
            for cmd in extension
                .manifest
                .commands
                .iter()
                .filter(|c| !c.thumbnail && !c.preview)
            {
                ext.push(CommandDef::new(
                    &format!("ext.{}.{}", extension.manifest.id, cmd.id),
                    cmd.title.clone(),
                    extension.manifest.name.clone(),
                    AtlasIcon::PuzzlePiece,
                    Action::Ext(crate::app::ext::ExtAction::Run {
                        ext_id: extension.manifest.id.clone(),
                        cmd_id: cmd.id.clone(),
                    }),
                ));
            }
        }

        let system = vec![CommandDef::new(
            "sys.settings",
            rust_i18n::t!("cmd_sys_settings_title"),
            rust_i18n::t!("cmd_sys_settings_desc"),
            AtlasIcon::Gear,
            Action::OpenSettings,
        )];

        vec![
            (
                "tabs",
                rust_i18n::t!("palette_cat_tabs").into(),
                AtlasIcon::SplitHorizontal,
                tabs,
            ),
            (
                "nav",
                rust_i18n::t!("palette_cat_nav").into(),
                AtlasIcon::ArrowsLeftRight,
                nav,
            ),
            (
                "view",
                rust_i18n::t!("palette_cat_view").into(),
                AtlasIcon::Eye,
                view,
            ),
            (
                "edit",
                rust_i18n::t!("palette_cat_edit").into(),
                AtlasIcon::PencilSimple,
                edit,
            ),
            (
                "files",
                rust_i18n::t!("palette_cat_files").into(),
                AtlasIcon::Trash,
                files,
            ),
            (
                "profiles",
                rust_i18n::t!("palette_cat_profiles").into(),
                AtlasIcon::UserSwitch,
                profiles,
            ),
            (
                "themes",
                rust_i18n::t!("palette_cat_themes").into(),
                AtlasIcon::Palette,
                themes,
            ),
            (
                "ext",
                rust_i18n::t!("palette_cat_ext").into(),
                AtlasIcon::PuzzlePiece,
                ext,
            ),
            (
                "system",
                rust_i18n::t!("palette_cat_system").into(),
                AtlasIcon::Gear,
                system,
            ),
        ]
    }
}
