//! Drag-and-drop neutral layer.
//!
//! All platform-specific implementations (Windows OLE, future macOS/Linux) live in
//! sub-modules and expose their functionality through the types defined here. Business
//! logic (move vs copy rules, hit-testing, action dispatch) lives exclusively in this
//! module — no `#[cfg(target_os)]` in the callers.
//!
//! # Design
//! ```text
//! dnd/mod.rs            — DropEffect, DropZones (neutral)
//! dnd/windows/mod.rs    — start_drag_real(), setup_dnd()
//! dnd/macos.rs          — (future) NSPasteboard / NSDraggingSource
//! dnd/linux.rs          — (future) XDND / wl_data_device
//! ```

use std::path::PathBuf;

// Platform implementation.
#[cfg(target_os = "windows")]
pub mod windows;

// ─── Core types ─────────────────────────────────────────────────────────────

/// Default outcome of a drop, decided by volume rules (same drive = move).
/// Platform-neutral so callers need not import Win32 `DROPEFFECT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropEffect {
    Copy,
    Move,
}

impl DropEffect {
    /// Returns `true` when source files should be removed after the drop
    /// (i.e., the operation was effectively a move).
    pub fn is_move(self) -> bool {
        matches!(self, DropEffect::Move)
    }
}

// ─── Drop-zone registry (used by the platform drop-target) ──────────────────

/// A single region in window-space coordinates that can accept a drop, paired
/// with the destination directory and the pane that owns it.
#[derive(Clone)]
pub struct DropZone {
    /// Window-space bounding box (origin = top-left of the window client area).
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Destination directory for dropped files.
    pub dest: PathBuf,
    /// Numeric pane identifier so the drop target can route correctly.
    pub pane_id: u64,
}

impl DropZone {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Per-frame registry of all visible drop zones, shared between the gpui render
/// path (writer) and the platform drop-target vtable (reader).
///
/// The render path calls [`DropZones::update`] on each frame only while a drag
/// is active (gated by [`DropZones::drag_active`]) to avoid work in the common
/// case. The platform drop-target calls [`DropZones::hit`] from the COM vtable.
#[derive(Default)]
pub struct DropZones {
    /// Set to `true` when the OS notifies us of an incoming drag (DragEnter).
    pub drag_active: bool,
    zones: Vec<DropZone>,
}

impl DropZones {
    /// Replace all zones for the current frame. Called from the render path.
    pub fn update(&mut self, zones: Vec<DropZone>) {
        self.zones = zones;
    }

    pub fn len(&self) -> usize {
        self.zones.len()
    }

    /// Find the deepest (last) zone that contains the given window-space point.
    /// Returns `None` when no zone matches.
    pub fn hit(&self, px: f32, py: f32) -> Option<&DropZone> {
        self.zones.iter().rev().find(|z| z.contains(px, py))
    }
}

// ─── Move-vs-copy heuristic (cross-platform) ────────────────────────────────

/// Returns the drop effect to use when no modifier keys are held, following
/// Windows Explorer semantics: same volume → move, different volume → copy.
///
/// On platforms without volume concepts (SFTP, cloud), returns `Copy`.
pub fn default_effect_for_paths(sources: &[PathBuf], dest: &PathBuf) -> DropEffect {
    // Resolve the destination's root (drive letter on Windows, mount point on Linux).
    let dest_root = volume_root(dest);
    let all_same = sources
        .iter()
        .all(|s| volume_root(s).as_deref() == dest_root.as_deref());
    if all_same {
        DropEffect::Move
    } else {
        DropEffect::Copy
    }
}

/// Applies modifier-key overrides (Ctrl → Copy, Shift → Move) on top of the
/// default effect. `ctrl` and `shift` correspond to the key states at drop time.
/// Used by platform drop-target implementations; also exported for tests.
#[allow(dead_code)]
pub fn effect_with_modifiers(default: DropEffect, ctrl: bool, shift: bool) -> DropEffect {
    if ctrl {
        DropEffect::Copy
    } else if shift {
        DropEffect::Move
    } else {
        default
    }
}

fn volume_root(path: &PathBuf) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        // On Windows the volume is the drive prefix ("C:\", "\\Server\Share\").
        let s = path.to_string_lossy();
        if s.starts_with(r"\\") {
            // UNC path — take the server+share components.
            let parts: Vec<&str> = s[2..].splitn(3, '\\').collect();
            if parts.len() >= 2 {
                return Some(format!(r"\\{}\{}", parts[0], parts[1]).to_lowercase());
            }
        }
        // Drive letter path.
        let prefix: String = s.chars().take(3).collect(); // "C:\"
        if prefix.len() == 3 && prefix.chars().nth(1) == Some(':') {
            return Some(prefix.to_lowercase());
        }
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        // On Unix, use the first two components as a heuristic (mount points).
        // A proper implementation would call `statvfs(2)`.
        let mut components = path.components();
        let _ = components.next(); // /
        let mount = components
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned());
        mount
    }
}
