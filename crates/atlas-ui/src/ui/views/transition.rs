//! A single, bounded entrance for a listing, independent of its virtual rows.
//! Atlas-3D's cross-slide is adapted to live content: chrome stays stationary,
//! geometry and hitboxes travel together, and filesystem refreshes stay direct.
use crate::app::App;
use atlas_model::{location::Location, pane::TabId, settings::ViewMode, tab::LoadState};
use gpui::{
    point, px, AnyElement, App as GpuiApp, Bounds, ContentMask, Element, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Window,
};
use std::time::{Duration, Instant};

#[cfg(feature = "ui-lab")]
pub mod diagnostic;

#[derive(Default)]
pub(crate) struct ViewTransition {
    displayed: Option<(Location, ViewMode)>,
    navigation_direction: Option<f32>,
    animation: Option<ViewportAnimation>,
}

#[derive(Clone, Copy)]
struct ViewportAnimation {
    started: Instant,
    duration: Duration,
    x: f32,
    y: f32,
    opacity: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct Sample {
    x: f32,
    y: f32,
    opacity: f32,
    active: bool,
}

impl ViewportAnimation {
    fn sample(self, now: Instant) -> Sample {
        let t =
            now.saturating_duration_since(self.started).as_secs_f32() / self.duration.as_secs_f32();
        let remaining = 1.0 - crate::ui::motion::ease_out_quart(t);
        Sample {
            x: self.x * remaining,
            y: self.y * remaining,
            opacity: 1.0 - (1.0 - self.opacity) * remaining,
            active: t < 1.0,
        }
    }
}

impl ViewTransition {
    pub(crate) fn navigate(&mut self, from: &Location, to: &Location, back: bool) {
        let parent = from
            .fs_dir()
            .zip(to.fs_dir())
            .is_some_and(|(old, new)| old != new && old.starts_with(new));
        self.navigation_direction = Some(if back || parent { -1.0 } else { 1.0 });
    }

    fn observe(
        &mut self,
        location: Location,
        mode: ViewMode,
        ready: bool,
        instant: bool,
        now: Instant,
    ) {
        if instant {
            self.animation = None;
        }
        // Loading and watcher results must never start a second entrance.
        if !ready {
            return;
        }
        if let Some((old_location, old_mode)) = &self.displayed {
            let navigated = *old_location != location;
            if (navigated || *old_mode != mode) && !instant {
                let opacity = self
                    .animation
                    .filter(|anim| anim.sample(now).active)
                    .map_or(0.55, |anim| anim.sample(now).opacity.max(0.55));
                self.animation = Some(if navigated {
                    ViewportAnimation {
                        started: now,
                        duration: Duration::from_millis(220),
                        x: 28.0 * self.navigation_direction.unwrap_or(1.0),
                        y: 0.0,
                        opacity,
                    }
                } else {
                    let direction = if view_order(&mode) >= view_order(old_mode) {
                        1.0
                    } else {
                        -1.0
                    };
                    ViewportAnimation {
                        started: now,
                        duration: Duration::from_millis(180),
                        x: 0.0,
                        y: 8.0 * direction,
                        opacity,
                    }
                });
            }
        }
        self.displayed = Some((location, mode));
        self.navigation_direction = None;
        if self.animation.is_some_and(|anim| !anim.sample(now).active) {
            self.animation = None;
        }
    }
}

fn view_order(mode: &ViewMode) -> u8 {
    match mode {
        ViewMode::Details => 0,
        ViewMode::Tree => 1,
        ViewMode::List => 2,
        ViewMode::Grid => 3,
        ViewMode::Miller => 4,
    }
}

impl App {
    pub(crate) fn sync_view_transition(&mut self, pane: TabId, cx: &GpuiApp) {
        let Some(tab) = self.tab_data_for(pane) else {
            return;
        };
        let location = tab.location.clone();
        let mode = tab.view_mode.clone();
        let ready = self.state.tab_states.get(&pane).is_some_and(|state| {
            if mode == ViewMode::Miller {
                state
                    .miller
                    .columns
                    .iter()
                    .any(|col| matches!(col.load_state, LoadState::Loaded))
            } else {
                !matches!(state.load_state, LoadState::Loading)
            }
        });
        let instant = cx.reduce_motion()
            || self.state.settings.disable_animations
            || self.suppress_window_resize_animations()
            || self.is_pane_or_dock_animating()
            || self.rubber.is_some()
            || self.rename_edit.is_some()
            || self.is_drag_active(cx.has_active_drag());
        self.pane_view_transitions.entry(pane).or_default().observe(
            location,
            mode,
            ready,
            instant,
            Instant::now(),
        );
    }

    pub(crate) fn transition_viewport(&self, pane: TabId, child: impl IntoElement) -> FileViewport {
        FileViewport {
            child: child.into_any_element(),
            animation: self
                .pane_view_transitions
                .get(&pane)
                .and_then(|state| state.animation),
        }
    }
}

/// Forwards the child's layout unchanged. Only prepaint coordinates and paint
/// opacity change, so virtualization, clipping and input use the same geometry.
pub(crate) struct FileViewport {
    child: AnyElement,
    animation: Option<ViewportAnimation>,
}

impl IntoElement for FileViewport {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for FileViewport {
    type RequestLayoutState = ();
    type PrepaintState = Sample;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut GpuiApp,
    ) -> Sample {
        let sample = self.animation.filter(|_| !cx.reduce_motion()).map_or(
            Sample {
                x: 0.0,
                y: 0.0,
                opacity: 1.0,
                active: false,
            },
            |animation| animation.sample(Instant::now()),
        );
        let zoom = f32::from(window.rem_size()) / crate::ui::BASE_PX;
        window.with_content_mask(
            Some(ContentMask {
                bounds,
                ..Default::default()
            }),
            |window| {
                window.with_element_offset(
                    point(px(sample.x * zoom), px(sample.y * zoom)),
                    |window| {
                        self.child.prepaint(window, cx);
                    },
                );
            },
        );
        sample
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        sample: &mut Sample,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) {
        window.with_content_mask(
            Some(ContentMask {
                bounds,
                ..Default::default()
            }),
            |window| {
                window.with_element_opacity(Some(sample.opacity), |window| {
                    self.child.paint(window, cx)
                });
            },
        );
        if sample.active {
            window.request_animation_frame();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn location(path: &str) -> Location {
        Location::Directory(path.into())
    }

    #[test]
    fn starts_after_loading_and_does_not_restart_on_refresh() {
        let now = Instant::now();
        let mut state = ViewTransition::default();
        state.observe(location("/a"), ViewMode::Details, true, false, now);
        state.navigate(&location("/a"), &location("/a/b"), false);
        state.observe(location("/a/b"), ViewMode::Grid, false, false, now);
        assert!(state.animation.is_none());
        state.observe(location("/a/b"), ViewMode::Grid, true, false, now);
        let animation = state.animation.unwrap();
        assert!(animation.x > 0.0);
        assert_eq!(animation.y, 0.0);
        state.observe(
            location("/a/b"),
            ViewMode::Grid,
            true,
            false,
            now + Duration::from_millis(50),
        );
        assert_eq!(state.animation.unwrap().started, animation.started);
        let end = now + Duration::from_millis(220);
        assert_eq!(animation.sample(end).opacity, 1.0);
        assert_eq!(animation.sample(end).x, 0.0);
        assert!(!animation.sample(end).active);
        state.observe(location("/a/b"), ViewMode::Grid, true, false, end);
        assert!(state.animation.is_none());
    }

    #[test]
    fn back_and_parent_reverse_direction_and_reduced_motion_cancels() {
        let now = Instant::now();
        let mut state = ViewTransition::default();
        state.observe(location("/a/b"), ViewMode::Grid, true, false, now);
        state.navigate(&location("/a/b"), &location("/a"), false);
        state.observe(location("/a"), ViewMode::Grid, true, false, now);
        assert!(state.animation.unwrap().x < 0.0);
        state.navigate(&location("/a"), &location("/other"), true);
        state.observe(location("/other"), ViewMode::Grid, true, false, now);
        assert!(state.animation.unwrap().x < 0.0);
        state.observe(location("/other"), ViewMode::List, true, true, now);
        assert!(state.animation.is_none());
        state.observe(location("/other"), ViewMode::List, true, false, now);
        assert!(state.animation.is_none());
        state.observe(location("/other"), ViewMode::Tree, true, false, now);
        assert!(state.animation.unwrap().y < 0.0);
    }

    #[test]
    fn a_finished_entrance_does_not_suppress_the_next_fade() {
        let now = Instant::now();
        let mut state = ViewTransition::default();
        state.observe(location("/a"), ViewMode::Details, true, false, now);
        state.observe(location("/a"), ViewMode::Tree, true, false, now);
        let later = now + Duration::from_secs(1);
        state.observe(location("/a"), ViewMode::List, true, false, later);
        let sample = state.animation.unwrap().sample(later);
        assert!(sample.opacity < 1.0);
        assert!(sample.active);
    }
}
