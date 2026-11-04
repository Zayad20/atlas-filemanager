//! Immediate-mode Settings Overlay.
//!
//! Provides a single-cycle O(1) arithmetic layout with `RectCut`, immediate
//! reactive settings updates, switch toggles, segmented selectors, sliders,
//! searchable keybindings table, interactive tooltips, and keyboard dismiss.

use crate::app::App as AtlasApp;
use crate::input::action::Action;
use crate::ui::imm::cut::RectCut;
use crate::ui::icons::AtlasIcon;
use crate::ui::surface::{
    chrome_ui_font, line_width, paint_fill, paint_rounded, paint_theme_icon, shape_text,
    shape_text_truncated, Slot,
};
use atlas_model::settings::{InspectorPlacement, ViewMode};
use gpui::ColorExt as _;
use gpui::{
    point, px, size, AnyElement, App, Bounds, BoxShadow, Corners, DispatchPhase, Element,
    ElementId, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement,
    KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, Pixels, Refineable, ScrollWheelEvent,
    Style, StyleRefinement, Styled, TextAlign, WeakEntity, Window,
};
use gpui_component::ActiveTheme;

/// Navigation sections for the settings overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingsTab {
    General = 0,
    Appearance = 1,
    Files = 2,
    Layout = 3,
    Extensions = 4,
    About = 5,
}

impl SettingsTab {
    pub const ALL: [SettingsTab; 6] = [
        SettingsTab::General,
        SettingsTab::Appearance,
        SettingsTab::Files,
        SettingsTab::Layout,
        SettingsTab::Extensions,
        SettingsTab::About,
    ];

    pub fn from_index(idx: usize) -> Self {
        match idx {
            1 => SettingsTab::Appearance,
            2 => SettingsTab::Files,
            3 => SettingsTab::Layout,
            4 => SettingsTab::Extensions,
            5 => SettingsTab::About,
            _ => SettingsTab::General,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::General => "General",
            SettingsTab::Appearance => "Appearance",
            SettingsTab::Files => "Files",
            SettingsTab::Layout => "Layout",
            SettingsTab::Extensions => "Extensions",
            SettingsTab::About => "About",
        }
    }

    pub fn icon(self) -> AtlasIcon {
        match self {
            SettingsTab::General => AtlasIcon::Gear,
            SettingsTab::Appearance => AtlasIcon::Palette,
            SettingsTab::Files => AtlasIcon::Folder,
            SettingsTab::Layout => AtlasIcon::SidebarSimple,
            SettingsTab::Extensions => AtlasIcon::TerminalWindow,
            SettingsTab::About => AtlasIcon::House,
        }
    }
}

/// A shortcut keybinding entry displayed in the keybindings table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeybindingEntry {
    pub category: &'static str,
    pub action: &'static str,
    pub keys: &'static str,
    pub description: &'static str,
}

pub const KEYBINDINGS: &[KeybindingEntry] = &[
    KeybindingEntry {
        category: "Navigation",
        action: "Navigate Up / Down",
        keys: "↑ / ↓",
        description: "Move selection in list or grid view",
    },
    KeybindingEntry {
        category: "Navigation",
        action: "Open Selected",
        keys: "Enter",
        description: "Open the focused folder or launch file",
    },
    KeybindingEntry {
        category: "Navigation",
        action: "Parent Directory",
        keys: "Alt+↑ / Backspace",
        description: "Navigate up to enclosing folder",
    },
    KeybindingEntry {
        category: "Navigation",
        action: "Jump First / Last",
        keys: "Home / End",
        description: "Jump to first or last item in current view",
    },
    KeybindingEntry {
        category: "Navigation",
        action: "Vim Navigation",
        keys: "h / j / k / l",
        description: "Modal navigation when Vim mode is toggled on",
    },
    KeybindingEntry {
        category: "Tabs",
        action: "Next Tab",
        keys: "Ctrl+Tab",
        description: "Switch to next open folder tab",
    },
    KeybindingEntry {
        category: "Tabs",
        action: "Previous Tab",
        keys: "Ctrl+Shift+Tab",
        description: "Switch to previous open folder tab",
    },
    KeybindingEntry {
        category: "Tabs",
        action: "Reload Tab",
        keys: "Ctrl+R",
        description: "Refresh directory contents and watch changes",
    },
    KeybindingEntry {
        category: "Search",
        action: "Filter View",
        keys: "Ctrl+F",
        description: "Instant in-place search filter in current tab",
    },
    KeybindingEntry {
        category: "Palettes",
        action: "Command Palette",
        keys: "Ctrl+Shift+P",
        description: "Open command launcher for all actions",
    },
    KeybindingEntry {
        category: "Palettes",
        action: "Goto Bar",
        keys: "Ctrl+P",
        description: "Fast jump to folder path or history bookmark",
    },
    KeybindingEntry {
        category: "Panels",
        action: "Toggle Inspector",
        keys: "Space",
        description: "Preview metadata, media thumbnails, and text",
    },
    KeybindingEntry {
        category: "Panels",
        action: "Toggle Sidebar",
        keys: "Ctrl+B",
        description: "Show or hide tree navigation sidebar",
    },
    KeybindingEntry {
        category: "Files",
        action: "Toggle Hidden",
        keys: "Ctrl+H",
        description: "Show or hide dotfiles and hidden system files",
    },
    KeybindingEntry {
        category: "Zoom",
        action: "Zoom In",
        keys: "Ctrl++",
        description: "Scale up user interface elements",
    },
    KeybindingEntry {
        category: "Zoom",
        action: "Zoom Out",
        keys: "Ctrl+-",
        description: "Scale down user interface elements",
    },
    KeybindingEntry {
        category: "Zoom",
        action: "Zoom Reset",
        keys: "Ctrl+0",
        description: "Reset interface scaling to 100%",
    },
    KeybindingEntry {
        category: "Overlays",
        action: "Dismiss Modal",
        keys: "Escape",
        description: "Close active settings dialog or palette",
    },
];

/// Colors extracted from the theme for immediate painting.
#[derive(Clone, Copy, Debug)]
pub struct ImmSettingsColors {
    pub background: Hsla,
    pub surface: Hsla,
    pub surface_hover: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub accent_fg: Hsla,
    pub card_bg: Hsla,
    pub switch_off: Hsla,
    pub switch_on: Hsla,
}

impl ImmSettingsColors {
    pub fn from_cx(cx: &App) -> Self {
        let t = cx.theme();
        Self {
            background: t.background,
            surface: t.popover,
            surface_hover: t.muted.opacity(0.24),
            foreground: t.popover_foreground,
            muted: t.muted_foreground,
            border: t.border,
            accent: t.accent,
            accent_fg: t.accent_foreground,
            card_bg: t.background.opacity(0.42),
            switch_off: t.border.opacity(0.85),
            switch_on: t.accent,
        }
    }
}

/// Identifiers for interactive hit slots inside the settings modal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingsHit {
    ThumbnailBudgetDec,
    ThumbnailBudgetInc,
    ThumbnailBudgetBar(f32),
    CloseButton,
    TabSelect(SettingsTab),
    ToggleEcoMode,
    ToggleHiddenFiles,
    ToggleVimMode,
    ToggleSidebar,
    ToggleInspector,
    ToggleAnimations,
    ToggleAutosave,
    ToggleDimPanes,
    ToggleTitlebar,
    LanguageSelect(usize), // 0: Auto, 1: EN, 2: ES
    ZoomDec,
    ZoomReset,
    ZoomInc,
    ZoomBar(f32), // fraction 0.0 .. 1.0
    GlassOpacityDec,
    GlassOpacityInc,
    GlassOpacityBar(f32),
    ScrollSmoothDec,
    ScrollSmoothInc,
    ScrollSmoothBar(f32),
    SidebarWidthDec,
    SidebarWidthInc,
    InspectorWidthDec,
    InspectorWidthInc,
    FontSizeDec,
    FontSizeInc,
    ThumbnailSizeDec,
    ThumbnailSizeInc,
    GridSpacingDec,
    GridSpacingInc,
    ThemeSelect(usize),
    ViewModeSelect(usize),
    InspectorPlacementSelect(usize),
    FontFamilySelect(usize),
    AboutUrl(&'static str),
}

/// Layout arithmetic container for the settings modal dialog.
#[derive(Clone, Debug)]
pub struct SettingsImmLayout {
    pub modal_slot: Slot,
    pub header_slot: Slot,
    pub title_slot: Slot,
    pub search_slot: Slot,
    pub close_slot: Slot,
    pub sidebar_slot: Slot,
    pub sidebar_tabs: Vec<(SettingsTab, Slot)>,
    pub divider_slot: Slot,
    pub content_viewport: Slot,
    pub scrollbar_slot: Option<Slot>,
    pub rows: Vec<SettingRowLayout>,
    pub active_tooltip: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SettingRowLayout {
    pub card_slot: Slot,
    pub info_slot: Slot,
    pub title: &'static str,
    pub description: &'static str,
    pub tooltip: &'static str,
    pub control_slot: Slot,
    pub control_kind: SettingControlKind,
}

#[derive(Clone, Debug)]
pub enum SettingControlKind {
    Toggle {
        value: bool,
        hit: SettingsHit,
    },
    Segmented {
        options: Vec<(&'static str, bool, SettingsHit)>,
    },
    Slider {
        display_val: String,
        progress: f32, // 0.0 .. 1.0
        dec_hit: SettingsHit,
        inc_hit: SettingsHit,
        bar_hit: fn(f32) -> SettingsHit,
    },
    KeybindingRow {
        category: &'static str,
        action: &'static str,
        keys: &'static str,
    },
    AboutAction {
        label: &'static str,
        url: &'static str,
    },
}

impl SettingsImmLayout {
    /// Pure arithmetic layout pass slicing rectangles using `RectCut`.
    pub fn compute(
        width: f32,
        height: f32,
        zoom: f32,
        active_tab: SettingsTab,
        query: &str,
        scroll_y: f32,
        app_settings: &atlas_model::settings::Settings,
        active_theme_name: &str,
    ) -> Self {
        let zoom = zoom.max(0.5);
        let modal_w = (width * 0.85).clamp(560.0 * zoom, 840.0 * zoom);
        let modal_h = (height * 0.78).clamp(380.0 * zoom, 620.0 * zoom);
        let modal_x = (width - modal_w) * 0.5;
        let modal_y = (height - modal_h) * 0.5;

        let mut root_cut = RectCut::new(modal_x, modal_y, modal_w, modal_h);
        let header_slot = root_cut.cut_top(50.0 * zoom);

        // Header slicing
        let mut h_cut = RectCut::new(
            header_slot.x,
            header_slot.y,
            header_slot.w,
            header_slot.h,
        );
        h_cut.pad(16.0 * zoom, 8.0 * zoom);
        let title_slot = h_cut.cut_left(110.0 * zoom);
        let close_slot = h_cut.cut_right(30.0 * zoom);
        h_cut.cut_right(10.0 * zoom); // spacing
        let search_slot = h_cut.remaining();

        // Body slicing: Sidebar + Divider + Content
        let body_slot = root_cut.remaining();
        let mut b_cut = RectCut::new(body_slot.x, body_slot.y, body_slot.w, body_slot.h);
        let sidebar_slot = b_cut.cut_left(180.0 * zoom);
        let divider_slot = b_cut.cut_left(1.0 * zoom);
        let content_viewport = b_cut.remaining();

        // Sidebar tabs slicing
        let mut s_cut = RectCut::new(
            sidebar_slot.x,
            sidebar_slot.y,
            sidebar_slot.w,
            sidebar_slot.h,
        );
        s_cut.pad(10.0 * zoom, 12.0 * zoom);
        let mut sidebar_tabs = Vec::with_capacity(6);
        for tab in SettingsTab::ALL {
            let tab_slot = s_cut.cut_top(36.0 * zoom);
            sidebar_tabs.push((tab, tab_slot));
            s_cut.cut_top(4.0 * zoom); // inter-tab gap
        }

        // Build content cards according to active tab and search query
        let query_lower = query.trim().to_lowercase();
        let show_all = query_lower.is_empty();

        let mut row_defs = Vec::new();

        // 1. General Tab Rows
        if show_all && active_tab == SettingsTab::General
            || (!show_all && match_query(&query_lower, &["thumbnail", "cache", "memory", "miniaturas"]))
        {
            let budget = app_settings.thumbnail_cache_mb.clamp(32, 1024);
            row_defs.push((
                "Thumbnail cache", "Memory budget for recently viewed thumbnails",
                "Keeps recently used images and evicts the least recently viewed when this budget is reached.",
                SettingControlKind::Slider {
                    display_val: format!("{budget} MB"), progress: (budget - 32) as f32 / 992.0,
                    dec_hit: SettingsHit::ThumbnailBudgetDec,
                    inc_hit: SettingsHit::ThumbnailBudgetInc,
                    bar_hit: SettingsHit::ThumbnailBudgetBar,
                }, 52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::General
            || (!show_all && match_query(&query_lower, &["language", "idioma", "english", "spanish"]))
        {
            let lang_idx = match app_settings.language.as_deref() {
                Some("en") => 1,
                Some("es") => 2,
                _ => 0,
            };
            row_defs.push((
                "Language",
                "Select user interface display language",
                "Choose between System Auto-detection, English, or Spanish localized interface strings.",
                SettingControlKind::Segmented {
                    options: vec![
                        ("Auto", lang_idx == 0, SettingsHit::LanguageSelect(0)),
                        ("EN", lang_idx == 1, SettingsHit::LanguageSelect(1)),
                        ("ES", lang_idx == 2, SettingsHit::LanguageSelect(2)),
                    ],
                },
                52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::General
            || (!show_all && match_query(&query_lower, &["zoom", "scale", "escala", "size"]))
        {
            let zoom_pct = (app_settings.global_zoom * 100.0).round();
            let progress = ((app_settings.global_zoom - 0.5) / 1.5).clamp(0.0, 1.0);
            row_defs.push((
                "Interface Zoom",
                "Scale UI components, typography, and controls",
                "Proportionally scales interface dimensions, text rendering, and icon bounds across all windows.",
                SettingControlKind::Slider {
                    display_val: format!("{zoom_pct:.0}%"),
                    progress,
                    dec_hit: SettingsHit::ZoomDec,
                    inc_hit: SettingsHit::ZoomInc,
                    bar_hit: SettingsHit::ZoomBar,
                },
                52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::General
            || (!show_all && match_query(&query_lower, &["eco", "ram", "memory", "background"]))
        {
            row_defs.push((
                "Eco Mode",
                "Shrink memory buffers and evict thumbnails when inactive",
                "Dramatically reduces RAM consumption by purging image caches and pausing layout watchers while Atlas is in the background.",
                SettingControlKind::Toggle {
                    value: app_settings.eco_mode,
                    hit: SettingsHit::ToggleEcoMode,
                },
                52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::General
            || (!show_all && match_query(&query_lower, &["vim", "navigation", "keys", "hjkl"]))
        {
            row_defs.push((
                "Vim Navigation Mode",
                "Navigate lists, grids, and directory trees using hjkl",
                "Enables modal keyboard traversal using traditional Vim keys without moving hands away from the home row.",
                SettingControlKind::Toggle {
                    value: app_settings.vim_mode,
                    hit: SettingsHit::ToggleVimMode,
                },
                52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::General
            || (!show_all && match_query(&query_lower, &["autosave", "session", "restore"]))
        {
            row_defs.push((
                "Autosave Session",
                "Automatically persist tabs and pane splits on exit",
                "Restores open directory panes, search filters, and active tab states across application restarts.",
                SettingControlKind::Toggle {
                    value: app_settings.autosave_session,
                    hit: SettingsHit::ToggleAutosave,
                },
                52.0 * zoom,
            ));
        }

        // 2. Appearance Tab Rows
        if show_all && active_tab == SettingsTab::Appearance
            || (!show_all && match_query(&query_lower, &["theme", "appearance", "dark", "light", "color"]))
        {
            let themes = [
                "Atlas Graphite Glass",
                "Atlas Midnight",
                "One Dark",
                "Dracula",
                "Nord",
            ];
            let options = themes
                .iter()
                .enumerate()
                .map(|(idx, name)| (*name, active_theme_name == *name, SettingsHit::ThemeSelect(idx)))
                .collect();
            row_defs.push((
                "Theme Preset",
                "Select active color scheme and visual skin",
                "Switches active palette and component token bindings immediately across all tabs and chrome.",
                SettingControlKind::Segmented { options },
                56.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::Appearance
            || (!show_all && match_query(&query_lower, &["titlebar", "window", "chrome"]))
        {
            row_defs.push((
                "Custom Titlebar",
                "Display integrated top window chrome with tabs",
                "Toggles custom styled top frame. When disabled, standard platform window manager chrome is retained.",
                SettingControlKind::Toggle {
                    value: app_settings.show_titlebar,
                    hit: SettingsHit::ToggleTitlebar,
                },
                52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::Appearance
            || (!show_all && match_query(&query_lower, &["glass", "opacity", "blur", "transparency"]))
        {
            let glass_pct = (app_settings.glass_opacity * 100.0).round();
            let progress = ((app_settings.glass_opacity - 0.20) / 0.80).clamp(0.0, 1.0);
            row_defs.push((
                "Glass Opacity",
                "Background transparency level for frosted glass themes",
                "Adjusts material density and acrylic blur intensity when using a Glass theme variant.",
                SettingControlKind::Slider {
                    display_val: format!("{glass_pct:.0}%"),
                    progress,
                    dec_hit: SettingsHit::GlassOpacityDec,
                    inc_hit: SettingsHit::GlassOpacityInc,
                    bar_hit: SettingsHit::GlassOpacityBar,
                },
                52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::Appearance
            || (!show_all && match_query(&query_lower, &["smooth", "scroll", "physics", "ease"]))
        {
            let smooth_pct = (app_settings.scroll_smoothness * 100.0).round();
            let progress = app_settings.scroll_smoothness.clamp(0.0, 1.0);
            row_defs.push((
                "Smooth Scrolling",
                "Momentum and easing physics for mouse-wheel movement",
                "Applies smooth glide interpolation to viewports. Set to 0% for instant step-by-step scrolling.",
                SettingControlKind::Slider {
                    display_val: format!("{smooth_pct:.0}%"),
                    progress,
                    dec_hit: SettingsHit::ScrollSmoothDec,
                    inc_hit: SettingsHit::ScrollSmoothInc,
                    bar_hit: SettingsHit::ScrollSmoothBar,
                },
                52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::Appearance
            || (!show_all && match_query(&query_lower, &["animations", "motion", "disable"]))
        {
            row_defs.push((
                "Disable Animations",
                "Render all transitions and elements in final state immediately",
                "Zeroes all animation durations to maximize CPU efficiency and battery life.",
                SettingControlKind::Toggle {
                    value: app_settings.disable_animations,
                    hit: SettingsHit::ToggleAnimations,
                },
                52.0 * zoom,
            ));
        }

        // 3. Files Tab Rows
        if show_all && active_tab == SettingsTab::Files
            || (!show_all && match_query(&query_lower, &["hidden", "files", "dotfiles", "show"]))
        {
            row_defs.push((
                "Show Hidden Files",
                "Reveal dotfiles and hidden system entries",
                "Displays Unix dot-prefixed items and Windows hidden attribute files in all file views.",
                SettingControlKind::Toggle {
                    value: app_settings.show_hidden,
                    hit: SettingsHit::ToggleHiddenFiles,
                },
                52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::Files
            || (!show_all && match_query(&query_lower, &["dim", "inactive", "panes", "focus"]))
        {
            row_defs.push((
                "Dim Inactive Panes",
                "Visually de-emphasize unfocused panes",
                "Lowers opacity of background tiles to focus visual attention on the active working folder.",
                SettingControlKind::Toggle {
                    value: app_settings.dim_inactive_panes,
                    hit: SettingsHit::ToggleDimPanes,
                },
                52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::Files
            || (!show_all && match_query(&query_lower, &["thumbnail", "size", "preview"]))
        {
            let val = app_settings.thumbnail_size;
            let progress = ((val - 64.0) / (256.0 - 64.0)).clamp(0.0, 1.0);
            row_defs.push((
                "Thumbnail Size",
                "Resolution and dimensions of icon and image previews",
                "Determines square pixel dimension for thumbnail rasterization in Grid view.",
                SettingControlKind::Slider {
                    display_val: format!("{val:.0}px"),
                    progress,
                    dec_hit: SettingsHit::ThumbnailSizeDec,
                    inc_hit: SettingsHit::ThumbnailSizeInc,
                    bar_hit: |_| SettingsHit::ThumbnailSizeInc,
                },
                52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::Files
            || (!show_all && match_query(&query_lower, &["grid", "spacing", "gap"]))
        {
            let val = app_settings.grid_spacing;
            let progress = ((val - 4.0) / (48.0 - 4.0)).clamp(0.0, 1.0);
            row_defs.push((
                "Grid Spacing",
                "Gap padding between thumbnail items in Grid view",
                "Controls horizontal and vertical margin separating items in the virtual grid.",
                SettingControlKind::Slider {
                    display_val: format!("{val:.0}px"),
                    progress,
                    dec_hit: SettingsHit::GridSpacingDec,
                    inc_hit: SettingsHit::GridSpacingInc,
                    bar_hit: |_| SettingsHit::GridSpacingInc,
                },
                52.0 * zoom,
            ));
        }

        // 4. Layout Tab Rows
        if show_all && active_tab == SettingsTab::Layout
            || (!show_all && match_query(&query_lower, &["view", "details", "grid", "list", "miller", "tree"]))
        {
            let cur = &app_settings.view_mode;
            let options = vec![
                ("Details", *cur == ViewMode::Details, SettingsHit::ViewModeSelect(0)),
                ("Grid", *cur == ViewMode::Grid, SettingsHit::ViewModeSelect(1)),
                ("List", *cur == ViewMode::List, SettingsHit::ViewModeSelect(2)),
                ("Miller", *cur == ViewMode::Miller, SettingsHit::ViewModeSelect(3)),
                ("Tree", *cur == ViewMode::Tree, SettingsHit::ViewModeSelect(4)),
            ];
            row_defs.push((
                "Default View Mode",
                "Initial view presentation when opening new tabs",
                "Select whether folders open in Details table, Icon Grid, Compact List, Columnar Miller Columns, or Hierarchical Tree.",
                SettingControlKind::Segmented { options },
                56.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::Layout
            || (!show_all && match_query(&query_lower, &["sidebar", "navigation"]))
        {
            row_defs.push((
                "Show Navigation Sidebar",
                "Toggle folder bookmarks and drive directory tree panel",
                "Shows quick access shortcuts, mounted drives, tags, and cloud volumes on the left edge.",
                SettingControlKind::Toggle {
                    value: app_settings.show_sidebar,
                    hit: SettingsHit::ToggleSidebar,
                },
                52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::Layout
            || (!show_all && match_query(&query_lower, &["inspector", "preview"]))
        {
            row_defs.push((
                "Show Inspector Panel",
                "Display file metadata, size breakdown, and image previews",
                "Opens the right-side inspector pane with EXIF details, hash sums, and tag editors.",
                SettingControlKind::Toggle {
                    value: app_settings.show_inspector,
                    hit: SettingsHit::ToggleInspector,
                },
                52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::Layout
            || (!show_all && match_query(&query_lower, &["placement", "inspector", "global", "tab"]))
        {
            let is_global = app_settings.inspector_placement == InspectorPlacement::Global;
            let options = vec![
                ("Global", is_global, SettingsHit::InspectorPlacementSelect(0)),
                ("Active Tab", !is_global, SettingsHit::InspectorPlacementSelect(1)),
            ];
            row_defs.push((
                "Inspector Placement",
                "Attach inspector globally or isolate per active tab",
                "Global inspector spans the whole window height, whereas Active Tab inspector sits inside each pane.",
                SettingControlKind::Segmented { options },
                52.0 * zoom,
            ));
        }

        if show_all && active_tab == SettingsTab::Layout
            || (!show_all && match_query(&query_lower, &["font", "typography", "family"]))
        {
            let cur = &app_settings.font_family;
            let fonts = ["Geist", "Segoe UI Variable", ".SystemUIFont"];
            let options = fonts
                .iter()
                .enumerate()
                .map(|(idx, name)| (*name, cur == *name, SettingsHit::FontFamilySelect(idx)))
                .collect();
            row_defs.push((
                "Font Family",
                "Typeface for chrome, breadcrumbs, and file labels",
                "Selects primary rendering font used across all UI text systems and tables.",
                SettingControlKind::Segmented { options },
                56.0 * zoom,
            ));
        }

        // 5. Extensions & Keybindings Rows
        if show_all && active_tab == SettingsTab::Extensions
            || (!show_all && match_query(&query_lower, &["key", "shortcut", "binding", "hotkey", "vim"]))
        {
            for kb in KEYBINDINGS {
                if show_all || match_query(&query_lower, &[kb.action, kb.keys, kb.category, kb.description]) {
                    row_defs.push((
                        kb.action,
                        kb.description,
                        "Global keyboard shortcut binding for immediate execution.",
                        SettingControlKind::KeybindingRow {
                            category: kb.category,
                            action: kb.action,
                            keys: kb.keys,
                        },
                        44.0 * zoom,
                    ));
                }
            }
        }

        // 6. About Tab Rows
        if show_all && active_tab == SettingsTab::About
            || (!show_all && match_query(&query_lower, &["about", "version", "community", "support", "discord", "web", "license", "privacy", "icons8"]))
        {
            row_defs.push((
                "Atlas Version",
                concat!("Atlas v", env!("CARGO_PKG_VERSION"), " — High-performance immediate file manager"),
                "Immediate-mode 60 FPS architecture built on Rust and GPUI.",
                SettingControlKind::AboutAction {
                    label: "Release Notes",
                    url: atlas_core::update::RELEASES_URL,
                },
                52.0 * zoom,
            ));
            for (title, description, label, url) in [
                ("Third-party licenses", "Read bundled copyright notices and license texts offline", "Open notices", crate::ui::overlays::legal::NOTICES_URL),
                ("Source code", "Corresponding source and build instructions for this release", "Download source", atlas_core::update::SOURCE_URL),
                ("Privacy policy", "Local files, optional cloud and website data", "Read policy", "https://atlasnative.com/privacy-policy"),
                ("Terms of service", "Purchase terms and software license information", "Read terms", "https://atlasnative.com/terms-of-service"),
                ("Icons by Icons8", "Liquid Glass file and folder artwork", "Visit Icons8", "https://icons8.com"),
                ("WinFsp - Windows File System Proxy", "Copyright (C) Bill Zissimopoulos", "WinFsp source", "https://github.com/winfsp/winfsp"),
            ] {
                row_defs.push((title, description, description, SettingControlKind::AboutAction { label, url }, 52.0 * zoom));
            }
            row_defs.push((
                "Community Discord",
                "Join discussions, share themes, and get help",
                "Connect with the Atlas developer community and report feedback directly.",
                SettingControlKind::AboutAction {
                    label: "Open Discord",
                    url: "https://discord.gg/aeAvQ27bgG",
                },
                52.0 * zoom,
            ));
            row_defs.push((
                "Official Website",
                "Documentation, user manuals, and feature roadmaps",
                "Visit the official home of Atlas file manager.",
                SettingControlKind::AboutAction {
                    label: "Visit Site",
                    url: "https://atlasnative.com/",
                },
                52.0 * zoom,
            ));
        }

        // Layout rows with scrolling
        let content_pad_x = 16.0 * zoom;
        let content_pad_y = 12.0 * zoom;
        let row_w = (content_viewport.w - content_pad_x * 2.0).max(0.0);
        let mut cur_y = content_viewport.y + content_pad_y - scroll_y;

        let mut rows = Vec::with_capacity(row_defs.len());
        let mut total_content_h = content_pad_y * 2.0;

        for (title, desc, tooltip, ctrl_kind, row_h) in row_defs {
            let card_slot = Slot::new(content_viewport.x + content_pad_x, cur_y, row_w, row_h);
            let mut r_cut = RectCut::new(card_slot.x, card_slot.y, card_slot.w, card_slot.h);
            r_cut.pad(12.0 * zoom, 6.0 * zoom);

            let ctrl_w = match &ctrl_kind {
                SettingControlKind::Toggle { .. } => 44.0 * zoom,
                SettingControlKind::Slider { .. } => 170.0 * zoom,
                SettingControlKind::Segmented { options } => {
                    (options.len() as f32 * 62.0 * zoom).min(card_slot.w * 0.55)
                }
                SettingControlKind::KeybindingRow { .. } => 140.0 * zoom,
                SettingControlKind::AboutAction { .. } => 110.0 * zoom,
            };

            let control_slot = r_cut.cut_right(ctrl_w);
            r_cut.cut_right(10.0 * zoom); // gap
            let info_slot = r_cut.remaining();

            rows.push(SettingRowLayout {
                card_slot,
                info_slot,
                title,
                description: desc,
                tooltip,
                control_slot,
                control_kind: ctrl_kind,
            });

            cur_y += row_h + 8.0 * zoom;
            total_content_h += row_h + 8.0 * zoom;
        }

        // Scrollbar computation
        let scrollbar_slot = if total_content_h > content_viewport.h && content_viewport.h > 0.0 {
            let thumb_h = (content_viewport.h * content_viewport.h / total_content_h)
                .max(24.0 * zoom)
                .min(content_viewport.h);
            let max_scroll = total_content_h - content_viewport.h;
            let thumb_y = content_viewport.y
                + (content_viewport.h - thumb_h) * (scroll_y / max_scroll).clamp(0.0, 1.0);
            Some(Slot::new(
                content_viewport.x + content_viewport.w - 8.0 * zoom,
                thumb_y,
                5.0 * zoom,
                thumb_h,
            ))
        } else {
            None
        };

        Self {
            modal_slot: Slot::new(modal_x, modal_y, modal_w, modal_h),
            header_slot,
            title_slot,
            search_slot,
            close_slot,
            sidebar_slot,
            sidebar_tabs,
            divider_slot,
            content_viewport,
            scrollbar_slot,
            rows,
            active_tooltip: None,
        }
    }
}

fn match_query(query: &str, terms: &[&str]) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    terms.iter().any(|term| {
        let t = term.to_lowercase();
        t.contains(q) || q.contains(&t)
    })
}

/// The immediate-mode immediate settings overlay element.
pub struct SettingsImmElement {
    id: ElementId,
    app: WeakEntity<AtlasApp>,
    zoom: f32,
    active_tab: SettingsTab,
    search_query: String,
    search_input: Option<AnyElement>,
    scroll_y: f32,
    style: StyleRefinement,
}

impl SettingsImmElement {
    pub fn new(
        app: WeakEntity<AtlasApp>,
        zoom: f32,
        active_tab: SettingsTab,
        search_query: String,
        search_input: Option<AnyElement>,
        scroll_y: f32,
    ) -> Self {
        Self {
            id: ElementId::Name("settings-imm-overlay".into()),
            app,
            zoom,
            active_tab,
            search_query,
            search_input,
            scroll_y,
            style: StyleRefinement::default(),
        }
    }
}

impl IntoElement for SettingsImmElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for SettingsImmElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

pub struct SettingsImmPrepaint {
    hitbox: Hitbox,
    layout: SettingsImmLayout,
    colors: ImmSettingsColors,
    zoom: f32,
    hovered_hit: Option<SettingsHit>,
    hovered_row_idx: Option<usize>,
}

impl Element for SettingsImmElement {
    type RequestLayoutState = Style;
    type PrepaintState = SettingsImmPrepaint;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let child_layout = self
            .search_input
            .as_mut()
            .map(|input| input.request_layout(window, cx));
        let layout_id = window.request_layout(style.clone(), child_layout.into_iter(), cx);
        (layout_id, style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _style: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let colors = ImmSettingsColors::from_cx(cx);
        let zoom = self.zoom;
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::BlockMouse);

        let (settings, active_theme) = if let Some(app) = self.app.upgrade() {
            let app_read = app.read(cx);
            (
                app_read.state.settings.clone(),
                app_read.state.profiles.active().theme.clone(),
            )
        } else {
            (atlas_model::settings::Settings::default(), String::new())
        };

        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);

        let layout = SettingsImmLayout::compute(
            width,
            height,
            zoom,
            self.active_tab,
            &self.search_query,
            self.scroll_y,
            &settings,
            &active_theme,
        );

        // Prepaint search input into search slot if present
        if let Some(input) = self.search_input.as_mut() {
            let input_bounds = layout.search_slot.to_bounds(bounds.origin);
            input.prepaint_at(input_bounds.origin, window, cx);
        }

        // Test hover
        let mouse_pos = window.mouse_position();
        let local_x = f32::from(mouse_pos.x - bounds.origin.x);
        let local_y = f32::from(mouse_pos.y - bounds.origin.y);

        let mut hovered_hit = None;
        let mut hovered_row_idx = None;

        if hitbox.is_hovered(window) {
            if layout.close_slot.contains(local_x, local_y) {
                hovered_hit = Some(SettingsHit::CloseButton);
            } else {
                for (tab, slot) in &layout.sidebar_tabs {
                    if slot.contains(local_x, local_y) {
                        hovered_hit = Some(SettingsHit::TabSelect(*tab));
                        break;
                    }
                }
            }

            if hovered_hit.is_none() {
                for (idx, row) in layout.rows.iter().enumerate() {
                    if row.card_slot.contains(local_x, local_y) {
                        hovered_row_idx = Some(idx);
                        // Check control hits
                        match &row.control_kind {
                            SettingControlKind::Toggle { hit, .. } => {
                                if row.control_slot.contains(local_x, local_y) {
                                    hovered_hit = Some(*hit);
                                }
                            }
                            SettingControlKind::Segmented { options } => {
                                let seg_w = row.control_slot.w / options.len() as f32;
                                for (opt_idx, (_, _, hit)) in options.iter().enumerate() {
                                    let seg_slot = Slot::new(
                                        row.control_slot.x + opt_idx as f32 * seg_w,
                                        row.control_slot.y,
                                        seg_w,
                                        row.control_slot.h,
                                    );
                                    if seg_slot.contains(local_x, local_y) {
                                        hovered_hit = Some(*hit);
                                        break;
                                    }
                                }
                            }
                            SettingControlKind::Slider {
                                dec_hit,
                                inc_hit,
                                bar_hit,
                                ..
                            } => {
                                let btn_w = 26.0 * zoom;
                                let dec_slot = Slot::new(
                                    row.control_slot.x,
                                    row.control_slot.y,
                                    btn_w,
                                    row.control_slot.h,
                                );
                                let inc_slot = Slot::new(
                                    row.control_slot.x + row.control_slot.w - btn_w,
                                    row.control_slot.y,
                                    btn_w,
                                    row.control_slot.h,
                                );
                                if dec_slot.contains(local_x, local_y) {
                                    hovered_hit = Some(*dec_hit);
                                } else if inc_slot.contains(local_x, local_y) {
                                    hovered_hit = Some(*inc_hit);
                                } else {
                                    let bar_x = row.control_slot.x + btn_w + 4.0 * zoom;
                                    let bar_w = (row.control_slot.w - (btn_w + 4.0 * zoom) * 2.0).max(1.0);
                                    let frac = ((local_x - bar_x) / bar_w).clamp(0.0, 1.0);
                                    hovered_hit = Some(bar_hit(frac));
                                }
                            }
                            SettingControlKind::AboutAction { url, .. } => {
                                if row.control_slot.contains(local_x, local_y) {
                                    hovered_hit = Some(SettingsHit::AboutUrl(url));
                                }
                            }
                            _ => {}
                        }
                        break;
                    }
                }
            }
        }

        SettingsImmPrepaint {
            hitbox,
            layout,
            colors,
            zoom,
            hovered_hit,
            hovered_row_idx,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _style: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let zoom = prepaint.zoom;
        let colors = prepaint.colors;
        let layout = &prepaint.layout;
        let modal_bounds = layout.modal_slot.to_bounds(bounds.origin);
        let font = chrome_ui_font(window);

        // 1. Modal backdrop shadow and window blur
        let corner_radius = 12.0 * zoom;
        let corners = Corners::all(px(corner_radius));
        window.paint_drop_shadows(
            modal_bounds,
            corners,
            &[
                BoxShadow::new(px(0.0), px(12.0 * zoom), gpui::black().opacity(0.38))
                    .blur_radius(px(32.0 * zoom)),
            ],
        );
        if colors.background.alpha < 1.0 {
            window.paint_backdrop_filter(modal_bounds, corners, &[gpui::Filter::Blur(px(20.0))]);
        }

        // 2. Modal surface container quad
        window.paint_quad(gpui::quad(
            modal_bounds,
            corners,
            colors.surface,
            gpui::Edges::all(px(1.0 * zoom)),
            colors.border,
            Default::default(),
        ));

        // 3. Header Row
        let header_bounds = layout.header_slot.to_bounds(bounds.origin);
        paint_fill(
            window,
            Bounds {
                origin: point(header_bounds.origin.x, header_bounds.bottom() - px(1.0 * zoom)),
                size: size(header_bounds.size.width, px(1.0 * zoom)),
            },
            colors.border,
        );

        // Header Title
        let title_line = shape_text("Settings", &font, px(17.0 * zoom), colors.foreground, window);
        let title_pt = point(
            bounds.origin.x + px(layout.title_slot.x),
            bounds.origin.y + px(layout.title_slot.y + (layout.title_slot.h - 18.0 * zoom) * 0.5),
        );
        let _ = title_line.paint(title_pt, px(layout.title_slot.h), TextAlign::Left, None, window, cx);

        // Header Close Button (X)
        let is_close_hovered = prepaint.hovered_hit == Some(SettingsHit::CloseButton);
        let close_bounds = layout.close_slot.to_bounds(bounds.origin);
        if is_close_hovered {
            paint_rounded(window, close_bounds, 6.0 * zoom, colors.surface_hover);
        }
        paint_theme_icon(
            window,
            cx,
            bounds.origin,
            layout.close_slot,
            AtlasIcon::X,
            false,
            15.0 * zoom,
            if is_close_hovered { colors.foreground } else { colors.muted },
        );

        // Search Input Slot
        let search_bounds = layout.search_slot.to_bounds(bounds.origin);
        paint_rounded(window, search_bounds, 6.0 * zoom, colors.card_bg);
        window.paint_quad(gpui::outline(search_bounds, colors.border.opacity(0.6), Default::default()).corner_radii(px(6.0 * zoom)));
        paint_theme_icon(
            window,
            cx,
            bounds.origin,
            Slot::new(layout.search_slot.x + 8.0 * zoom, layout.search_slot.y + 7.0 * zoom, 14.0 * zoom, 14.0 * zoom),
            AtlasIcon::Search,
            false,
            13.0 * zoom,
            colors.muted,
        );

        // Paint search input element if provided
        if let Some(input) = self.search_input.as_mut() {
            input.paint(window, cx);
        } else if self.search_query.is_empty() {
            let placeholder = shape_text("Search settings and shortcuts…", &font, px(12.5 * zoom), colors.muted, window);
            let p_pt = point(
                search_bounds.origin.x + px(28.0 * zoom),
                search_bounds.origin.y + px((layout.search_slot.h - 14.0 * zoom) * 0.5),
            );
            let _ = placeholder.paint(p_pt, search_bounds.size.height, TextAlign::Left, None, window, cx);
        }

        // 4. Sidebar Tabs & Divider
        let divider_bounds = layout.divider_slot.to_bounds(bounds.origin);
        paint_fill(window, divider_bounds, colors.border);

        for (tab, slot) in &layout.sidebar_tabs {
            let is_active = *tab == self.active_tab;
            let is_hovered = prepaint.hovered_hit == Some(SettingsHit::TabSelect(*tab));
            let tab_bounds = slot.to_bounds(bounds.origin);

            if is_active {
                paint_rounded(window, tab_bounds, 6.0 * zoom, colors.accent.opacity(0.18));
                // Active indicator pill on the left
                let ind_slot = Slot::new(slot.x + 2.0 * zoom, slot.y + 6.0 * zoom, 3.0 * zoom, slot.h - 12.0 * zoom);
                paint_rounded(window, ind_slot.to_bounds(bounds.origin), 1.5 * zoom, colors.accent);
            } else if is_hovered {
                paint_rounded(window, tab_bounds, 6.0 * zoom, colors.surface_hover);
            }

            // Tab Icon
            let icon_slot = Slot::new(slot.x + 10.0 * zoom, slot.y + 9.0 * zoom, 16.0 * zoom, 16.0 * zoom);
            let icon_color = if is_active { colors.accent } else if is_hovered { colors.foreground } else { colors.muted };
            paint_theme_icon(window, cx, bounds.origin, icon_slot, tab.icon(), false, 15.0 * zoom, icon_color);

            // Tab Label
            let label_line = shape_text(
                tab.label(),
                &font,
                px(13.0 * zoom),
                if is_active { colors.foreground } else if is_hovered { colors.foreground } else { colors.muted },
                window,
            );
            let label_pt = point(
                bounds.origin.x + px(slot.x + 32.0 * zoom),
                bounds.origin.y + px(slot.y + (slot.h - 14.0 * zoom) * 0.5),
            );
            let _ = label_line.paint(label_pt, px(slot.h), TextAlign::Left, None, window, cx);
        }

        // 5. Scrollable Content Body
        let content_bounds = layout.content_viewport.to_bounds(bounds.origin);
        window.with_content_mask(Some(gpui::ContentMask { bounds: content_bounds, fade_out: Default::default() }), |window| {
            for (idx, row) in layout.rows.iter().enumerate() {
                let card_b = row.card_slot.to_bounds(bounds.origin);
                let is_card_hovered = prepaint.hovered_row_idx == Some(idx);

                // Card background & outline
                paint_rounded(window, card_b, 8.0 * zoom, if is_card_hovered { colors.surface_hover } else { colors.card_bg });
                window.paint_quad(
                    gpui::outline(
                        card_b,
                        if is_card_hovered { colors.accent.opacity(0.4) } else { colors.border.opacity(0.35) },
                        Default::default(),
                    )
                    .corner_radii(px(8.0 * zoom)),
                );

                // Title & Description
                let title_line = shape_text(row.title, &font, px(13.0 * zoom), colors.foreground, window);
                let title_pt = point(bounds.origin.x + px(row.info_slot.x), bounds.origin.y + px(row.info_slot.y));
                let _ = title_line.paint(title_pt, px(18.0 * zoom), TextAlign::Left, None, window, cx);

                let desc_line = shape_text_truncated(
                    row.description,
                    &font,
                    px(11.0 * zoom),
                    colors.muted,
                    row.info_slot.w,
                    window,
                );
                let desc_pt = point(bounds.origin.x + px(row.info_slot.x), bounds.origin.y + px(row.info_slot.y + 18.0 * zoom));
                let _ = desc_line.paint(desc_pt, px(16.0 * zoom), TextAlign::Left, None, window, cx);

                // Control Slot Painting
                match &row.control_kind {
                    SettingControlKind::Toggle { value, hit } => {
                        let is_toggle_hovered = prepaint.hovered_hit == Some(*hit);
                        let sw_w = 38.0 * zoom;
                        let sw_h = 22.0 * zoom;
                        let sw_slot = Slot::new(
                            row.control_slot.x + row.control_slot.w - sw_w,
                            row.control_slot.y + (row.control_slot.h - sw_h) * 0.5,
                            sw_w,
                            sw_h,
                        );
                        let sw_bounds = sw_slot.to_bounds(bounds.origin);

                        // Track
                        let track_color = if *value { colors.switch_on } else { colors.switch_off };
                        paint_rounded(window, sw_bounds, sw_h * 0.5, track_color);
                        if is_toggle_hovered {
                            window.paint_quad(gpui::outline(sw_bounds, colors.foreground.opacity(0.5), Default::default()).corner_radii(px(sw_h * 0.5)));
                        }

                        // Knob
                        let knob_sz = sw_h - 4.0 * zoom;
                        let knob_x = if *value {
                            sw_bounds.right() - px(knob_sz + 2.0 * zoom)
                        } else {
                            sw_bounds.left() + px(2.0 * zoom)
                        };
                        let knob_bounds = Bounds {
                            origin: point(knob_x, sw_bounds.top() + px(2.0 * zoom)),
                            size: size(px(knob_sz), px(knob_sz)),
                        };
                        paint_rounded(window, knob_bounds, knob_sz * 0.5, colors.accent_fg);
                    }
                    SettingControlKind::Segmented { options } => {
                        let ctrl_b = row.control_slot.to_bounds(bounds.origin);
                        paint_rounded(window, ctrl_b, 6.0 * zoom, colors.background.opacity(0.5));
                        window.paint_quad(gpui::outline(ctrl_b, colors.border, Default::default()).corner_radii(px(6.0 * zoom)));

                        let seg_w = row.control_slot.w / options.len() as f32;
                        for (opt_idx, (label, selected, hit)) in options.iter().enumerate() {
                            let seg_slot = Slot::new(
                                row.control_slot.x + opt_idx as f32 * seg_w,
                                row.control_slot.y,
                                seg_w,
                                row.control_slot.h,
                            );
                            let seg_b = seg_slot.to_bounds(bounds.origin);
                            let is_opt_hovered = prepaint.hovered_hit == Some(*hit);

                            if *selected {
                                paint_rounded(window, seg_b, 5.0 * zoom, colors.accent);
                            } else if is_opt_hovered {
                                paint_rounded(window, seg_b, 5.0 * zoom, colors.surface_hover);
                            }

                            let opt_line = shape_text(
                                *label,
                                &font,
                                px(11.5 * zoom),
                                if *selected { colors.accent_fg } else { colors.foreground },
                                window,
                            );
                            let w = line_width(&opt_line);
                            let opt_pt = point(
                                seg_b.origin.x + px((seg_slot.w - w) * 0.5),
                                seg_b.origin.y + px((seg_slot.h - 14.0 * zoom) * 0.5),
                            );
                            let _ = opt_line.paint(opt_pt, seg_b.size.height, TextAlign::Left, None, window, cx);
                        }
                    }
                    SettingControlKind::Slider {
                        display_val,
                        progress,
                        dec_hit,
                        inc_hit,
                        ..
                    } => {
                        let btn_w = 26.0 * zoom;
                        let dec_slot = Slot::new(row.control_slot.x, row.control_slot.y, btn_w, row.control_slot.h);
                        let inc_slot = Slot::new(row.control_slot.x + row.control_slot.w - btn_w, row.control_slot.y, btn_w, row.control_slot.h);
                        let dec_b = dec_slot.to_bounds(bounds.origin);
                        let inc_b = inc_slot.to_bounds(bounds.origin);

                        // Buttons - and +
                        paint_rounded(window, dec_b, 5.0 * zoom, if prepaint.hovered_hit == Some(*dec_hit) { colors.surface_hover } else { colors.background.opacity(0.4) });
                        window.paint_quad(gpui::outline(dec_b, colors.border, Default::default()).corner_radii(px(5.0 * zoom)));
                        let minus_line = shape_text("−", &font, px(13.0 * zoom), colors.foreground, window);
                        let _ = minus_line.paint(point(dec_b.origin.x + px(8.0 * zoom), dec_b.origin.y + px(4.0 * zoom)), dec_b.size.height, TextAlign::Left, None, window, cx);

                        paint_rounded(window, inc_b, 5.0 * zoom, if prepaint.hovered_hit == Some(*inc_hit) { colors.surface_hover } else { colors.background.opacity(0.4) });
                        window.paint_quad(gpui::outline(inc_b, colors.border, Default::default()).corner_radii(px(5.0 * zoom)));
                        let plus_line = shape_text("+", &font, px(13.0 * zoom), colors.foreground, window);
                        let _ = plus_line.paint(point(inc_b.origin.x + px(8.0 * zoom), inc_b.origin.y + px(4.0 * zoom)), inc_b.size.height, TextAlign::Left, None, window, cx);

                        // Center Progress Bar & Badge
                        let bar_slot = Slot::new(
                            row.control_slot.x + btn_w + 6.0 * zoom,
                            row.control_slot.y + (row.control_slot.h - 6.0 * zoom) * 0.5,
                            (row.control_slot.w - (btn_w + 6.0 * zoom) * 2.0).max(1.0),
                            6.0 * zoom,
                        );
                        let bar_b = bar_slot.to_bounds(bounds.origin);
                        paint_rounded(window, bar_b, 3.0 * zoom, colors.border.opacity(0.5));
                        let fill_w = bar_b.size.width * *progress;
                        paint_rounded(
                            window,
                            Bounds { origin: bar_b.origin, size: size(fill_w, bar_b.size.height) },
                            3.0 * zoom,
                            colors.accent,
                        );

                        // Value badge text
                        let val_line = shape_text(display_val, &font, px(11.0 * zoom), colors.foreground, window);
                        let w = line_width(&val_line);
                        let val_pt = point(
                            bar_b.origin.x + px((bar_slot.w - w) * 0.5),
                            bounds.origin.y + px(row.control_slot.y - 10.0 * zoom),
                        );
                        let _ = val_line.paint(val_pt, px(12.0 * zoom), TextAlign::Left, None, window, cx);
                    }
                    SettingControlKind::KeybindingRow { keys, .. } => {
                        let badge_w = 90.0 * zoom;
                        let badge_h = 24.0 * zoom;
                        let badge_slot = Slot::new(
                            row.control_slot.x + row.control_slot.w - badge_w,
                            row.control_slot.y + (row.control_slot.h - badge_h) * 0.5,
                            badge_w,
                            badge_h,
                        );
                        let badge_b = badge_slot.to_bounds(bounds.origin);
                        paint_rounded(window, badge_b, 5.0 * zoom, colors.accent.opacity(0.14));
                        window.paint_quad(gpui::outline(badge_b, colors.accent.opacity(0.4), Default::default()).corner_radii(px(5.0 * zoom)));
                        let key_line = shape_text(*keys, &font, px(11.0 * zoom), colors.accent, window);
                        let kw = line_width(&key_line);
                        let key_pt = point(
                            badge_b.origin.x + px((badge_slot.w - kw) * 0.5),
                            badge_b.origin.y + px((badge_slot.h - 13.0 * zoom) * 0.5),
                        );
                        let _ = key_line.paint(key_pt, badge_b.size.height, TextAlign::Left, None, window, cx);
                    }
                    SettingControlKind::AboutAction { label, url } => {
                        let btn_b = row.control_slot.to_bounds(bounds.origin);
                        let is_btn_hovered = prepaint.hovered_hit == Some(SettingsHit::AboutUrl(*url));
                        paint_rounded(window, btn_b, 6.0 * zoom, if is_btn_hovered { colors.accent } else { colors.surface_hover });
                        window.paint_quad(gpui::outline(btn_b, colors.border, Default::default()).corner_radii(px(6.0 * zoom)));
                        let act_line = shape_text(
                            *label,
                            &font,
                            px(11.5 * zoom),
                            if is_btn_hovered { colors.accent_fg } else { colors.foreground },
                            window,
                        );
                        let w = line_width(&act_line);
                        let act_pt = point(
                            btn_b.origin.x + px((row.control_slot.w - w) * 0.5),
                            btn_b.origin.y + px((row.control_slot.h - 14.0 * zoom) * 0.5),
                        );
                        let _ = act_line.paint(act_pt, btn_b.size.height, TextAlign::Left, None, window, cx);
                    }
                }
            }
        });

        // 6. Interactive Tooltip Hint Bar
        if let Some(row_idx) = prepaint.hovered_row_idx {
            if let Some(row) = layout.rows.get(row_idx) {
                let tip_h = 24.0 * zoom;
                let tip_slot = Slot::new(
                    layout.content_viewport.x + 16.0 * zoom,
                    layout.content_viewport.y + layout.content_viewport.h - tip_h - 6.0 * zoom,
                    layout.content_viewport.w - 32.0 * zoom,
                    tip_h,
                );
                let tip_b = tip_slot.to_bounds(bounds.origin);
                paint_rounded(window, tip_b, 6.0 * zoom, colors.surface);
                window.paint_quad(gpui::outline(tip_b, colors.accent.opacity(0.35), Default::default()).corner_radii(px(6.0 * zoom)));
                let tip_str = format!("💡 {}", row.tooltip);
                let tip_line = shape_text_truncated(&tip_str, &font, px(11.0 * zoom), colors.muted, tip_slot.w - 16.0 * zoom, window);
                let tip_pt = point(tip_b.origin.x + px(8.0 * zoom), tip_b.origin.y + px(4.0 * zoom));
                let _ = tip_line.paint(tip_pt, tip_b.size.height, TextAlign::Left, None, window, cx);
            }
        }

        // 7. Scrollbar
        if let Some(thumb_slot) = layout.scrollbar_slot {
            paint_rounded(
                window,
                thumb_slot.to_bounds(bounds.origin),
                2.5 * zoom,
                colors.muted.opacity(0.55),
            );
        }

        // 8. Event Handlers (Mouse, Scroll, Keyboard)
        let Some(app_entity) = self.app.upgrade() else {
            return;
        };

        // ScrollWheel
        let app_scroll = app_entity.clone();
        let hitbox = prepaint.hitbox.clone();
        let content_vp = layout.content_viewport;
        let total_content_h = layout.rows.last().map(|r| r.card_slot.y + r.card_slot.h).unwrap_or(0.0)
            - content_vp.y
            + 24.0 * zoom;
        let max_scroll = (total_content_h - content_vp.h).max(0.0);

        window.on_mouse_event::<ScrollWheelEvent>(move |ev, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            let pos = ev.position;
            if content_vp.to_bounds(bounds.origin).contains(&pos) {
                let dy = f32::from(ev.delta.pixel_delta(px(32.0 * zoom)).y);
                app_scroll.update(cx, |app, cx| {
                    let old_y = f32::from(app.settings_scroll.offset().y);
                    let new_y = (old_y + dy).clamp(-max_scroll, 0.0);
                    app.settings_scroll.set_offset(point(px(0.0), px(new_y)));
                    cx.notify();
                });
                cx.stop_propagation();
            }
        });

        // Mouse Down (Clicks on controls, tabs, buttons, close)
        let app_click = app_entity.clone();
        let hovered_hit = prepaint.hovered_hit;
        let hitbox_click = prepaint.hitbox.clone();
        let modal_slot = layout.modal_slot;
        window.on_mouse_event::<MouseDownEvent>(move |ev, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox_click.is_hovered(window) {
                return;
            }
            if ev.button != MouseButton::Left {
                return;
            }
            let local_x = f32::from(ev.position.x - bounds.origin.x);
            let local_y = f32::from(ev.position.y - bounds.origin.y);
            if !modal_slot.contains(local_x, local_y) {
                app_click.update(cx, |app, cx| {
                    app.state.overlay.show_settings = false;
                    app.settings_input = None;
                    cx.notify();
                });
                cx.stop_propagation();
                return;
            }
            if let Some(hit) = hovered_hit {
                app_click.update(cx, |app, cx| {
                    match hit {
                        SettingsHit::CloseButton => {
                            app.state.overlay.show_settings = false;
                            app.settings_input = None;
                        }
                        SettingsHit::TabSelect(tab) => {
                            app.settings_section = tab.index();
                            if let Some(input) = app.settings_input.clone() {
                                input.update(cx, |state, cx| state.set_value("", window, cx));
                            }
                        }
                        SettingsHit::ThumbnailBudgetDec | SettingsHit::ThumbnailBudgetInc | SettingsHit::ThumbnailBudgetBar(_) => {
                            let current = app.state.settings.thumbnail_cache_mb;
                            let next = match hit {
                                SettingsHit::ThumbnailBudgetDec => current.saturating_sub(32),
                                SettingsHit::ThumbnailBudgetInc => current.saturating_add(32),
                                SettingsHit::ThumbnailBudgetBar(fraction) => (32.0 + fraction.clamp(0.0, 1.0) * 992.0).round() as usize,
                                _ => current,
                            }.clamp(32, 1024);
                            app.state.settings.thumbnail_cache_mb = next;
                            app.thumb_textures.set_budget_mb(next);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::ToggleEcoMode => {
                            app.state.settings.eco_mode = !app.state.settings.eco_mode;
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::ToggleHiddenFiles => {
                            app.process_actions(vec![Action::ToggleHidden]);
                        }
                        SettingsHit::ToggleVimMode => {
                            app.process_actions(vec![Action::ToggleVimMode]);
                        }
                        SettingsHit::ToggleSidebar => {
                            app.process_actions(vec![Action::ToggleSidebar]);
                        }
                        SettingsHit::ToggleInspector => {
                            app.state.settings.show_inspector = !app.state.settings.show_inspector;
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::ToggleAnimations => {
                            app.state.settings.disable_animations = !app.state.settings.disable_animations;
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::ToggleAutosave => {
                            app.state.settings.autosave_session = !app.state.settings.autosave_session;
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::ToggleDimPanes => {
                            app.state.settings.dim_inactive_panes = !app.state.settings.dim_inactive_panes;
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::ToggleTitlebar => {
                            app.state.settings.show_titlebar = !app.state.settings.show_titlebar;
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::LanguageSelect(idx) => {
                            let (lang, locale) = match idx {
                                1 => (Some("en".to_string()), "en"),
                                2 => (Some("es".to_string()), "es"),
                                _ => (None, "en"),
                            };
                            app.state.settings.language = lang;
                            rust_i18n::set_locale(locale);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::ZoomDec => {
                            app.process_actions(vec![Action::ZoomOut]);
                        }
                        SettingsHit::ZoomReset => {
                            app.process_actions(vec![Action::ZoomReset]);
                        }
                        SettingsHit::ZoomInc => {
                            app.process_actions(vec![Action::ZoomIn]);
                        }
                        SettingsHit::ZoomBar(frac) => {
                            let new_zoom = (0.5 + frac * 1.5).clamp(0.5, 2.0);
                            app.state.settings.global_zoom = (new_zoom * 20.0).round() / 20.0;
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::GlassOpacityDec => {
                            let cur = app.state.settings.glass_opacity;
                            app.state.settings.glass_opacity = (cur - 0.05).clamp(0.20, 1.0);
                            app.theme_dirty = true;
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::GlassOpacityInc => {
                            let cur = app.state.settings.glass_opacity;
                            app.state.settings.glass_opacity = (cur + 0.05).clamp(0.20, 1.0);
                            app.theme_dirty = true;
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::GlassOpacityBar(frac) => {
                            app.state.settings.glass_opacity = (0.20 + frac * 0.80).clamp(0.20, 1.0);
                            app.theme_dirty = true;
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::ScrollSmoothDec => {
                            let cur = app.state.settings.scroll_smoothness;
                            app.state.settings.scroll_smoothness = (cur - 0.1).clamp(0.0, 1.0);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::ScrollSmoothInc => {
                            let cur = app.state.settings.scroll_smoothness;
                            app.state.settings.scroll_smoothness = (cur + 0.1).clamp(0.0, 1.0);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::ScrollSmoothBar(frac) => {
                            app.state.settings.scroll_smoothness = frac.clamp(0.0, 1.0);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::SidebarWidthDec => {
                            app.state.settings.sidebar_width = (app.state.settings.sidebar_width - 20.0).clamp(160.0, 420.0);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::SidebarWidthInc => {
                            app.state.settings.sidebar_width = (app.state.settings.sidebar_width + 20.0).clamp(160.0, 420.0);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::InspectorWidthDec => {
                            app.state.settings.inspector_width = (app.state.settings.inspector_width - 20.0).clamp(180.0, 2000.0);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::InspectorWidthInc => {
                            app.state.settings.inspector_width = (app.state.settings.inspector_width + 20.0).clamp(180.0, 2000.0);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::FontSizeDec => {
                            app.state.settings.font_size = (app.state.settings.font_size - 1.0).clamp(10.0, 24.0);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::FontSizeInc => {
                            app.state.settings.font_size = (app.state.settings.font_size + 1.0).clamp(10.0, 24.0);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::ThumbnailSizeDec => {
                            app.state.settings.thumbnail_size = (app.state.settings.thumbnail_size - 16.0).clamp(64.0, 256.0);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::ThumbnailSizeInc => {
                            app.state.settings.thumbnail_size = (app.state.settings.thumbnail_size + 16.0).clamp(64.0, 256.0);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::GridSpacingDec => {
                            app.state.settings.grid_spacing = (app.state.settings.grid_spacing - 2.0).clamp(4.0, 48.0);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::GridSpacingInc => {
                            app.state.settings.grid_spacing = (app.state.settings.grid_spacing + 2.0).clamp(4.0, 48.0);
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::ThemeSelect(idx) => {
                            let themes = ["Atlas Graphite Glass", "Atlas Midnight", "One Dark", "Dracula", "Nord"];
                            if let Some(t) = themes.get(idx) {
                                app.set_theme(t.to_string());
                            }
                        }
                        SettingsHit::ViewModeSelect(idx) => {
                            let modes = [ViewMode::Details, ViewMode::Grid, ViewMode::List, ViewMode::Miller, ViewMode::Tree];
                            if let Some(m) = modes.get(idx) {
                                app.state.settings.view_mode = m.clone();
                                atlas_core::util::config::save_settings(&app.state.settings);
                            }
                        }
                        SettingsHit::InspectorPlacementSelect(idx) => {
                            app.state.settings.inspector_placement = if idx == 0 { InspectorPlacement::Global } else { InspectorPlacement::ActiveTab };
                            atlas_core::util::config::save_settings(&app.state.settings);
                        }
                        SettingsHit::FontFamilySelect(idx) => {
                            let fonts = ["Geist", "Segoe UI Variable", ".SystemUIFont"];
                            if let Some(f) = fonts.get(idx) {
                                app.state.settings.font_family = f.to_string();
                                atlas_core::util::config::save_settings(&app.state.settings);
                            }
                        }
                        SettingsHit::AboutUrl(url) => {
                            if url == crate::ui::overlays::legal::NOTICES_URL {
                                crate::ui::overlays::legal::open_notices();
                            } else {
                                let _ = open::that(url);
                            }
                        }
                    }
                    cx.notify();
                });
                cx.stop_propagation();
            }
        });

        // Escape Key Dismiss
        let app_esc = app_entity.clone();
        window.on_key_event::<KeyDownEvent>(move |ev, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || ev.keystroke.key != "escape" {
                return;
            }
            app_esc.update(cx, |app, cx| {
                app.state.overlay.show_settings = false;
                app.settings_input = None;
                cx.notify();
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_tab_indices_and_mapping() {
        assert_eq!(SettingsTab::General.index(), 0);
        assert_eq!(SettingsTab::Appearance.index(), 1);
        assert_eq!(SettingsTab::Files.index(), 2);
        assert_eq!(SettingsTab::Layout.index(), 3);
        assert_eq!(SettingsTab::Extensions.index(), 4);
        assert_eq!(SettingsTab::About.index(), 5);

        assert_eq!(SettingsTab::from_index(0), SettingsTab::General);
        assert_eq!(SettingsTab::from_index(1), SettingsTab::Appearance);
        assert_eq!(SettingsTab::from_index(2), SettingsTab::Files);
        assert_eq!(SettingsTab::from_index(3), SettingsTab::Layout);
        assert_eq!(SettingsTab::from_index(4), SettingsTab::Extensions);
        assert_eq!(SettingsTab::from_index(5), SettingsTab::About);
    }

    #[test]
    fn test_rect_cut_modal_dialog_layout() {
        let settings = atlas_model::settings::Settings::default();
        let layout = SettingsImmLayout::compute(
            1000.0,
            800.0,
            1.0,
            SettingsTab::General,
            "",
            0.0,
            &settings,
            "Atlas Graphite Glass",
        );

        // Modal should be horizontally and vertically centered
        assert!(layout.modal_slot.w >= 560.0 && layout.modal_slot.w <= 840.0);
        assert!(layout.modal_slot.h >= 380.0 && layout.modal_slot.h <= 620.0);
        assert!((layout.modal_slot.x - (1000.0 - layout.modal_slot.w) * 0.5).abs() < 1e-3);
        assert!((layout.modal_slot.y - (800.0 - layout.modal_slot.h) * 0.5).abs() < 1e-3);

        // Header slot should be exactly top of modal with 50px height
        assert_eq!(layout.header_slot.y, layout.modal_slot.y);
        assert_eq!(layout.header_slot.h, 50.0);
        assert_eq!(layout.header_slot.w, layout.modal_slot.w);

        // Close button sits on the right
        assert!(layout.close_slot.x + layout.close_slot.w <= layout.header_slot.x + layout.header_slot.w);

        // Sidebar width should be ~180px
        assert_eq!(layout.sidebar_slot.w, 180.0);
        assert_eq!(layout.divider_slot.w, 1.0);

        // Content viewport takes remainder
        let expected_content_w = layout.modal_slot.w - 180.0 - 1.0;
        assert_eq!(layout.content_viewport.w, expected_content_w);
    }

    #[test]
    fn test_tab_navigation_slicing() {
        let settings = atlas_model::settings::Settings::default();
        let layout = SettingsImmLayout::compute(
            1000.0,
            800.0,
            1.0,
            SettingsTab::Appearance,
            "",
            0.0,
            &settings,
            "One Dark",
        );

        assert_eq!(layout.sidebar_tabs.len(), 6);
        for i in 0..layout.sidebar_tabs.len() {
            let (tab, slot) = &layout.sidebar_tabs[i];
            assert_eq!(tab.index(), i);
            assert_eq!(slot.h, 36.0);
            if i > 0 {
                let (_, prev_slot) = &layout.sidebar_tabs[i - 1];
                // Non-overlapping with 4px gap
                assert_eq!(slot.y, prev_slot.y + prev_slot.h + 4.0);
            }
        }
    }

    #[test]
    fn test_setting_row_control_slots() {
        let settings = atlas_model::settings::Settings::default();
        let layout = SettingsImmLayout::compute(
            1000.0,
            800.0,
            1.0,
            SettingsTab::General,
            "",
            0.0,
            &settings,
            "One Dark",
        );

        // General tab should generate rows (Language, Zoom, Eco, Vim, Autosave)
        assert!(!layout.rows.is_empty());
        for row in &layout.rows {
            // Info slot and control slot must fit inside card slot without overlap
            assert!(row.info_slot.x >= row.card_slot.x);
            assert!(row.control_slot.x + row.control_slot.w <= row.card_slot.x + row.card_slot.w);
            assert!(row.info_slot.x + row.info_slot.w <= row.control_slot.x);
        }
    }

    #[test]
    fn test_keybindings_filtering() {
        let settings = atlas_model::settings::Settings::default();
        // Query for "vim"
        let layout = SettingsImmLayout::compute(
            1000.0,
            800.0,
            1.0,
            SettingsTab::Extensions,
            "vim",
            0.0,
            &settings,
            "One Dark",
        );

        assert!(!layout.rows.is_empty());
        for row in &layout.rows {
            let matches = row.title.to_lowercase().contains("vim")
                || row.description.to_lowercase().contains("vim");
            assert!(matches);
        }
    }

    #[test]
    fn test_settings_search_filtering() {
        let settings = atlas_model::settings::Settings::default();
        // Search query for "hidden"
        let layout = SettingsImmLayout::compute(
            1000.0,
            800.0,
            1.0,
            SettingsTab::General,
            "hidden",
            0.0,
            &settings,
            "One Dark",
        );

        assert_eq!(layout.rows.len(), 1);
        assert_eq!(layout.rows[0].title, "Show Hidden Files");
    }

    #[test]
    fn test_slider_math_clamping() {
        let min_opacity = 0.20;
        let max_opacity = 1.00;
        let frac_zero = 0.0_f32;
        let frac_one = 1.0_f32;
        let frac_mid = 0.5_f32;

        let val_zero = (min_opacity + frac_zero * (max_opacity - min_opacity)).clamp(min_opacity, max_opacity);
        assert!((val_zero - 0.20).abs() < 1e-4);

        let val_one = (min_opacity + frac_one * (max_opacity - min_opacity)).clamp(min_opacity, max_opacity);
        assert!((val_one - 1.00).abs() < 1e-4);

        let val_mid = (min_opacity + frac_mid * (max_opacity - min_opacity)).clamp(min_opacity, max_opacity);
        assert!((val_mid - 0.60).abs() < 1e-4);
    }
}
