//! Shared reference-list motion for Recent and the staging shelf. Retained exits
//! contain only labels and icons, never stale event handlers or file operations.
use crate::{
    app::App,
    ui::{
        dg::motion::{PresenceMotion, RowMotion},
        icons::{theme_icon, AtlasIcon},
    },
};
use gpui::{
    div, point, prelude::*, px, AnyElement, Entity, Hsla, IntoElement, RenderOnce, SharedString,
    Window,
};
use std::{collections::HashMap, path::PathBuf, time::Instant};

pub(crate) struct ReferenceItem {
    pub path: PathBuf,
    pub label: String,
    pub content: AnyElement,
}

#[derive(Clone)]
struct Snapshot {
    path: PathBuf,
    label: String,
    index: usize,
}

#[derive(IntoElement)]
pub(crate) struct ReferenceCollection {
    pub id: SharedString,
    pub app: Entity<App>,
    pub items: Vec<ReferenceItem>,
    pub extent: f32,
    pub gap: f32,
    pub horizontal: bool,
    pub icon: AtlasIcon,
    pub color: Hsla,
    pub windows: bool,
    pub empty: Option<AnyElement>,
}

impl RenderOnce for ReferenceCollection {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let now = Instant::now();
        let zoom = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        let extent = self.extent * zoom;
        let gap = self.gap * zoom;
        let instant = cx.reduce_motion()
            || self.app.read(cx).rubber.is_some()
            || self.app.read(cx).is_drag_active(cx.has_active_drag());
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| {
            (
                PresenceMotion::<PathBuf, Snapshot>::new(),
                RowMotion::<PathBuf>::new(0, [0.0, extent, gap]),
            )
        });
        let mut content = HashMap::new();
        let values = self
            .items
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                let path = item.path.clone();
                content.insert(path.clone(), item.content);
                (
                    path,
                    Snapshot {
                        path: item.path,
                        label: item.label,
                        index,
                    },
                )
            })
            .collect();
        let (samples, positions, active) = state.update(cx, |state, _| {
            let (mut samples, fading) = state.0.sample(values, 0, instant, now);
            samples.sort_by_key(|sample| (sample.value.index, sample.interactive));
            let mut offset = 0.0;
            let targets: Vec<_> = samples
                .iter()
                .enumerate()
                .map(|(index, sample)| {
                    let target = (sample.value.path.clone(), index, offset);
                    offset += (extent + gap) * sample.opacity;
                    target
                })
                .collect();
            let (positions, moving) = state
                .1
                .sample(&targets, 0, [0.0, extent, gap], instant, now);
            (samples, positions, fading || moving)
        });
        if samples.is_empty() {
            return div()
                .w_full()
                .h_full()
                .children(self.empty)
                .into_any_element();
        }
        let mut list = div()
            .id(self.id)
            .flex()
            .flex_none()
            .relative()
            .when(self.horizontal, |list| {
                list.flex_row().w_auto().items_center()
            })
            .when(!self.horizontal, |list| list.flex_col().w_full());
        let mut layout_offset = 0.0;
        for (sample, (_, presented, _)) in samples.into_iter().zip(positions) {
            let amount = (extent + gap) * sample.opacity;
            let offset = presented - layout_offset;
            layout_offset += amount;
            let child = if sample.interactive {
                content
                    .remove(&sample.value.path)
                    .expect("live reference has content")
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .text_sm()
                    .text_color(self.color)
                    .when(self.horizontal, |row| {
                        row.w(px(extent)).min_h(crate::ui::rem_px(74.0)).flex_col()
                    })
                    .when(!self.horizontal, |row| row.w_full().h(px(extent)))
                    .child(theme_icon(self.icon, 16.0, self.color, self.windows))
                    .child(div().min_w_0().truncate().child(sample.value.label))
                    .into_any_element()
            };
            list = list.child(
                div()
                    .flex_none()
                    .overflow_hidden()
                    .opacity(sample.opacity)
                    .when(self.horizontal, |slot| {
                        slot.w(px(amount)).paint_offset(point(px(offset), px(0.0)))
                    })
                    .when(!self.horizontal, |slot| {
                        slot.w_full()
                            .h(px(amount))
                            .paint_offset(point(px(0.0), px(offset)))
                    })
                    .child(child),
            );
        }
        if active {
            list = list.child(
                gpui::canvas(
                    |_, _, _| {},
                    |_, _, window, _| window.request_animation_frame(),
                )
                .size(px(0.0)),
            );
        }
        list.into_any_element()
    }
}
