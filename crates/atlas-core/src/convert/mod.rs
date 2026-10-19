//! Local, non-destructive conversion pipeline. Catalog -> immutable plan -> workers.
//! Engines are discovered off-thread; no engine or codec is bundled by this module.
mod books;
mod catalog;
mod contacts;
mod native;
mod pcm;
mod process;
mod subtitles;
mod tools;
mod vector;

pub use catalog::{options, Backend, ConversionOption, Format, FORMATS};
pub use tools::{Tool, Tools};

use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

#[derive(Clone, Debug)]
pub struct Settings {
    pub quality: u8,
    /// Maximum image edge; None preserves the original dimensions.
    pub max_edge: Option<u32>,
    /// None saves beside each source.
    pub output_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: 90,
            max_edge: None,
            output_dir: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Command {
    pub source: PathBuf,
    pub target: &'static Format,
    pub backend: Backend,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub commands: Vec<Command>,
    pub settings: Settings,
    pub tools: Tools,
}

impl Plan {
    pub fn build(
        paths: &[PathBuf],
        extension: &str,
        settings: Settings,
        tools: Tools,
    ) -> Result<Self> {
        let target = FORMATS
            .iter()
            .find(|f| f.extension == extension)
            .context("Unknown output format")?;
        if paths.is_empty() {
            bail!("Select at least one file");
        }
        let mut commands = Vec::with_capacity(paths.len());
        for source in paths {
            if !source.is_file() {
                bail!("Not a local file: {}", source.display());
            }
            if catalog::extension(source) == target.extension {
                continue;
            }
            let backend =
                catalog::route(source, target, &tools).context("Incompatible conversion")?;
            if !backend.available(&tools) {
                bail!("Install {} to convert this file", backend.name());
            }
            commands.push(Command {
                source: std::path::absolute(source)?,
                target,
                backend,
            });
        }
        if commands.is_empty() {
            bail!("These files already use the selected format");
        }
        if settings.output_dir.as_ref().is_some_and(|p| !p.is_dir()) {
            bail!("Output folder does not exist");
        }
        Ok(Self {
            commands,
            settings,
            tools,
        })
    }
}

#[derive(Clone, Debug)]
pub enum Outcome {
    Created { source: PathBuf, output: PathBuf },
    Failed { source: PathBuf, error: String },
    Cancelled { source: PathBuf },
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub outcomes: Vec<Outcome>,
}

impl Report {
    pub fn outputs(&self) -> Vec<PathBuf> {
        self.outcomes
            .iter()
            .filter_map(|r| {
                if let Outcome::Created { output, .. } = r {
                    Some(output.clone())
                } else {
                    None
                }
            })
            .collect()
    }
    pub fn errors(&self) -> Vec<String> {
        self.outcomes
            .iter()
            .filter_map(|r| {
                if let Outcome::Failed { source, error } = r {
                    Some(format!("{}: {error}", source.display()))
                } else {
                    None
                }
            })
            .collect()
    }
    pub fn cancelled(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|r| matches!(r, Outcome::Cancelled { .. }))
            .count()
    }
}

/// Two isolated lanes cap simultaneous decoders/processes and peak memory. Results
/// retain input order; only the monotonic completion counter is shared by workers.
pub fn execute(
    plan: &Plan,
    cancel: &Arc<AtomicBool>,
    progress: impl Fn(usize, usize, &Path) + Sync,
) -> Report {
    let done = AtomicUsize::new(0);
    let total = plan.commands.len();
    let run = |command: &Command| {
        let outcome = if cancel.load(Ordering::Relaxed) {
            Outcome::Cancelled {
                source: command.source.clone(),
            }
        } else {
            match execute_one(command, plan, cancel) {
                Ok(output) => Outcome::Created {
                    source: command.source.clone(),
                    output,
                },
                Err(_) if cancel.load(Ordering::Relaxed) => Outcome::Cancelled {
                    source: command.source.clone(),
                },
                Err(error) => Outcome::Failed {
                    source: command.source.clone(),
                    error: format!("{error:#}"),
                },
            }
        };
        progress(
            done.fetch_add(1, Ordering::Relaxed) + 1,
            total,
            &command.source,
        );
        outcome
    };
    let outcomes = match rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .thread_name(|n| format!("atlas-convert-{n}"))
        .build()
    {
        Ok(pool) => pool.install(|| plan.commands.par_iter().map(run).collect()),
        Err(_) => plan.commands.iter().map(run).collect(),
    };
    Report { outcomes }
}

fn execute_one(command: &Command, plan: &Plan, cancel: &AtomicBool) -> Result<PathBuf> {
    let parent = plan
        .settings
        .output_dir
        .as_deref()
        .or_else(|| command.source.parent())
        .context("Missing output directory")?;
    // Keep a path guard rather than an open file handle: system codecs can
    // require exclusive access to their destination. The guard still removes
    // every failed/cancelled temporary and publishes with noclobber atomically.
    let mut output = tempfile::Builder::new()
        .prefix(".atlas-convert-")
        .suffix(&format!(".{}", command.target.extension))
        .tempfile_in(parent)?
        .into_temp_path();
    match command.backend {
        Backend::Image => native::image(command, &plan.settings, &output)?,
        Backend::Data => native::data(command, &output)?,
        Backend::Sheet => native::sheet(command, &output)?,
        Backend::Markdown => native::markdown(command, &output)?,
        Backend::Text => native::text(command, &output)?,
        Backend::Archive => native::archive(command, &output, cancel)?,
        Backend::Subtitles => subtitles::convert(command, &output)?,
        Backend::Pcm => pcm::convert(command, &output, cancel)?,
        Backend::Vector => vector::convert(command, &plan.settings, &output)?,
        Backend::Contacts => contacts::convert(command, &output)?,
        Backend::Book => books::convert(command, &output)?,
        Backend::SystemAudio => {
            #[cfg(target_os = "windows")]
            {
                use atlas_win_space::media::{convert_audio, AudioTarget};
                let target = match command.target.extension {
                    "wav" => AudioTarget::Wave,
                    "mp3" => AudioTarget::Mp3,
                    "aac" => AudioTarget::Aac,
                    "m4a" => AudioTarget::M4a,
                    _ => bail!("Unsupported Windows audio target"),
                };
                let bridge;
                let source = if ["aiff", "aifc", "au"]
                    .contains(&catalog::extension(&command.source).as_str())
                {
                    bridge = tempfile::Builder::new()
                        .prefix(".atlas-pcm-")
                        .suffix(".wav")
                        .tempfile_in(parent)?
                        .into_temp_path();
                    let pcm = Command {
                        source: command.source.clone(),
                        target: FORMATS
                            .iter()
                            .find(|f| f.extension == "wav")
                            .context("Missing WAV format")?,
                        backend: Backend::Pcm,
                    };
                    pcm::convert(&pcm, &bridge, cancel)?;
                    bridge.as_ref()
                } else {
                    command.source.as_path()
                };
                convert_audio(source, &output, target, plan.settings.quality, cancel)
                    .map_err(anyhow::Error::msg)?;
            }
            #[cfg(not(target_os = "windows"))]
            bail!("Windows audio conversion is unavailable on this platform");
        }
        Backend::External(tool) => process::convert(tool, command, plan, &output, cancel)?,
    }
    if cancel.load(Ordering::Relaxed) {
        bail!("Cancelled");
    }
    if std::fs::metadata(&output)?.len() == 0 {
        bail!("The engine produced an empty file");
    }
    std::fs::File::options()
        .read(true)
        .write(true)
        .open(&output)?
        .sync_all()?;
    // persist_noclobber atomically claims a destination, including races with
    // other workers/applications. Never truncate an existing file or a symlink.
    for index in 0..10_000 {
        let destination = output_name(&command.source, parent, command.target.extension, index);
        match output.persist_noclobber(&destination) {
            Ok(()) => return Ok(destination),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                output = error.path
            }
            Err(error) => return Err(error.error.into()),
        }
    }
    bail!("Too many files with the same output name")
}

fn output_name(source: &Path, parent: &Path, extension: &str, index: usize) -> PathBuf {
    let mut stem = source.file_stem().unwrap_or_default().to_os_string();
    if catalog::extension(source).starts_with("tar.") {
        stem = Path::new(&stem)
            .file_stem()
            .unwrap_or_default()
            .to_os_string();
    }
    if index > 0 {
        stem.push(format!(" ({index})"));
    }
    stem.push(format!(".{extension}"));
    parent.join(stem)
}

#[cfg(test)]
mod tests;
