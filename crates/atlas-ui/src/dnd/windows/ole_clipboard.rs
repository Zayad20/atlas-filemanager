//! System-clipboard interop via OLE, reusing the drag-and-drop `AtlasDataObject`.
//!
//! Copy/Cut publish `CF_HDROP` + `CFSTR_PREFERREDDROPEFFECT` (+ "PNG"/`CF_DIB`
//! for a single image) with `OleSetClipboard`, so Explorer and other apps can
//! paste Atlas files. Paste reads the same formats back with `OleGetClipboard`.
//!
//! Cut semantics follow the shell protocol: when another app pastes our cut it
//! writes `Performed DropEffect` via `SetData` — the data object forwards that
//! to the App ([`BgResult::ClipboardPerformed`]) which trashes the sources
//! (unoptimized move) or just invalidates the clipboard (optimized move).
#![allow(unsafe_code)]
// reason: OLE clipboard and HGLOBAL access are inherently unsafe Win32 calls.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;
use std::sync::Arc;

use atlas_model::app_state::BgResult;
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    OleFlushClipboard, OleGetClipboard, OleSetClipboard, DROPEFFECT_COPY, DROPEFFECT_MOVE,
};

use super::clipboard_formats::{cf_performed_drop_effect, cf_preferred_drop_effect};
use super::data_object::{build_bytes_hglobal, AtlasDataObject, FormatEntry};
use super::drop_target::{is_atlas_source, read_cf_hdrop};

/// Don't decode images above this size for the PNG/DIB clipboard extras —
/// `set_clipboard_files` runs on the UI thread.
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

const fn fmt_u32(cf: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: cf,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

unsafe fn read_u32_format(obj: &IDataObject, cf: u16) -> Option<u32> {
    use windows::Win32::System::Ole::ReleaseStgMedium;
    let fmt = fmt_u32(cf);
    let mut medium = obj.GetData(std::ptr::from_ref(&fmt)).ok()?;
    if medium.u.hGlobal.is_invalid() {
        return None;
    }
    let ptr = GlobalLock(medium.u.hGlobal) as *const u32;
    let val = if ptr.is_null() {
        None
    } else {
        Some(ptr.read())
    };
    if !ptr.is_null() {
        let _ = GlobalUnlock(medium.u.hGlobal);
    }
    ReleaseStgMedium(std::ptr::from_mut(&mut medium));
    val
}

// ─── image extras ────────────────────────────────────────────────────────────

fn is_image_ext(path: &Path) -> bool {
    path.extension().is_some_and(|e| {
        matches!(
            e.to_string_lossy().to_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" | "ico"
        )
    })
}

/// "PNG" registered clipboard format (used by browsers, Discord, GIMP…).
fn cf_png() -> u16 {
    use std::sync::OnceLock;
    use windows::core::w;
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    static ID: OnceLock<u16> = OnceLock::new();
    *ID.get_or_init(|| unsafe { RegisterClipboardFormatW(w!("PNG")) as u16 })
}

/// Build a packed 32-bpp `CF_DIB` (BITMAPINFOHEADER + bottom-up BGRA rows).
fn build_dib(img: &image::RgbaImage) -> Vec<u8> {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut out = Vec::with_capacity(40 + (w * h * 4) as usize);
    // BITMAPINFOHEADER
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&w.to_le_bytes()); // biWidth
    out.extend_from_slice(&h.to_le_bytes()); // biHeight (positive = bottom-up)
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    out.extend_from_slice(&0u32.to_le_bytes()); // biSizeImage
    out.extend_from_slice(&[0u8; 16]); // x/y ppm, clr used/important
                                       // Bottom-up BGRA pixel rows.
    for row in img.rows().rev() {
        for px in row {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    out
}

/// PNG + DIB clipboard entries for a single image file, when cheap enough.
fn image_format_entries(paths: &[PathBuf]) -> Vec<FormatEntry> {
    let [path] = paths else { return Vec::new() };
    if !is_image_ext(path)
        || !path
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() <= MAX_IMAGE_BYTES)
    {
        return Vec::new();
    }
    #[allow(clippy::disallowed_methods)]
    // reason: ≤20 MB read on Ctrl+C of a single image; OleSetClipboard must run
    // on this (STA) thread anyway.
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(img) = image::load_from_memory(&bytes) else {
        return Vec::new();
    };
    let rgba = img.to_rgba8();
    let png_bytes = if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        bytes
    } else {
        let mut buf = std::io::Cursor::new(Vec::new());
        if img.write_to(&mut buf, image::ImageFormat::Png).is_err() {
            return Vec::new();
        }
        buf.into_inner()
    };
    let dib = build_dib(&rgba);
    unsafe {
        vec![
            FormatEntry::new_hglobal(cf_png(), build_bytes_hglobal(&png_bytes)),
            FormatEntry::new_hglobal(
                windows::Win32::System::Ole::CF_DIB.0,
                build_bytes_hglobal(&dib),
            ),
        ]
    }
}

// ─── public API ──────────────────────────────────────────────────────────────

/// Publish `paths` to the system clipboard as a copy (`cut = false`) or cut.
/// `tx` receives [`BgResult::ClipboardPerformed`] when a paste target reports
/// what it did with the data.
pub fn set_clipboard_files(
    paths: &[PathBuf],
    cut: bool,
    tx: flume::Sender<BgResult>,
) -> Result<(), String> {
    let effect = if cut {
        DROPEFFECT_MOVE
    } else {
        DROPEFFECT_COPY
    };
    let extras = if cut {
        Vec::new()
    } else {
        image_format_entries(paths)
    };
    let obj = AtlasDataObject::with_extras(
        paths,
        effect,
        Arc::new(AtomicU32::new(0)),
        extras,
        Some((tx, paths.to_vec())),
    )
    .ok_or_else(|| "no se pudo preparar el contenido".to_string())?;
    unsafe { OleSetClipboard(&obj) }.map_err(|e| e.message())
}

pub struct ClipboardContents {
    pub paths: Vec<PathBuf>,
    /// The source prefers a move (it was a cut).
    pub preferred_move: bool,
    /// The clipboard content was put there by Atlas itself.
    pub from_atlas: bool,
}

/// Read file paths (`CF_HDROP`) from the system clipboard, if any.
pub fn get_clipboard_files() -> Option<ClipboardContents> {
    unsafe {
        let obj = OleGetClipboard().ok()?;
        let paths = read_cf_hdrop(&obj);
        if paths.is_empty() {
            return None;
        }
        let preferred = read_u32_format(&obj, cf_preferred_drop_effect()).unwrap_or(0);
        Some(ClipboardContents {
            paths,
            preferred_move: preferred & DROPEFFECT_MOVE.0 != 0,
            from_atlas: is_atlas_source(&obj),
        })
    }
}

/// After successfully pasting files cut in another app: report
/// `Performed DropEffect = MOVE` back to the source data object so it clears
/// its cut state (e.g. Explorer un-ghosts and stops offering the cut).
pub fn acknowledge_external_cut() {
    unsafe {
        let Ok(obj) = OleGetClipboard() else { return };
        let hg = super::data_object::build_u32_hglobal(DROPEFFECT_MOVE.0);
        if hg == 0 {
            return;
        }
        let mut medium: windows::Win32::System::Com::STGMEDIUM = std::mem::zeroed();
        medium.tymed = TYMED_HGLOBAL.0 as u32;
        medium.u.hGlobal = windows::Win32::Foundation::HGLOBAL(hg as *mut _);
        medium.pUnkForRelease = std::mem::ManuallyDrop::new(None);
        let fmt = fmt_u32(cf_performed_drop_effect());
        // fRelease = TRUE: the data object takes ownership of the HGLOBAL.
        let _ = obj.SetData(std::ptr::from_ref(&fmt), std::ptr::from_ref(&medium), true);
        // The cut is consumed; an empty clipboard avoids a second stale paste.
        let _ = OleSetClipboard(None);
    }
}

/// Empty the system clipboard (after our own cut has been consumed).
pub fn clear() {
    unsafe {
        let _ = OleSetClipboard(None);
    }
}

/// Make the clipboard contents outlive the app (called on shutdown).
pub fn flush() {
    unsafe {
        let _ = OleFlushClipboard();
    }
}
