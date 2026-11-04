//! Complete page export, staged on the destination volume and published as one folder.
#[cfg(not(windows))]
use super::Tool;
use super::{Command, Plan};
#[cfg(not(windows))]
use anyhow::Context;
use anyhow::{bail, ensure, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn execute(
    command: &Command,
    plan: &Plan,
    parent: &Path,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    let stage = tempfile::Builder::new()
        .prefix(".atlas-pages-")
        .tempdir_in(parent)?;
    ensure!(!cancel.load(Ordering::Relaxed), "Cancelled");
    render(&command.source, stage.path(), command, plan, cancel)?;
    let count = std::fs::read_dir(stage.path())?.count();
    ensure!(
        count > 0 && count <= 10_000,
        "No pages rendered or page count exceeded"
    );
    ensure!(!cancel.load(Ordering::Relaxed), "Cancelled");
    for index in 0..10_000 {
        let mut name = command
            .source
            .file_stem()
            .unwrap_or_default()
            .to_os_string();
        name.push(format!(" - {}", command.target.label));
        if index > 0 {
            name.push(format!(" ({index})"));
        }
        let destination = parent.join(name);
        match rename_noclobber(stage.path(), &destination) {
            Ok(()) => return Ok(destination),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    bail!("Too many folders with the same output name")
}

#[cfg(windows)]
fn rename_noclobber(source: &Path, target: &Path) -> std::io::Result<()> {
    atlas_win_space::pdf::publish_pages(source, target).map_err(|error| {
        if target.symlink_metadata().is_ok() {
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, error)
        } else {
            error
        }
    })
}

#[cfg(unix)]
fn rename_noclobber(source: &Path, target: &Path) -> std::io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        target,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(Into::into)
}

#[cfg(windows)]
fn render(
    pdf: &Path,
    directory: &Path,
    command: &Command,
    plan: &Plan,
    cancel: &AtomicBool,
) -> Result<()> {
    let pages = atlas_win_space::pdf::render_pages(pdf, directory, plan.settings.max_edge, cancel)
        .map_err(anyhow::Error::msg)?;
    if command.target.extension == "jpg" {
        for page in pages {
            ensure!(!cancel.load(Ordering::Relaxed), "Cancelled");
            let source = page.path;
            let target = source.with_extension("jpg");
            let pixels = image::open(&source)?.to_rgb8();
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                std::fs::File::create(target)?,
                plan.settings.quality.clamp(1, 100),
            );
            encoder.set_pixel_density(image::codecs::jpeg::PixelDensity::dpi(page.dpi));
            encoder.encode_image(&image::DynamicImage::ImageRgb8(pixels))?;
            std::fs::remove_file(source)?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn render(
    pdf: &Path,
    directory: &Path,
    command: &Command,
    plan: &Plan,
    cancel: &AtomicBool,
) -> Result<()> {
    let mut render = std::process::Command::new(
        plan.tools
            .get(Tool::Magick)
            .context("ImageMagick is unavailable")?,
    );
    render
        .args([
            "-limit", "memory", "256MiB", "-limit", "map", "512MiB", "-density", "300",
        ])
        .arg(pdf);
    if let Some(edge) = plan.settings.max_edge {
        render.args(["-resize", &format!("{edge}x{edge}>")]);
    }
    if command.target.extension == "jpg" {
        render.args(["-background", "white", "-alpha", "remove", "-alpha", "off"]);
    }
    render
        .args([
            "-scene",
            "1",
            "-quality",
            &plan.settings.quality.clamp(1, 100).to_string(),
        ])
        .arg(directory.join(format!("page-%04d.{}", command.target.extension)));
    super::process::run(render, cancel)
}
