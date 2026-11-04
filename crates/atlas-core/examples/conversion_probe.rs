//! Reproducible conversion audit. This example is never linked into Atlas.
use anyhow::{bail, Context, Result};
use atlas_core::convert::{execute, native_options, Plan, Settings, Tool, Tools};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Arc};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let mut tools = Tools::discover();
    // Isolate a validation engine without changing the user's installation.
    if let Some(directory) = std::env::var_os("ATLAS_PROBE_TOOLS") {
        tools = Tools::discover_with(Tool::ALL.into_iter().map(|tool| {
            (
                tool.executable().to_owned(),
                PathBuf::from(&directory).join(format!(
                    "{}{}",
                    tool.executable(),
                    std::env::consts::EXE_SUFFIX
                )),
            )
        }));
    }
    if args.first().is_some_and(|arg| arg == "--matrix") {
        let inputs = "png jpg webp bmp tiff gif ico avif heic heif jxl svg svgz psd psb tga dds ppm pgm pbm exr hdr cr2 cr3 nef arw dng orf raf rw2 xcf jp2 j2k pcx mp3 wav flac m4a aac ogg opus aiff aifc au wma amr ape ac3 alac wv mp4 webm mkv mov avi wmv flv mpeg mpg m4v ts m2ts 3gp ogv vob doc docx docm dotx odt rtf txt html md epub rst tex org fb2 pdf xlsx xlsm xls xlsb ods csv tsv ppt pptx pptm ppsx odp json jsonl toml vcf srt vtt ass ssa smi lrc ttml zip 7z rar tar tar.gz tar.bz2 tar.xz gz bz2 xz cab iso mobi azw azw3 pages rtfd textClipping";
        let rows: Vec<_> = inputs
            .split_whitespace()
            .map(|input| {
                let path = PathBuf::from(format!("fixture.{input}"));
                let routes: Vec<_> = native_options(&[path], &tools).iter().map(|option| json!({
                "target": option.target.extension,
                "engines": option.engines,
                "available": option.available(),
                "missing": option.missing.iter().map(|tool| tool.name()).collect::<Vec<_>>(),
                "validation": "catalog_only",
            })).collect();
                json!({"input": input, "routes": routes})
            })
            .collect();
        let engines: Vec<_> = Tool::ALL.into_iter().map(|tool| json!({
            "engine": tool.name(), "path": tools.get(tool), "enabled_for_conversion": tool == Tool::Opus,
        })).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"engines": engines, "inputs": rows}))?
        );
        return Ok(());
    }
    if args.len() != 3 {
        bail!("Usage: conversion_probe --matrix | INPUT TARGET OUTPUT_DIRECTORY");
    }
    let source = PathBuf::from(&args[0]);
    let original = std::fs::read(&source)?;
    let plan = Plan::build_native(
        std::slice::from_ref(&source),
        args[1].to_str().context("Expected a target extension")?,
        Settings {
            output_dir: Some(PathBuf::from(&args[2])),
            ..Settings::default()
        },
        tools,
    )?;
    let report = execute(&plan, &Arc::new(AtomicBool::new(false)), |_, _, _| {});
    let unchanged = std::fs::read(&source)? == original;
    let result = json!({
        "source": source,
        "target": plan.commands[0].target.extension,
        "engine": plan.commands[0].backend.name(),
        "outputs": report.outputs(),
        "errors": report.errors(),
        "original_unchanged": unchanged,
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    anyhow::ensure!(unchanged, "Conversion changed the original");
    anyhow::ensure!(report.errors().is_empty(), "Conversion failed");
    Ok(())
}
