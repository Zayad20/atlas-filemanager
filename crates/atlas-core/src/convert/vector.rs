//! SVG/SVGZ packaging is our code; rasterization reuses Atlas's resvg dependency.
use super::{native, Command, Settings};
use anyhow::{ensure, Context, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

const MAX_XML: u64 = 64 * 1024 * 1024;

pub(super) fn convert(command: &Command, settings: &Settings, output: &Path) -> Result<()> {
    let mut file = File::open(&command.source)?;
    ensure!(
        file.metadata()?.len() <= MAX_XML,
        "SVG input is limited to 64 MiB"
    );
    let mut signature = [0; 2];
    file.read_exact(&mut signature)?;
    let mut reader: Box<dyn Read> = if signature == [0x1f, 0x8b] {
        Box::new(flate2::read::MultiGzDecoder::new(File::open(
            &command.source,
        )?))
    } else {
        Box::new(File::open(&command.source)?)
    };
    let mut xml = Vec::new();
    reader.by_ref().take(MAX_XML + 1).read_to_end(&mut xml)?;
    ensure!(
        xml.len() as u64 <= MAX_XML,
        "Expanded SVG is limited to 64 MiB"
    );
    let text = std::str::from_utf8(&xml)
        .context("Expected UTF-8 SVG")?
        .trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(text)?;
    ensure!(
        document.root_element().tag_name().name() == "svg",
        "Expected an SVG document"
    );
    match command.target.extension {
        "svg" => {
            std::fs::write(output, &xml)?;
        }
        "svgz" => {
            let mut gzip = flate2::write::GzEncoder::new(
                File::create(output)?,
                flate2::Compression::default(),
            );
            gzip.write_all(&xml)?;
            gzip.finish()?;
        }
        _ => {
            for node in document.descendants().filter(|n| {
                n.is_element() && ["image", "feImage", "use"].contains(&n.tag_name().name())
            }) {
                for attribute in node.attributes().filter(|a| a.name() == "href") {
                    ensure!(
                        attribute.value().starts_with('#')
                            || attribute.value().starts_with("data:"),
                        "SVG references an external resource; embed the resource before converting"
                    );
                }
            }
            let mut options = resvg::usvg::Options::default();
            // SVG content cannot read neighboring/absolute files during export.
            options.image_href_resolver.resolve_string = Box::new(|_, _| None);
            options.fontdb_mut().load_system_fonts();
            let tree = resvg::usvg::Tree::from_str(text, &options)?;
            let size = tree.size();
            let edge = if command.target.extension == "ico" {
                Some(settings.max_edge.unwrap_or(256).min(256))
            } else {
                settings.max_edge
            };
            let scale = edge.map_or(1.0, |e| {
                (e.max(1) as f32 / size.width().max(size.height())).min(1.0)
            });
            let width = (size.width() * scale).ceil().max(1.0) as u32;
            let height = (size.height() * scale).ceil().max(1.0) as u32;
            ensure!(
                u64::from(width) * u64::from(height) <= 128 * 1024 * 1024,
                "SVG rasterization exceeds 512 MiB; choose a smaller image size"
            );
            let mut pixels = resvg::tiny_skia::Pixmap::new(width, height)
                .context("Unable to allocate the SVG raster")?;
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(scale, scale),
                &mut pixels.as_mut(),
            );
            // tiny-skia is premultiplied RGBA; image's encoders need straight RGBA.
            let mut data = pixels.take();
            for pixel in data.chunks_exact_mut(4) {
                let alpha = u32::from(pixel[3]);
                if alpha > 0 && alpha < 255 {
                    for channel in &mut pixel[..3] {
                        *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                    }
                }
            }
            let image = image::RgbaImage::from_raw(width, height, data)
                .context("Invalid SVG raster size")?;
            native::write_image(
                image::DynamicImage::ImageRgba8(image),
                command,
                settings,
                output,
            )?;
        }
    }
    Ok(())
}
