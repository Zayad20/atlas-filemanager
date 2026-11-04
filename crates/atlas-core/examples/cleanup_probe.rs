//! Read-only end-to-end scanner probe. Never prepares or executes cleanup.
use atlas_core::cleanup::{ScanJob, ScanMode};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn main() {
    #[cfg(windows)]
    if atlas_win_space::mft_space::helper::try_run() {
        return;
    }
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().expect("root"));
    let output = PathBuf::from(args.next().expect("report.json"));
    let limit = args
        .next()
        .and_then(|v| v.to_str().and_then(|s| s.parse::<u64>().ok()))
        .unwrap_or(60);
    let fast = args.next().is_some_and(|v| v == "fast");
    let start = Instant::now();
    let job = ScanJob::start(
        root.clone(),
        1,
        if fast {
            ScanMode::FastFolder
        } else {
            ScanMode::Folder
        },
    );
    let mut snapshots = 0;
    let mut first = None;
    loop {
        if start.elapsed() > Duration::from_secs(limit) {
            job.cancel();
        }
        let snapshot = match job.receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(snapshot) => snapshot,
            Err(flume::RecvTimeoutError::Timeout) => continue,
            Err(error) => panic!("Scanner stopped: {error}"),
        };
        snapshots += 1;
        if fast && snapshot.message.as_deref() == Some("cleanup_fast_fallback") {
            job.cancel();
        }
        if first.is_none() && snapshot.entry_count > 0 {
            first = Some(start.elapsed().as_secs_f64() * 1000.);
        }
        if snapshot.status.terminal() {
            let result = serde_json::json!({"root":root,"elapsed_ms":start.elapsed().as_secs_f64()*1000.,
                "first_result_ms":first,"files":snapshot.entry_count,"bytes":snapshot.bytes,
                "status":format!("{:?}",snapshot.status),"skipped":snapshot.skipped,"errors":snapshot.errors,
                "snapshots":snapshots,"backend":format!("{:?}",snapshot.backend),"backend_note":snapshot.backend_note});
            std::fs::write(output, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
            break;
        }
    }
}
