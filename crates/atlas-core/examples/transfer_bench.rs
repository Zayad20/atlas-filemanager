use atlas_core::fs::transfer::{execute, TransferPair};
use atlas_model::app_state::BgResult;
use atlas_model::ops::{Resolution, TransferKind, TransferOrigin};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

const LARGE_BYTES: usize = 128 * 1024 * 1024;
const SMALL_FILES: usize = 2_000;
const SMALL_BYTES: usize = 4 * 1024;
const RUNS: usize = 3;

fn create_file(path: &Path, bytes: usize) -> io::Result<()> {
    let mut file = File::create(path)?;
    let block = vec![0xA5; (1024 * 1024).min(bytes)];
    let mut remaining = bytes;
    while remaining > 0 {
        let count = remaining.min(block.len());
        file.write_all(&block[..count])?;
        remaining -= count;
    }
    file.sync_all()
}

async fn copy_once(src: PathBuf, dst: PathBuf, id: u64) -> Duration {
    let (tx, rx) = flume::unbounded();
    let started = Instant::now();
    execute(
        vec![TransferPair {
            src,
            dst,
            on_conflict: Resolution::Rename,
        }],
        TransferKind::Copy,
        TransferOrigin::Menu,
        id,
        tx,
        Arc::new(AtomicBool::new(false)),
        Vec::new(),
    )
    .await;
    while let Ok(message) = rx.recv() {
        if let BgResult::TransferComplete { report, .. } = message {
            assert!(report.errors.is_empty(), "{:?}", report.errors);
            break;
        }
    }
    started.elapsed()
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn windows_default_copy(source: &Path, destination: &Path) -> io::Result<()> {
    if source.is_file() {
        fs::copy(source, destination)?;
        return Ok(());
    }
    fs::create_dir_all(destination)?;
    for entry in walkdir::WalkDir::new(source).min_depth(1) {
        let entry = entry?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(io::Error::other)?;
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    let output = temp.path().join("output");
    fs::create_dir_all(&source)?;
    fs::create_dir_all(&output)?;

    let large = source.join("large.bin");
    create_file(&large, LARGE_BYTES)?;

    let small = source.join("small");
    fs::create_dir(&small)?;
    for index in 0..SMALL_FILES {
        create_file(&small.join(format!("{index:05}.bin")), SMALL_BYTES)?;
    }

    println!("case,runs,median_ms,throughput_mib_s,files_s");
    let mut windows_large_runs = Vec::new();
    let mut atlas_large_runs = Vec::new();
    for run in 0..RUNS {
        let windows_destination = output.join(format!("windows-large-{run}.bin"));
        let atlas_destination = output.join(format!("atlas-large-{run}.bin"));
        if run % 2 == 0 {
            let started = Instant::now();
            windows_default_copy(&large, &windows_destination)?;
            windows_large_runs.push(started.elapsed());
            atlas_large_runs.push(copy_once(large.clone(), atlas_destination, run as u64).await);
        } else {
            atlas_large_runs.push(copy_once(large.clone(), atlas_destination, run as u64).await);
            let started = Instant::now();
            windows_default_copy(&large, &windows_destination)?;
            windows_large_runs.push(started.elapsed());
        }
    }
    let windows_elapsed = median(windows_large_runs);
    let windows_mib_s = LARGE_BYTES as f64 / 1_048_576.0 / windows_elapsed.as_secs_f64();
    println!(
        "windows_default_large_128_mib,{RUNS},{},{windows_mib_s:.2},-",
        windows_elapsed.as_millis()
    );
    let elapsed = median(atlas_large_runs);
    let mib_s = LARGE_BYTES as f64 / 1_048_576.0 / elapsed.as_secs_f64();
    println!(
        "atlas_large_128_mib,{RUNS},{},{mib_s:.2},-",
        elapsed.as_millis()
    );

    let mut windows_small_runs = Vec::new();
    let mut atlas_small_runs = Vec::new();
    for run in 0..RUNS {
        let windows_destination = output.join(format!("windows-small-{run}"));
        let atlas_destination = output.join(format!("atlas-small-{run}"));
        if run % 2 == 0 {
            let started = Instant::now();
            windows_default_copy(&small, &windows_destination)?;
            windows_small_runs.push(started.elapsed());
            atlas_small_runs
                .push(copy_once(small.clone(), atlas_destination, (RUNS + run) as u64).await);
        } else {
            atlas_small_runs
                .push(copy_once(small.clone(), atlas_destination, (RUNS + run) as u64).await);
            let started = Instant::now();
            windows_default_copy(&small, &windows_destination)?;
            windows_small_runs.push(started.elapsed());
        }
    }
    let windows_elapsed = median(windows_small_runs);
    let windows_mib_s =
        (SMALL_FILES * SMALL_BYTES) as f64 / 1_048_576.0 / windows_elapsed.as_secs_f64();
    let windows_files_s = SMALL_FILES as f64 / windows_elapsed.as_secs_f64();
    println!(
        "windows_default_small_2k_x_4k,{RUNS},{},{windows_mib_s:.2},{windows_files_s:.2}",
        windows_elapsed.as_millis()
    );
    let elapsed = median(atlas_small_runs);
    let mib_s = (SMALL_FILES * SMALL_BYTES) as f64 / 1_048_576.0 / elapsed.as_secs_f64();
    let files_s = SMALL_FILES as f64 / elapsed.as_secs_f64();
    println!(
        "atlas_small_2k_x_4k,{RUNS},{},{mib_s:.2},{files_s:.2}",
        elapsed.as_millis()
    );
    Ok(())
}
