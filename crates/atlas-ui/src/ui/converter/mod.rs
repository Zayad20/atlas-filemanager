//! One upstream session, disposable wheel nodes, immutable worker commands.
mod geometry;
#[cfg(feature = "conversion-lab")]
pub(crate) mod lab;
mod render;
mod wheel;

use crate::app::App;
use atlas_core::convert::{self, Outcome, Plan, Report, Settings, Tools};
use atlas_model::app_state::BgResult;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub(crate) enum Phase {
    Discovering,
    Choosing,
    Running {
        cancel: Arc<AtomicBool>,
        done: usize,
        total: usize,
        current: PathBuf,
    },
    Finished(Report),
}

pub(crate) struct Session {
    sources: Vec<PathBuf>,
    tools: Tools,
    catalog: Vec<convert::ConversionOption>,
    phase: Phase,
    selected: usize,
    settings: Settings,
    expanded: bool,
    minimized: bool,
    #[cfg(feature = "conversion-lab")]
    wheel_bounds: Option<gpui::Bounds<gpui::Pixels>>,
    tx: flume::Sender<Event>,
    rx: flume::Receiver<Event>,
}

enum Event {
    Discovered(Tools, Vec<convert::ConversionOption>),
    Destination(PathBuf),
    Progress {
        done: usize,
        total: usize,
        current: PathBuf,
    },
    Finished(Report),
}

impl Session {
    pub(crate) fn visible(&self) -> bool {
        !self.minimized
    }
    fn running(&self) -> bool {
        matches!(self.phase, Phase::Running { .. })
    }
    fn options(&self) -> &[convert::ConversionOption] {
        &self.catalog
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Phase::Running { cancel, .. } = &self.phase {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl App {
    pub(crate) fn open_converter(&mut self) {
        if let Some(session) = self.converter.as_mut().filter(|s| s.running()) {
            session.minimized = false;
            return;
        }
        if self
            .state
            .active_tab_data()
            .is_none_or(|tab| tab.location.fs_dir().is_none())
        {
            self.toast_error(rust_i18n::t!("convert_local_only").into_owned());
            return;
        }
        let sources = self.selected_paths();
        self.open_converter_for(sources);
    }

    pub(crate) fn open_converter_for(&mut self, sources: Vec<PathBuf>) {
        if let Some(session) = self.converter.as_mut().filter(|s| s.running()) {
            session.minimized = false;
            return;
        }
        if sources.is_empty() {
            self.toast_error(rust_i18n::t!("convert_select").into_owned());
            return;
        }
        let (tx, rx) = flume::unbounded();
        let scan_sources = sources.clone();
        self.converter = Some(Session {
            sources,
            tools: Tools::default(),
            catalog: Vec::new(),
            phase: Phase::Discovering,
            selected: 0,
            settings: Settings::default(),
            expanded: false,
            minimized: false,
            #[cfg(feature = "conversion-lab")]
            wheel_bounds: None,
            tx: tx.clone(),
            rx,
        });
        let wake = self.tx.clone();
        self.rt.spawn_blocking(move || {
            let tools = Tools::discover();
            let catalog = convert::options(&scan_sources, &tools);
            let _ = tx.send(Event::Discovered(tools, catalog));
            let _ = wake.send(BgResult::ConversionWake);
        });
    }

    pub(crate) fn drain_converter(&mut self) {
        let Some(session) = self.converter.as_mut() else {
            return;
        };
        while let Ok(event) = session.rx.try_recv() {
            match event {
                Event::Discovered(tools, catalog) => {
                    session.tools = tools;
                    session.catalog = catalog;
                    session.phase = Phase::Choosing;
                }
                Event::Destination(path) => session.settings.output_dir = Some(path),
                Event::Progress {
                    done,
                    total,
                    current,
                } => {
                    if let Phase::Running {
                        done: completed,
                        total: count,
                        current: path,
                        ..
                    } = &mut session.phase
                    {
                        if done >= *completed {
                            *completed = done;
                            *count = total;
                            *path = current;
                        }
                    }
                }
                Event::Finished(report) => session.phase = Phase::Finished(report),
            }
        }
    }

    fn start_conversion(&mut self) {
        let Some(session) = self.converter.as_mut() else {
            return;
        };
        if !matches!(session.phase, Phase::Choosing) {
            return;
        }
        let options = session.options();
        let Some(option) = options.get(session.selected).filter(|o| o.available()) else {
            return;
        };
        let extension = option.target.extension;
        let sources = session.sources.clone();
        let settings = session.settings.clone();
        let tools = session.tools.clone();
        let tx = session.tx.clone();
        let wake = self.tx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        session.phase = Phase::Running {
            cancel: cancel.clone(),
            done: 0,
            total: sources.len(),
            current: sources[0].clone(),
        };
        self.rt.spawn_blocking(move || {
            let report = match Plan::build(&sources, extension, settings, tools) {
                Ok(plan) => convert::execute(&plan, &cancel, |done, total, current| {
                    let _ = tx.send(Event::Progress {
                        done,
                        total,
                        current: current.to_path_buf(),
                    });
                    let _ = wake.send(BgResult::ConversionWake);
                }),
                Err(error) => Report {
                    outcomes: vec![Outcome::Failed {
                        source: sources[0].clone(),
                        error: format!("{error:#}"),
                    }],
                },
            };
            let _ = tx.send(Event::Finished(report));
            let _ = wake.send(BgResult::ConversionWake);
        });
    }

    fn dismiss_converter(&mut self) {
        if let Some(session) = self.converter.as_mut().filter(|s| s.running()) {
            session.minimized = true;
        } else {
            self.converter = None;
        }
    }

    fn cancel_conversion(&mut self) {
        if let Some(Session {
            phase: Phase::Running { cancel, .. },
            ..
        }) = &self.converter
        {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    fn choose_conversion_directory(&mut self) {
        let Some(session) = &self.converter else {
            return;
        };
        let tx = session.tx.clone();
        let wake = self.tx.clone();
        self.rt.spawn(async move {
            if let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await {
                let _ = tx.send(Event::Destination(folder.path().to_path_buf()));
                let _ = wake.send(BgResult::ConversionWake);
            }
        });
    }
}
