//! Walking an `HMENU` produced by `IContextMenu::QueryContextMenu` into flat,
//! `Send` [`NativeMenuItem`] data: titles, separators, state, canonical verbs
//! (for de-duplication against Atlas's own items) and rasterised icons.
#![allow(unsafe_code)]

use std::ffi::c_void;

use windows::core::PWSTR;
use windows::Win32::Graphics::Gdi::{
    GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::UI::Shell::{IContextMenu, IContextMenu2, IContextMenu3, GCS_VERBW};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMenuItemCount, GetMenuItemInfoW, HMENU, MENUITEMINFOW, MFS_DEFAULT, MFS_DISABLED,
    MFS_GRAYED, MFT_SEPARATOR, MIIM_BITMAP, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_STRING,
    MIIM_SUBMENU, WM_INITMENUPOPUP,
};

use super::{NativeItemKind, NativeMenuItem, RawIcon};

/// Verbs that Atlas already provides in its own section; the native duplicates
/// are dropped so the menu shows each action once.
const DEDUPE_VERBS: &[&str] = &["cut", "copy", "paste", "delete", "rename", "open"];

/// Walk `hmenu` into flat items. Submenus are always left lazy
/// (`children: None`, filled on demand via `ExpandSubmenu`) so the initial build
/// only enumerates the top level — no eager `WM_INITMENUPOPUP` round-trips to
/// slow third-party handlers (Open with, Send to, 7-Zip).
pub(crate) fn walk_menu(
    hmenu: HMENU,
    icm: &IContextMenu,
    id_first: u32,
) -> Vec<NativeMenuItem> {
    let mut out = Vec::new();
    unsafe {
        let count = GetMenuItemCount(Some(hmenu));
        for i in 0..count {
            // First pass: get the string length + fixed fields.
            let mut info = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_STRING
                    | MIIM_ID
                    | MIIM_FTYPE
                    | MIIM_SUBMENU
                    | MIIM_BITMAP
                    | MIIM_STATE,
                ..Default::default()
            };
            if GetMenuItemInfoW(hmenu, i as u32, true, &mut info).is_err() {
                continue;
            }

            if (info.fType.0 & MFT_SEPARATOR.0) != 0 {
                out.push(NativeMenuItem {
                    id: 0,
                    title: String::new(),
                    verb: None,
                    icon: None,
                    kind: NativeItemKind::Separator,
                    disabled: false,
                    is_default: false,
                });
                continue;
            }

            // Second pass: read the label into a buffer of the reported length.
            let mut title = String::new();
            if info.cch > 0 {
                let mut buf = vec![0u16; info.cch as usize + 1];
                let mut info2 = MENUITEMINFOW {
                    cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_STRING,
                    dwTypeData: PWSTR(buf.as_mut_ptr()),
                    cch: buf.len() as u32,
                    ..Default::default()
                };
                if GetMenuItemInfoW(hmenu, i as u32, true, &mut info2).is_ok() {
                    let len = info2.cch as usize;
                    title = String::from_utf16_lossy(&buf[..len.min(buf.len())]);
                }
            }
            // Strip the trailing `\t` accelerator hint Windows packs into labels.
            let title = title
                .split('\t')
                .next()
                .unwrap_or(&title)
                .replace('&', "")
                .trim()
                .to_string();
            if title.is_empty() {
                continue;
            }

            let item_id = info.wID.saturating_sub(id_first);
            let disabled = (info.fState.0 & (MFS_DISABLED.0 | MFS_GRAYED.0)) != 0;
            let is_default = (info.fState.0 & MFS_DEFAULT.0) != 0;

            // Canonical verb (best-effort; many shell items have none).
            let verb = command_verb(icm, item_id);
            if let Some(v) = &verb {
                if DEDUPE_VERBS.contains(&v.to_ascii_lowercase().as_str()) {
                    continue;
                }
            }

            let icon = bitmap_to_rgba(info.hbmpItem);

            let kind = if !info.hSubMenu.is_invalid() {
                NativeItemKind::Submenu { children: None }
            } else {
                NativeItemKind::Normal
            };

            out.push(NativeMenuItem {
                id: item_id,
                title,
                verb,
                icon,
                kind,
                disabled,
                is_default,
            });
        }
    }
    out
}

/// Populate a submenu via the `WM_INITMENUPOPUP` its dynamic handler (Open with,
/// Send to, 7-Zip) expects, then enumerate it. Used by the lazy `ExpandSubmenu`
/// path. `pos` is informational for most handlers (they key off `wParam = hsub`).
pub(crate) fn walk_submenu(
    hsub: HMENU,
    icm: &IContextMenu,
    icm2: Option<&IContextMenu2>,
    icm3: Option<&IContextMenu3>,
    id_first: u32,
) -> Vec<NativeMenuItem> {
    unsafe {
        let wparam = windows::Win32::Foundation::WPARAM(hsub.0 as usize);
        let lparam = windows::Win32::Foundation::LPARAM(0);
        if let Some(icm3) = icm3 {
            let mut res = windows::Win32::Foundation::LRESULT(0);
            let _ = icm3.HandleMenuMsg2(WM_INITMENUPOPUP, wparam, lparam, Some(&mut res));
        } else if let Some(icm2) = icm2 {
            let _ = icm2.HandleMenuMsg(WM_INITMENUPOPUP, wparam, lparam);
        }
    }
    walk_menu(hsub, icm, id_first)
}

/// `GetCommandString(GCS_VERBW)`; `None` on the (common) failure path.
unsafe fn command_verb(icm: &IContextMenu, item_id: u32) -> Option<String> {
    let mut buf = [0u16; 64];
    let r = icm.GetCommandString(
        item_id as usize,
        GCS_VERBW,
        None,
        windows::core::PSTR(buf.as_mut_ptr() as *mut u8),
        buf.len() as u32,
    );
    if r.is_err() {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    if len == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// Convert a menu-item `HBITMAP` to straight RGBA. Returns `None` for the
/// `HBMMENU_*` sentinels, owner-draw/callback bitmaps, or on any GDI failure.
unsafe fn bitmap_to_rgba(hbm: HBITMAP) -> Option<RawIcon> {
    let raw = hbm.0 as isize;
    // HBMMENU_CALLBACK == -1; HBMMENU_SYSTEM..=HBMMENU_POPUP_CLOSE are 1..=11.
    if raw == 0 || raw == -1 || (1..=11).contains(&raw) {
        return None;
    }

    let mut bm = BITMAP::default();
    let got = GetObjectW(
        hbm.into(),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bm as *mut _ as *mut c_void),
    );
    if got == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
        return None;
    }
    let w = bm.bmWidth as u32;
    let h = bm.bmHeight as u32;

    let hdc = GetDC(None);
    if hdc.is_invalid() {
        return None;
    }

    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: bm.bmWidth,
            biHeight: -bm.bmHeight, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bgra = vec![0u8; (w * h * 4) as usize];
    let scanlines = GetDIBits(
        hdc,
        hbm,
        0,
        h,
        Some(bgra.as_mut_ptr() as *mut c_void),
        &mut bi,
        DIB_RGB_COLORS,
    );
    ReleaseDC(None, hdc);
    if scanlines == 0 {
        return None;
    }

    // BGRA premultiplied → straight RGBA. If alpha is uniformly zero (legacy
    // bitmaps with no alpha channel), force opaque.
    let any_alpha = bgra.iter().skip(3).step_by(4).any(|&a| a != 0);
    let mut rgba = vec![0u8; bgra.len()];
    for px in 0..(w * h) as usize {
        let b = bgra[px * 4];
        let g = bgra[px * 4 + 1];
        let r = bgra[px * 4 + 2];
        let a = if any_alpha { bgra[px * 4 + 3] } else { 255 };
        let (r, g, b) = if any_alpha && a != 0 {
            // un-premultiply
            let un = |c: u8| ((c as u32 * 255) / a as u32).min(255) as u8;
            (un(r), un(g), un(b))
        } else {
            (r, g, b)
        };
        rgba[px * 4] = r;
        rgba[px * 4 + 1] = g;
        rgba[px * 4 + 2] = b;
        rgba[px * 4 + 3] = a;
    }

    Some(RawIcon { w, h, rgba })
}
