//! Rename-active-profile dialog (gpui-component `Dialog`).
#![allow(clippy::too_many_lines)] // reason: cohesive overlay kept whole

use crate::app::App;
use crate::input::action::Action;
use gpui::prelude::*;
use gpui::{
    px, Context, Focusable, Window,
};
use gpui_component::WindowExt;

impl App {
    /// Open the rename-active-profile dialog using gpui-component's `Dialog`
    /// (mounted by the window `Root`). The dialog owns its overlay, focus,
    /// Esc/overlay-close and the OK/Cancel buttons; we only supply a native text
    /// input and the confirm callback. Called once per open (from `render` when
    /// `overlay.renaming_profile` is latched true).
    pub(crate) fn open_rename_profile_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self.state.profiles.active().name.clone();
        // The input must outlive a single frame, so keep it on `App`.
        let input = crate::ui::text_input::single_line(window, cx, "", name);
        self.rename_profile_input = Some(input.clone());
        window.focus(&input.read(cx).focus_handle(cx), cx);

        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input = input.clone();
            let ok_view = view.clone();
            dialog
                .w(px(360.))
                .title(rust_i18n::t!("profile_rename_title").to_string())
                .button_props(
                    gpui_component::dialog::DialogButtonProps::default()
                        .ok_text(rust_i18n::t!("profile_rename_save").to_string())
                        .cancel_text(rust_i18n::t!("profile_rename_cancel").to_string()),
                )
                .content({
                    let input = input.clone();
                    move |content, _w, _cx| {
                        content.child(crate::ui::text_input::bordered(&input))
                    }
                })
                .on_ok(move |_ev, _window, cx| {
                    let value = input.read(cx).value().to_string();
                    ok_view.update(cx, |app, cx| {
                        app.process_actions(vec![Action::RenameActiveProfile(value.clone())]);
                        app.rename_profile_input = None;
                        cx.notify();
                    });
                    true // close the dialog
                })
        });
    }

}

