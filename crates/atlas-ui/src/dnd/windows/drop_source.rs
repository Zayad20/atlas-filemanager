//! `IDropSource` implementation — Atlas drag-out source.
//!
//! Drives a `DoDragDrop` loop: continues while LMB is held and Escape is not
//! pressed; uses default OS drag cursors (no custom feedback).
#![allow(unsafe_code)]
// reason: COM interface implementation requires unsafe.

use windows::core::{implement, BOOL, HRESULT};
use windows::Win32::Foundation::{
    DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK,
};
use windows::Win32::System::Ole::{IDropSource, IDropSource_Impl, DROPEFFECT};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};

/// Atlas `IDropSource`. Stateless — only standard Win32 drag-cursor logic.
#[implement(IDropSource)]
pub struct AtlasDropSource;

impl IDropSource_Impl for AtlasDropSource_Impl {
    fn QueryContinueDrag(&self, fescapepressed: BOOL, grfkeystate: MODIFIERKEYS_FLAGS) -> HRESULT {
        if fescapepressed.as_bool() {
            return DRAGDROP_S_CANCEL;
        }
        if !grfkeystate.contains(MK_LBUTTON) {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}
