//! gpui UI layer.
pub mod root;
// Window chrome (tab pills, breadcrumb, search pill, marquee) + pane interactions
// (rubber-band, inline rename, type-to-filter) split out of `root.rs`.
pub mod chrome;
pub mod interactions;
// Shared file drag-and-drop wiring used by every view.
pub mod drag;
// File-view renderers (Details/Grid/List/Miller) split out of `root.rs`.
pub mod views;
// Liquid-glass surface styling (Figma "Design" frame).
pub mod glass;
pub mod icons;
// FASE5: inspector preview panel + overlay layers.
pub mod inspector;
// Overlay focus arbiter: the single authority for keyboard ownership across
// modals/popovers/menus. Yields to gpui-component Dialog/Sheet; resolves the
// app's own overlays via an ordered claim table.
pub mod overlay_focus;
pub mod overlays;
// Unified fuzzy-palette engine (goto / commands / ext share one renderer), hosted
// in its own view so the open animation only re-renders the overlay (not App +
// every file pane). See module docs.
pub mod palette;
pub mod palette_host;
pub mod search_shell;
// Atlas–style right-click menu: Atlas items + native Windows shell items.
pub mod shell_context_menu;
// Shared single-line text input (native cursor/selection/clipboard) reused by
// every overlay field and inline rename.
pub mod text_input;
// FASE4: downscaled grid thumbnail cache.
pub mod thumbs;
// FASE A: central tabs/splits on the gpui-component Dock.
pub mod dock_glue;
pub mod pane_panel;
// FASE B: left sidebar as a Dock left_dock panel.
pub mod sidebar;
