//! One upstream session, disposable wheel nodes, immutable worker commands.
mod geometry;
#[cfg(feature = "conversion-lab")]
pub(crate) mod lab;
mod render;
mod wheel;

use crate::app::App;
use atlas_core::convert::{self, Outcome, Plan, Report, Settings, Tool, Tools};
use atlas_core::ext::{registry::ToolInstallRequest, tool_package};
use atlas_model::app_state::BgResult;
use std::collections::BTreeMap;
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
    discovery: u64,
    installers: BTreeMap<Tool, ToolInstallRequest>,
    disabled_providers: BTreeMap<Tool, String>,
    package: Option<PackageJob>,
    package_error: Option<String>,
    #[cfg(feature = "conversion-lab")]
    wheel_bounds: Option<gpui::Bounds<gpui::Pixels>>,
    tx: flume::Sender<Event>,
    rx: flume::Receiver<Event>,
}

enum Event {
    Discovered {
        revision: u64,
        tools: Tools,
        catalog: Vec<convert::ConversionOption>,
    },
    PackageProgress(tool_package::Progress),
    PackageFinished(Result<PathBuf, String>),
    Destination(PathBuf),
    Progress {
        done: usize,
        total: usize,
        current: PathBuf,
    },
    Finished(Report),
}

struct PackageJob {
    tool: Tool,
    cancel: Arc<AtomicBool>,
    progress: tool_package::Progress,
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
        if let Some(package) = &self.package {
            package.cancel.store(true, Ordering::Relaxed);
        }
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
        self.converter = Some(Session {
            sources,
            tools: Tools::default(),
            catalog: Vec::new(),
            phase: Phase::Discovering,
            selected: 0,
            settings: Settings::default(),
            expanded: false,
            minimized: false,
            discovery: 0,
            installers: BTreeMap::new(),
            disabled_providers: BTreeMap::new(),
            package: None,
            package_error: None,
            #[cfg(feature = "conversion-lab")]
            wheel_bounds: None,
            tx: tx.clone(),
            rx,
        });
        self.refresh_conversion_tools();
    }

    /// Snapshot registry state on the UI lane; filesystem/codecs stay on workers.
    pub(crate) fn refresh_conversion_tools(&mut self) {
        let Some(session) = self.converter.as_mut().filter(|s| !s.running()) else {
            return;
        };
        session.installers = Tool::ALL
            .into_iter()
            .filter_map(|tool| {
                self.ext_registry
                    .install_request_for_tool(tool.executable())
                    .filter(|request| {
                        !request.tool.archive_root.is_empty()
                            && matches!(request.tool.archive_format.as_str(), "zip" | "tgz")
                    })
                    .map(|request| (tool, request))
            })
            .collect();
        session.disabled_providers.clear();
        for provider in self
            .ext_registry
            .extensions
            .iter()
            .filter(|extension| !extension.enabled)
        {
            for spec in &provider.manifest.tools {
                if (spec.os.is_empty() || spec.os == std::env::consts::OS)
                    && (spec.arch.is_empty() || spec.arch == std::env::consts::ARCH)
                {
                    if let Some(tool) = Tool::ALL
                        .into_iter()
                        .find(|tool| tool.executable() == spec.name)
                    {
                        session
                            .disabled_providers
                            .insert(tool, provider.manifest.id.clone());
                    }
                }
            }
        }
        session.discovery += 1;
        let revision = session.discovery;
        let scan_sources = session.sources.clone();
        let tx = session.tx.clone();
        if matches!(session.phase, Phase::Choosing) {
            session.phase = Phase::Discovering;
        }
        let managed = self.ext_registry.managed_tools();
        let wake = self.tx.clone();
        self.rt.spawn_blocking(move || {
            let tools = Tools::discover_with(managed);
            let catalog = convert::options(&scan_sources, &tools);
            let _ = tx.send(Event::Discovered {
                revision,
                tools,
                catalog,
            });
            let _ = wake.send(BgResult::ConversionWake);
        });
    }

    pub(crate) fn drain_converter(&mut self) {
        let mut reload = false;
        let Some(session) = self.converter.as_mut() else {
            return;
        };
        while let Ok(event) = session.rx.try_recv() {
            match event {
                Event::Discovered {
                    revision,
                    tools,
                    catalog,
                } if revision == session.discovery && !session.running() => {
                    let selected = session
                        .catalog
                        .get(session.selected)
                        .map(|option| option.target.extension);
                    session.tools = tools;
                    session.catalog = catalog;
                    session.selected = selected
                        .and_then(|extension| {
                            session
                                .catalog
                                .iter()
                                .position(|option| option.target.extension == extension)
                        })
                        .unwrap_or(0);
                    if matches!(session.phase, Phase::Discovering) {
                        session.phase = Phase::Choosing;
                    }
                }
                Event::Discovered { .. } => {}
                Event::PackageProgress(progress) => {
                    if let Some(package) = &mut session.package {
                        package.progress = progress;
                    }
                }
                Event::PackageFinished(result) => {
                    let cancelled = session
                        .package
                        .take()
                        .is_some_and(|package| package.cancel.load(Ordering::Relaxed));
                    match result {
                        Ok(_) => {
                            session.package_error = None;
                            reload = true;
                        }
                        Err(error) if !cancelled => session.package_error = Some(error),
                        Err(_) => {}
                    }
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
        if reload {
            self.reload_ext_registry();
        }
    }

    fn install_conversion_engine(&mut self, tool: Tool) {
        let Some(session) = self
            .converter
            .as_mut()
            .filter(|s| !s.running() && s.package.is_none())
        else {
            return;
        };
        let Some(request) = session.installers.get(&tool).cloned() else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        session.package = Some(PackageJob {
            tool,
            cancel: cancel.clone(),
            progress: tool_package::Progress::Download {
                bytes: 0,
                total: (request.tool.download_bytes != 0).then_some(request.tool.download_bytes),
            },
        });
        session.package_error = None;
        let tx = session.tx.clone();
        let wake = self.tx.clone();
        self.rt.spawn(async move {
            let events = tx.clone();
            let progress_wake = wake.clone();
            let result = tool_package::install(
                request.provider_dir,
                request.tool,
                cancel,
                move |progress| {
                    let _ = events.send(Event::PackageProgress(progress));
                    let _ = progress_wake.send(BgResult::ConversionWake);
                },
            )
            .await
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(Event::PackageFinished(result));
            let _ = wake.send(BgResult::ConversionWake);
        });
    }

    fn activate_selected_conversion_engine(&mut self) {
        let Some(session) = self.converter.as_ref() else {
            return;
        };
        let Some(tool) = session
            .catalog
            .get(session.selected)
            .and_then(|option| option.missing.first())
            .copied()
        else {
            return;
        };
        if let Some(id) = session.disabled_providers.get(&tool).cloned() {
            self.handle_ext_action(crate::app::ext::ExtAction::SetEnabled { id, enabled: true });
        } else {
            self.install_conversion_engine(tool);
        }
    }

    fn cancel_conversion_package(&mut self) {
        if let Some(package) = self.converter.as_ref().and_then(|s| s.package.as_ref()) {
            package.cancel.store(true, Ordering::Relaxed);
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
