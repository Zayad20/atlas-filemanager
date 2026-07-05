//! Windows OLE drag-and-drop — platform implementation.
//!
//! # Entry points
//! - [`setup_dnd`] — called once in `App::new`; revokes gpui's drop target,
//!   registers our own, stores the HWND.
//! - [`start_drag_real`] — called from the gpui `on_drag` closure when the
//!   user drags real filesystem paths; blocks via `DoDragDrop` until the drop
//!   completes or is cancelled.
//!
//! # Crate-level `unsafe_code` note
//! The workspace forbids `unsafe_code` globally. Each file in this module
//! uses `#![allow(unsafe_code)]` at the top because COM, HGLOBAL and raw Win32
//! require it. All callers outside `dnd/windows/` are 100% safe.
#![allow(unsafe_code)]

pub mod clipboard_formats;
pub mod data_object;
pub mod drop_source;
pub mod drop_target;
pub mod ole_clipboard;

use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Ole::IDropTarget;
use windows::Win32::System::Ole::{
    DoDragDrop, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GetCursorPos, PostMessageW, SetWindowLongPtrW, GWLP_WNDPROC, WM_LBUTTONUP,
    WM_USER,
};

use crate::dnd::DropZones;
use data_object::AtlasDataObject;
use drop_source::AtlasDropSource;
use drop_target::DropPayload;

/// Events pushed by the OLE layer and drained by `App::drain_dnd_queue`.
pub enum DndEvent {
    /// Current drag cursor position over the Atlas window.
    DragOver { x: f32, y: f32 },
    /// The external/native drag left the Atlas window.
    DragLeave,
    /// Something was dropped onto an Atlas window.
    DropIn(DropPayload),
    /// A drag *out of* Atlas ended with the target reporting MOVE: the source
    /// files must now be deleted (to trash, recoverable).
    DragOutMoved { paths: Vec<PathBuf> },
    /// An OLE-layer failure the user should see as a toast.
    Error(String),
}

/// Shared queue type between the COM/WndProc layer and the App.
pub type DndQueue = Arc<Mutex<Vec<DndEvent>>>;

// ─── Deferred drag (avoids RefCell double-borrow) ────────────────────────────
//
// `DoDragDrop` runs a nested Win32 message loop. If called directly from a
// gpui event-handler closure (which holds a `borrow_mut` on gpui's App
// RefCell), that nested loop dispatches paint/mouse events → gpui tries a
// second `borrow_mut` → panic.
//
// Fix: the on_drag closure calls `trigger_drag()` which stores the drag params
// in a thread_local and posts WM_ATLAS_START_DRAG.  The message is processed by
// our WndProc hook *after* the on_drag closure returns and the RefCell borrow is
// released.

/// Custom Win32 message: start an OLE drag. Posted by `trigger_drag()`.
const WM_ATLAS_START_DRAG: u32 = WM_USER + 0x0D1A;

/// Original WndProc before we replaced it; chained from `atlas_wndproc`.
static ORIGINAL_WNDPROC: AtomicIsize = AtomicIsize::new(0);

struct PendingDrag {
    hwnd: isize,
    paths: Vec<PathBuf>,
    /// `Some` when dragging from inside an archive (Fase 5).
    archive_items: Option<Vec<(PathBuf, PathBuf)>>,
    /// Where to report move-completion / errors back to the App.
    queue: Option<DndQueue>,
    copy_only: bool,
}

thread_local! {
    static PENDING_DRAG: std::cell::Cell<Option<PendingDrag>> =
        std::cell::Cell::new(None);
}

/// WndProc hook. Intercepts `WM_ATLAS_START_DRAG` and calls `DoDragDrop` there,
/// outside any gpui entity borrow. All other messages are forwarded to the
/// original WndProc.
unsafe extern "system" fn atlas_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_ATLAS_START_DRAG {
        let pending = PENDING_DRAG.with(|p| p.take());
        if let Some(pd) = pending {
            if let Some(items) = pd.archive_items {
                if !items.is_empty() {
                    start_drag_archive(pd.hwnd, &items, pd.queue.as_ref());
                }
            } else if pd.copy_only && !pd.paths.is_empty() {
                start_drag_copy_only(pd.hwnd, &pd.paths);
            } else if !pd.paths.is_empty() {
                start_drag_and_handle(pd.hwnd, &pd.paths, pd.queue.as_ref());
            }
            // DoDragDrop's nested message loop consumes the real WM_LBUTTONUP that
            // ends the drag, so gpui never sees it and leaves cx.active_drag set,
            // causing the drag preview to stay painted after the drag finishes.
            // Inject a synthetic mouse-up so gpui can clean up its drag state.
            inject_mouse_up(HWND(pd.hwnd as *mut _));
        }
        return LRESULT(0);
    }
    let orig = ORIGINAL_WNDPROC.load(Ordering::Relaxed);
    if orig != 0 {
        // SAFETY: orig is a valid WNDPROC stored during setup_dnd.
        // SAFETY: orig is the valid WNDPROC isize returned by SetWindowLongPtrW.
        let wndproc: windows::Win32::UI::WindowsAndMessaging::WNDPROC =
            std::mem::transmute::<isize, windows::Win32::UI::WindowsAndMessaging::WNDPROC>(orig);
        CallWindowProcW(wndproc, hwnd, msg, wparam, lparam)
    } else {
        LRESULT(0)
    }
}

/// Post a synthetic `WM_LBUTTONUP` at the current cursor position to `hwnd`.
///
/// After `DoDragDrop` returns, the OS drag loop has consumed the real
/// `WM_LBUTTONUP` that released the mouse button. gpui never sees it and
/// therefore never clears `cx.active_drag`, leaving any drag preview element
/// painted indefinitely. Injecting a synthetic mouse-up lets gpui clean up.
///
/// The message is posted (not sent) so it is processed after this WndProc
/// returns; the `WNDPROC` chain forwards it to gpui's handler.
unsafe fn inject_mouse_up(hwnd: HWND) {
    let mut pt = POINT { x: 0, y: 0 };
    let _ = GetCursorPos(&mut pt);
    let _ = ScreenToClient(hwnd, &mut pt);
    // Pack (x, y) into lParam: low word = x, high word = y.
    let lparam = LPARAM(((pt.y as u32) << 16 | (pt.x as u16 as u32)) as isize);
    let _ = PostMessageW(Some(hwnd), WM_LBUTTONUP, WPARAM(0), lparam);
}

/// Install the Atlas WndProc hook for `hwnd`. Called once from `setup_dnd`.
fn install_wndproc_hook(hwnd: HWND) {
    unsafe {
        let orig = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, atlas_wndproc as *const () as isize);
        ORIGINAL_WNDPROC.store(orig, Ordering::Relaxed);
    }
}

/// Schedule an OLE drag to start on the next Win32 message, outside any
/// gpui entity borrow. Safe to call from a gpui event-handler closure.
///
/// `archive_items` is `Some` when dragging virtual files from an archive.
pub fn trigger_drag(
    hwnd_raw: isize,
    paths: Vec<PathBuf>,
    archive_items: Option<Vec<(PathBuf, PathBuf)>>,
    queue: Option<DndQueue>,
) {
    PENDING_DRAG.with(|p| {
        p.set(Some(PendingDrag {
            hwnd: hwnd_raw,
            paths,
            archive_items,
            queue,
            copy_only: false,
        }));
    });
    unsafe {
        let _ = PostMessageW(
            Some(HWND(hwnd_raw as *mut _)),
            WM_ATLAS_START_DRAG,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

pub fn trigger_drag_copy_only(hwnd_raw: isize, paths: Vec<PathBuf>) {
    PENDING_DRAG.with(|p| {
        p.set(Some(PendingDrag {
            hwnd: hwnd_raw,
            paths,
            archive_items: None,
            queue: None,
            copy_only: true,
        }));
    });
    unsafe {
        let _ = PostMessageW(
            Some(HWND(hwnd_raw as *mut _)),
            WM_ATLAS_START_DRAG,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

// ─── setup_dnd ───────────────────────────────────────────────────────────────

/// Per-window drag-and-drop state shared between the COM vtable and the App.
pub struct DndState {
    /// Drop target COM object — kept alive here (COM refcount).
    #[allow(dead_code)]
    target: IDropTarget,
    /// Drop zones updated every render frame (when a drag is active).
    pub zones: Arc<Mutex<DropZones>>,
    /// Events pushed by the OLE layer; drained by `App` each tick.
    pub action_queue: DndQueue,
}

/// Register Atlas as the OLE drop target for `hwnd`, replacing gpui's default.
///
/// `scale_factor` converts Win32 screen pixels to logical gpui pixels (HiDPI).
///
/// # Panics
/// Panics in debug builds if `RegisterDragDrop` fails. In release builds the
/// error is logged and `None` is returned.
pub fn setup_dnd(hwnd_raw: isize, scale_factor: f32) -> Option<DndState> {
    let hwnd = HWND(hwnd_raw as *mut _);
    let zones = Arc::new(Mutex::new(DropZones::default()));
    let action_queue = Arc::new(Mutex::new(Vec::new()));

    match drop_target::AtlasDropTarget::register(
        hwnd,
        scale_factor,
        zones.clone(),
        action_queue.clone(),
    ) {
        Ok(target) => {
            install_wndproc_hook(hwnd);
            Some(DndState {
                target,
                zones,
                action_queue,
            })
        }
        Err(err) => {
            tracing::error!(?err, "failed to register Atlas drop target");
            debug_assert!(false, "RegisterDragDrop failed: {err}");
            None
        }
    }
}

// ─── start_drag_real ─────────────────────────────────────────────────────────

/// Start an OLE drag of real filesystem `paths`.
///
/// Blocks until the drag operation completes (drop or cancel). Returns the
/// `DROPEFFECT` reported by the receiving target — the caller is responsible
/// for deleting source files if the effect is `DROPEFFECT_MOVE`.
///
/// Passing `DROPEFFECT_COPY | DROPEFFECT_MOVE` in `allowed` lets Explorer
/// choose move/copy based on volume + modifier keys.
///
/// # Blocking behaviour
/// `DoDragDrop` runs a nested Win32 message loop. During this loop:
/// - Mouse events go to the OS drag subsystem, not to our WndProc.
/// - WM_PAINT is serviced by DirectX/gpui's existing compositor; no re-entrant
///   `&mut App` access occurs (render is a separate path from event dispatch).
pub fn start_drag_real(hwnd_raw: isize, paths: &[PathBuf], allowed: DROPEFFECT) -> DROPEFFECT {
    let _ = hwnd_raw; // HWND not needed for DoDragDrop itself (IDropSource has no HWND field)

    let performed = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let Some(data_obj) = AtlasDataObject::new(paths, allowed, performed.clone()) else {
        tracing::error!("drag aborted: CF_HDROP allocation failed");
        return DROPEFFECT_NONE;
    };
    let source: windows::Win32::System::Ole::IDropSource = AtlasDropSource.into();

    let mut effect = DROPEFFECT_NONE;
    let hr = unsafe { DoDragDrop(&data_obj, &source, allowed, &mut effect) };

    // Some targets only report via CFSTR_PERFORMEDDROPEFFECT on the data object.
    let performed_val = performed.load(std::sync::atomic::Ordering::Relaxed);
    if performed_val != 0 {
        effect = DROPEFFECT(performed_val);
    }

    if hr.is_err() && hr != windows::Win32::Foundation::DRAGDROP_S_DROP {
        tracing::warn!("DoDragDrop returned unexpected hr: {:?}", hr);
    }

    effect
}

fn push_event(queue: Option<&DndQueue>, event: DndEvent) {
    if let Some(q) = queue {
        if let Ok(mut q) = q.lock() {
            q.push(event);
        }
    }
}

/// Convenience: start a drag and handle the "delete source on move" logic.
///
/// After `DoDragDrop`, if the target reported a MOVE, a [`DndEvent::DragOutMoved`]
/// is queued so the App trashes the sources (invariant: the intra-Atlas drop
/// target always reports COPY — see `drop_target.rs` — so an Atlas→Atlas move
/// never reaches this path and can't double-delete).
pub fn start_drag_and_handle(hwnd_raw: isize, paths: &[PathBuf], queue: Option<&DndQueue>) {
    let allowed = DROPEFFECT_COPY | DROPEFFECT_MOVE;
    let effect = start_drag_real(hwnd_raw, paths, allowed);

    if effect == DROPEFFECT_MOVE {
        push_event(
            queue,
            DndEvent::DragOutMoved {
                paths: paths.to_vec(),
            },
        );
    }
}

pub fn start_drag_copy_only(hwnd_raw: isize, paths: &[PathBuf]) {
    let _ = start_drag_real(hwnd_raw, paths, DROPEFFECT_COPY);
}

/// Fase 5: Start an OLE drag of virtual files (inside an archive).
///
/// Extracts `items` (archive_path, inner_path pairs) to a session-unique temp
/// directory synchronously, then delegates to `start_drag_and_handle`.
/// The temp directory is left on disk; the OS (or next startup) cleans it up.
pub fn start_drag_archive(hwnd_raw: isize, items: &[(PathBuf, PathBuf)], queue: Option<&DndQueue>) {
    if items.is_empty() {
        return;
    }

    // Build a unique temp session dir under %TEMP%\atlas-dnd.
    let session = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0);
    let temp_dir = std::env::temp_dir()
        .join("atlas-dnd")
        .join(session.to_string());
    if let Err(e) = std::fs::create_dir_all(&temp_dir) {
        push_event(
            queue,
            DndEvent::Error(format!("No se pudo preparar la carpeta temporal: {e}")),
        );
        return;
    }

    // Group by archive so each archive is opened once.
    let mut by_archive: std::collections::HashMap<PathBuf, Vec<PathBuf>> =
        std::collections::HashMap::new();
    for (archive, inner) in items {
        by_archive
            .entry(archive.clone())
            .or_default()
            .push(inner.clone());
    }

    let mut extracted: Vec<PathBuf> = Vec::new();
    for (archive_path, inners) in &by_archive {
        let Some(provider) = atlas_core::fs::archive::open(archive_path) else {
            push_event(
                queue,
                DndEvent::Error(format!(
                    "No se pudo abrir el archivo «{}» (¿dañado o protegido?)",
                    archive_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                )),
            );
            continue;
        };
        for inner in inners {
            let name = inner
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".to_string());
            let dest = temp_dir.join(&name);
            match provider.extract_to(inner, &dest) {
                Ok(()) => extracted.push(dest),
                Err(e) => push_event(
                    queue,
                    DndEvent::Error(format!("No se pudo extraer «{name}»: {e}")),
                ),
            }
        }
    }

    if extracted.is_empty() {
        return;
    }

    // Virtual items are always COPY — the source lives in a zip, not on disk.
    let _ = start_drag_real(hwnd_raw, &extracted, DROPEFFECT_COPY);
}
