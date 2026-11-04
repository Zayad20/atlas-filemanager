#![allow(dead_code)] // reason: retained API surface not exercised by every GPUI flow yet
use crate::ops::TransferConflictState;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

/// Shared state for the fuzzy palettes (goto bar, command palette, ext palette):
/// query text, the highlighted row, and a scroll-into-view request.
/// Framework-neutral — the open-animation origin is recomputed by the renderer,
/// not stored here.
#[derive(Default)]
pub struct FuzzyOverlayState {
    pub query: String,
    pub selected_idx: usize,
    pub scroll_to_selected: bool,
    /// Wall-clock start of the open tween (set on reset/open).
    pub opening_started_at: Option<std::time::Instant>,
    /// Tab toggle: show every item's description line, not just the selected one's.
    pub expanded: bool,
    /// Section ids the user collapsed via their headers.
    pub collapsed_sections: std::collections::HashSet<String>,
}

impl FuzzyOverlayState {
    pub fn reset(&mut self) {
        self.query.clear();
        self.selected_idx = 0;
        self.scroll_to_selected = false;
        self.opening_started_at = Some(std::time::Instant::now());
        self.expanded = false;
        self.collapsed_sections.clear();
    }
}

/// A single filesystem item's stat data for the [`PropertiesState`] overlay.
/// Read once (synchronously) when the overlay opens — not kept live.
#[derive(Debug, Clone)]
pub struct PropertyRow {
    pub name: String,
    pub path: PathBuf,
    pub kind: crate::file_entry::FileKind,
    pub size: Option<u64>,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub link_target: Option<PathBuf>,
    pub error: Option<String>,
    #[cfg(unix)]
    pub permissions: Option<UnixPermissions>,
    pub readonly: bool,
    pub is_symlink: bool,
}

/// Original inode identity and mode, plus the pending rwx edit.
#[cfg(unix)]
#[derive(Debug, Clone)]
pub struct UnixPermissions {
    pub uid: u32,
    pub gid: u32,
    pub device: u64,
    pub inode: u64,
    pub mode: u32,
    pub edited_mode: u32,
}

/// Cross-platform "Properties" overlay state (F3 #15) — the non-Windows
/// counterpart to the native shell Properties dialog. Rows are stat'd up front
/// so the overlay doesn't touch the filesystem on every render.
#[derive(Debug, Clone, Default)]
pub struct PropertiesState {
    pub rows: Vec<PropertyRow>,
    pub saving: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Error,
    Info,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub kind: ToastKind,
    pub created_at: Instant,
}

impl Toast {
    pub fn new(message: String, kind: ToastKind) -> Self {
        Self {
            message,
            kind,
            created_at: Instant::now(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GuideCoach {
    pub topic: usize,
    /// Index of the step waiting for the user; `== steps` once finished.
    pub step: usize,
    /// Fingerprint of the state the current step watches (`u64::MAX` = capture on next frame).
    pub base: u64,
}

#[derive(Default)]
pub struct OverlayState {
    pub show_goto_bar: bool,
    pub show_palette: bool,
    pub show_settings: bool,
    pub show_getting_started: bool,
    /// Feature guide overlay (task-oriented help) and its selected topic.
    pub show_guide: bool,
    pub guide_topic: usize,
    /// Interactive coach: the guide topic being practised and its current step.
    pub guide_coach: Option<GuideCoach>,
    pub getting_started_step: usize,
    pub getting_started_loading_since: Option<std::time::Instant>,
    pub getting_started_welcome_since: Option<std::time::Instant>,
    pub goto_state: FuzzyOverlayState,
    pub palette_state: FuzzyOverlayState,
    pub show_ext_palette: bool,
    pub ext_palette_state: FuzzyOverlayState,
    pub ext_palette_items: Vec<String>,
    #[cfg(not(target_arch = "wasm32"))]
    pub ext_palette_reply: Option<std::sync::mpsc::Sender<String>>,
    pub transfer_conflict: Option<TransferConflictState>,
    pub show_analyzer: bool,
    /// The rename-active-profile dialog is open.
    pub renaming_profile: bool,
    /// File-operation messages to show to the user as dismissible toasts.
    pub op_errors: std::collections::VecDeque<Toast>,
    /// Cross-platform "Properties" overlay (F3 #15). `None` when closed.
    pub properties: Option<PropertiesState>,
}
