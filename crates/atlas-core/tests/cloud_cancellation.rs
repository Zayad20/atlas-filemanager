//! End-to-end host -> sidecar -> rclone cancellation, without user remotes.
#![cfg(feature = "cloud-backend")]
use atlas_core::cloud::{
    control::{TransferCancelled, TransferControl},
    provider::CloudProvider,
    sidecar_provider::SidecarProvider,
};
use std::{path::Path, sync::Arc, time::Duration};

fn size_on_disk(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires rclone and a built target/debug/atlas-cloud-extension"]
async fn acknowledged_cancellation_stops_writes_across_the_process_boundary() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let binary = if cfg!(windows) {
        "atlas-cloud-extension.exe"
    } else {
        "atlas-cloud-extension"
    };
    let compiled = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug")
        .join(binary);
    std::fs::copy(compiled, root.path().join(binary))?;
    std::env::set_var("RCLONE_CONFIG", root.path().join("isolated.conf"));
    std::env::set_var("RCLONE_BWLIMIT", "128K");
    let provider = SidecarProvider::spawn(root.path(), binary, &[])?;
    let destination = root.path().join("remote");
    std::fs::create_dir(&destination)?;
    provider
        .configure(
            "isolated-account",
            "alias",
            serde_json::json!({"remote":destination}),
        )
        .await?;
    let source = root.path().join("large.bin");
    std::fs::File::create(&source)?.set_len(8 * 1024 * 1024)?;
    let control = TransferControl::default();
    let cancel = control.clone();
    let task = async {
        tokio::join!(
            provider.upload_controlled(
                "isolated-account",
                &source,
                "large.bin",
                Arc::new(|_, _| {}),
                control
            ),
            async move {
                tokio::time::sleep(Duration::from_millis(700)).await;
                cancel.cancel();
            }
        )
        .0
    };
    let error = tokio::time::timeout(Duration::from_secs(30), task)
        .await?
        .err();
    // The upload must fail with the structured cancellation acknowledgement.
    anyhow::ensure!(
        error.is_some(),
        "rate-limited upload completed before cancellation"
    );
    anyhow::ensure!(
        error.unwrap().is::<TransferCancelled>(),
        "cancellation was not confirmed by rclone"
    );
    let after_ack = size_on_disk(&destination);
    tokio::time::sleep(Duration::from_secs(1)).await;
    anyhow::ensure!(
        size_on_disk(&destination) == after_ack,
        "rclone continued writing after cancellation acknowledgement"
    );
    anyhow::ensure!(source.exists(), "cancellation removed the local source");
    provider.prepare_shutdown().await?;
    Ok(())
}
