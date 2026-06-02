use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
use x11rb::wrapper::ConnectionExt as _;

/// Requests compositor blur for the whole X11 window. KWin and compatible
/// compositors treat this atom as a hint; unsupported compositors ignore it.
pub fn request_blur(cc: &eframe::CreationContext<'_>) {
    let Ok(handle) = cc.window_handle() else {
        return;
    };
    let window = match handle.as_raw() {
        RawWindowHandle::Xlib(handle) => handle.window as u32,
        RawWindowHandle::Xcb(handle) => handle.window.get(),
        _ => return,
    };

    if let Err(err) = set_blur_atom(window) {
        tracing::debug!("unable to request X11 compositor blur: {err}");
    }
}

fn set_blur_atom(window: u32) -> Result<(), Box<dyn std::error::Error>> {
    let (connection, _) = x11rb::connect(None)?;
    let atom = connection
        .intern_atom(false, b"_KDE_NET_WM_BLUR_BEHIND_REGION")?
        .reply()?
        .atom;
    connection.change_property32(PropMode::REPLACE, window, atom, AtomEnum::CARDINAL, &[0])?;
    connection.flush()?;
    Ok(())
}
