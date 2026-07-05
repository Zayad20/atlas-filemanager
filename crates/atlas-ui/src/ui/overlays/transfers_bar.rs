//! Cloud transfers bar (bottom-left, dismissible) — F3 #13.
//!
//! Renders `state.cloud.transfers` while `state.cloud.show_transfers` is set.
//! Backend (queueing, progress, pause/resume/cancel/retry) already lives in
//! `app/cloud.rs::CloudAction` / the rclone worker; this is display-only.

use crate::app::cloud::CloudAction;
use crate::app::App;
use crate::ui::icons::{icon, AtlasIcon};
use atlas_model::cloud_model::{Transfer, TransferKind, TransferState};
use gpui::prelude::*;
use gpui::{deferred, div, px, AnyElement, Context, MouseButton, SharedString, Window};
use gpui_component::{h_flex, v_flex, ActiveTheme};

const BAR_W: f32 = 360.;
const ROW_MAX_BYTES_LABEL: f32 = 90.;

impl App {
    pub(crate) fn render_cloud_transfers_bar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (popover, fg, muted, border, danger, success) = {
            let t = cx.theme();
            (
                t.popover,
                t.popover_foreground,
                t.muted_foreground,
                t.border,
                t.danger,
                t.success,
            )
        };
        let transfers: Vec<Transfer> = self.state.cloud.transfers.clone();
        let view = cx.entity();

        let mut rows = v_flex().gap_2();
        for t in &transfers {
            rows = rows.child(transfer_row(t, fg, muted, border, danger, success, &view));
        }

        let close_view = view.clone();
        let header = h_flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .text_color(fg)
                    .child(rust_i18n::t!("cloud_transfers_title").to_string()),
            )
            .child(
                div()
                    .id("cloud-transfers-close")
                    .size(px(20.))
                    .rounded(px(crate::ui::RADIUS))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .cursor_pointer()
                    .hover(move |h| h.bg(border).text_color(fg))
                    .child(icon(AtlasIcon::X, 11., muted))
                    .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
                        close_view.update(cx, |this, cx| {
                            this.handle_cloud_action(CloudAction::ShowTransfers(false));
                            cx.notify();
                        });
                    }),
            );

        deferred(
            div().absolute().bottom(px(24.)).left(px(24.)).child(
                v_flex()
                    .id("cloud-transfers-bar")
                    .gap_2()
                    .w(px(BAR_W))
                    .bg(popover)
                    .text_color(fg)
                    .border_1()
                    .border_color(border)
                    .rounded_lg()
                    .shadow_lg()
                    .occlude()
                    .p_3()
                    .child(header)
                    .child(rows),
            ),
        )
        .into_any_element()
    }
}

fn transfer_row(
    t: &Transfer,
    fg: gpui::Hsla,
    muted: gpui::Hsla,
    border: gpui::Hsla,
    danger: gpui::Hsla,
    success: gpui::Hsla,
    view: &gpui::Entity<App>,
) -> impl IntoElement {
    let kind_icon = match t.kind {
        TransferKind::Download => AtlasIcon::Download,
        TransferKind::Upload => AtlasIcon::ArrowUp,
    };
    let pct = (t.fraction() * 100.0).round() as i32;
    let state_label = match &t.state {
        TransferState::Queued => rust_i18n::t!("cloud_transfer_queued").to_string(),
        TransferState::Running => format!("{pct}%"),
        TransferState::Paused => rust_i18n::t!("cloud_transfer_paused").to_string(),
        TransferState::Done => rust_i18n::t!("cloud_transfer_done").to_string(),
        TransferState::Failed(_) => rust_i18n::t!("cloud_transfer_failed").to_string(),
    };
    let state_color = match &t.state {
        TransferState::Failed(_) => danger,
        TransferState::Done => success,
        _ => muted,
    };
    let bar_pct = t.fraction().clamp(0.0, 1.0);
    let id = t.id;

    let mut actions = h_flex().gap_1().flex_shrink_0();
    match &t.state {
        TransferState::Running => {
            actions = actions.child(transfer_btn(
                AtlasIcon::Square,
                muted,
                fg,
                border,
                view,
                CloudAction::PauseTransfer(id),
                format!("xfer-pause-{id}"),
            ));
            actions = actions.child(transfer_btn(
                AtlasIcon::X,
                muted,
                fg,
                border,
                view,
                CloudAction::CancelTransfer(id),
                format!("xfer-cancel-{id}"),
            ));
        }
        TransferState::Paused | TransferState::Queued => {
            actions = actions.child(transfer_btn(
                AtlasIcon::CaretRight,
                muted,
                fg,
                border,
                view,
                CloudAction::ResumeTransfer(id),
                format!("xfer-resume-{id}"),
            ));
            actions = actions.child(transfer_btn(
                AtlasIcon::X,
                muted,
                fg,
                border,
                view,
                CloudAction::CancelTransfer(id),
                format!("xfer-cancel-{id}"),
            ));
        }
        TransferState::Failed(_) => {
            actions = actions.child(transfer_btn(
                AtlasIcon::ArrowClockwise,
                muted,
                fg,
                border,
                view,
                CloudAction::RetryTransfer(id),
                format!("xfer-retry-{id}"),
            ));
        }
        TransferState::Done => {}
    }

    h_flex()
        .id(SharedString::from(format!("xfer-row-{id}")))
        .gap_2()
        .items_center()
        .child(icon(kind_icon, 14., muted))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .text_xs()
                        .text_color(fg)
                        .truncate()
                        .child(t.label.clone()),
                )
                .child(
                    div().h(px(3.)).w_full().rounded_full().bg(border).child(
                        div()
                            .h(px(3.))
                            .rounded_full()
                            .bg(state_color)
                            .w(gpui::relative(bar_pct)),
                    ),
                ),
        )
        .child(
            div()
                .flex_shrink_0()
                .w(px(ROW_MAX_BYTES_LABEL))
                .text_xs()
                .text_color(state_color)
                .child(state_label),
        )
        .child(actions)
}

#[allow(clippy::too_many_arguments)]
fn transfer_btn(
    ic: AtlasIcon,
    muted: gpui::Hsla,
    fg: gpui::Hsla,
    border: gpui::Hsla,
    view: &gpui::Entity<App>,
    action: CloudAction,
    id: String,
) -> impl IntoElement {
    let view = view.clone();
    div()
        .id(SharedString::from(id))
        .size(px(18.))
        .rounded(px(crate::ui::RADIUS))
        .flex()
        .items_center()
        .justify_center()
        .text_color(muted)
        .cursor_pointer()
        .hover(move |h| h.bg(border).text_color(fg))
        .child(icon(ic, 10., muted))
        .on_mouse_down(MouseButton::Left, move |_ev, _w, cx| {
            let action = action.clone();
            view.update(cx, |this, cx| {
                this.handle_cloud_action(action);
                cx.notify();
            });
        })
}
