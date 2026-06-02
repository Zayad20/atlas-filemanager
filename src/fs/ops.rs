use crate::state::app_state::BgResult;
use crate::state::pane::TabId;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Copy each `(source, destination)` pair. Destinations are full target paths, so
/// the caller decides overwrite vs. auto-rename semantics; an existing destination
/// is overwritten/merged.
pub async fn copy_files(
    pairs: Vec<(PathBuf, PathBuf)>,
    op_id: u64,
    tx: flume::Sender<BgResult>,
    affected_tabs: Vec<TabId>,
) {
    let total = pairs.len() as u64;
    for (i, (src, dst_file)) in pairs.iter().enumerate() {
        let _ = tx.send(BgResult::OpProgress {
            id: op_id,
            done: i as u64,
            total,
            current_file: src.clone(),
        });
        if let Err(e) = copy_recursive(src, dst_file).await {
            let _ = tx.send(BgResult::OpComplete {
                id: op_id,
                result: Err(e.to_string()),
                affected_tabs,
            });
            return;
        }
    }
    let _ = tx.send(BgResult::OpComplete {
        id: op_id,
        result: Ok(()),
        affected_tabs,
    });
}

/// Move each `(source, destination)` pair. Destinations are full target paths.
pub async fn move_files(
    pairs: Vec<(PathBuf, PathBuf)>,
    op_id: u64,
    tx: flume::Sender<BgResult>,
    affected_tabs: Vec<TabId>,
) {
    let total = pairs.len() as u64;
    for (i, (src, dst_file)) in pairs.iter().enumerate() {
        let _ = tx.send(BgResult::OpProgress {
            id: op_id,
            done: i as u64,
            total,
            current_file: src.clone(),
        });
        // Try atomic rename first, fall back to copy+delete (cross-device, or
        // Windows where renaming onto an existing target fails).
        if tokio::fs::rename(&src, dst_file).await.is_err() {
            if let Err(e) = copy_recursive(src, dst_file).await {
                let _ = tx.send(BgResult::OpComplete {
                    id: op_id,
                    result: Err(e.to_string()),
                    affected_tabs,
                });
                return;
            }
            let _ = remove_path(src).await;
        }
    }
    let _ = tx.send(BgResult::OpComplete {
        id: op_id,
        result: Ok(()),
        affected_tabs,
    });
}

pub async fn delete_files(
    paths: Vec<PathBuf>,
    to_trash: bool,
    op_id: u64,
    tx: flume::Sender<BgResult>,
    affected_tabs: Vec<TabId>,
) {
    for path in &paths {
        let result = if to_trash {
            let p = path.clone();
            tokio::task::spawn_blocking(move || trash::delete(&p).map_err(|e| e.to_string()))
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
        } else {
            remove_path(path).await.map_err(|e| e.to_string())
        };
        if let Err(e) = result {
            let _ = tx.send(BgResult::OpComplete {
                id: op_id,
                result: Err(e),
                affected_tabs,
            });
            return;
        }
    }
    let _ = tx.send(BgResult::OpComplete {
        id: op_id,
        result: Ok(()),
        affected_tabs,
    });
}

pub async fn rename_file(
    from: PathBuf,
    to: PathBuf,
    op_id: u64,
    tx: flume::Sender<BgResult>,
    affected_tabs: Vec<TabId>,
) {
    let result = tokio::fs::rename(&from, &to)
        .await
        .map_err(|e| e.to_string());
    let _ = tx.send(BgResult::OpComplete {
        id: op_id,
        result,
        affected_tabs,
    });
}

pub async fn create_dir(
    path: PathBuf,
    op_id: u64,
    tx: flume::Sender<BgResult>,
    affected_tabs: Vec<TabId>,
) {
    let result = tokio::fs::create_dir_all(&path)
        .await
        .map_err(|e| e.to_string());
    let _ = tx.send(BgResult::OpComplete {
        id: op_id,
        result,
        affected_tabs,
    });
}

async fn copy_recursive(src: &Path, dst: &Path) -> Result<()> {
    if src.is_dir() {
        tokio::fs::create_dir_all(dst).await?;
        let mut rd = tokio::fs::read_dir(src).await?;
        while let Ok(Some(entry)) = rd.next_entry().await {
            let child_dst = dst.join(entry.file_name());
            Box::pin(copy_recursive(&entry.path(), &child_dst)).await?;
        }
    } else {
        tokio::fs::copy(src, dst).await?;
    }
    Ok(())
}

async fn remove_path(path: &Path) -> Result<()> {
    if path.is_dir() {
        tokio::fs::remove_dir_all(path).await?;
    } else {
        tokio::fs::remove_file(path).await?;
    }
    Ok(())
}
