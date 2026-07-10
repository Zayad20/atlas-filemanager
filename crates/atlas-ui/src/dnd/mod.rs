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
    /// Last drag cursor in logical window coordinates. The OLE modal loop can
    /// starve the async queue drain, while the paint canvas still ticks.
    pub cursor: Option<(f32, f32)>,
    zones: Vec<DropZone>,
}

impl DropZones {
    /// Replace all zones for the current frame. Called from the render path.
    pub fn update(&mut self, zones: Vec<DropZone>) {
        self.zones = zones;
    }

    /// Rewrite the destination of every zone owned by `pane_id`. Called when a
    /// pane navigates while an OLE drag is in flight: the committed zones still
    /// carry the pre-navigation directory, and the nested `DoDragDrop` loop can
    /// resolve a drop before the next frame recommits fresh zones.
    pub fn set_dest_for_pane(&mut self, pane_id: u64, dest: PathBuf) {
        for zone in &mut self.zones {
            if zone.pane_id == pane_id {
                zone.dest.clone_from(&dest);
            }
        }
    }

    /// Drop all zones owned by `pane_id` (pane navigated to a location with no
    /// filesystem directory — tag/recycle/remote — so it can't accept OLE drops).
    pub fn remove_pane(&mut self, pane_id: u64) {
        self.zones.retain(|zone| zone.pane_id != pane_id);
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
    let all_same = sources
        .iter()
        .all(|s| atlas_core::fs::paths::same_volume(s, dest));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_same_distro_moves_by_default() {
        assert_eq!(
            default_effect_for_paths(
                &[PathBuf::from(r"\\wsl$\Ubuntu\home\a")],
                &PathBuf::from(r"\\wsl.localhost\Ubuntu\tmp")
            ),
            DropEffect::Move
        );
    }

    #[test]
    fn wsl_different_distro_copies_by_default() {
        assert_eq!(
            default_effect_for_paths(
                &[PathBuf::from(r"\\wsl$\Ubuntu\home\a")],
                &PathBuf::from(r"\\wsl.localhost\Debian\home")
            ),
            DropEffect::Copy
        );
    }
}
