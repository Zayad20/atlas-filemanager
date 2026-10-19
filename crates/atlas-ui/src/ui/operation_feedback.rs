//! Feedback follows real paths, independently of selection and presentation order.
use crate::app::App;
use gpui::{px, Bounds, Corners, Hsla, Pixels, WeakEntity, Window};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct OperationFeedback {
    pending: HashMap<u64, HashSet<PathBuf>>,
    completed: HashMap<PathBuf, (Instant, Option<Instant>)>,
    tags: HashSet<(atlas_model::tag_cache::TagId, PathBuf)>,
    visibility_checked: HashSet<(PathBuf, atlas_model::pane::TabId)>,
}

impl OperationFeedback {
    pub fn begin(&mut self, id: u64, paths: impl IntoIterator<Item = PathBuf>) {
        self.pending.insert(id, paths.into_iter().collect());
    }

    pub fn finish(&mut self, id: u64) {
        self.pending.remove(&id);
    }

    pub fn begin_tag(&mut self, tag: atlas_model::tag_cache::TagId, paths: &[PathBuf]) {
        self.tags
            .extend(paths.iter().cloned().map(|path| (tag, path)));
    }

    pub fn finish_tag(&mut self, tag: atlas_model::tag_cache::TagId, paths: &[PathBuf]) {
        for path in paths {
            self.tags.remove(&(tag, path.clone()));
        }
    }

    pub fn complete(&mut self, paths: impl IntoIterator<Item = PathBuf>, now: Instant) {
        self.prune(now);
        // Keep feedback bounded even for recursive transfers with huge reports.
        for path in paths.into_iter().take(512) {
            if self.completed.len() >= 1024 {
                self.completed.clear();
                self.visibility_checked.clear();
            }
            self.visibility_checked
                .retain(|(checked, _)| checked != &path);
            self.completed.insert(path, (now, None));
        }
    }

    fn prune(&mut self, now: Instant) {
        self.completed.retain(|_, (received, shown)| {
            now.saturating_duration_since(*received) < Duration::from_secs(8)
                && shown.is_none_or(|shown| {
                    now.saturating_duration_since(shown) < Duration::from_millis(900)
                })
        });
        self.visibility_checked
            .retain(|(path, _)| self.completed.contains_key(path));
    }

    fn visibility_candidates(
        &mut self,
        pane: atlas_model::pane::TabId,
        dir: &Path,
    ) -> HashSet<PathBuf> {
        self.prune(Instant::now());
        self.completed
            .keys()
            .filter(|path| {
                path.parent() == Some(dir)
                    && !self.visibility_checked.contains(&((*path).clone(), pane))
            })
            .cloned()
            .collect()
    }

    fn sample(&mut self, path: &Path, now: Instant, reduced: bool) -> (f32, bool) {
        self.prune(now);
        if let Some((_, shown)) = self.completed.get_mut(path) {
            let start = *shown.get_or_insert(now);
            let t = now.saturating_duration_since(start).as_secs_f32() / 0.9;
            return (
                if reduced {
                    0.65
                } else {
                    0.75 * (1.0 - t).max(0.0)
                },
                true,
            );
        }
        (
            if self.pending.values().any(|paths| paths.contains(path))
                || self.tags.iter().any(|(_, tagged)| tagged == path)
            {
                0.24
            } else {
                0.0
            },
            false,
        )
    }
}

#[derive(Clone)]
pub(crate) struct HiddenResultNotice {
    dir: PathBuf,
    paths: Vec<PathBuf>,
}

impl App {
    /// Called after background results, not on each animation frame. Only known
    /// listing entries can be classified as hidden by a filter.
    pub(crate) fn update_hidden_operation_notices(&mut self) -> Vec<atlas_model::pane::TabId> {
        let mut changed = Vec::new();
        for pane in self.state.all_pane_tile_ids() {
            let Some(tab) = self.tab_data_for(pane) else {
                continue;
            };
            let Some(dir) = tab.location.fs_dir().map(Path::to_path_buf) else {
                continue;
            };
            if tab.view_mode == atlas_model::settings::ViewMode::Miller {
                continue;
            }
            let details = tab.view_mode == atlas_model::settings::ViewMode::Details;
            let mut candidates = self
                .operation_feedback
                .borrow_mut()
                .visibility_candidates(pane, &dir);
            if let Some(notice) = self
                .hidden_operation_notices
                .get(&pane)
                .filter(|notice| notice.dir == dir)
            {
                candidates.extend(notice.paths.iter().cloned());
            }
            if candidates.is_empty() {
                continue;
            }
            let Some(ts) = self.state.tab_states.get_mut(&pane) else {
                continue;
            };
            if !matches!(ts.load_state, atlas_model::tab::LoadState::Loaded) {
                continue;
            }
            ts.visible_indices();
            let known: HashSet<_> = ts
                .search
                .original_entries
                .as_ref()
                .unwrap_or(&ts.entries)
                .iter()
                .filter(|entry| candidates.contains(&entry.path()))
                .map(|entry| entry.path())
                .collect();
            let types = self.type_filters.get(&pane);
            let shown: HashSet<_> = ts
                .visible
                .iter()
                .filter_map(|index| ts.entries.get(*index))
                .filter(|entry| {
                    !details
                        || types.is_none_or(|types| {
                            types.is_empty()
                                || types
                                    .contains(&crate::ui::views::details::entry_type_label(entry))
                        })
                })
                .filter(|entry| known.contains(&entry.path()))
                .map(|entry| entry.path())
                .collect();
            let hidden: Vec<_> = known.difference(&shown).cloned().collect();
            self.operation_feedback
                .borrow_mut()
                .visibility_checked
                .extend(known.into_iter().map(|path| (path, pane)));
            if let Some(notice) = self.hidden_operation_notices.get_mut(&pane) {
                let before = notice.paths.len();
                notice.paths.retain(|path| hidden.contains(path));
                if before != notice.paths.len() {
                    changed.push(pane);
                }
                if notice.paths.is_empty() {
                    self.hidden_operation_notices.remove(&pane);
                }
            }
            if !hidden.is_empty() {
                let notice = self
                    .hidden_operation_notices
                    .entry(pane)
                    .or_insert_with(|| HiddenResultNotice {
                        dir: dir.clone(),
                        paths: Vec::new(),
                    });
                if notice.dir != dir {
                    notice.dir = dir;
                    notice.paths.clear();
                }
                for path in hidden {
                    if !notice.paths.contains(&path) {
                        notice.paths.push(path);
                    }
                }
                changed.push(pane);
            }
        }
        changed
    }

    pub(crate) fn reveal_hidden_operation_results(&mut self, pane: atlas_model::pane::TabId) {
        let Some(notice) = self.hidden_operation_notices.remove(&pane) else {
            return;
        };
        if self
            .tab_data_for(pane)
            .and_then(|tab| tab.location.fs_dir())
            != Some(notice.dir.as_path())
        {
            return;
        }
        self.type_filters.remove(&pane);
        self.size_filters.remove(&pane);
        let Some(ts) = self.state.tab_states.get_mut(&pane) else {
            return;
        };
        if let Some(entries) = ts.search.original_entries.take() {
            ts.set_entries(entries);
        }
        ts.search.reset();
        ts.filter.clear();
        ts.show_files = true;
        ts.show_folders = true;
        ts.selection.selected = ts
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| notice.paths.contains(&entry.path()))
            .map(|(index, _)| index)
            .collect();
        ts.selection.keyboard_cursor = ts.selection.selected.iter().min().copied();
        ts.selection.selection_anchor = ts.selection.keyboard_cursor;
        ts.selection.scroll_to_selected = true;
        self.dirty_panes.insert(pane);
    }

    pub(crate) fn render_hidden_operation_notices(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        use gpui::{div, prelude::*, SharedString};
        use gpui_component::ActiveTheme;
        let live: HashMap<_, _> = self
            .hidden_operation_notices
            .keys()
            .filter_map(|pane| {
                self.tab_data_for(*pane)
                    .and_then(|tab| tab.location.fs_dir())
                    .map(|dir| (*pane, dir.to_path_buf()))
            })
            .collect();
        self.hidden_operation_notices
            .retain(|pane, notice| live.get(pane) == Some(&notice.dir));
        let mut elements = Vec::new();
        for (pane, notice) in &self.hidden_operation_notices {
            let Some(bounds) = self.pane_view_bounds.get(pane) else {
                continue;
            };
            let pane = *pane;
            elements.push(
                div()
                    .id(SharedString::from(format!("hidden-operation-{}", pane.0)))
                    .absolute()
                    .left(bounds.left() + px(8.0))
                    .top(bounds.bottom() - px(42.0))
                    .w((bounds.size.width - px(16.0)).max(px(0.0)))
                    .h(px(34.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .occlude()
                    .rounded_md()
                    .bg(cx.theme().popover)
                    .text_color(cx.theme().popover_foreground)
                    .text_sm()
                    .child(div().min_w_0().flex_1().truncate().child(
                        rust_i18n::t!("op_results_hidden", count = notice.paths.len()).into_owned(),
                    ))
                    .child(
                        div()
                            .id(("show-operation-results", pane.0))
                            .cursor_pointer()
                            .text_color(cx.theme().accent)
                            .child(rust_i18n::t!("op_results_reveal").into_owned())
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.reveal_hidden_operation_results(pane);
                                    this.notify_pane(pane, cx);
                                    cx.notify();
                                    cx.stop_propagation();
                                }),
                            ),
                    )
                    .child(
                        div()
                            .id(("dismiss-operation-results", pane.0))
                            .cursor_pointer()
                            .child(crate::ui::icons::icon(
                                crate::ui::icons::AtlasIcon::X,
                                12.0,
                                cx.theme().muted_foreground,
                            ))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.hidden_operation_notices.remove(&pane);
                                    cx.notify();
                                    cx.stop_propagation();
                                }),
                            ),
                    )
                    .into_any_element(),
            );
        }
        elements
    }
}

/// Called inside each view's existing clip, using its presented bounds. Paint
/// only: never steals focus, changes selection or introduces a second hit model.
pub(crate) fn paint_file_feedback(
    app: &WeakEntity<App>,
    path: &Path,
    bounds: Bounds<Pixels>,
    color: Hsla,
    radius: f32,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let Some(app) = app.upgrade() else {
        return;
    };
    let (alpha, active) = app.read(cx).operation_feedback.borrow_mut().sample(
        path,
        Instant::now(),
        cx.reduce_motion(),
    );
    if alpha > 0.0 {
        let mut color = color;
        color.alpha = alpha;
        window.paint_quad(
            gpui::outline(bounds, color, Default::default()).corner_radii(Corners::all(px(radius))),
        );
    }
    if active {
        window.request_animation_frame();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_waits_for_actual_listing_and_does_not_highlight_failed_paths() {
        let now = Instant::now();
        let good = PathBuf::from("destination/renamed.txt");
        let failed = PathBuf::from("destination/failed.txt");
        let mut feedback = OperationFeedback::default();
        feedback.begin(1, [good.clone(), failed.clone()]);
        feedback.finish(1);
        feedback.complete([good.clone()], now);
        assert_eq!(feedback.sample(&failed, now, false), (0.0, false));
        assert_eq!(
            feedback.sample(&good, now + Duration::from_secs(2), false),
            (0.75, true)
        );
        assert_eq!(
            feedback.sample(&good, now + Duration::from_secs(3), false),
            (0.0, false)
        );
    }

    #[test]
    fn repeated_result_is_checked_again_in_each_pane() {
        let now = Instant::now();
        let dir = PathBuf::from("destination");
        let path = dir.join("file.txt");
        let pane = atlas_model::layout_tree::TileId(1);
        let other = atlas_model::layout_tree::TileId(2);
        let mut feedback = OperationFeedback::default();
        feedback.complete([path.clone()], now);
        feedback.visibility_checked.insert((path.clone(), pane));
        assert!(feedback.visibility_candidates(pane, &dir).is_empty());
        assert!(feedback.visibility_candidates(other, &dir).contains(&path));
        feedback.complete([path.clone()], now + Duration::from_millis(20));
        assert!(feedback.visibility_candidates(pane, &dir).contains(&path));
        assert!(feedback
            .visibility_candidates(pane, Path::new("elsewhere"))
            .is_empty());
    }

    #[test]
    fn concurrent_pending_operations_survive_another_operations_cancellation() {
        let now = Instant::now();
        let path = PathBuf::from("source/file");
        let mut feedback = OperationFeedback::default();
        feedback.begin(1, [path.clone()]);
        feedback.begin(2, [path.clone()]);
        feedback.finish(1);
        assert_eq!(feedback.sample(&path, now, false), (0.24, false));
        feedback.finish(2);
        assert_eq!(feedback.sample(&path, now, false), (0.0, false));
    }
}
